//! Integration and pure-logic tests for `BrainPage`, `SessionsPage` provenance, and `Shell`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn has_swiftc() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    Command::new("which")
        .arg("swiftc")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_swift_brain_ui_selftest() {
    if !has_swiftc() {
        eprintln!("swiftc not found, skipping swift_brain_ui test");
        return;
    }

    let root = repo_root();
    let out_dir = root.join("target").join("swift_test_bin");
    std::fs::create_dir_all(&out_dir).expect("create out_dir");
    let bin_path = out_dir.join("brain_ui_selftest");

    let files_to_compile = [
        root.join("native")
            .join("bootstrap")
            .join("Supervisor.swift"),
        root.join("native")
            .join("bootstrap")
            .join("CoreSupervisor.swift"),
        root.join("native")
            .join("bootstrap")
            .join("SocketWorker.swift"),
        root.join("native").join("bootstrap").join("Probe.swift"),
        root.join("native")
            .join("bootstrap")
            .join("StatusModel.swift"),
        root.join("native")
            .join("bootstrap")
            .join("ConfigForm.swift"),
        root.join("native").join("bootstrap").join("Actions.swift"),
        root.join("native")
            .join("bootstrap")
            .join("Permissions.swift"),
        root.join("native").join("bootstrap").join("LogTail.swift"),
        root.join("native")
            .join("bootstrap")
            .join("StatusPanel.swift"),
        root.join("native").join("bootstrap").join("Shell.swift"),
        root.join("native")
            .join("bootstrap")
            .join("SettingsPage.swift"),
        root.join("native")
            .join("bootstrap")
            .join("ContactsPage.swift"),
        root.join("native").join("bootstrap").join("McpPage.swift"),
        root.join("native")
            .join("bootstrap")
            .join("SessionsPage.swift"),
        root.join("native")
            .join("bootstrap")
            .join("BrainPage.swift"),
        root.join("native").join("eyes").join("Screens.swift"),
        root.join("tests")
            .join("host_brain_selftest")
            .join("main.swift"),
    ];

    let mut cmd = Command::new("swiftc");
    cmd.arg("-O");
    cmd.arg("-DHOST_APP_T3");
    cmd.arg("-DHOST_APP_CORE");
    for f in &files_to_compile {
        cmd.arg(f);
    }
    cmd.arg("-o");
    cmd.arg(&bin_path);

    let compile_status = cmd.status().expect("execute swiftc");
    assert!(compile_status.success(), "swiftc compilation failed");

    // 构造隔离的合成环境，用于真实 Rust control + Swift client 契约集成测试
    let temp_dir =
        std::env::temp_dir().join(format!("test_brain_ui_contract_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let state_dir = temp_dir.join("state");
    let history_dir = state_dir.join("history");
    std::fs::create_dir_all(&history_dir).expect("create history_dir");
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
        .expect("chmod state_dir");

    let cfg_path = temp_dir.join("config.json");
    let cfg_json = serde_json::json!({
        "schema_version": 1,
        "host": "local",
        "account_id": "contract_test_acct",
        "state_dir": state_dir.to_str().unwrap(),
        "allowlist": [
            {"recipient": "契约联系人", "kind": "private"}
        ]
    });
    std::fs::write(&cfg_path, cfg_json.to_string()).expect("write config.json");

    let db_path = history_dir.join("history.sqlite3");
    let init_sql = r#"
CREATE TABLE contacts(
  contact_id TEXT PRIMARY KEY, account_id TEXT NOT NULL, kind TEXT NOT NULL,
  display_name TEXT NOT NULL, merged_into TEXT REFERENCES contacts(contact_id),
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE brain_turns (
  account_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  snapshot_id TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  input_message_ids TEXT NOT NULL,
  observation_json TEXT NOT NULL,
  decision_json TEXT,
  model_json TEXT NOT NULL DEFAULT '{}',
  send_key TEXT,
  execution_json TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(account_id, turn_id)
);
CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
INSERT INTO schema_migrations(version, applied_at) VALUES (1, 1), (2, 1), (3, 1), (4, 1), (5, 1);

INSERT INTO contacts(contact_id, account_id, kind, display_name, created_at, updated_at)
VALUES ('c_contract', 'contract_test_acct', 'private', '契约联系人', 1700000000, 1700000000);

CREATE TABLE contact_names(
  id INTEGER PRIMARY KEY, contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  account_id TEXT NOT NULL, raw_name TEXT NOT NULL, name_key TEXT NOT NULL,
  source TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
  UNIQUE(contact_id, raw_name));

INSERT INTO contact_names(id, contact_id, account_id, raw_name, name_key, source, first_seen_at, last_seen_at)
VALUES (1, 'c_contract', 'contract_test_acct', '契约联系人', '契约联系人', 'config', 1700000000, 1700000000);

INSERT INTO brain_turns(account_id, turn_id, contact_id, snapshot_id, snapshot_json, input_message_ids, observation_json, decision_json, model_json, send_key, execution_json, created_at, updated_at)
VALUES (
  'contract_test_acct',
  'turn_contract_real',
  'c_contract',
  'snap_contract_1',
  '{"assembly_sha256":"sha_test","compiled":"你好！我是真实契约助手。"}',
  '["msg_1"]',
  '{"bubbles_count":1}',
  '{"action":"send","reply":"您好，服务条款如下"}',
  '{"provider":"mock","route":"fast"}',
  'send_key_1',
  '{"phase":"delivered"}',
  1700000000,
  1700000000
);
"#;

    let sqlite_status = Command::new("sqlite3")
        .arg(&db_path)
        .arg(init_sql)
        .status()
        .expect("initialize history.sqlite3");
    assert!(sqlite_status.success(), "sqlite3 init failed");

    let wechat_mcp_bin = root.join("target").join("debug").join("wechat-mcp");

    let run_output = Command::new(&bin_path)
        .env("CFFIXED_USER_HOME", &temp_dir)
        .env("REAL_CONTROL_CONFIG", &cfg_path)
        .env("WECHAT_MCP_BIN", &wechat_mcp_bin)
        .env("REAL_CONTROL_TEST_TURN_ID", "turn_contract_real")
        .output()
        .expect("run brain_ui_selftest");

    let _ = std::fs::remove_dir_all(&temp_dir);

    let stdout_str = String::from_utf8_lossy(&run_output.stdout);
    let stderr_str = String::from_utf8_lossy(&run_output.stderr);
    println!("=== Brain UI Selftest Output ===\n{stdout_str}");
    if !stderr_str.is_empty() {
        eprintln!("=== Stderr ===\n{stderr_str}");
    }

    assert!(
        run_output.status.success(),
        "brain_ui_selftest exited with status {:?}",
        run_output.status
    );
    assert!(stdout_str.contains("Total passed:"));
    assert!(stdout_str.contains("failed: 0"));
}
