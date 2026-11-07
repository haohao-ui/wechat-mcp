//! Screen identification integration tests.
//! Migrated from `tests/test_screens.py`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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

fn eyes_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("eyes_selftest");

        let sources = [
            root.join("native").join("eyes").join("Screens.swift"),
            root.join("tests").join("eyes_selftest").join("main.swift"),
        ];

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        for s in &sources {
            cmd.arg(s);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of eyes_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_cli(command: &str, payload: &Value) -> Option<Value> {
    let bin = match eyes_selftest_bin() {
        Some(b) => b,
        None => {
            eprintln!("swiftc not available, skipping test");
            return None;
        }
    };
    let payload_str = serde_json::to_string(payload).expect("valid json payload");
    let output = Command::new(bin)
        .args([command, &payload_str])
        .output()
        .expect("failed to execute eyes_selftest");
    assert!(
        output.status.success(),
        "eyes_selftest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let val: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid json output from eyes_selftest");
    Some(val)
}

fn assert_json_num_eq(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            assert_eq!(a.len(), e.len(), "keys count mismatch: {:?} vs {:?}", a, e);
            for (k, ev) in e {
                let av = a.get(k).unwrap_or_else(|| panic!("missing key {} in {:?}", k, a));
                assert_json_num_eq(av, ev);
            }
        }
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(a.len(), e.len(), "array length mismatch: {:?} vs {:?}", a, e);
            for (av, ev) in a.iter().zip(e.iter()) {
                assert_json_num_eq(av, ev);
            }
        }
        (Value::Number(a), Value::Number(e)) => {
            let af = a.as_f64().expect("valid f64");
            let ef = e.as_f64().expect("valid f64");
            assert!(
                (af - ef).abs() < 1e-6,
                "number mismatch: {} vs {}",
                af,
                ef
            );
        }
        _ => {
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn test_main_window_only() {
    let mut payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.tencent.xinWeChat",
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("main"));

    // Tolerates <= 2 points difference
    payload["windows"][0]["bounds"] = serde_json::json!({"x": 100.0, "y": 80.0, "w": 881.5, "h": 638.5});
    let res2 = run_cli("identify", &payload).unwrap();
    assert_eq!(res2.get("screen").and_then(|v| v.as_str()), Some("main"));
}

#[test]
fn test_context_menu_popup() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            },
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 101,
                "bounds": {"x": 350.0, "y": 200.0, "w": 160.0, "h": 220.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.tencent.xinWeChat",
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("context_menu"));
}

#[test]
fn test_save_panel_focus() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.tencent.xinWeChat",
            "focused_identifier": "saveAsNameTextField"
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("save_panel"));
}

#[test]
fn test_image_viewer_makes_other_window() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            },
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 200.0, "y": 150.0, "w": 1024.0, "h": 768.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.tencent.xinWeChat",
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("other_wechat_window"));
}

#[test]
fn test_missing_focus_info_is_unknown() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": null,
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("unknown"));
}

#[test]
fn test_other_app_frontmost_is_unknown() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.apple.finder",
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0]
    });
    let res = match run_cli("identify", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("unknown"));
}

#[test]
fn test_cg_rows_convert() {
    let rows = serde_json::json!([
        // 1. Standard CGWindowList dictionary representation
        {
            "kCGWindowOwnerName": "WeChat",
            "kCGWindowOwnerPID": 1234,
            "kCGWindowLayer": 0,
            "kCGWindowBounds": {"X": 50, "Y": 60, "Width": 880, "Height": 640},
            "kCGWindowIsOnscreen": true
        },
        // 2. Simplified keys representation
        {
            "owner": "微信",
            "pid": 1234,
            "layer": 101,
            "bounds": {"x": 10, "y": 20, "w": 100, "h": 200},
            "on_screen": true
        },
        // 3. Missing owner -> drop
        {
            "pid": 1234,
            "layer": 0,
            "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
            "on_screen": true
        },
        // 4. Missing pid -> drop
        {
            "owner": "WeChat",
            "layer": 0,
            "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
            "on_screen": true
        },
        // 5. Missing layer -> drop
        {
            "owner": "WeChat",
            "pid": 1234,
            "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
            "on_screen": true
        },
        // 6. Missing bounds -> drop
        {
            "owner": "WeChat",
            "pid": 1234,
            "layer": 0,
            "on_screen": true
        },
        // 7. Missing onScreen -> drop
        {
            "owner": "WeChat",
            "pid": 1234,
            "layer": 0,
            "bounds": {"x": 0, "y": 0, "w": 100, "h": 100}
        }
    ]);
    let res = match run_cli("convert_rows", &rows) {
        Some(r) => r,
        None => return,
    };
    let arr = res.as_array().expect("array output");
    assert_eq!(arr.len(), 2);
    assert_json_num_eq(
        &arr[0],
        &serde_json::json!({
            "owner": "WeChat",
            "pid": 1234,
            "layer": 0,
            "bounds": {"x": 50.0, "y": 60.0, "w": 880.0, "h": 640.0},
            "on_screen": true
        }),
    );
    assert_json_num_eq(
        &arr[1],
        &serde_json::json!({
            "owner": "微信",
            "pid": 1234,
            "layer": 101,
            "bounds": {"x": 10.0, "y": 20.0, "w": 100.0, "h": 200.0},
            "on_screen": true
        }),
    );
}

#[test]
fn test_blocking_menu() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            },
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 101,
                "bounds": {"x": 350.0, "y": 200.0, "w": 160.0, "h": 220.0},
                "on_screen": true
            }
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("context_menu"));
}

#[test]
fn test_blocking_image_viewer() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 200.0, "y": 150.0, "w": 1024.0, "h": 768.0},
                "on_screen": true
            },
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("other_wechat_window"));
}

#[test]
fn test_blocking_save_panel_focus() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": "saveAsNameTextField"
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("save_panel"));
}

#[test]
fn test_other_app_frontmost_not_blocking() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "focus": {
            "frontmost_bundle": "com.apple.finder",
            "focused_identifier": null
        },
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(false));
    assert!(res.get("screen").map_or(true, |v| v.is_null()));
}

#[test]
fn test_window_behind_main_not_blocking() {
    let payload = serde_json::json!({
        "windows": [
            {"owner": "WeChat", "pid": 1234, "layer": 0,
             "bounds": {"x": 102.0, "y": 118.0, "w": 880.0, "h": 640.0}, "on_screen": true},
            {"owner": "WeChat", "pid": 1234, "layer": 0,
             "bounds": {"x": 428.0, "y": 343.0, "w": 360.0, "h": 124.0}, "on_screen": true}
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(false));
    assert!(res.get("screen").map_or(true, |v| v.is_null()));
}

#[test]
fn test_window_in_front_of_main_blocks() {
    let payload = serde_json::json!({
        "windows": [
            {"owner": "WeChat", "pid": 1234, "layer": 0,
             "bounds": {"x": 428.0, "y": 343.0, "w": 360.0, "h": 124.0}, "on_screen": true},
            {"owner": "WeChat", "pid": 1234, "layer": 0,
             "bounds": {"x": 102.0, "y": 118.0, "w": 880.0, "h": 640.0}, "on_screen": true}
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(res.get("screen").and_then(|v| v.as_str()), Some("other_wechat_window"));
}

#[test]
fn test_main_only_not_blocking() {
    let payload = serde_json::json!({
        "windows": [
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                "on_screen": true
            }
        ],
        "wechat_pid": 1234,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(false));
    assert!(res.get("screen").map_or(true, |v| v.is_null()));
}

#[test]
fn test_menubar_icon_does_not_block() {
    let payload = serde_json::json!({
        "windows": [
            {"owner": "微信", "pid": 49995, "layer": 25, "bounds": {"x": 751.0, "y": 0.0, "w": 52.0, "h": 24.0}, "on_screen": true},
            {"owner": "微信", "pid": 49995, "layer": 0, "bounds": {"x": 195.0, "y": 101.0, "w": 880.0, "h": 640.0}, "on_screen": true},
            {"owner": "WeChat RPA Test", "pid": 777, "layer": 0, "bounds": {"x": 424.0, "y": 623.0, "w": 700.0, "h": 588.0}, "on_screen": true}
        ],
        "wechat_pid": 49995,
        "main": [880.0, 640.0],
        "focused_identifier": null
    });
    let res = match run_cli("blocking", &payload) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res.get("blocked").and_then(|v| v.as_bool()), Some(false));
}
