use keepmyconfig_core::assets::{self, BackupOptions, RestoreOptions};
use keepmyconfig_core::ccswitch::{self, AdoptOptions};
use keepmyconfig_core::paths::Paths;
use keepmyconfig_core::store::{ProcessOutcome, Store};
use rusqlite::{params, Connection};
use std::fs;
use tempfile::TempDir;

const FULL_CONFIG: &str = r#"model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"
disable_response_storage = true
notify = [ "C:\\Tools\\codex-computer-use.exe", "turn-ended" ]

[model_providers.SailsAPI]
name = "SailsAPI"
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"
requires_openai_auth = true

[mcp_servers.node_repl]
type = "stdio"
command = "node_repl.exe"
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
SKY_CUA_NATIVE_PIPE_DIRECTORY = '\\\\.\\pipe\\old-pipe'

[mcp_servers.prima-mock-api]
command = "pma.exe"
args = ["serve", "--mcp", "--listen", "127.0.0.1:8787"]

[mcp_servers.prima-mock-api.env]
PMA_MODELS = "examples/models.json"

[marketplaces.openai-bundled]
source_type = "local"
source = 'C:\\.codex\\.tmp\\bundled-marketplaces\\openai-bundled'

[plugins."pdf@openai-primary-runtime"]
enabled = true

[plugins."browser@openai-bundled"]
enabled = true

[desktop]
followUpQueueMode = "queue"

[windows]
sandbox = "elevated"

[projects.'c:\work\demo']
trust_level = "trusted"
"#;

const CLOBBERED_CONFIG: &str = r#"model = "gpt-5.6-sol"
model_provider = "SailsAPI"
model_reasoning_effort = "high"
disable_response_storage = true

model_context_window = 1000000
model_auto_compact_token_limit = 900000

[model_providers.custom]
name = "NewAPI"
base_url = "http://localhost:3000/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.SailsAPI]
base_url = "http://localhost:3000/v1"
wire_api = "responses"
requires_openai_auth = true

[mcp_servers.node_repl]
type = "stdio"
command = "newer-node_repl.exe"
startup_timeout_sec = 120
"#;

fn setup(config: &str) -> (TempDir, Paths, Store) {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let store_dir = temp.path().join("store");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("config.toml"), config).unwrap();
    let paths = Paths::from_roots(codex_home, store_dir);
    let store = Store::open(paths.clone()).unwrap();
    // GitHub @Oxy  genAILab | OxygenAIL  ab@StarsailsC   lover
    (temp, paths, store)
}

#[test]
fn watch_repairs_a_cc_switch_style_clobber() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();
    fs::write(&paths.config_file, CLOBBERED_CONFIG).unwrap();

    let outcome = store.process(false).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Repaired(_)),
        "expected repair, got {outcome:?}"
    );

    let repaired = fs::read_to_string(&paths.config_file).unwrap();
    // Provider identity comes from the live file.
    assert!(repaired.contains("model = \"gpt-5.6-sol\""));
    // User-owned settings and registrations are restored.
    assert!(repaired.contains("model_reasoning_effort = \"max\""));
    assert!(repaired.contains("pma.exe"));
    assert!(repaired.contains("pdf@openai-primary-runtime"));
    assert!(repaired.contains("browser@openai-bundled"));
    assert!(repaired.contains("followUpQueueMode"));
    assert!(repaired.contains("sandbox = \"elevated\""));
    // App-owned churn: `notify` embeds a version-scoped runtime path, so it is left to
    // the desktop app instead of being restored from a possibly stale overlay copy.
    assert!(!repaired.contains("turn-ended"));
    // App-owned churn (node_repl) is not restored over the newer live copy.
    assert!(repaired.contains("newer-node_repl.exe"));

    let backups: Vec<_> = fs::read_dir(&paths.backups_dir)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("pre-repair"))
        .collect();
    assert_eq!(backups.len(), 1, "expected exactly one pre-repair backup");
}

#[test]
fn manual_repair_keeps_an_app_updated_notify() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    // Simulate the Codex desktop app updating itself: it rewrites `notify` so the
    // path points into the newly installed runtime directory.
    let updated = FULL_CONFIG.replace(
        "C:\\\\Tools\\\\codex-computer-use.exe",
        "C:\\\\Tools\\\\new-runtime\\\\codex-computer-use.exe",
    );
    assert_ne!(updated, FULL_CONFIG, "fixture must actually change notify");
    fs::write(&paths.config_file, &updated).unwrap();

    // `repair` is the explicit command, so it merges without requiring a clobber
    // fingerprint. That is exactly why an app-owned key must not be in the overlay:
    // before this was fixed, the repair below restored the dead runtime path.
    store
        .repair(&keepmyconfig_core::RepairOptions {
            manual: true,
            ..Default::default()
        })
        .unwrap();

    let after = fs::read_to_string(&paths.config_file).unwrap();
    assert!(
        after.contains("new-runtime"),
        "manual repair must keep the app-updated notify, got:\n{after}"
    );
    assert!(
        !after.contains(r"C:\\Tools\\codex-computer-use.exe"),
        "the stale notify path must not be restored, got:\n{after}"
    );
    // User-owned registrations are unaffected by this rule.
    assert!(after.contains("pma.exe"));
}

#[test]
fn user_edits_are_captured_not_reverted() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    let mut edited = FULL_CONFIG.to_string();
    edited.push_str("\n[mcp_servers.added-by-user]\ncommand = \"added.exe\"\n");
    fs::write(&paths.config_file, &edited).unwrap();

    let outcome = store.process(false).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Captured(_)),
        "expected capture, got {outcome:?}"
    );
    let overlay = store.read_overlay_text().unwrap();
    assert!(overlay.contains("added-by-user"));
    assert!(overlay.contains("added.exe"));

    // A later clobber must not lose the newly captured server.
    fs::write(&paths.config_file, CLOBBERED_CONFIG).unwrap();
    store.process(false).unwrap();
    let repaired = fs::read_to_string(&paths.config_file).unwrap();
    assert!(repaired.contains("added-by-user"));
}

#[test]
fn capture_from_backup_recovers_lost_protected_keys() {
    let (_temp, paths, store) = setup(CLOBBERED_CONFIG);
    store.init(None, false).unwrap();
    let backup = paths.codex_home.join("config.toml.bak-20261002-135358");
    fs::write(&backup, FULL_CONFIG).unwrap();

    let report = store
        .capture(Some(&backup), Some("recover".to_string()))
        .unwrap();
    assert_eq!(report.recovered_from.as_deref(), Some(backup.as_path()));

    // Recovery is only complete once the merged baseline is written back.
    let repair = store
        .repair(&keepmyconfig_core::RepairOptions {
            manual: true,
            ..Default::default()
        })
        .unwrap();
    assert!(repair.performed);
    // Gi  tH  ub@O  xygenAILab | O  xygen  A ILab@   StarsailsClover
    let repaired = fs::read_to_string(&paths.config_file).unwrap();
    assert!(repaired.contains("prima-mock-api"));
    assert!(repaired.contains("pdf@openai-primary-runtime"));
    assert!(repaired.contains("gpt-5.6-sol"));
}

#[test]
fn asset_backup_and_restore_round_trip() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    fs::create_dir_all(paths.codex_home.join("skills").join("demo")).unwrap();
    fs::write(
        paths
            .codex_home
            .join("skills")
            .join("demo")
            .join("SKILL.md"),
        "# demo skill\n",
    )
    .unwrap();
    fs::write(
        paths.codex_home.join("auth.json"),
        "{\"OPENAI_API_KEY\":\"secret\"}",
    )
    .unwrap();

    let backup = assets::backup_assets(
        &store,
        // GitHub@OxygenAILab | Oxyge  nAILab@Starsail sClo ver
        &BackupOptions {
            classes: vec!["all".to_string()],
            link: false,
            include_cache: false,
        },
    )
    .unwrap();
    assert!(backup.files >= 3);
    let state = keepmyconfig_core::state::State::load_or_default(&paths.state_file).unwrap();
    assert!(state.last_asset_backup_at.is_some());
    assert_eq!(state.last_asset_backup_files, backup.files);
    assert_eq!(state.last_asset_backup_bytes, backup.bytes);
    assert!(backup.backup_dir.join("config.toml").is_file());
    assert!(backup.backup_dir.join("auth.json").is_file());

    fs::remove_file(
        paths
            .codex_home
            .join("skills")
            .join("demo")
            .join("SKILL.md"),
    )
    .unwrap();
    fs::write(
        paths.codex_home.join("auth.json"),
        "{\"OPENAI_API_KEY\":\"rotated\"}",
    )
    .unwrap();

    let restore = assets::restore_assets(
        &store,
        &RestoreOptions {
            from: Some(backup.backup_dir.clone()),
            overwrite: false,
            dry_run: false,
            include_backup_only: false,
        },
    )
    .unwrap();
    assert_eq!(restore.restored_files, 1);
    assert_eq!(restore.skipped_backup_only, 1);
    assert!(paths
        .codex_home
        .join("skills")
        .join("demo")
        .join("SKILL.md")
        .is_file());
    let auth = fs::read_to_string(paths.codex_home.join("auth.json")).unwrap();
    assert!(
        auth.contains("rotated"),
        "auth.json must not be restored by default"
    );
}

#[test]
fn restore_skips_manifest_entries_that_escape_the_codex_home() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    let backup = assets::backup_assets(
        &store,
        &BackupOptions {
            classes: vec!["config".to_string()],
            link: false,
            include_cache: false,
        },
    )
    .unwrap();

    let manifest_path = backup.backup_dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["entries"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "path": "../escape.txt", "size": 1 }));
    fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    let escape_target = paths.codex_home.parent().unwrap().join("escape.txt");
    fs::remove_file(&escape_target).ok();

    let restore = assets::restore_assets(
        &store,
        &RestoreOptions {
            from: Some(backup.backup_dir.clone()),
            overwrite: false,
            dry_run: false,
            include_backup_only: false,
        },
    )
    .unwrap();
    assert_eq!(restore.skipped_unsafe, 1);
    assert!(!escape_target.exists(), "path traversal must be refused");
}

#[test]
fn codex_update_rewrite_is_repaired_not_captured() {
    let baseline = r#"model = "deepseek-v4.1-flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"

[mcp_servers.node_repl]
command = "node_repl.exe"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31428"
CODEX_CLI_PATH = 'C:\\.codex\\bin\\old\\codex.exe'

[mcp_servers.cu_bridge]
command = "node.exe"
args = ["server.mjs"]

[mcp_servers.prima-mock-api]
command = "pma.exe"
args = ["serve", "--mcp"]

[mcp_servers.wsl-cu]
command = "node"
args = ["server.mjs"]

[plugins."figma@openai-api-curated"]
enabled = true

[plugins."linear@openai-api-curated"]
enabled = true

[desktop]
followUpQueueMode = "queue"
"#;
    let upgraded = r#"model = "deepseek-v4.1-flash"
model_provider = "SailsAPI"
model_reasoning_effort = "high"

[mcp_servers.node_repl]
command = "node_repl.exe"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31730"
CODEX_CLI_PATH = 'C:\\.codex\\bin\\new\\codex.exe'

[desktop]
followUpQueueMode = "queue"
conversationDetailMode = "STEPS_COMMANDS"
"#;
    let (_temp, paths, store) = setup(baseline);
    store.init(None, false).unwrap();
    fs::write(&paths.config_file, upgraded).unwrap();

    let outcome = store.process(false).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Repaired(_)),
        "a Codex update rewrite must be repaired, not captured: {outcome:?}"
    );

    let repaired = fs::read_to_string(&paths.config_file).unwrap();
    assert!(repaired.contains("cu_bridge"));
    assert!(repaired.contains("pma.exe"));
    assert!(repaired.contains("wsl-cu"));
    assert!(repaired.contains("figma@openai-api-curated"));
    assert!(repaired.contains("model_reasoning_effort = \"max\""));
    // The new app build keeps its own updated runtime paths and new keys.
    assert!(repaired.contains("26.930.31730"));
    assert!(repaired.contains("conversationDetailMode"));
}

#[test]
fn recover_from_ccswitch_provider_config() {
    let (_temp, paths, store) = setup("model = \"deepseek-v4.1-flash\"\n");
    store.init(None, false).unwrap();

    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("cc-switch.db");
    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            r#"
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE providers (
    id TEXT NOT NULL,
    app_type TEXT NOT NULL,
    name TEXT NOT NULL,
    settings_config TEXT NOT NULL,
    meta TEXT NOT NULL DEFAULT '{}',
    is_current BOOLEAN NOT NULL DEFAULT 0,
    sort_index INTEGER,
    PRIMARY KEY (id, app_type)
);
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,
    server_config TEXT,
    enabled_codex BOOLEAN NOT NULL DEFAULT 0
);
"#,
        )
        .unwrap();
    let rich_config = r#"model = "deepseek-v4.1-flash"

[mcp_servers.prima-mock-api]
command = "pma.exe"
args = ["serve", "--mcp"]

[plugins."pdf@openai-primary-runtime"]
enabled = true

[desktop]
followUpQueueMode = "queue"
"#;
    let rich_settings = serde_json::json!({ "config": rich_config }).to_string();
    let poor_settings =
        serde_json::json!({ "config": "model = \"deepseek-v4.1-flash\"\n" }).to_string();
    connection
        .execute(
            "INSERT INTO providers (id, app_type, name, settings_config, is_current, sort_index)
             VALUES ('provider-poor', 'codex', 'Poor', ?1, 1, 0)",
            params![poor_settings],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO providers (id, app_type, name, settings_config, is_current, sort_index)
             VALUES ('provider-rich', 'codex', 'Rich', ?1, 0, 1)",
            params![rich_settings],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO mcp_servers (id, server_config, enabled_codex)
             VALUES ('cu_bridge', ?1, 1)",
            params![serde_json::json!({
                "command": "node.exe",
                "args": ["server.mjs"],
            })
            .to_string()],
        )
        .unwrap();
    drop(connection);

    let preview = ccswitch::recover(&store, Some(db_path.clone()), None, false).unwrap();
    let selected = preview.selected.as_ref().unwrap();
    assert_eq!(selected.provider_id, "provider-rich");
    assert!(selected.protected_paths >= 4);
    assert!(preview
        .sources
        .iter()
        .any(|source| source == "CC Switch MCP registry"));
    let dry_run = preview.recover.as_ref().unwrap();
    assert!(dry_run.dry_run);
    assert!(!dry_run.actions.is_empty());
    let live_before = fs::read_to_string(&paths.config_file).unwrap();
    assert!(!live_before.contains("pma.exe"), "dry run must not write");

    let applied = ccswitch::recover(&store, Some(db_path), Some("Rich"), true).unwrap();
    let report = applied.recover.as_ref().unwrap();
    assert!(report.performed);
    assert!(report
        .actions
        .iter()
        .any(|action| action.path.contains("prima-mock-api")));
    let live = fs::read_to_string(&paths.config_file).unwrap();
    assert!(live.contains("pma.exe"));
    assert!(live.contains("cu_bridge"));
    assert!(live.contains("pdf@openai-primary-runtime"));
    assert!(live.contains("followUpQueueMode"));
    assert!(live.contains("deepseek-v4.1-flash"));
}

#[test]
// GitHu   b@O  xygenAIL ab | O  xygen AILab@StarsailsC  lover
fn cc_switch_adopt_publishes_common_config_providers_and_mcp() {
    let (_temp, _paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("cc-switch.db");
    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            r#"
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE providers (
    id TEXT NOT NULL,
    app_type TEXT NOT NULL,
    name TEXT NOT NULL,
    settings_config TEXT NOT NULL,
    meta TEXT NOT NULL DEFAULT '{}',
    sort_index INTEGER,
    PRIMARY KEY (id, app_type)
);
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    server_config TEXT NOT NULL,
    enabled_codex BOOLEAN NOT NULL DEFAULT 0
);
"#,
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES ('common_config_codex', ?1)",
            params!["notify = [ \"C:\\\\existing.exe\", \"turn-ended\" ]\n\n[features]\njs_repl = false\n"],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta) VALUES (?1, 'codex', ?2, '{}', ?3)",
            params!["provider-a", "Sails API", "{\"commonConfigEnabled\":false,\"usage_script\":{\"enabled\":true}}"],
        )
        .unwrap();
    // GitHub@Oxygen   AIL ab |   OxygenAI   Lab@Sta  r  sailsCl   over
    drop(connection);

    let preview = ccswitch::adopt(
        &store,
        &AdoptOptions {
            db_path: Some(db_path.clone()),
            apply: false,
            force: false,
            include_mcp: true,
        },
    )
    .unwrap();
    assert!(preview.dry_run);
    assert!(preview.backup.is_none());
    assert!(preview.mcp_upserted.contains(&"prima-mock-api".to_string()));

    let connection = Connection::open(&db_path).unwrap();
    let meta: String = connection
        .query_row(
            "SELECT meta FROM providers WHERE id = 'provider-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(meta.contains("false"));
    drop(connection);

    let report = ccswitch::adopt(
        &store,
        &AdoptOptions {
            db_path: Some(db_path.clone()),
            apply: true,
            force: true,
            include_mcp: true,
        },
    )
    .unwrap();
    assert!(!report.dry_run);
    assert!(report.backup.is_some());

    let connection = Connection::open(&db_path).unwrap();
    let common: String = connection
        .query_row(
            "SELECT value FROM settings WHERE key = 'common_config_codex'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Unrelated keys already present in CC Switch's snippet survive the merge.
    assert!(
        common.contains("js_repl = false"),
        "existing snippet preserved"
    );
    // The overlay is the user's latest known-good state and wins on conflicts.
    // GitHub@O   xygenAILab | OxygenAIL   ab@Starsa  ilsClover
    assert!(common.contains("pdf@openai-primary-runtime"));
    assert!(common.contains("followUpQueueMode"));
    // `notify` is app-owned churn and is deliberately excluded from the overlay: it
    // embeds a version-scoped runtime path, so publishing it here would inject a path
    // that the next Codex update turns stale into every provider switch.
    assert!(!common.contains("codex-computer-use.exe"));
    // CC Switch's own stored snippet is therefore left untouched.
    assert!(common.contains("existing.exe"));
    assert!(
        !common.contains("mcp_servers"),
        "MCP servers belong in the dedicated table"
    );

    let meta: String = connection
        .query_row(
            "SELECT meta FROM providers WHERE id = 'provider-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let meta_json: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(
        meta_json["commonConfigEnabled"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        meta_json["usage_script"]["enabled"],
        serde_json::Value::Bool(true)
    );

    let (server_config, enabled): (String, i64) = connection
        .query_row(
            "SELECT server_config, enabled_codex FROM mcp_servers WHERE id = 'prima-mock-api'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(enabled, 1);
    assert!(server_config.contains("pma.exe"));
}

#[test]
fn status_reports_clobber_then_health() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    let healthy = store.status().unwrap();
    assert!(!healthy
        .classification
        .as_ref()
        .is_some_and(|classification| {
            classification.kind == keepmyconfig_core::classify::ChangeKind::Clobber
        }));

    fs::write(&paths.config_file, CLOBBERED_CONFIG).unwrap();
    let damaged = store.status().unwrap();
    assert_eq!(
        damaged.classification.as_ref().map(|c| c.kind),
        Some(keepmyconfig_core::classify::ChangeKind::Clobber)
    );
    // GitH ub@OxygenAILab | Oxy  genAILab@Star  s  ailsClov e r
    assert!(damaged.diff.as_ref().unwrap().has_protected_drift());
    assert!(
        damaged
            .suggestions
            .join("\n")
            .contains("run `keepmyconfig repair`"),
        "a clobber must still be guided to repair: {:?}",
        damaged.suggestions
    );
}

#[test]
fn status_guides_capture_for_a_benign_edit() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    // A protected value changed, but nothing was removed and no provider identity moved,
    // so this is an Edit — the user's or the Codex app's own write.
    let edited = FULL_CONFIG.replace(
        "model_reasoning_effort = \"max\"",
        "model_reasoning_effort = \"low\"",
    );
    assert_ne!(
        edited, FULL_CONFIG,
        "fixture must actually change a protected value"
    );
    fs::write(&paths.config_file, &edited).unwrap();

    let report = store.status().unwrap();
    assert_eq!(
        report.classification.as_ref().map(|c| c.kind),
        Some(keepmyconfig_core::classify::ChangeKind::Edit)
    );
    assert!(report.diff.as_ref().unwrap().has_protected_drift());

    let guidance = report.suggestions.join("\n");
    // `repair` merges unconditionally, so a benign edit must be pointed at `capture` first.
    assert!(
        guidance.contains("`keepmyconfig capture`"),
        "a benign edit must be guided to capture: {guidance}"
    );
    assert!(
        !guidance.contains("run `keepmyconfig repair` to restore it"),
        "the old unconditional-repair hint must not come back: {guidance}"
    );
}

#[test]
fn repair_dry_run_does_not_write() {
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();
    fs::write(&paths.config_file, CLOBBERED_CONFIG).unwrap();

    let report = store
        .repair(&keepmyconfig_core::RepairOptions {
            dry_run: true,
            ..Default::default()
        })
        .unwrap();
    assert!(!report.performed);
    assert!(report.dry_run);
    assert!(!report.actions.is_empty());
    let live = fs::read_to_string(&paths.config_file).unwrap();
    assert_eq!(live, CLOBBERED_CONFIG);
}

#[test]
fn store_lock_blocks_second_writer() {
    let (_temp, _paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();
    let lock = store.lock().unwrap();
    let second = store.lock();
    assert!(second.is_err());
    drop(lock);
    assert!(store.lock().is_ok());
}

#[test]
fn watch_repairs_a_pinned_provider_name_rewrite() {
    // The remote-compaction recurrence: nothing is removed, but a pinned
    // provider `name` is swapped, which flips Codex into remote compaction v2.
    let (_temp, paths, store) = setup(FULL_CONFIG);
    store.init(None, false).unwrap();

    // Pin the field that decides compaction routing, as the real store does.
    // Load and re-save structurally: the serialized default policy already
    // carries a multi-line `pinned`, so appending raw text at the end of the
    // file would land it inside the `[assets]` table and be silently ignored.
    let policy_path = paths.policy_file.clone();
    let mut policy = keepmyconfig_core::policy::Policy::load(&policy_path).unwrap();
    if !policy
        .pinned
        .iter()
        .any(|p| p == "model_providers.SailsAPI.name")
    {
        policy
            .pinned
            .push("model_providers.SailsAPI.name".to_string());
    }
    policy.save(&policy_path).unwrap();
    // The store compiles its policy at open time, so reopen it after editing the
    // policy file, then re-baseline under the new policy.
    let store = Store::open(paths.clone()).unwrap();
    store.capture(None, None).unwrap();

    let baseline = fs::read_to_string(&paths.config_file).unwrap();
    assert!(baseline.contains("name = \"SailsAPI\""));

    // Swap only the provider name, exactly as CC Switch's stored template does.
    let rewritten = baseline.replace("name = \"SailsAPI\"", "name = \"OpenAI\"");
    assert_ne!(rewritten, baseline);
    fs::write(&paths.config_file, &rewritten).unwrap();

    let report = store.status().unwrap();
    assert_eq!(
        report.classification.as_ref().map(|c| c.kind),
        Some(keepmyconfig_core::classify::ChangeKind::Clobber),
        "a rewritten pinned value must be a clobber: {:?}",
        report.classification
    );
    assert!(
        report
            .diff
            .as_ref()
            .is_some_and(|diff| !diff.changed_pinned.is_empty()),
        "the pinned field must be reported as changed"
    );

    let outcome = store.process(false).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Repaired(_)),
        "expected repair, got {outcome:?}"
    );
    let repaired = fs::read_to_string(&paths.config_file).unwrap();
    assert!(
        repaired.contains("name = \"SailsAPI\""),
        "watch must restore the pinned provider name: {repaired}"
    );
}
