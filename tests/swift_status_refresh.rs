//! Live status publication updates the visible pages without reopening them.
use std::{path::PathBuf, process::Command};

#[test]
fn visible_pages_follow_host_status_without_discarding_input() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = std::env::temp_dir().join(format!("wechat-ui-refresh-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).unwrap();
    let bin = temp.join("refresh-selftest");
    let mut sources: Vec<_> = std::fs::read_dir(root.join("native/bootstrap"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "swift")
                && p.file_name().unwrap() != "main.swift"
        })
        .collect();
    sources.sort();
    let output = Command::new("swiftc")
        .args(&sources)
        .arg(root.join("native/eyes/Screens.swift"))
        .arg(root.join("tests/host_refresh_selftest/main.swift"))
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = Command::new(&bin)
        .env("CFFIXED_USER_HOME", &temp)
        .env("WECHAT_MCP_BIN", "/usr/bin/false")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(temp);
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
