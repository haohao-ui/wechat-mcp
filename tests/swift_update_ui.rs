//! Integration and UI fixture tests for `UpdatePage`, `UpdateClient`, `Shell`, and `SettingsPage`.

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

    // 运行测试可执行文件
    let run_status = Command::new(&bin_path)
        .status()
        .expect("execute update_ui_selftest binary");
    assert!(run_status.success(), "update_ui_selftest binary failed");
}
