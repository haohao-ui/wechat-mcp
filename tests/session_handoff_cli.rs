use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use wechat_mcp::core::history::store::HistoryStore;

#[test]
fn handoff_persists_across_control_processes() {
    let root = std::env::temp_dir().join(format!("t8-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let config = root.join("config.json");
    fs::write(
        &config,
        json!({"schema_version":1,"host":"local","account_id":"account_a",
        "state_dir":root,"allowlist":[{"recipient":"Alice","kind":"private"}]})
        .to_string(),
    )
    .unwrap();
    let store = HistoryStore::open(&root).unwrap();
    let id = store
        .ensure_contact(
            "account_a",
            &json!({"recipient":"Alice","kind":"private"}),
            false,
        )
        .unwrap();
    drop(store);
    let run = |action: &str, request: Value| {
        let result = Command::new(env!("CARGO_BIN_EXE_wechat-mcp"))
            .args([
                "control",
                action,
                "--config",
                config.to_str().unwrap(),
                "--request-json",
                &request.to_string(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "control failed without exposing output"
        );
        assert!(result.stderr.is_empty());
        serde_json::from_slice::<Value>(&result.stdout).unwrap()
    };
    for human in [true, true, false, false, true] {
        let result = run(
            "session-handoff",
            json!({"session_id":id,"to":if human {"human"} else {"bot"}}),
        );
        assert_eq!(result["data"]["handoff"], human);
        let detail = run("session-detail", json!({"contact_id":id}));
        assert_eq!(detail["data"]["session"]["handoff"], human);
        let list = run("sessions-list", json!({}));
        assert_eq!(list["data"]["sessions"][0]["handoff"], human);
    }
    fs::remove_dir_all(root).unwrap();
}
