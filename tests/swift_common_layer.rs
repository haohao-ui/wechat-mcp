//! Common geometry and coordinate conversion layer integration tests.
//! Migrated from `tests/test_common_layer.py`.

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

fn common_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("common_selftest");

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
        let main_swift = root
            .join("tests")
            .join("common_selftest")
            .join("main.swift");

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
                "swiftc compilation of common_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_cli(case_name: &str) -> Option<Value> {
    let bin = match common_selftest_bin() {
        Some(b) => b,
        None => {
            eprintln!("swiftc not available, skipping test");
            return None;
        }
    };
    let output = Command::new(bin)
        .args(["geometry", case_name])
        .output()
        .expect("failed to execute common_selftest");
    assert!(
        output.status.success(),
        "common_selftest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let val: Value =
        serde_json::from_slice(&output.stdout).expect("invalid json output from common_selftest");
    Some(val)
}

fn assert_json_eq(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            assert_eq!(a.len(), e.len(), "keys count mismatch: {:?} vs {:?}", a, e);
            for (k, ev) in e {
                let av = a
                    .get(k)
                    .unwrap_or_else(|| panic!("missing key {} in {:?}", k, a));
                assert_json_eq(av, ev);
            }
        }
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(
                a.len(),
                e.len(),
                "array length mismatch: {:?} vs {:?}",
                a,
                e
            );
            for (av, ev) in a.iter().zip(e.iter()) {
                assert_json_eq(av, ev);
            }
        }
        (Value::Number(a), Value::Number(e)) => {
            let af = a.as_f64().expect("valid f64");
            let ef = e.as_f64().expect("valid f64");
            assert!((af - ef).abs() < 1e-6, "number mismatch: {} vs {}", af, ef);
        }
        _ => {
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn test_window_screen_round_trip() {
    let data = match run_cli("window_screen_round_trip") {
        Some(d) => d,
        None => return,
    };
    assert_json_eq(
        &data["screen"],
        &serde_json::json!({"x": 415.0, "y": 110.0}),
    );
    assert_json_eq(
        &data["round_trip"],
        &serde_json::json!({"x": 315.0, "y": 60.0}),
    );
}

#[test]
fn test_negative_display_origin() {
    let data = match run_cli("negative_display_origin") {
        Some(d) => d,
        None => return,
    };
    assert_json_eq(
        &data["screen_point"],
        &serde_json::json!({"x": -1700.0, "y": -100.0}),
    );
    assert_json_eq(
        &data["round_trip"],
        &serde_json::json!({"x": 100.0, "y": 20.0}),
    );
    assert_json_eq(
        &data["matched_display"],
        &serde_json::json!({"x": -1920.0, "y": -180.0, "w": 1920.0, "h": 1080.0}),
    );
}

#[test]
fn test_shared_display_edge_belongs_to_one_display() {
    let data = match run_cli("shared_display_edge") {
        Some(d) => d,
        None => return,
    };
    assert_json_eq(
        &data["edge"],
        &serde_json::json!({"x": 0.0, "y": 0.0, "w": 1440.0, "h": 900.0}),
    );
    assert!(data["beyond"].is_null());
}

#[test]
fn test_retina_crop_matches_points() {
    let data = match run_cli("retina_crop_matches_points") {
        Some(d) => d,
        None => return,
    };
    assert_json_eq(
        &data["scale2_crop"],
        &serde_json::json!({"x": 630.0, "y": 120.0, "w": 1130.0, "h": 872.0}),
    );
    assert_json_eq(
        &data["scale1_crop"],
        &serde_json::json!({"x": 315.0, "y": 60.0, "w": 565.0, "h": 436.0}),
    );
    assert_json_eq(
        &data["screen_rect"],
        &serde_json::json!({"x": 315.0, "y": 60.0, "w": 565.0, "h": 436.0}),
    );
}

#[test]
fn test_rejects_bad_scale() {
    let data = match run_cli("rejects_bad_scale") {
        Some(d) => d,
        None => return,
    };
    assert_eq!(data["scale0_is_nil"], true);
    assert_eq!(data["scale3_is_nil"], true);
    assert_eq!(data["scale_neg_is_nil"], true);
    assert_eq!(data["scale1_is_not_nil"], true);
    assert_eq!(data["scale2_is_not_nil"], true);
}

#[test]
fn test_rect_contains() {
    let data = match run_cli("rect_contains") {
        Some(d) => d,
        None => return,
    };
    assert_eq!(data["inside"], true);
    assert_eq!(data["on_edge"], true);
    assert_eq!(data["outside"], false);
}

#[test]
fn test_displays() {
    let data = match run_cli("displays") {
        Some(d) => d,
        None => return,
    };
    assert!(data.get("count").is_some());
    assert!(data.get("displays").map_or(false, |v| v.is_array()));
}
