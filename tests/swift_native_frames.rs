//! Swift Native Frames and OCR Integration Tests.
//! Migrated from `tests/test_native_frames.py` (excluding RapidOCR-specific test cases).

use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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

fn frames_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("frames")
}

fn default_layout_path() -> PathBuf {
    repo_root()
        .join("native")
        .join("layout")
        .join("wechat-4.1.15.json")
}

fn default_window_json() -> String {
    let p = frames_dir().join("window-880x640.json");
    std::fs::read_to_string(&p).expect("read window-880x640.json")
}

fn wechat_bubbles_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("wechat_bubbles");

        let mut sources = Vec::new();
        let common_dir = root.join("native").join("common");
        if let Ok(entries) = std::fs::read_dir(common_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().map_or(false, |ext| ext == "swift") {
                    sources.push(p);
                }
            }
        }
        sources.sort();
        sources.push(root.join("native").join("eyes").join("Anchors.swift"));

        let bubbles_swift = root.join("native").join("wechat_bubbles.swift");

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        cmd.arg("-parse-as-library");
        cmd.arg(&bubbles_swift);
        for s in &sources {
            cmd.arg(s);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of wechat_bubbles failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn wechat_ocr_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("wechat_ocr");

        let ocr_swift = root.join("native").join("wechat_ocr.swift");
        let mut cmd = Command::new("swiftc");
        cmd.arg(&ocr_swift);
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of wechat_ocr failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

use std::sync::Mutex;

/// A Vision test that could not run here (load, no Vision, no swiftc). Locally it is reported and skipped;
/// with CI=true or WECHAT_STRICT_VISION=1 it fails, so a skipped test is never counted as a pass in CI.
fn skip_note(message: &str) {
    let strict = std::env::var("CI").map(|v| v == "true").unwrap_or(false)
        || std::env::var("WECHAT_STRICT_VISION")
            .map(|v| v == "1")
            .unwrap_or(false);
    if strict {
        panic!("{message} (strict mode: skipping is a failure)");
    }
    eprintln!("{message}");
}

static VISION_LOCK: Mutex<()> = Mutex::new(());

enum RunResult {
    Success(Value),
    ErrorJson(Value),
    Timeout,
    VisionBusy(String),
    Skipped(String),
}

fn run_bubbles(
    name: &str,
    window: Option<&str>,
    env: Option<&[(&str, &str)]>,
    timeout: Duration,
) -> RunResult {
    let _lock = VISION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let bin = match wechat_bubbles_bin() {
        Some(b) => b,
        None => return RunResult::Skipped("swiftc not available".into()),
    };

    let temp = TempDir::new("bubbles_run");
    let no_ocr = temp.path.join("no-ocr.json");
    let default_layout = default_layout_path();
    let default_win = default_window_json();
    let win_arg = window.unwrap_or(&default_win);

    let png_path = frames_dir().join(format!("{}.png", name));

    let stdout_path = temp.path.join("out.txt");
    let stderr_path = temp.path.join("err.txt");
    let stdout_file = File::create(&stdout_path).expect("create stdout file");
    let stderr_file = File::create(&stderr_path).expect("create stderr file");

    let mut cmd = Command::new(bin);
    cmd.arg(png_path.to_str().unwrap());
    cmd.arg(win_arg);

    cmd.env("WECHAT_OCR_CONFIG", no_ocr.to_str().unwrap());
    cmd.env("WECHAT_LAYOUT", default_layout.to_str().unwrap());
    cmd.env_remove("WECHAT_OCR_ENGINE");

    if let Some(entries) = env {
        for (k, v) in entries {
            cmd.env(k, v);
        }
    }

    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunResult::Skipped(format!("spawn failed: {}", e)),
    };

    let start = Instant::now();
    let mut exit_status = None;
    while start.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return RunResult::Skipped(format!("wait failed: {}", e)),
        }
    }

    if exit_status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return RunResult::Timeout;
    }

    let mut stdout_str = String::new();
    let mut stderr_str = String::new();
    let _ = File::open(&stdout_path).and_then(|mut f| f.read_to_string(&mut stdout_str));
    let _ = File::open(&stderr_path).and_then(|mut f| f.read_to_string(&mut stderr_str));

    // A deterministic refusal (`invalid("…")`, e.g. invalid_ocr_engine) is a real result the test must check,
    // even when it surfaces during warm-up; only a warm-up failure without a reason means Vision is busy.
    let deterministic = stderr_str.contains("invalid(\"");
    // e5rtError: the Neural Engine runtime failed transiently (seen 2026-11-08 under load); environmental.
    let environmental = stderr_str.contains("vision warm-up failed")
        || stderr_str.contains("ANECompilerService")
        || stderr_str.contains("e5rtError");
    if !deterministic && environmental {
        return RunResult::VisionBusy(stderr_str);
    }

    let status = exit_status.unwrap();
    if !status.success() {
        return RunResult::ErrorJson(serde_json::json!({
            "error": stderr_str.trim()
        }));
    }

    match serde_json::from_str::<Value>(&stdout_str) {
        Ok(val) => RunResult::Success(val),
        Err(e) => RunResult::ErrorJson(serde_json::json!({
            "error": format!("json decode error: {}, stdout: {}", e, stdout_str)
        })),
    }
}

struct ExpectedBubble {
    direction: &'static str,
    kind: &'static str,
    text: &'static str,
}

struct FrameCase {
    profile: &'static str,
    scale: Option<i64>,
    title: &'static str,
    bubbles: Vec<ExpectedBubble>,
}

fn get_top_bubbles() -> Vec<ExpectedBubble> {
    vec![
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "text",
            text: "你好",
        },
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "text",
            text: "在吗",
        },
        ExpectedBubble {
            direction: "outbound_visual",
            kind: "text",
            text: "客服助手",
        },
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "text",
            text: "你是美女吗",
        },
        ExpectedBubble {
            direction: "outbound_visual",
            kind: "text",
            text: "没有外貌",
        },
    ]
}

fn get_bottom_bubbles() -> Vec<ExpectedBubble> {
    vec![
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "file",
            text: "岗位信息更新2.13.pdf",
        },
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "text",
            text: "测试结束",
        },
        ExpectedBubble {
            direction: "inbound_visual",
            kind: "text",
            text: "浅色测试",
        },
    ]
}

fn get_frame_cases() -> HashMap<&'static str, FrameCase> {
    let mut m = HashMap::new();
    m.insert(
        "r1-dark-top",
        FrameCase {
            profile: "wechat-4-dark",
            scale: Some(1),
            title: "午夜咖啡",
            bubbles: get_top_bubbles(),
        },
    );
    m.insert(
        "r1-dark-bottom",
        FrameCase {
            profile: "wechat-4-dark",
            scale: Some(1),
            title: "午夜咖啡",
            bubbles: get_bottom_bubbles(),
        },
    );
    m.insert(
        "r1-light-top",
        FrameCase {
            profile: "wechat-4-light",
            scale: Some(1),
            title: "午夜咖啡",
            bubbles: get_top_bubbles(),
        },
    );
    m.insert(
        "r1-light-bottom",
        FrameCase {
            profile: "wechat-4-light",
            scale: Some(1),
            title: "午夜咖啡",
            bubbles: get_bottom_bubbles(),
        },
    );
    m.insert(
        "retina-dark-top",
        FrameCase {
            profile: "wechat-4-dark",
            scale: Some(2),
            title: "午夜咖啡",
            bubbles: get_top_bubbles(),
        },
    );
    m.insert(
        "retina-light-top",
        FrameCase {
            profile: "wechat-4-light",
            scale: Some(2),
            title: "午夜咖啡",
            bubbles: get_top_bubbles(),
        },
    );
    let mut retina_dark_bot = get_bottom_bubbles();
    retina_dark_bot.push(ExpectedBubble {
        direction: "inbound_visual",
        kind: "text",
        text: "你好",
    });
    m.insert(
        "retina-dark-bottom",
        FrameCase {
            profile: "wechat-4-dark",
            scale: Some(2),
            title: "午夜咖啡",
            bubbles: retina_dark_bot,
        },
    );
    let mut retina_light_bot = get_bottom_bubbles();
    retina_light_bot.push(ExpectedBubble {
        direction: "inbound_visual",
        kind: "text",
        text: "你好",
    });
    m.insert(
        "retina-light-bottom",
        FrameCase {
            profile: "wechat-4-light",
            scale: Some(2),
            title: "午夜咖啡",
            bubbles: retina_light_bot,
        },
    );
    m
}

#[test]
fn test_missing_layout_is_refused() {
    let res = run_bubbles(
        "r1-dark-top",
        None,
        Some(&[("WECHAT_LAYOUT", "/nonexistent.json")]),
        Duration::from_secs(30),
    );
    let frame = match res {
        RunResult::Success(v) | RunResult::ErrorJson(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_missing_layout_is_refused timed out under heavy load"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_missing_layout_is_refused Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
    };
    let err = frame.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        err.contains("layout_missing") || err.contains("layout_unreadable"),
        "error '{}' does not contain layout_missing or layout_unreadable",
        err
    );
    assert!(frame.get("bubbles").is_none());
}

#[test]
fn test_layout_drives_the_crop() {
    let temp = TempDir::new("crop_layout");
    let mut layout: Value =
        serde_json::from_str(&std::fs::read_to_string(default_layout_path()).unwrap()).unwrap();
    let current_h = layout["regions"]["chat_content"]["h"].as_i64().unwrap();
    layout["regions"]["chat_content"]["h"] = serde_json::json!(current_h - 200);

    let layout_path = temp.path.join("short-chat.json");
    std::fs::write(&layout_path, serde_json::to_string(&layout).unwrap()).unwrap();

    let res = run_bubbles(
        "r1-dark-bottom",
        None,
        Some(&[("WECHAT_LAYOUT", layout_path.to_str().unwrap())]),
        Duration::from_secs(30),
    );
    let frame = match res {
        RunResult::Success(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_layout_drives_the_crop timed out under heavy load"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_layout_drives_the_crop Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
        RunResult::ErrorJson(e) => panic!("unexpected error: {:?}", e),
    };
    assert!(frame.get("error").is_none());
    if let Some(bubbles) = frame.get("bubbles").and_then(|v| v.as_array()) {
        for b in bubbles {
            let t = b.get("text").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !t.contains("浅色测试"),
                "found '浅色测试' in bubble text: {}",
                t
            );
        }
    }
}

#[test]
fn test_frames_report_a_passing_layout_check() {
    let cases = get_frame_cases();
    for (name, case) in cases {
        let res = run_bubbles(name, None, None, Duration::from_secs(30));
        let frame = match res {
            RunResult::Success(v) => v,
            RunResult::Timeout => {
                skip_note(&format!(
                    "SKIPPED: test_frames_report_a_passing_layout_check timed out on {}",
                    name
                ));
                return;
            }
            RunResult::VisionBusy(s) => {
                skip_note(&format!(
                    "SKIPPED: test_frames_report_a_passing_layout_check Vision busy on {}: {}",
                    name, s
                ));
                return;
            }
            RunResult::Skipped(s) => {
                skip_note(&format!("SKIPPED: {}", s));
                return;
            }
            RunResult::ErrorJson(e) => panic!("error for {}: {:?}", name, e),
        };
        let check = frame.get("layout_check");
        assert!(check.is_some(), "missing layout_check in {}", name);
        let check = check.unwrap();
        assert_eq!(
            check.get("theme").and_then(|v| v.as_str()),
            Some(case.profile)
        );
        let divider = check["measured"]["input_divider"].as_f64().unwrap();
        let border = check["measured"]["list_border"].as_f64().unwrap();
        assert!(
            (divider - 496.0).abs() <= 1.0,
            "{} divider: {}",
            name,
            divider
        );
        assert!((border - 300.0).abs() <= 1.0, "{} border: {}", name, border);
    }
}

#[test]
fn test_layout_disagreeing_with_screen_is_refused() {
    let temp = TempDir::new("shifted_divider");
    let mut layout: Value =
        serde_json::from_str(&std::fs::read_to_string(default_layout_path()).unwrap()).unwrap();
    for region in &["chat", "chat_content"] {
        let h = layout["regions"][region]["h"].as_i64().unwrap();
        layout["regions"][region]["h"] = serde_json::json!(h - 30);
    }
    let input_y = layout["regions"]["input"]["y"].as_i64().unwrap();
    let input_h = layout["regions"]["input"]["h"].as_i64().unwrap();
    layout["regions"]["input"]["y"] = serde_json::json!(input_y - 30);
    layout["regions"]["input"]["h"] = serde_json::json!(input_h + 30);

    let path = temp.path.join("shifted-divider.json");
    std::fs::write(&path, serde_json::to_string(&layout).unwrap()).unwrap();

    for name in &["r1-light-bottom", "retina-dark-bottom"] {
        let res = run_bubbles(
            name,
            None,
            Some(&[("WECHAT_LAYOUT", path.to_str().unwrap())]),
            Duration::from_secs(30),
        );
        let frame = match res {
            RunResult::Success(v) | RunResult::ErrorJson(v) => v,
            RunResult::Timeout => {
                skip_note(&format!(
                    "SKIPPED: test_layout_disagreeing_with_screen_is_refused timed out on {}",
                    name
                ));
                continue;
            }
            RunResult::VisionBusy(s) => {
                skip_note(&format!(
                    "SKIPPED: test_layout_disagreeing_with_screen_is_refused Vision busy on {}: {}",
                    name, s
                ));
                continue;
            }
            RunResult::Skipped(s) => {
                skip_note(&format!("SKIPPED: {}", s));
                return;
            }
        };
        let err = frame.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            err.contains("layout_mismatch:"),
            "{}: error '{}' does not contain layout_mismatch:",
            name,
            err
        );
        assert!(
            err.contains("input_divider"),
            "{}: error '{}' does not contain input_divider",
            name,
            err
        );
        assert!(frame.get("bubbles").is_none());
    }
}

#[test]
fn test_any_file_card_is_a_file_not_text() {
    let res = run_bubbles("r1-light-xls-card", None, None, Duration::from_secs(30));
    let frame = match res {
        RunResult::Success(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_any_file_card_is_a_file_not_text timed out under heavy load"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_any_file_card_is_a_file_not_text Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
        RunResult::ErrorJson(e) => panic!("error: {:?}", e),
    };
    assert!(frame.get("error").is_none());
    let bubbles = frame["bubbles"].as_array().expect("bubbles array");
    let files: Vec<&Value> = bubbles.iter().filter(|b| b["kind"] == "file").collect();
    assert_eq!(files.len(), 1, "expected 1 file bubble, got: {:?}", bubbles);
    let filename = files[0]["filename"].as_str().unwrap().replace(' ', "");
    assert!(filename.ends_with("最新.xls"), "filename was: {}", filename);
    assert!(!bubbles.iter().any(|b| {
        b["kind"] == "text"
            && b["text"].as_str().map_or(false, |t| {
                let first_line = t.lines().next().unwrap_or("");
                first_line.contains(".xls") && t.contains("27.4M")
            })
    }));
}

#[test]
fn test_frames_parse_to_expected_bubbles() {
    let cases = get_frame_cases();
    for (name, case) in cases {
        let res = run_bubbles(name, None, None, Duration::from_secs(45));
        let frame = match res {
            RunResult::Success(v) => v,
            RunResult::Timeout => {
                skip_note(&format!(
                    "SKIPPED: test_frames_parse_to_expected_bubbles timed out on {}",
                    name
                ));
                return;
            }
            RunResult::VisionBusy(s) => {
                skip_note(&format!(
                    "SKIPPED: test_frames_parse_to_expected_bubbles Vision busy on {}: {}",
                    name, s
                ));
                return;
            }
            RunResult::Skipped(s) => {
                skip_note(&format!("SKIPPED: {}", s));
                return;
            }
            RunResult::ErrorJson(e) => panic!("error for {}: {:?}", name, e),
        };
        assert!(
            frame.get("error").is_none(),
            "frame {} had error: {:?}",
            name,
            frame.get("error")
        );
        assert_eq!(frame["profile"], case.profile);
        if let Some(scale) = case.scale {
            assert_eq!(frame["scale"], scale);
        }
        assert_eq!(frame["title_lines"], serde_json::json!([case.title]));
        assert_eq!(frame["clipped_bubble"], false);

        let bubbles = frame["bubbles"].as_array().unwrap();
        assert_eq!(
            bubbles.len(),
            case.bubbles.len(),
            "{} bubble count mismatch: got {} vs expected {}",
            name,
            bubbles.len(),
            case.bubbles.len()
        );
        for (b, exp) in bubbles.iter().zip(case.bubbles.iter()) {
            assert_eq!(b["direction"], exp.direction);
            assert_eq!(b["kind"], exp.kind);
            let text = b["text"].as_str().unwrap().replace('\n', "");
            assert!(
                text.contains(exp.text),
                "{} text '{}' does not contain expected '{}'",
                name,
                text,
                exp.text
            );
        }
    }
}

#[test]
#[ignore = "known gap: no image-bubble fixture committed yet (needs a redacted 测试群 screenshot with an image); never counted as a pass"]
fn test_image_bubbles_carry_phash() {
    let cases = get_frame_cases();
    let mut images = Vec::new();
    for (name, _) in cases {
        let res = run_bubbles(name, None, None, Duration::from_secs(30));
        let frame = match res {
            RunResult::Success(v) => v,
            RunResult::Timeout | RunResult::VisionBusy(_) => continue,
            RunResult::Skipped(_) => return,
            RunResult::ErrorJson(_) => continue,
        };
        if let Some(bubbles) = frame.get("bubbles").and_then(|v| v.as_array()) {
            for b in bubbles {
                if b["kind"] == "image" {
                    images.push(b.clone());
                }
            }
        }
    }
    if images.is_empty() {
        skip_note(&format!(
            "SKIPPED: test_image_bubbles_carry_phash (no image bubbles in committed fixtures yet)"
        ));
        return;
    }
    for b in images {
        let phash = b["phash"].as_str().unwrap();
        assert_eq!(phash.len(), 16);
        assert!(u64::from_str_radix(phash, 16).is_ok());
    }
}

#[test]
fn test_default_engine_reports_vision() {
    let res = run_bubbles("r1-dark-top", None, None, Duration::from_secs(30));
    let frame = match res {
        RunResult::Success(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_default_engine_reports_vision timed out under heavy load"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_default_engine_reports_vision Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
        RunResult::ErrorJson(e) => panic!("error: {:?}", e),
    };
    assert_eq!(
        frame["ocr"],
        serde_json::json!({"engine": "vision", "engine_used": "vision"})
    );
}

#[test]
fn test_invalid_engine_is_refused() {
    let temp = TempDir::new("bad_ocr");
    let bad_config = temp.path.join("bad-ocr.json");
    std::fs::write(&bad_config, "{\"engine\":\"paddle\"}").unwrap();

    let res1 = run_bubbles(
        "r1-dark-top",
        None,
        Some(&[("WECHAT_OCR_CONFIG", bad_config.to_str().unwrap())]),
        Duration::from_secs(30),
    );
    let frame1 = match res1 {
        RunResult::Success(v) | RunResult::ErrorJson(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_invalid_engine_is_refused timed out"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_invalid_engine_is_refused Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
    };
    assert!(
        frame1
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("invalid_ocr_engine"),
        "expected invalid_ocr_engine, got {:?}",
        frame1
    );

    let res2 = run_bubbles(
        "r1-dark-top",
        None,
        Some(&[("WECHAT_OCR_ENGINE", "paddle")]),
        Duration::from_secs(30),
    );
    let frame2 = match res2 {
        RunResult::Success(v) | RunResult::ErrorJson(v) => v,
        RunResult::Timeout => {
            skip_note(&format!(
                "SKIPPED: test_invalid_engine_is_refused timed out"
            ));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_invalid_engine_is_refused Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
    };
    assert!(
        frame2
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("invalid_ocr_engine"),
        "expected invalid_ocr_engine, got {:?}",
        frame2
    );
}

#[test]
fn test_unknown_theme_refused() {
    let res = run_bubbles("unknown-gray", None, None, Duration::from_secs(30));
    let frame = match res {
        RunResult::Success(v) | RunResult::ErrorJson(v) => v,
        RunResult::Timeout => {
            skip_note(&format!("SKIPPED: test_unknown_theme_refused timed out"));
            return;
        }
        RunResult::VisionBusy(s) => {
            skip_note(&format!(
                "SKIPPED: test_unknown_theme_refused Vision busy: {}",
                s
            ));
            return;
        }
        RunResult::Skipped(s) => {
            skip_note(&format!("SKIPPED: {}", s));
            return;
        }
    };
    let err = frame.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(err.contains("uncalibrated_theme"), "error was: {}", err);
}

#[test]
fn test_cli_rejects_invalid_engine() {
    let bin = match wechat_ocr_bin() {
        Some(b) => b,
        None => {
            skip_note(&format!(
                "SKIPPED: test_cli_rejects_invalid_engine swiftc not available"
            ));
            return;
        }
    };
    let temp = TempDir::new("ocr_cli_test");
    let no_ocr = temp.path.join("no-ocr.json");
    let ocr_tmp = temp.path.join("ocr-tmp");
    let frame_png = frames_dir().join("r1-dark-top.png");

    // The OCR CLI warms Vision too: serialize with the other Vision tests (parallel runs made the Neural
    // Engine fail with e5rtError, 2026-11-08).
    let _lock = VISION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Case 1: WECHAT_OCR_ENGINE="paddle"
    let out1 = Command::new(bin)
        .arg(frame_png.to_str().unwrap())
        .env("WECHAT_OCR_CONFIG", no_ocr.to_str().unwrap())
        .env("WECHAT_OCR_TMP", ocr_tmp.to_str().unwrap())
        .env("WECHAT_OCR_ENGINE", "paddle")
        .output()
        .expect("run ocr");
    assert!(!out1.status.success());
    let stderr1 = String::from_utf8_lossy(&out1.stderr);
    assert!(stderr1.contains("WECHAT_OCR_ENGINE"), "stderr: {}", stderr1);

    // Case 2: WECHAT_OCR_CONFIG bad.json
    let bad_config = temp.path.join("bad.json");
    std::fs::write(&bad_config, "{\"engine\":\"paddle\"}").unwrap();
    let mut cmd2 = Command::new(bin);
    cmd2.arg(frame_png.to_str().unwrap())
        .env("WECHAT_OCR_CONFIG", bad_config.to_str().unwrap())
        .env("WECHAT_OCR_TMP", ocr_tmp.to_str().unwrap())
        .env_remove("WECHAT_OCR_ENGINE");
    let out2 = cmd2.output().expect("run ocr");
    assert!(!out2.status.success());
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr2.contains("invalid_ocr_engine"),
        "stderr: {}",
        stderr2
    );
}
