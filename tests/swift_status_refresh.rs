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
    // A stale install marker must never override the executable bundle version.
    let app = temp.join("VersionProbe.app/Contents");
    std::fs::create_dir_all(app.join("MacOS")).unwrap();
    std::fs::copy(&bin, app.join("MacOS/refresh-selftest")).unwrap();
    std::fs::write(app.join("Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>refresh-selftest</string><key>CFBundleIdentifier</key><string>local.wechat-mcp.host</string><key>CFBundleShortVersionString</key><string>9.8.7</string><key>CFBundleVersion</key><string>123</string></dict></plist>"#).unwrap();
    std::fs::create_dir_all(temp.join(".wechat-rpa")).unwrap();
    std::fs::write(temp.join(".wechat-rpa/VERSION"), "stale-install-marker").unwrap();
    let version = Command::new(app.join("MacOS/refresh-selftest"))
        .arg("--host-version")
        .env("CFFIXED_USER_HOME", &temp)
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "9.8.7 (123)"
    );
    let _ = std::fs::remove_dir_all(temp);
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
