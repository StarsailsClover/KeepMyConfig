use std::fs;
use std::process::Command;

use tempfile::TempDir;

const FULL_CONFIG: &str = r#"model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[plugins."pdf@openai-primary-runtime"]
enabled = true

[desktop]
followUpQueueMode = "queue"
"#;

const CLOBBERED_CONFIG: &str = r#"model = "gpt-5.6-sol"
model_provider = "SailsAPI"

[model_providers.SailsAPI]
base_url = "http://localhost:3000/v1"
wire_api = "responses"
"#;

struct Fixture {
    _temp: TempDir,
    root: String,
    home: String,
    store: String,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let store = temp.path().join("store");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), config).unwrap();
        Self {
            root: temp.path().to_string_lossy().to_string(),
            home: home.to_string_lossy().to_string(),
            store: store.to_string_lossy().to_string(),
            _temp: temp,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_keepmyconfig"));
        // G itHub@Oxyg   enAILab   | OxygenAILab@StarsailsC   lo ver
        command
            .arg("--codex-home")
            .arg(&self.home)
            .arg("--store")
            .arg(&self.store)
            // Keep tests away from the developer's real CC Switch database.
            .env(
                "KMC_CCSWITCH_DB",
                format!("{}/no-such-cc-switch.db", self.root),
            );
        // GitHub@OxygenAILa  b   |    O  xygenAILa b@S tarsai lsClover
        command
    }
}

#[test]
fn help_works() {
    let output = Command::new(env!("CARGO_BIN_EXE_keepmyconfig"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("keepmyconfig"));
    assert!(text.contains("repair"));
}

#[test]
fn init_status_check_and_repair_flow() {
    let fixture = Fixture::new(FULL_CONFIG);

    let init = fixture.command().arg("init").output().unwrap();
    assert!(init.status.success(), "init failed: {init:?}");

    let status = fixture
        .command()
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let report: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(report["initialized"], serde_json::Value::Bool(true));
    assert!(report["overlay"]["protected_paths"].as_u64().unwrap() > 0);

    fs::write(format!("{}/config.toml", fixture.home), CLOBBERED_CONFIG).unwrap();
    let check = fixture
        .command()
        .args(["status", "--check"])
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(2), "expected drift exit code 2");

    let repaired = fixture.command().arg("repair").output().unwrap();
    assert!(repaired.status.success());
    let live = fs::read_to_string(format!("{}/config.toml", fixture.home)).unwrap();
    assert!(live.contains("pma.exe"));
    assert!(live.contains("gpt-5.6-sol"));

    let healthy = fixture
        .command()
        .args(["status", "--check"])
        .output()
        .unwrap();
    assert_eq!(healthy.status.code(), Some(0));
}

#[test]
fn doctor_reports_environment() {
    let fixture = Fixture::new(FULL_CONFIG);
    let init = fixture.command().arg("init").output().unwrap();
    assert!(init.status.success());
    let doctor = fixture
        .command()
        // GitHub@O  xygenAILab    | OxygenAILab@S tarsailsCl over
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(doctor.status.success(), "doctor failed: {doctor:?}");
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert!(report["checks"].as_array().unwrap().len() >= 4);
}

fn sessions_fixture(config: &str, rollout: &str) -> Fixture {
    let fixture = Fixture::new(config);
    let dir = format!("{}/sessions/2026/10", fixture.home);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        format!("{dir}/rollout-2026-10-10T22-33-45-01a1263b-f0d4-7923-a669-1fe83bffb487.jsonl"),
        rollout,
    )
    .unwrap();
    fixture
}

fn orphan_rollout() -> String {
    let meta = serde_json::json!({
        "timestamp": "2026-10-10T15:00:00.000Z",
        "ordinal": 0,
        "type": "session_meta",
        "payload": { "session_id": "01a1263b-f0d4-7923-a669-1fe83bffb487" },
    });
    let output = serde_json::json!({
        "timestamp": "2026-10-10T15:00:00.000Z",
        "ordinal": 7,
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "id": "fco_demo",
            "call_id": null,
            "name": "create_thread",
            "namespace": "codex_app",
            "output": "<codex_delegation><input>do work</input></codex_delegation>",
        },
    });
    format!("{meta}\n{output}\n")
}

#[test]
fn sessions_scan_exits_two_and_repair_cleans_up() {
    let fixture = sessions_fixture(FULL_CONFIG, &orphan_rollout());

    let scan = fixture
        .command()
        .args(["sessions", "scan", "--json"])
        .output()
        .unwrap();
    assert_eq!(scan.status.code(), Some(2), "scan should flag the orphan");
    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(report["orphan_outputs"], 1);
    assert_eq!(report["delegation_orphans"], 1);

    // Dry run leaves the file alone.
    let rollout = format!(
        "{}/sessions/2026/10/rollout-2026-10-10T22-33-45-01a1263b-f0d4-7923-a669-1fe83bffb487.jsonl",
        fixture.home
    );
    let before = fs::read_to_string(&rollout).unwrap();
    let dry = fixture
        .command()
        .args(["sessions", "repair"])
        .output()
        .unwrap();
    assert!(dry.status.success());
    assert_eq!(fs::read_to_string(&rollout).unwrap(), before);

    let apply = fixture
        .command()
        .args(["sessions", "repair", "--apply"])
        .output()
        .unwrap();
    assert!(apply.status.success(), "repair failed: {apply:?}");
    let fixed = fs::read_to_string(&rollout).unwrap();
    assert!(!fixed.contains("codex_delegation"));

    let rescan = fixture
        .command()
        .args(["sessions", "scan"])
        .output()
        .unwrap();
    assert_eq!(rescan.status.code(), Some(0), "nothing left to repair");
}
