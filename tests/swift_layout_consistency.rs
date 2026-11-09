//! Layout consistency tests.
//! Migrated from `tests/test_layout_consistency.py`.

use serde_json::Value;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn source(rel_path: &str) -> String {
    let p = repo_root().join(rel_path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {}", p.display(), e))
}

fn load_layout() -> Value {
    let path = repo_root()
        .join("native")
        .join("layout")
        .join("wechat-4.1.15.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("invalid json in wechat-4.1.15.json")
}

fn rect(layout: &Value, name: &str) -> (i64, i64, i64, i64) {
    let r = &layout["regions"][name];
    (
        r["x"].as_i64().expect("x"),
        r["y"].as_i64().expect("y"),
        r["w"].as_i64().expect("w"),
        r["h"].as_i64().expect("h"),
    )
}

#[test]
fn test_window_size() {
    let layout = load_layout();
    let width = layout["window"]["width"].as_i64().unwrap();
    let height = layout["window"]["height"].as_i64().unwrap();
    assert_eq!((width, height), (880, 640));
    assert!(!source("native/wechat_bubbles.swift").contains("captured.width == 880*$0"));
}

#[test]
fn test_session_list() {
    let bot_sessions = source("native/bot/Sessions.swift");
    assert!(!bot_sessions.contains("let sessionListRect = CGRect"));
    assert!(!bot_sessions.contains("row.y + 12"));
}

#[test]
fn test_chat_area_for_scroll_and_media() {
    assert!(
        !source("native/bot/Worker.swift")
            .contains("CGRect(x:wx+315,y:wy+60,width:565,height:436)")
    );
    assert!(
        !source("native/bot/Files.swift")
            .contains("+315+Double(x),y:(window[\"y\"] as! Double)+60+Double(y)")
    );
}

#[test]
fn test_chat_content_crop_for_bubbles() {
    assert!(
        !source("native/wechat_bubbles.swift").contains("CGRect(x:315,y:60,width:full.width-325")
    );
}

#[test]
fn test_title_crop() {
    assert!(
        !source("native/wechat_bubbles.swift")
            .contains("CGRect(x:250,y:0,width:full.width-260,height:50)")
    );
}

#[test]
fn test_input_ocr_region_in_host() {
    let layout = load_layout();
    let (x, y, w, h) = rect(&layout, "input_ocr");
    let rust = source("src/wechat.rs");
    let s1 = format!("win_x + {}.0", x);
    let s2 = format!("win_y + win_h - {}.0", 640 - y);
    let s3 = format!("win_w - {}.0", 880 - w);
    assert!(rust.contains(&s1), "src/wechat.rs: {} not found", s1);
    assert!(rust.contains(&s2), "src/wechat.rs: {} not found", s2);
    assert!(rust.contains(&s3), "src/wechat.rs: {} not found", s3);

    // Regex check: win_w - %d\.0,\s*%d\.0 across potential whitespace and newlines
    let prefix = format!("win_w - {}.0,", 880 - w);
    let suffix = format!("{}.0", h);
    let mut found = false;
    let mut cursor = 0;
    while let Some(pos) = rust[cursor..].find(&prefix) {
        let abs_pos = cursor + pos + prefix.len();
        let after = rust[abs_pos..].trim_start();
        if after.starts_with(&suffix) {
            found = true;
            break;
        }
        cursor = abs_pos;
    }
    assert!(
        found,
        "src/wechat.rs regex pattern 'win_w - {}.0,\\s*{}.0' not found",
        880 - w,
        h
    );
}

#[test]
fn test_scroll_tick() {
    let layout = load_layout();
    let tick = layout["scroll"]["points_per_tick"].as_i64().unwrap();
    let policy_path = repo_root()
        .join("scripts")
        .join("wechat_bot")
        .join("policy.py");
    if policy_path.exists() {
        let policy_content = std::fs::read_to_string(&policy_path).unwrap();
        let expected = format!("SCROLL_STEP_POINTS = {}", tick);
        assert!(
            policy_content.contains(&expected),
            "scripts/wechat_bot/policy.py: {} not found",
            expected
        );
    }
    assert!(!source("native/bot/Worker.swift").contains("direction==\"up\" ? 10 : -10"));
}

#[test]
fn test_save_as_menu_labels() {
    assert!(!source("native/bot/Files.swift").contains("\"另存为…\""));
}

#[test]
fn test_theme_thresholds() {
    let f = source("native/wechat_bubbles.swift");
    assert!(!f.contains("r < 37 && g < 37 && b < 37"));
    assert!(!f.contains("r >= 38 && r <= 82"));
    assert!(!f.contains("r >= 246 && g >= 246 && b >= 246"));
    assert!(!f.contains("r >= 228 && r <= 244"));
}

#[test]
fn test_every_click_or_read_path_checks_anchors_first() {
    let worker = source("native/bot/Worker.swift");
    for op in &["sessions", "open_session"] {
        let target = format!("if operation == \"{}\"", op);
        let start = worker
            .find(&target)
            .unwrap_or_else(|| panic!("{} not found", target));
        let rest = &worker[start..];
        let end = rest.find("\n        }\n").expect("end of branch");
        let branch = &rest[..end];
        assert!(
            branch.contains("checkedScreen("),
            "checkedScreen( not in {}",
            op
        );
        assert!(
            !branch.contains("normalizedFrame("),
            "normalizedFrame( found in {}",
            op
        );
    }
    let bubbles = source("native/wechat_bubbles.swift");
    let start = bubbles
        .find("func analyzeFrame(")
        .expect("analyzeFrame not found");
    let rest = &bubbles[start..];
    let end = rest
        .find("guard let title")
        .expect("guard let title not found");
    let analyze = &rest[..end];
    assert!(analyze.contains("try checkAnchors(captured"));
}

#[test]
fn test_every_operation_gates_first() {
    let worker = source("native/bot/Worker.swift");
    let cap_idx = worker
        .find("func capture(")
        .expect("func capture( not found");
    let cap_rest = &worker[cap_idx..];
    let cap_end = cap_rest.find("\n    }\n").expect("capture end");
    let capture_fn = &cap_rest[..cap_end];
    assert!(
        capture_fn.contains("screenGate("),
        "capture must call screenGate"
    );

    for op in &[
        "sessions",
        "open_session",
        "scroll_chat",
        "prepare_file",
        "prepare_image",
    ] {
        let target = format!("if operation == \"{}\"", op);
        let start = worker
            .find(&target)
            .unwrap_or_else(|| panic!("target {} not found", target));
        let rest = &worker[start..];
        let end = rest.find("\n        }\n").expect("end of branch");
        let branch = &rest[..end];
        assert!(
            branch.contains("screenGate("),
            "{} must call screenGate",
            op
        );
    }
}

#[test]
fn test_capture_checks_anchors_before_reusing_a_cached_frame() {
    let worker = source("native/bot/Worker.swift");
    let start = worker
        .find("func capture(_ recipient: String")
        .expect("func capture(_ recipient: String not found");
    let rest = &worker[start..];
    let end = rest.find("let reused").expect("let reused not found");
    let body = &rest[..end];
    assert!(body.contains("checkAnchors(image"));
}

#[test]
fn test_row_operations_check_only_session_list_anchors() {
    let worker = source("native/bot/Worker.swift");
    for op in &["sessions", "open_session"] {
        let target = format!("if operation == \"{}\"", op);
        let start = worker
            .find(&target)
            .unwrap_or_else(|| panic!("target {} not found", target));
        let rest = &worker[start..];
        let end = rest.find("\n        }\n").expect("end of branch");
        let branch = &rest[..end];
        assert!(
            branch.contains("checkedScreen(window, regions: [\"session_list\"])"),
            "checkedScreen(window, regions: [\"session_list\"]) not found in {}",
            op
        );
    }
}
