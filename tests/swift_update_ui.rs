//! Integration and UI fixture tests for `UpdatePage`, `UpdateClient`, `Shell`, and `SettingsPage`.
//! Covers real Rust control contract, unconfigured status, unverified trust roots, and offscreen full-height PNG fixtures.

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
fn test_swift_update_ui_selftest() {
    if !has_swiftc() {
        eprintln!("swiftc not found, skipping swift_update_ui test");
        return;
    }

    let root = repo_root();
    let out_dir = root.join("target").join("swift_test_bin");
    std::fs::create_dir_all(&out_dir).expect("create out_dir");
    let bin_path = out_dir.join("update_ui_selftest");

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
        root.join("native")
            .join("bootstrap")
            .join("UpdatePage.swift"),
        root.join("native").join("eyes").join("Screens.swift"),
        root.join("tests")
            .join("host_update_selftest")
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
        std::env::temp_dir().join(format!("test_update_ui_contract_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let state_dir = temp_dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("create state_dir");
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
        .expect("chmod state_dir");

    let cfg_path = temp_dir.join("config.json");
    let cfg_json = serde_json::json!({
        "schema_version": 1,
        "host": "local",
        "account_id": "update_contract_test_acct",
        "state_dir": state_dir.to_str().unwrap(),
        "allowlist": [{"recipient": "测试联系人", "kind": "private"}]
    });
    std::fs::write(&cfg_path, cfg_json.to_string()).expect("write config.json");
    std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600))
        .expect("chmod config.json");

    let wechat_mcp_bin = root.join("target").join("debug").join("wechat-mcp");

    // 运行测试可执行文件（传入真实 Rust control 配置环境变量）
    let run_output = Command::new(&bin_path)
        .env("CFFIXED_USER_HOME", &temp_dir)
        .env("REAL_UPDATE_CONFIG", &cfg_path)
        .env("WECHAT_MCP_BIN", &wechat_mcp_bin)
        .output()
        .expect("execute update_ui_selftest binary");

    let _ = std::fs::remove_dir_all(&temp_dir);

    let stdout_str = String::from_utf8_lossy(&run_output.stdout);
    let stderr_str = String::from_utf8_lossy(&run_output.stderr);
    println!("=== Update UI Selftest Output ===\n{stdout_str}");
    if !stderr_str.is_empty() {
        eprintln!("=== Stderr ===\n{stderr_str}");
    }

    assert!(
        run_output.status.success(),
        "update_ui_selftest binary failed"
    );
}
