//! Common layout format, validation and fallback tests.
//! Migrated from `tests/test_common_layout.py`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
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

fn layout_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("layout_selftest");

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
        let main_swift = root.join("tests").join("common_selftest").join("layout_main.swift");

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        for s in &sources {
            cmd.arg(s);
        }
        cmd.arg(&main_swift);
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of layout_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_cli(args: &[&str]) -> Option<Value> {
    let bin = match layout_selftest_bin() {
        Some(b) => b,
        None => {
            eprintln!("swiftc not available, skipping test");
            return None;
        }
    };
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute layout_selftest");
    assert!(
        output.status.success(),
        "layout_selftest failed (exit code {:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let val: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid json output from layout_selftest");
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
fn test_default_layout_loads() {
    let default_path = repo_root().join("native").join("layout").join("wechat-4.1.15.json");
    let res = match run_cli(&["layout", default_path.to_str().unwrap()]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res["ok"], true);
    assert_eq!(res["schema"], 1);
    assert_eq!(res["wechat_version"], "4.1.15");
    assert_json_num_eq(&res["window"], &serde_json::json!({"width": 880.0, "height": 640.0}));
    assert_json_num_eq(&res["chat"], &serde_json::json!({"x": 315.0, "y": 60.0, "w": 565.0, "h": 436.0}));
    assert_json_num_eq(&res["session_list"], &serde_json::json!({"x": 60.0, "y": 60.0, "w": 240.0, "h": 580.0}));
    assert_json_num_eq(&res["title"], &serde_json::json!({"x": 250.0, "y": 0.0, "w": 620.0, "h": 50.0}));
    assert_json_num_eq(&res["input"], &serde_json::json!({"x": 315.0, "y": 496.0, "w": 565.0, "h": 144.0}));
    assert_eq!(res["points_per_tick"].as_f64(), Some(90.0));
    assert_eq!(res["lines_per_tick"].as_i64(), Some(10));
    assert_eq!(res["anchors_count"].as_i64(), Some(3));
    let save_as = res["menu_labels"]["save_as"].as_array().expect("save_as list");
    assert!(save_as.iter().any(|v| v.as_str() == Some("另存为…")));
}

#[test]
fn test_invalid_layouts_are_refused() {
    let default_path = repo_root().join("native").join("layout").join("wechat-4.1.15.json");
    if !default_path.exists() {
        return;
    }
    let valid_data: Value = serde_json::from_str(&std::fs::read_to_string(&default_path).unwrap()).unwrap();

    let temp = TempDir::new("layout_invalid");
    let tpath = &temp.path;

    // 1. Missing required region 'chat'
    {
        let mut d = valid_data.clone();
        d["regions"].as_object_mut().unwrap().remove("chat");
        let fpath = tpath.join("missing_chat.json");
        std::fs::write(&fpath, serde_json::to_string(&d).unwrap()).unwrap();
        let res = run_cli(&["layout", fpath.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_missing_region:chat");
    }

    // 2. Region exceeds window bounds (chat.w + chat.x > 880)
    {
        let mut d = valid_data.clone();
        d["regions"]["chat"]["w"] = serde_json::json!(600.0);
        let fpath = tpath.join("out_of_bounds.json");
        std::fs::write(&fpath, serde_json::to_string(&d).unwrap()).unwrap();
        let res = run_cli(&["layout", fpath.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_region_out_of_window:chat");
    }

    // 3. Region width is zero
    {
        let mut d = valid_data.clone();
        d["regions"]["chat"]["w"] = serde_json::json!(0.0);
        let fpath = tpath.join("zero_width.json");
        std::fs::write(&fpath, serde_json::to_string(&d).unwrap()).unwrap();
        let res = run_cli(&["layout", fpath.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_invalid_value:chat.w");
    }

    // 4. Unsupported schema version
    {
        let mut d = valid_data.clone();
        d["schema"] = serde_json::json!(2);
        let fpath = tpath.join("unsupported_schema.json");
        std::fs::write(&fpath, serde_json::to_string(&d).unwrap()).unwrap();
        let res = run_cli(&["layout", fpath.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_schema_unsupported");
    }

    // 5. Invalid scroll points_per_tick <= 0
    {
        let mut d = valid_data.clone();
        d["scroll"]["points_per_tick"] = serde_json::json!(0);
        let fpath = tpath.join("zero_scroll_step.json");
        std::fs::write(&fpath, serde_json::to_string(&d).unwrap()).unwrap();
        let res = run_cli(&["layout", fpath.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_invalid_value:scroll.points_per_tick");
    }

    // 6. Non-JSON file
    {
        let non_json_path = tpath.join("not_json.json");
        std::fs::write(&non_json_path, "THIS IS NOT JSON {{{").unwrap();
        let res = run_cli(&["layout", non_json_path.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_unreadable");
    }

    // 7. File > 256KB (300KB)
    {
        let large_path = tpath.join("large.json");
        let content = serde_json::to_vec(&valid_data).unwrap();
        let pad_size = if content.len() < 300 * 1024 {
            300 * 1024 - content.len()
        } else {
            1024
        };
        let mut large_content = content;
        large_content.resize(large_content.len() + pad_size, b' ');
        std::fs::write(&large_path, large_content).unwrap();
        let res = run_cli(&["layout", large_path.to_str().unwrap()]).unwrap();
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "layout_too_large");
    }
}

#[test]
fn test_version_memo_survives_flaky_lookup() {
    let res = match run_cli(&[
        "memo",
        "100:4.1.15",
        "none",
        "100:-",
        "200:-",
        "none",
        "200:4.2.0",
        "gone",
    ]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(
        res["resolved"],
        serde_json::json!(["4.1.15", "4.1.15", "4.1.15", null, null, "4.2.0", null])
    );
}

#[test]
fn test_version_memo_needs_a_first_reading() {
    let res = match run_cli(&["memo", "none", "100:-"]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(res["resolved"], serde_json::json!([null, null]));
}

#[test]
fn test_version_read_from_the_process_executable() {
    let temp = TempDir::new("bundle_test");
    let app = temp.path.join("Apps").join("WeChat.app");
    let macos_dir = app.join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos_dir).unwrap();

    let plist_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleShortVersionString</key>
    <string>4.1.15</string>
</dict>
</plist>"#;
    std::fs::write(app.join("Contents").join("Info.plist"), plist_content).unwrap();

    let exe_path = macos_dir.join("WeChat");
    let res = match run_cli(&["bundle", exe_path.to_str().unwrap()]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(
        res,
        serde_json::json!({
            "bundle": app.to_str().unwrap(),
            "version": "4.1.15"
        })
    );
}

#[test]
fn test_executable_path_of_a_live_process() {
    let res = match run_cli(&["exe"]) {
        Some(r) => r,
        None => return,
    };
    let returned_path = PathBuf::from(res["path"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    let expected_bin = layout_selftest_bin().unwrap().canonicalize().unwrap();
    assert_eq!(returned_path, expected_bin);
}

#[test]
fn test_executable_outside_an_app_has_no_version() {
    let res = match run_cli(&["bundle", "/usr/bin/true"]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(
        res,
        serde_json::json!({
            "bundle": null,
            "version": null
        })
    );
}

#[test]
fn test_file_card_identity_ignores_ocr_spacing() {
    let a = match run_cli(&["filecard", "耳机报价参数2024最新.xls", "27.4M"]) {
        Some(r) => r,
        None => return,
    };
    let b = run_cli(&["filecard", "耳机报价参数2024 最新.xls", "27.4 M"]).unwrap();
    let c = run_cli(&["filecard", "耳机报价参数2025最新.xls", "27.4M"]).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn test_version_fallback_picks_nearest_lower() {
    let temp = TempDir::new("version_fallback");
    let p = &temp.path;
    std::fs::write(p.join("wechat-4.1.12.json"), "{}").unwrap();
    std::fs::write(p.join("wechat-4.1.15.json"), "{}").unwrap();
    std::fs::write(p.join("wechat-4.2.0.json"), "{}").unwrap();

    // Exact match
    let res1 = match run_cli(&["fallback", p.to_str().unwrap(), "4.1.15"]) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(
        res1["path"],
        serde_json::json!(p.join("wechat-4.1.15.json").to_str().unwrap())
    );

    // Nearest lower version (4.1.20 -> 4.1.15)
    let res2 = run_cli(&["fallback", p.to_str().unwrap(), "4.1.20"]).unwrap();
    assert_eq!(
        res2["path"],
        serde_json::json!(p.join("wechat-4.1.15.json").to_str().unwrap())
    );

    // No version <= 4.0.1 -> null
    let res3 = run_cli(&["fallback", p.to_str().unwrap(), "4.0.1"]).unwrap();
    assert!(res3["path"].is_null());
}
