//! Swift Action Module integration tests.
//! Migrated from `tests/test_action_module.py`.

use serde_json::Value;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const EPS: f64 = 1e-6;

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

fn action_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("action_selftest");

        let mut sources = Vec::new();
        let action_dir = root.join("native").join("action");
        if let Ok(entries) = std::fs::read_dir(action_dir) {
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
            .join("action_selftest")
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
                "swiftc compilation of action_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn wechat_click_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        std::fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("wechat_click");

        let mut sources = Vec::new();
        for dir in &[
            root.join("native").join("click"),
            root.join("native").join("action"),
        ] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().map_or(false, |ext| ext == "swift") {
                        sources.push(p);
                    }
                }
            }
        }
        sources.sort();

        let mut cmd = Command::new("swiftc");
        for s in &sources {
            cmd.arg(s);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of wechat_click failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_action_cli(args: &[&str]) -> Value {
    let bin = action_selftest_bin().expect("action_selftest binary not available");
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute action_selftest");
    assert!(
        output.status.success(),
        "action_selftest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("invalid json output from action_selftest")
}

fn get_action_samples() -> &'static Vec<Value> {
    static SAMPLES: OnceLock<Vec<Value>> = OnceLock::new();
    SAMPLES.get_or_init(|| {
        let val = run_action_cli(&["clicks", "5000", "7"]);
        val.as_array()
            .expect("clicks samples must be a json array")
            .clone()
    })
}

fn run_scenario(name: &str, seed: u64) -> Value {
    let seed_str = seed.to_string();
    let bin = action_selftest_bin().expect("action_selftest binary not available");
    let output = Command::new(bin)
        .args(["exec", name, &seed_str])
        .output()
        .expect("failed to execute action_selftest scenario");
    assert!(
        output.status.success(),
        "scenario {} failed: {}",
        name,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("invalid json output from scenario")
}

fn get_kinds(out: &Value) -> Vec<String> {
    out["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect()
}

fn inside(point: (f64, f64), rect: &Value) -> bool {
    let (x, y) = point;
    let rx = rect["x"].as_f64().unwrap();
    let ry = rect["y"].as_f64().unwrap();
    let rw = rect["w"].as_f64().unwrap();
    let rh = rect["h"].as_f64().unwrap();
    rx - EPS <= x && x <= rx + rw + EPS && ry - EPS <= y && y <= ry + rh + EPS
}

fn plan_wechat_click(args: &[&str]) -> (i32, Option<Value>, String) {
    let bin = wechat_click_bin().expect("wechat_click binary not available");
    let mut cmd = Command::new(bin);
    cmd.arg("--plan-only");
    cmd.args(args);
    let output = cmd.output().expect("failed to execute wechat_click");
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let val = if code == 0 {
        Some(serde_json::from_slice(&output.stdout).expect("invalid json from wechat_click"))
    } else {
        None
    };
    (code, val, stderr)
}

fn plan_wechat_click_with_stdin(
    args: &[&str],
    input_str: &str,
    envs: Option<&[(&str, &str)]>,
) -> (i32, Option<Value>, String) {
    let bin = wechat_click_bin().expect("wechat_click binary not available");
    let mut cmd = Command::new(bin);
    cmd.arg("--plan-only");
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    if let Some(env_list) = envs {
        for (k, v) in env_list {
            cmd.env(k, v);
        }
    }
    let mut child = cmd.spawn().expect("failed to spawn wechat_click");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input_str.as_bytes());
    }
    let output = child
        .wait_with_output()
        .expect("failed to wait on wechat_click");
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let val = serde_json::from_slice(&output.stdout).ok();
    (code, val, stderr)
}

fn get_press(events: &[Value]) -> &Value {
    let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(&kinds[kinds.len() - 2..], &["left_down", "left_up"]);
    &events[events.len() - 2]
}

fn get_keys(combo: &[&str]) -> Vec<(String, i64, bool)> {
    let mut args = vec!["key"];
    args.extend_from_slice(combo);
    let (code, out, stderr) = plan_wechat_click(&args);
    assert_eq!(code, 0, "key plan failed: {}", stderr);
    let events = out.unwrap()["events"].as_array().unwrap().clone();
    events
        .iter()
        .map(|e| {
            (
                e["type"].as_str().unwrap().to_string(),
                e["code"].as_i64().unwrap(),
                e["cmd"].as_bool().unwrap(),
            )
        })
        .collect()
}

// ============================================================================
// ActionModuleTests (7 tests)
// ============================================================================

#[test]
fn test_press_always_inside_effective_region_or_refused() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    let mut kinds = HashSet::new();
    for s in samples {
        kinds.insert(s["kind"].as_str().unwrap());
        if s["region"].is_null() {
            assert!(s["press"].is_null(), "{:?}", s);
        } else {
            assert!(!s["press"].is_null(), "{:?}", s);
            let px = s["press"][0].as_f64().unwrap();
            let py = s["press"][1].as_f64().unwrap();
            assert!(inside((px, py), &s["region"]), "{:?}", s);
        }
    }
    let expected: HashSet<&str> = [
        "normal",
        "tiny",
        "half_off_screen",
        "start_inside",
        "off_screen",
    ]
    .into_iter()
    .collect();
    assert_eq!(kinds, expected);
    assert!(samples.iter().any(|s| s["region"].is_null()));
}

#[test]
fn test_peak_speed_in_first_half() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    let moves: Vec<_> = samples
        .iter()
        .filter(|s| !s["press"].is_null() && s["distance"].as_f64().unwrap() >= 30.0)
        .collect();
    let early = moves
        .iter()
        .filter(|s| s["peak_ratio"].as_f64().unwrap() < 0.5)
        .count();
    assert!((early as f64) / (moves.len() as f64) >= 0.95);
}

#[test]
fn test_duration_follows_fitts() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    for s in samples {
        if s["press"].is_null() {
            continue;
        }
        let dur = s["duration"].as_f64().unwrap();
        let fitts = s["fitts"].as_f64().unwrap();
        assert!(dur >= 0.5 * fitts - EPS, "{:?}", s);
        assert!(dur <= 2.0 * fitts + EPS, "{:?}", s);
    }
    let hard: Vec<f64> = samples
        .iter()
        .filter(|s| {
            !s["press"].is_null()
                && s["distance"].as_f64().unwrap() > 600.0
                && s["kind"].as_str().unwrap() == "tiny"
        })
        .map(|s| s["duration"].as_f64().unwrap())
        .collect();
    let easy: Vec<f64> = samples
        .iter()
        .filter(|s| {
            !s["press"].is_null()
                && s["distance"].as_f64().unwrap() < 150.0
                && s["target_width"].as_f64().unwrap() > 60.0
        })
        .map(|s| s["duration"].as_f64().unwrap())
        .collect();
    assert!(!hard.is_empty() && !easy.is_empty());
    let hard_avg: f64 = hard.iter().sum::<f64>() / hard.len() as f64;
    let easy_avg: f64 = easy.iter().sum::<f64>() / easy.len() as f64;
    assert!(hard_avg > easy_avg);
}

#[test]
fn test_tremor_bounded_and_vanishes() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    for s in samples {
        if s["press"].is_null() {
            continue;
        }
        let max_dev = s["max_deviation"].as_f64().unwrap();
        let tremor = s["tremor"].as_f64().unwrap();
        let drift = s["drift"].as_f64().unwrap();
        let tail_dev = s["tail_deviation"].as_f64().unwrap();
        assert!(max_dev <= 2.0 * tremor + drift + EPS, "{:?}", s);
        assert!(tail_dev <= 0.5, "{:?}", s);
    }
}

#[test]
fn test_points_stay_on_screen_and_no_nan() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    let mut negative = 0;
    for s in samples {
        assert!(s["finite"].as_bool().unwrap(), "{:?}", s);
        assert!(s["on_screen"].as_bool().unwrap(), "{:?}", s);
        if s["screen"]["x"].as_f64().unwrap() < 0.0 {
            negative += 1;
        }
    }
    assert!(
        negative > 0,
        "some samples use a screen left of the main display"
    );
}

#[test]
fn test_small_targets_need_more_corrections() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let samples = get_action_samples();
    let tiny: Vec<f64> = samples
        .iter()
        .filter(|s| !s["press"].is_null() && s["kind"].as_str().unwrap() == "tiny")
        .map(|s| s["corrections"].as_f64().unwrap())
        .collect();
    let large: Vec<f64> = samples
        .iter()
        .filter(|s| {
            !s["press"].is_null()
                && s["target_width"].as_f64().unwrap() >= 120.0
                && s["target_height"].as_f64().unwrap() >= 40.0
        })
        .map(|s| s["corrections"].as_f64().unwrap())
        .collect();
    assert!(!tiny.is_empty() && !large.is_empty());
    let tiny_avg: f64 = tiny.iter().sum::<f64>() / tiny.len() as f64;
    let large_avg: f64 = large.iter().sum::<f64>() / large.len() as f64;
    assert!(tiny_avg > large_avg);
    let max_corr = samples
        .iter()
        .map(|s| s["corrections"].as_i64().unwrap())
        .max()
        .unwrap();
    assert!(max_corr <= 3);
    assert!(tiny_avg < 2.5);
}

#[test]
fn test_same_seed_same_plan() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    assert_eq!(
        run_action_cli(&["clicks", "50", "11"]),
        run_action_cli(&["clicks", "50", "11"])
    );
    assert_ne!(
        run_action_cli(&["clicks", "50", "11"]),
        run_action_cli(&["clicks", "50", "12"])
    );
}

// ============================================================================
// ExecutorTests (10 tests)
// ============================================================================

#[test]
fn test_click_event_order() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for seed in 1..40 {
        let out = run_scenario("click", seed);
        assert!(out["error"].is_null());
        let kinds = get_kinds(&out);
        let len = kinds.len();
        assert_eq!(&kinds[len - 2..], &["left_down", "left_up"]);
        let prefix = &kinds[..len - 2];
        assert!(
            prefix.iter().all(|k| k == "moved") && kinds.len() > 3,
            "{:?}",
            kinds
        );
        let events = out["events"].as_array().unwrap();
        let down = &events[len - 2];
        let up = &events[len - 1];
        assert!(inside(
            (down["x"].as_f64().unwrap(), down["y"].as_f64().unwrap()),
            &out["region"]
        ));
        assert_eq!(
            (down["x"].as_f64().unwrap(), down["y"].as_f64().unwrap()),
            (up["x"].as_f64().unwrap(), up["y"].as_f64().unwrap())
        );
        let dt = up["t"].as_f64().unwrap() - down["t"].as_f64().unwrap();
        assert!(0.06 - EPS <= dt && dt <= 0.12 + EPS);
    }
}

#[test]
fn test_unreachable_target_never_presses() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("unreachable", 3);
    assert_eq!(out["error"].as_str().unwrap(), "target_not_reached");
    let kinds = get_kinds(&out);
    assert!(!kinds.contains(&"left_down".to_string()) && !kinds.contains(&"left_up".to_string()));
}

#[test]
fn test_user_move_aborts() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("interfere_move", 3);
    assert_eq!(out["error"].as_str().unwrap(), "user_input_detected");
    let kinds = get_kinds(&out);
    assert!(!kinds.contains(&"left_down".to_string()));
}

#[test]
fn test_user_move_between_down_and_up_still_releases() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("interfere_down", 3);
    assert_eq!(out["error"].as_str().unwrap(), "user_input_detected");
    let kinds = get_kinds(&out);
    let len = kinds.len();
    assert_eq!(&kinds[len - 2..], &["left_down", "left_up"]);
}

#[test]
fn test_pointer_moved_between_actions_is_not_user_input() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("moved_between_actions", 3);
    assert!(out["error"].is_null());
    let events = out["events"].as_array().unwrap();
    let count_down = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "left_down")
        .count();
    assert_eq!(count_down, 2);
    let moved_events: Vec<_> = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "moved")
        .collect();
    let second = moved_events.last().unwrap();
    let sx = second["x"].as_f64().unwrap();
    let sy = second["y"].as_f64().unwrap();
    assert!(
        200.0 <= sx && sx <= 280.0 && 200.0 <= sy && sy <= 230.0,
        "{:?}",
        second
    );
}

#[test]
fn test_pointer_reported_late_is_not_user_input() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for seed in 1..30 {
        let out = run_scenario("lagging_cursor", seed);
        assert!(out["error"].is_null(), "failed on seed {}", seed);
    }
    let out = run_scenario("lagging_cursor_interfere", 3);
    assert_eq!(out["error"].as_str().unwrap(), "user_input_detected");
}

#[test]
fn test_instant_profile_single_move() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("instant", 3);
    assert!(out["error"].is_null());
    let kinds = get_kinds(&out);
    assert_eq!(kinds, vec!["moved", "left_down", "left_up"]);
    let events = out["events"].as_array().unwrap();
    let first = &events[0];
    assert!(inside(
        (first["x"].as_f64().unwrap(), first["y"].as_f64().unwrap()),
        &out["region"]
    ));
}

#[test]
fn test_scroll_keeps_tick_size() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("scroll", 3);
    assert!(out["error"].is_null());
    let events = out["events"].as_array().unwrap();
    let ticks: Vec<_> = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "scroll")
        .collect();
    let lines: Vec<_> = ticks.iter().map(|t| t["lines"].as_i64().unwrap()).collect();
    assert_eq!(lines, vec![10, 10, 10]);
    for tick in &ticks {
        assert!(inside(
            (tick["x"].as_f64().unwrap(), tick["y"].as_f64().unwrap()),
            &out["region"]
        ));
    }
    for i in 0..ticks.len() - 1 {
        let a = &ticks[i];
        let b = &ticks[i + 1];
        let dt = b["t"].as_f64().unwrap() - a["t"].as_f64().unwrap();
        assert!(0.08 - EPS <= dt && dt <= 0.2 + EPS);
    }
    let kinds = get_kinds(&out);
    assert!(!kinds.contains(&"left_down".to_string()));
}

#[test]
fn test_press_stays_inside_the_container() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for seed in 1..60 {
        let out = run_scenario("within", seed);
        let events = out["events"].as_array().unwrap();
        let down = events
            .iter()
            .find(|e| e["type"].as_str().unwrap() == "left_down")
            .unwrap();
        assert!(
            inside(
                (down["x"].as_f64().unwrap(), down["y"].as_f64().unwrap()),
                &out["region"]
            ),
            "{:?}",
            (seed, down)
        );
    }
    let disjoint = run_scenario("within_disjoint", 3);
    assert_eq!(disjoint["error"].as_str().unwrap(), "target_not_reached");
}

#[test]
fn test_right_click_uses_right_button() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let out = run_scenario("right_click", 3);
    assert!(out["error"].is_null());
    let kinds = get_kinds(&out);
    let len = kinds.len();
    assert_eq!(&kinds[len - 2..], &["right_down", "right_up"]);
    let events = out["events"].as_array().unwrap();
    let down = &events[len - 2];
    assert_eq!(
        out["pressed"],
        serde_json::json!([down["x"].as_f64().unwrap(), down["y"].as_f64().unwrap()])
    );
}

// ============================================================================
// WechatClickCliTests (12 tests)
// ============================================================================

#[test]
fn test_point_form_clicks_within_a_small_square() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for _ in 0..10 {
        let (code, out, stderr) = plan_wechat_click(&["500", "400"]);
        assert_eq!(code, 0, "stderr: {}", stderr);
        let events = out.unwrap()["events"].as_array().unwrap().clone();
        let down = get_press(&events);
        assert!((down["x"].as_f64().unwrap() - 500.0).abs() <= 4.0 + EPS);
        assert!((down["y"].as_f64().unwrap() - 400.0).abs() <= 4.0 + EPS);
    }
}

#[test]
fn test_size_form_uses_the_effective_region() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for _ in 0..10 {
        let (code, out, stderr) = plan_wechat_click(&["500", "400", "200", "40"]);
        assert_eq!(code, 0, "stderr: {}", stderr);
        let events = out.unwrap()["events"].as_array().unwrap().clone();
        let down = get_press(&events);
        let dx = down["x"].as_f64().unwrap();
        let dy = down["y"].as_f64().unwrap();
        assert!(
            440.0 - EPS <= dx && dx <= 560.0 + EPS && 388.0 - EPS <= dy && dy <= 412.0 + EPS,
            "{:?}",
            down
        );
    }
}

#[test]
fn test_key_cmd_v_presses_command_around_v() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    assert_eq!(
        get_keys(&["cmd+v"]),
        vec![
            ("key_down".into(), 55, true),
            ("key_down".into(), 9, true),
            ("key_up".into(), 9, true),
            ("key_up".into(), 55, false),
        ]
    );
    assert_eq!(get_keys(&["cmd+a"])[1], ("key_down".into(), 0, true));
    assert_eq!(get_keys(&["cmd+c"])[1], ("key_down".into(), 8, true));
}

#[test]
fn test_key_return_has_no_modifier() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    assert_eq!(
        get_keys(&["return"]),
        vec![("key_down".into(), 36, false), ("key_up".into(), 36, false),]
    );
}

#[test]
fn test_key_rejects_unknown_combo() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let combos: &[&[&str]] = &[&["cmd+q"], &["x"], &[], &["cmd+v", "extra"]];
    for combo in combos {
        let mut args = vec!["key"];
        args.extend_from_slice(combo);
        let (code, _, _) = plan_wechat_click(&args);
        assert_ne!(code, 0, "{:?}", combo);
    }
}

#[test]
fn test_invalid_arguments_fail() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let cases: &[&[&str]] = &[
        &["500"],
        &["a", "b"],
        &["500", "400", "-3", "10"],
        &["500", "400", "10"],
    ];
    for args in cases {
        let (code, _, _) = plan_wechat_click(args);
        assert_ne!(code, 0, "{:?}", args);
    }
}

#[test]
fn test_input_text_plan_replaces_then_reads_back() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let (code, out, stderr) =
        plan_wechat_click_with_stdin(&["input-text", "500", "600"], "你好\n第二行", None);
    assert_eq!(code, 0, "stderr: {}", stderr);
    let out = out.unwrap();
    let events = out["events"].as_array().unwrap();
    let downs: Vec<i64> = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "key_down" && e["code"].as_i64().unwrap() != 55)
        .map(|e| e["code"].as_i64().unwrap())
        .collect();
    assert_eq!(downs, vec![0, 9, 0, 8]); // ⌘A ⌘V ⌘A ⌘C
    assert_eq!(events[0]["type"].as_str().unwrap(), "moved");
    let pasteboard = out["pasteboard"].as_array().unwrap();
    let ops: Vec<&str> = pasteboard
        .iter()
        .map(|p| p["op"].as_str().unwrap())
        .collect();
    assert_eq!(
        ops,
        vec!["save", "write_text", "write_sentinel", "read", "restore"]
    );
}

#[test]
fn test_input_text_mismatch_clears_and_exits_4_without_text() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let (code, out, stderr) = plan_wechat_click_with_stdin(
        &["input-text", "500", "600"],
        "秘密正文",
        Some(&[("WECHAT_PLAN_READBACK", "mismatch")]),
    );
    assert_eq!(code, 4);
    assert!(stderr.contains("input_readback_mismatch"));
    assert!(!stderr.contains("秘密正文"));
    let out = out.unwrap();
    let events = out["events"].as_array().unwrap();
    let downs: Vec<i64> = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "key_down" && e["code"].as_i64().unwrap() != 55)
        .map(|e| e["code"].as_i64().unwrap())
        .collect();
    assert_eq!(&downs[downs.len() - 2..], &[0, 51]); // ⌘A ⌫ at the end
    let pasteboard = out["pasteboard"].as_array().unwrap();
    let restore_count = pasteboard
        .iter()
        .filter(|p| p["op"].as_str().unwrap() == "restore")
        .count();
    assert_eq!(restore_count, 1);
}

#[test]
fn test_input_text_sentinel_reports_copy_missed() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let (code, _, stderr) = plan_wechat_click_with_stdin(
        &["input-text", "500", "600"],
        "你好",
        Some(&[("WECHAT_PLAN_READBACK", "sentinel")]),
    );
    assert_eq!(code, 4);
    assert!(stderr.contains("复制未取到输入框内容，应为 2 字"));
}

#[test]
fn test_input_text_rejects_empty_or_huge_body() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    for body in &["".to_string(), "x".repeat(16385)] {
        let (code, _, _) = plan_wechat_click_with_stdin(&["input-text", "500", "600"], body, None);
        assert_eq!(code, 1, "failed on length {}", body.len());
    }
}

#[test]
fn test_clear_input_plan() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let (code, out, stderr) = plan_wechat_click(&["clear-input"]);
    assert_eq!(code, 0, "stderr: {}", stderr);
    let events = out.unwrap()["events"].as_array().unwrap().clone();
    let downs: Vec<i64> = events
        .iter()
        .filter(|e| e["type"].as_str().unwrap() == "key_down" && e["code"].as_i64().unwrap() != 55)
        .map(|e| e["code"].as_i64().unwrap())
        .collect();
    assert_eq!(downs, vec![0, 51]);
}

#[test]
fn test_new_key_combos() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    assert_eq!(
        get_keys(&["delete"]),
        vec![("key_down".into(), 51, false), ("key_up".into(), 51, false)]
    );
    assert_eq!(
        get_keys(&["escape"]),
        vec![("key_down".into(), 53, false), ("key_up".into(), 53, false)]
    );
    assert_eq!(get_keys(&["cmd+1"])[1], ("key_down".into(), 18, true));
}
