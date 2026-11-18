//! Anchor verification integration tests.
//! Migrated from `tests/test_anchors.py`.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir_name = format!("{}_{}_{}_{}", prefix, std::process::id(), count, nanos);
        let path = std::env::temp_dir().join(dir_name);
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn anchors_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("anchors_selftest");

        let sources = [
            root.join("native").join("common").join("Geometry.swift"),
            root.join("native")
                .join("common")
                .join("WechatLayout.swift"),
            root.join("native").join("eyes").join("Anchors.swift"),
            root.join("tests")
                .join("eyes_selftest")
                .join("anchors_main.swift"),
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
                "swiftc compilation of anchors_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn frames_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("frames")
}

fn layout_path() -> PathBuf {
    repo_root()
        .join("native")
        .join("layout")
        .join("wechat-4.1.15.json")
}

fn run_anchors(frame_name: &str, extra_args: Option<&[&str]>) -> Option<Value> {
    let bin = match anchors_selftest_bin() {
        Some(b) => b,
        None => {
            eprintln!("swiftc not available, skipping test");
            return None;
        }
    };
    let frames = frames_dir();
    let png_path = if frames.join(format!("{}.png", frame_name)).exists() {
        frames.join(format!("{}.png", frame_name))
    } else {
        frames.join(frame_name)
    };
    // 真实微信截图夹具已从仓库移除（隐私），缺夹具时跳过而不是断言失败。
    if !png_path.exists() {
        eprintln!("SKIPPED: private fixture {} removed", frame_name);
        return None;
    }

    let mut cmd = Command::new(bin);
    cmd.arg(png_path.to_str().unwrap());
    cmd.arg(layout_path().to_str().unwrap());
    if let Some(extras) = extra_args {
        cmd.args(extras);
    }

    let output = cmd.output().expect("failed to execute anchors_selftest");
    assert!(
        output.status.success(),
        "CLI error (code {:?}):\nSTDERR: {}\nSTDOUT: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let val: Value =
        serde_json::from_slice(&output.stdout).expect("invalid json output from anchors_selftest");
    Some(val)
}

fn get_fixtures() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("r1-dark-top", "wechat-4-dark");
    m.insert("r1-dark-bottom", "wechat-4-dark");
    m.insert("r1-light-top", "wechat-4-light");
    m.insert("r1-light-bottom", "wechat-4-light");
    m.insert("retina-dark-top", "wechat-4-dark");
    m.insert("retina-dark-bottom", "wechat-4-dark");
    m.insert("retina-light-top", "wechat-4-light");
    m.insert("retina-light-bottom", "wechat-4-light");
    m
}

#[test]
fn test_measure_finds_real_edges_far_from_a_wrong_map() {
    let bin = match anchors_selftest_bin() {
        Some(b) => b,
        None => return,
    };
    let layout_file = layout_path();
    let mut layout: Value =
        serde_json::from_str(&std::fs::read_to_string(&layout_file).unwrap()).unwrap();

    layout["regions"]["chat"]["h"] =
        serde_json::json!(layout["regions"]["chat"]["h"].as_i64().unwrap() - 30);
    layout["regions"]["chat_content"]["h"] =
        serde_json::json!(layout["regions"]["chat_content"]["h"].as_i64().unwrap() - 30);
    layout["regions"]["input"]["y"] =
        serde_json::json!(layout["regions"]["input"]["y"].as_i64().unwrap() - 30);
    layout["regions"]["input"]["h"] =
        serde_json::json!(layout["regions"]["input"]["h"].as_i64().unwrap() + 30);
    layout["regions"]["session_list"]["w"] =
        serde_json::json!(layout["regions"]["session_list"]["w"].as_i64().unwrap() - 40);

    if !frames_dir().join("r1-dark-top.png").exists() {
        eprintln!("SKIPPED: private fixtures removed");
        return;
    }

    let temp = TempDir::new("wrong_layout");
    let wrong_path = temp.path.join("wrong.json");
    std::fs::write(&wrong_path, serde_json::to_string(&layout).unwrap()).unwrap();

    let names = [
        "r1-dark-top",
        "r1-light-bottom",
        "retina-dark-bottom",
        "retina-light-top",
        "r1-light-xls-card",
    ];

    for name in &names {
        for path in &[layout_file.to_str().unwrap(), wrong_path.to_str().unwrap()] {
            let png_path = frames_dir().join(format!("{}.png", name));
            let output = Command::new(bin)
                .args([png_path.to_str().unwrap(), path, "--measure"])
                .output()
                .expect("run measure");
            assert!(
                output.status.success(),
                "measure failed for {} with {}: {}",
                name,
                path,
                String::from_utf8_lossy(&output.stderr)
            );
            let out: Value = serde_json::from_slice(&output.stdout).unwrap();
            let input_divider = out["measured"]["input_divider"].as_f64().unwrap();
            let list_border = out["measured"]["list_border"].as_f64().unwrap();
            assert_eq!(input_divider.round() as i64, 496, "{} input_divider", name);
            assert!(
                (list_border - 300.0).abs() <= 1.0,
                "{} list_border: {}",
                name,
                list_border
            );
        }
    }
}

#[test]
fn test_no_chat_open_blocks_reading_but_not_the_session_list() {
    let full = match run_anchors("r1-light-no-chat", None) {
        Some(f) => f,
        None => return,
    };
    assert_eq!(full["passed"], false);
    let failures = full["failures"].as_array().unwrap();
    assert!(failures.iter().any(|v| v.as_str() == Some("input_divider")));

    let listed = run_anchors("r1-light-no-chat", Some(&["--regions", "session_list"])).unwrap();
    assert_eq!(listed["passed"], true);
    assert_eq!(listed["theme"], "wechat-4-light");
}

#[test]
fn test_session_list_scope_still_catches_a_moved_list() {
    let listed = match run_anchors(
        "r1-light-bottom",
        Some(&["--regions", "session_list", "--shift", "30,0"]),
    ) {
        Some(l) => l,
        None => return,
    };
    let failures = listed["failures"].as_array().unwrap();
    assert!(failures.iter().any(|v| v.as_str() == Some("list_border")));
}

#[test]
fn test_all_fixtures_pass() {
    let fixtures = get_fixtures();
    for (name, expected_theme) in fixtures {
        let res = match run_anchors(name, None) {
            Some(r) => r,
            None => return,
        };
        assert_eq!(res["passed"], true, "Fixture {} failed: {:?}", name, res);
        assert_eq!(res["failures"], serde_json::json!([]));
        assert_eq!(res["theme"], expected_theme);
    }
}

#[test]
fn test_measured_positions_match_layout() {
    let fixtures = get_fixtures();
    for (name, _) in fixtures {
        let res = match run_anchors(name, None) {
            Some(r) => r,
            None => return,
        };
        let measured = &res["measured"];
        assert!(measured.get("input_divider").is_some());
        assert!(measured.get("list_border").is_some());
        let input_divider = measured["input_divider"].as_f64().unwrap();
        let list_border = measured["list_border"].as_f64().unwrap();
        assert!(
            (input_divider - 496.0).abs() <= 1.0,
            "{} input_divider: {}",
            name,
            input_divider
        );
        assert!(
            (list_border - 300.0).abs() <= 1.0,
            "{} list_border: {}",
            name,
            list_border
        );
    }
}

#[test]
fn test_shifted_frame_fails() {
    let res_v = match run_anchors("r1-dark-top", Some(&["--shift", "0,30"])) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res_v["passed"], false);
    let failures_v = res_v["failures"].as_array().unwrap();
    assert!(
        failures_v
            .iter()
            .any(|v| v.as_str() == Some("input_divider"))
    );

    let res_h = run_anchors("r1-dark-top", Some(&["--shift", "30,0"])).unwrap();
    assert_eq!(res_h["passed"], false);
    let failures_h = res_h["failures"].as_array().unwrap();
    assert!(failures_h.iter().any(|v| v.as_str() == Some("list_border")));
}

#[test]
fn test_blank_title_fails() {
    let res = match run_anchors("r1-dark-top", Some(&["--blank", "title"])) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res["passed"], false);
    let failures = res["failures"].as_array().unwrap();
    assert!(failures.iter().any(|v| v.as_str() == Some("title_text")));
}

#[test]
fn test_unknown_gray_fails_theme() {
    let res = match run_anchors("unknown-gray.png", None) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res["passed"], false);
    let failures = res["failures"].as_array().unwrap();
    assert!(failures.iter().any(|v| v.as_str() == Some("theme")));
    assert!(res["theme"].is_null());
}

#[test]
fn test_popup_over_divider_fails() {
    let res = match run_anchors(
        "r1-dark-top",
        Some(&["--patch", "315,420,200,150,240,240,240"]),
    ) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res["passed"], false);
    let failures = res["failures"].as_array().unwrap();
    assert!(failures.iter().any(|v| v.as_str() == Some("input_divider")));
}
