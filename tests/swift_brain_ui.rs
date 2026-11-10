//! Integration and pure-logic tests for BrainPage, SessionsPage provenance, and Shell.

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
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
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
        root.join("native").join("bootstrap").join("Supervisor.swift"),
        root.join("native").join("bootstrap").join("CoreSupervisor.swift"),
        root.join("native").join("bootstrap").join("SocketWorker.swift"),
        root.join("native").join("bootstrap").join("Probe.swift"),
        root.join("native").join("bootstrap").join("StatusModel.swift"),
        root.join("native").join("bootstrap").join("ConfigForm.swift"),
        root.join("native").join("bootstrap").join("Actions.swift"),
        root.join("native").join("bootstrap").join("Permissions.swift"),
        root.join("native").join("bootstrap").join("LogTail.swift"),
        root.join("native").join("bootstrap").join("StatusPanel.swift"),
        root.join("native").join("bootstrap").join("Shell.swift"),
        root.join("native").join("bootstrap").join("SettingsPage.swift"),
        root.join("native").join("bootstrap").join("ContactsPage.swift"),
        root.join("native").join("bootstrap").join("McpPage.swift"),
        root.join("native").join("bootstrap").join("SessionsPage.swift"),
        root.join("native").join("bootstrap").join("BrainPage.swift"),
        root.join("native").join("eyes").join("Screens.swift"),
        root.join("tests").join("host_brain_selftest").join("main.swift"),
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

    let run_output = Command::new(&bin_path)
        .output()
        .expect("run brain_ui_selftest");

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
