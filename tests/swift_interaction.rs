use std::{path::Path, process::Command};

#[test]
fn generic_hand_and_observation_binding_post_no_real_events() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bin = root.join("target/swift_test_bin/interaction_selftest");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-target",
        &format!(
            "{}-apple-macos12",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            }
        ),
    ]);
    for name in ["Model.swift"] {
        cmd.arg(root.join("native/interaction").join(name));
    }
    cmd.arg(root.join("native/common/Geometry.swift"));
    for name in [
        "Profile.swift",
        "Trajectory.swift",
        "Executor.swift",
        "Actions.swift",
    ] {
        cmd.arg(root.join("native/action").join(name));
    }
    cmd.arg(root.join("tests/interaction_selftest/main.swift"))
        .arg("-o")
        .arg(&bin);
    let compile = cmd.output().expect("swiftc required on macOS");
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let output = Command::new(&bin).output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for (name, result) in results.as_object().unwrap() {
        assert_eq!(result, true, "{name}");
    }
}

#[test]
fn generic_desktop_helper_compiles_for_macos12() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bin = root.join("target/swift_test_bin/desktop_helper");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-target",
        &format!(
            "{}-apple-macos12",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            }
        ),
    ]);
    for folder in ["native/interaction", "native/action"] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(folder))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "swift"))
            .collect();
        files.sort();
        cmd.args(files);
    }
    cmd.arg(root.join("native/common/Geometry.swift"))
        .arg("-o")
        .arg(bin);
    let result = cmd.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
