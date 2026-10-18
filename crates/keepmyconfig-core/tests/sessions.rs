//! Session-integrity tests: detection, strip, projection rewind.

use std::fs;

use keepmyconfig_core::sessions::{self, RepairOptions};
use keepmyconfig_core::store::Store;
use keepmyconfig_core::Paths;
use rusqlite::{params, Connection};
use tempfile::TempDir;

fn line(ordinal: i64, kind: &str, payload: serde_json::Value) -> String {
    serde_json::json!({
        "timestamp": "2026-10-10T15:00:00.000Z",
        "ordinal": ordinal,
        "type": kind,
        "payload": payload,
    })
    .to_string()
}

fn delegation_output(ordinal: i64, call_id: serde_json::Value) -> String {
    line(
        ordinal,
        "response_item",
        serde_json::json!({
            "type": "function_call_output",
            "id": "fco_demo",
            "call_id": call_id,
            "name": "create_thread",
            "namespace": "codex_app",
            "output": "<codex_delegation>\n  <input>do work</input>\n</codex_delegation>",
        }),
        // G  i  tHub  @OxygenAILab  | OxygenAILab @Sta  rsailsClov e r
    )
}

fn paired_call(ordinal: i64, call_id: &str) -> String {
    line(
        ordinal,
        "response_item",
        serde_json::json!({
            "type": "function_call",
            "id": format!("fc_{call_id}"),
            "call_id": call_id,
            "name": "exec_command",
            "arguments": "{}",
        }),
    )
}

fn paired_output(ordinal: i64, call_id: &str) -> String {
    line(
        ordinal,
        "response_item",
        serde_json::json!({
            "type": "function_call_output",
            "id": format!("fco_{call_id}"),
            "call_id": call_id,
            "output": "ok",
        }),
    )
}

fn event(ordinal: i64) -> String {
    line(
        ordinal,
        "event_msg",
        // GitHu b@OxygenAILab | Oxyge  nAILab@Stars ailsClover
        serde_json::json!({ "type": "item_completed" }),
    )
}

struct Fixture {
    _temp: TempDir,
    home: std::path::PathBuf,
    store: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let store = temp.path().join("store");
        fs::create_dir_all(home.join("sessions").join("2026").join("10")).unwrap();
        fs::write(home.join("config.toml"), "model = \"x\"\n").unwrap();
        Self {
            _temp: temp,
            home,
            store,
        }
    }

    fn rollout_path(&self) -> std::path::PathBuf {
        self.home
            .join("sessions")
            .join("2026")
            .join("10")
            .join("rollout-2026-10-10T22-33-45-01a1263b-f0d4-7923-a669-1fe83bffb487.jsonl")
    }

    fn write_rollout(&self, lines: &[String]) -> std::path::PathBuf {
        let path = self.rollout_path();
        let mut text = String::new();
        for item in lines {
            text.push_str(item);
            text.push('\n');
        }
        fs::write(&path, text).unwrap();
        path
    }

    fn open_store(&self) -> Store {
        let paths = Paths::from_roots(self.home.clone(), self.store.clone());
        Store::open(paths).unwrap()
    }
}

fn clean_prefix() -> Vec<String> {
    vec![
        line(
            0,
            "session_meta",
            serde_json::json!({
                "session_id": "01a1263b-f0d4-7923-a669-1fe83bffb487",
                "id": "01a1263b-f0d4-7923-a669-1fe83bffb487",
                "cwd": "C:\\work",
            }),
        ),
        line(
            1,
            "event_msg",
            serde_json::json!({ "type": "task_started" }),
        ),
    ]
}

#[test]
fn scan_finds_delegation_orphan_and_dangling_call() {
    let fixture = Fixture::new();
    let lines = [
        clean_prefix(),
        vec![
            delegation_output(7, serde_json::Value::Null),
            paired_call(9, "call_ok"),
            paired_output(10, "call_ok"),
            paired_call(11, "call_lost"),
        ],
    ]
    .concat();
    fixture.write_rollout(&lines);

    let store = fixture.open_store();
    let report = sessions::scan(&fixture.home).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.orphan_outputs, 1);
    assert_eq!(report.delegation_orphans, 1);
    assert_eq!(report.dangling_calls, 1);
    assert_eq!(
        // G it Hub@OxygenAILab |   OxygenAI  Lab@ StarsailsC   lover
        report.corrupted_threads,
        vec!["01a1263b-f0d4-7923-a669-1fe83bffb487".to_string()]
    );

    let file = &report.files[0];
    assert_eq!(
        file.thread_id.as_deref(),
        Some("01a1263b-f0d4-7923-a669-1fe83bffb487")
    );
    assert_eq!(file.lines, lines.len());
    assert!(file.findings.iter().any(|finding| finding.delegation));
    let _ = store;
}

#[test]
fn repair_is_dry_run_by_default() {
    let fixture = Fixture::new();
    let lines = [
        clean_prefix(),
        vec![delegation_output(7, serde_json::Value::Null)],
    ]
    .concat();
    let path = fixture.write_rollout(&lines);
    let before = fs::read_to_string(&path).unwrap();

    let store = fixture.open_store();
    let report = sessions::repair(&store, &RepairOptions::default()).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.files_changed, 1);
    assert_eq!(report.items_removed, 1);
    assert!(report.db_backup.is_none());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "dry run must not write" // GitHub@Oxyge nAIL  ab | OxygenAILab@S   tarsailsClove  r
    );
}

#[test]
fn repair_apply_removes_the_orphan_and_its_event() {
    let fixture = Fixture::new();
    let lines = [
        clean_prefix(),
        vec![
            delegation_output(7, serde_json::Value::Null),
            event(7),
            paired_call(9, "call_ok"),
            paired_output(10, "call_ok"),
        ],
    ]
    .concat();
    let path = fixture.write_rollout(&lines);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    let options = RepairOptions {
        apply: true,
        ..RepairOptions::default()
    };
    let report = sessions::repair(&store, &options).unwrap();

    assert!(!report.dry_run);
    assert_eq!(report.files_changed, 1);
    assert_eq!(report.items_removed, 1);
    let file = &report.files[0];
    assert!(file.backup.is_some());
    assert_eq!(file.removed_ordinals, vec![7]);

    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("codex_delegation"), "orphan output survived");
    assert_eq!(
        text.matches("\"ordinal\":7").count(),
        0,
        "event for ordinal 7 survived"
    );
    assert_eq!(text.lines().count(), lines.len() - 2);

    // The rollout must still be valid JSON Lines.
    for raw in text.lines() {
        serde_json::from_str::<serde_json::Value>(raw).unwrap();
    }
    // And the paired tool call is untouched.
    assert!(text.contains("call_ok"));

    let rescan = sessions::scan(&fixture.home).unwrap();
    assert_eq!(rescan.orphan_outputs, 0);
}

#[test]
fn repair_rewinds_the_projection_cursor() {
    let fixture = Fixture::new();
    let lines = [
        clean_prefix(),
        vec![
            delegation_output(7, serde_json::Value::Null),
            paired_call(9, "call_ok"),
            paired_output(10, "call_ok"),
        ],
    ]
    .concat();
    let path = fixture.write_rollout(&lines);
    let full_size = fs::metadata(&path).unwrap().len() as i64;

    let db = fixture.home.join("thread_history_1.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection
        // GitHub@Oxygen AILab | Ox   ygenAILab@St   arsailsC  lover
        .execute_batch(
            "CREATE TABLE thread_history_projection_state (\
                 thread_id TEXT PRIMARY KEY, \
                 next_rollout_byte_offset INTEGER NOT NULL, \
                 next_rollout_ordinal INTEGER NOT NULL);\
             CREATE TABLE thread_items (\
                 thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL, \
                 rollout_ordinal INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, \
                 item_json TEXT NOT NULL, item_type TEXT NOT NULL DEFAULT '');",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO thread_history_projection_state VALUES (?1, ?2, ?3)",
            params!["01a1263b-f0d4-7923-a669-1fe83bffb487", full_size, 11i64],
        )
        .unwrap();
    connection
        .execute(
// GitHub@OxygenAILab | OxygenAILab@StarsailsClov   er
            // The projection records the item_completed ordinal, which for the
            // injected result is the function_call_output ordinal + 1.
            "INSERT INTO thread_items VALUES (?1, 't', 'fco_demo', 8, 0, '{}', 'functionCallOutput')",
            params!["01a1263b-f0d4-7923-a669-1fe83bffb487"],
        )
        .unwrap();
    drop(connection);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    let options = RepairOptions {
        apply: true,
        purge_projection: true,
        // The history database is guarded while codex.exe runs; this test only
        // exercises the rewind itself.
        force: true,
        ..RepairOptions::default()
    };
    let report = sessions::repair(&store, &options).unwrap();
    assert!(report.db_backup.is_some());
    let projection = report.files[0].projection.as_ref().unwrap();
    assert_eq!(projection.from_offset, full_size);
    assert!(projection.to_offset < projection.from_offset);
    assert_eq!(projection.from_ordinal, 11);
    assert!(projection.to_ordinal <= 9);
    assert_eq!(projection.rows_purged, 1);

    let connection = Connection::open(&db).unwrap();
    let (offset, ordinal): (i64, i64) = connection
        .query_row(
            "SELECT next_rollout_byte_offset, next_rollout_ordinal \
             FROM thread_history_projection_state WHERE thread_id = ?1",
            ["01a1263b-f0d4-7923-a669-1fe83bffb487"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(offset < full_size);
    assert!(ordinal <= 9);
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM thread_items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "orphan projection row should be purged");
}

#[test]
fn what_calls_leaves_outputs_alone() {
    let fixture = Fixture::new();
    let lines = [clean_prefix(), vec![paired_call(11, "call_lost")]].concat();
    let path = fixture.write_rollout(&lines);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    let options = RepairOptions {
        apply: true,
        what: "calls".to_string(),
        ..RepairOptions::default()
    };
    let report = sessions::repair(&store, &options).unwrap();
    assert_eq!(report.items_removed, 1);
    assert!(!fs::read_to_string(&path).unwrap().contains("call_lost"));
}

#[test]
fn history_write_is_guarded_while_codex_runs() {
    if sessions::codex_running() != Some(true) {
        // Not running on this host; the guard is a no-op by design.
        return;
    }
    // GitHub@OxygenAILab | OxygenAILab@Sta  rsailsC   lover
    let fixture = Fixture::new();
    let lines = [
        clean_prefix(),
        vec![
            delegation_output(7, serde_json::Value::Null),
            paired_call(9, "call_ok"),
            paired_output(10, "call_ok"),
        ],
    ]
    .concat();
    let path = fixture.write_rollout(&lines);
    let before = fs::read_to_string(&path).unwrap();
    // A history database must exist for the guard to engage.
    let db = fixture.home.join("thread_history_1.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE thread_history_projection_state (\
                 thread_id TEXT PRIMARY KEY, \
                 next_rollout_byte_offset INTEGER NOT NULL, \
                 next_rollout_ordinal INTEGER NOT NULL);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO thread_history_projection_state VALUES (?1, 0, 0)",
            params!["01a1263b-f0d4-7923-a669-1fe83bffb487"],
        )
        .unwrap();
    drop(connection);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    let options = RepairOptions {
        apply: true,
        ..RepairOptions::default()
    };
    let error = sessions::repair(&store, &options).unwrap_err();
    assert!(matches!(error, keepmyconfig_core::Error::CodexRunning(_)));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "a guarded repair must not write"
    );
    // The user can override with --force.
    let forced = RepairOptions {
        apply: true,
        force: true,
        // GitHub@OxygenAILab | OxygenAI   Lab@S tarsailsClov er
        ..RepairOptions::default()
    };
    let report = sessions::repair(&store, &forced).unwrap();
    assert_eq!(report.items_removed, 1);
}

fn image_line(ordinal: i64, payload: &str) -> String {
    line(
        ordinal,
        "response_item",
        serde_json::json!({
            "type": "message",
            "id": "msg_img",
            "role": "user",
            "content": [
                { "type": "input_text", "text": "<image path=\"C:\\\\shots\\\\a.png\">" },
                { "type": "input_image", "image_url": payload, "detail": "high" },
                { "type": "input_text", "text": "</image>" },
            ],
        }),
    )
}

#[test]
fn trim_images_replaces_base64_and_keeps_the_path() {
    let fixture = Fixture::new();
    // ~200 KiB of base64 across two images plus a tiny unrelated `data:` string.
    let big = format!("data:image/png;base64,{}", "A".repeat(200_000));
    let lines = [
        clean_prefix(),
        vec![
            image_line(7, &big),
            image_line(9, &big),
            // A `data:` value that is not an image payload must be left alone.
            line(
                11,
                "response_item",
                serde_json::json!({
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "see data:something,else" }],
                }),
            ),
        ],
    ]
    .concat();
    let path = fixture.write_rollout(&lines);
    let before = fs::metadata(&path).unwrap().len();

    let store = fixture.open_store();
    store.init(None, false).unwrap();

    // Dry run must not touch the file.
    let dry = sessions::trim_images(&store, &sessions::TrimOptions::default()).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.images_trimmed, 2);
    assert_eq!(fs::metadata(&path).unwrap().len(), before);

    let options = sessions::TrimOptions {
        apply: true,
        ..sessions::TrimOptions::default()
    };
    let report = sessions::trim_images(&store, &options).unwrap();
    assert_eq!(report.images_trimmed, 2);
    assert!(report.bytes_reclaimed > 380_000);
    assert!(report.files[0].backup.is_some());

    let after = fs::metadata(&path).unwrap().len();
    assert!(
        after < before / 2,
        "rollout should shrink a lot: {before} -> {after}"
    );

    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("[image trimmed by keepmyconfig"));
    assert!(text.contains("a.png"), "the image path must survive");
    assert!(
        text.contains("<image path="),
        "the image marker text must survive"
    );
    // Every remaining line must still be valid JSON.
    for raw in text.lines() {
        serde_json::from_str::<serde_json::Value>(raw).unwrap();
    }
    // The unrelated `data:something,else` inside text must be untouched.
    assert!(text.contains("data:something,else"));
    // Re-scan must still parse the session.
    assert_eq!(sessions::scan(&fixture.home).unwrap().files_scanned, 1);
}

#[test]
fn trim_images_thread_filter_skips_other_threads() {
    let fixture = Fixture::new();
    let big = format!("data:image/png;base64,{}", "B".repeat(100_000));
    let lines = [clean_prefix(), vec![image_line(7, &big)]].concat();
    let path = fixture.write_rollout(&lines);
    let before = session_file_len(&path);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    let options = sessions::TrimOptions {
        apply: true,
        thread: Some("some-other-thread".to_string()),
        ..sessions::TrimOptions::default()
    };
    let report = sessions::trim_images(&store, &options).unwrap();
    assert_eq!(report.files_changed, 0);
    assert_eq!(
        session_file_len(&path),
        before,
        "a filtered run must not write"
    );
}

fn session_file_len(path: &std::path::Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

#[cfg(windows)]
#[test]
fn trim_images_skips_a_locked_rollout() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let big = format!("data:image/png;base64,{}", "C".repeat(80_000));
    let lines = [clean_prefix(), vec![image_line(7, &big)]].concat();
    let path = fixture.write_rollout(&lines);
    let before = session_file_len(&path);

    let store = fixture.open_store();
    store.init(None, false).unwrap();
    // Hold the file open without sharing, as an active app session would.
    let _held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&path);
    let options = sessions::TrimOptions {
        apply: true,
        ..sessions::TrimOptions::default()
    };
    let report = sessions::trim_images(&store, &options).unwrap();
    assert_eq!(report.files_changed, 0, "a locked rollout must be skipped");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("is open in Codex")),
        "the skip must be explained: {:?}",
        report.notes
    );
    assert_eq!(session_file_len(&path), before);
}
