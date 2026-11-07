//! Swift Host Component integration tests.
//! Migrated from:
//! - `tests/test_host_status.py`
//! - `tests/test_host_supervisor.py`
//! - `tests/test_host_core.py`

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn host_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("host_selftest");

        let files_to_compile = [
            root.join("native").join("bootstrap").join("Supervisor.swift"),
            root.join("native").join("bootstrap").join("CoreSupervisor.swift"),
            root.join("native").join("bootstrap").join("SocketWorker.swift"),
            root.join("native").join("bootstrap").join("StatusModel.swift"),
            root.join("native").join("bootstrap").join("ConfigForm.swift"),
            root.join("native").join("bootstrap").join("Actions.swift"),
            root.join("native").join("eyes").join("Screens.swift"),
            root.join("tests").join("host_selftest").join("main.swift"),
        ];

        for f in &files_to_compile {
            if !f.exists() {
                eprintln!("required Swift file does not exist: {}", f.display());
                return None;
            }
        }

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        cmd.arg("-DHOST_APP_T3");
        cmd.arg("-DHOST_APP_CORE");
        for f in &files_to_compile {
            cmd.arg(f);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of host_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_host_cmd(command: &str, payload: &Value) -> Result<Value, String> {
    let bin = host_selftest_bin().expect("host_selftest binary not available");
    let payload_str = serde_json::to_string(payload).unwrap();
    let output = Command::new(bin)
        .arg(command)
        .arg(&payload_str)
        .output()
        .map_err(|e| format!("failed to execute host_selftest: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "host_selftest {} failed (code {:?}): {}",
            command,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid json output: {}", e))
}

fn create_fake_script(dir: &Path, body: &str) -> PathBuf {
    static SCRIPT_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let count = SCRIPT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("fake_script_{}.sh", count));
    let content = format!("#!/bin/bash\n{}\n", body);
    fs::write(&path, content).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn read_plist_as_json(path: &Path) -> Value {
    let output = Command::new("plutil")
        .arg("-convert")
        .arg("json")
        .arg("-o")
        .arg("-")
        .arg(path)
        .output()
        .expect("failed to run plutil");
    assert!(
        output.status.success(),
        "plutil failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("invalid json from plutil")
}

fn default_input() -> Value {
    serde_json::json!({
        "permissions": {
            "screen_recording_authorized": true,
            "accessibility_trusted": true
        },
        "desktop_service": {
            "socket_exists": true,
            "status": "running",
            "pid": 12345
        },
        "bot": {
            "launchd_loaded": true,
            "health_state": "watching",
            "health_updated_at": 10000.0,
            "now": 10010.0,
            "needs_recalibration": false
        },
        "sessions": {
            "session_a": {
                "state": "watching"
            }
        },
        "wechat": {
            "is_running": true,
            "pid": 8888,
            "version": "4.1.15",
            "main_bounds": {"x": 100, "y": 100, "width": 880, "height": 640},
            "windows": [
                {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
                 "bounds": {"x": 100, "y": 100, "width": 880, "height": 640}}
            ]
        },
        "layout": {
            "layout_exists": true,
            "version": "4.1.15"
        },
        "disk": {
            "free_bytes": 20_i64 * 1024 * 1024 * 1024,
            "attachment_bytes": 100_i64 * 1024 * 1024
        }
    })
}

fn default_status_input() -> Value {
    serde_json::json!({
        "permissions": {
            "screen_recording_authorized": true,
            "accessibility_trusted": true
        },
        "desktop_service": {
            "socket_exists": true,
            "status": "running",
            "pid": 12345
        },
        "bot": {
            "launchd_loaded": false,
            "core_enabled": true,
            "core_status": "running",
            "core_pid": 54321,
            "health_state": "watching",
            "health_updated_at": 10000.0,
            "now": 10010.0,
            "needs_recalibration": false
        },
        "sessions": {
            "session_a": {
                "state": "watching"
            }
        },
        "wechat": {
            "is_running": true,
            "pid": 8888,
            "version": "4.1.15",
            "main_bounds": {"x": 100, "y": 100, "width": 880, "height": 640},
            "windows": [
                {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
                 "bounds": {"x": 100, "y": 100, "width": 880, "height": 640}}
            ]
        },
        "layout": {
            "layout_exists": true,
            "version": "4.1.15"
        },
        "disk": {
            "free_bytes": 20_i64 * 1024 * 1024 * 1024,
            "attachment_bytes": 100_i64 * 1024 * 1024
        }
    })
}

// ============================================================================
// HostStatusTests (from test_host_status.py, 25 tests)
// ============================================================================

#[test]
fn test_status_stale_health_red() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["bot"]["health_updated_at"] = serde_json::json!(10000.0);
    inp["bot"]["now"] = serde_json::json!(10125.0);

    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("bot"));
    assert_eq!(item_map["bot"]["level"], "red");
    assert!(item_map["bot"]["reason"].as_str().unwrap().contains("120"));
}

#[test]
fn test_status_recalibration_red() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["bot"]["health_state"] = serde_json::json!("needs_recalibration");

    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("bot"));
    assert_eq!(item_map["bot"]["level"], "red");
    assert!(item_map["bot"]["reason"].as_str().unwrap().contains("校准"));
}

#[test]
fn test_status_waiting_session_yellow_names_code() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["sessions"]["user_vip"] = serde_json::json!({
        "state": "waiting_for_wechat_window",
        "code": "screen_not_main:other_wechat_window"
    });

    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("sessions"));
    assert_eq!(item_map["sessions"]["level"], "yellow");
    assert!(item_map["sessions"]["reason"].as_str().unwrap().contains("user_vip"));
    assert!(item_map["sessions"]["reason"].as_str().unwrap().contains("screen_not_main:other_wechat_window"));
}

#[test]
fn test_status_front_window_red_behind_ok() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 1. 挡在前面
    let mut inp_front = default_input();
    inp_front["wechat"]["windows"] = serde_json::json!([
        {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
         "bounds": {"x": 150, "y": 150, "width": 300, "height": 200}},
        {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
         "bounds": {"x": 100, "y": 100, "width": 880, "height": 640}}
    ]);
    let items_front = run_host_cmd("status", &inp_front).unwrap();
    let dict_front: HashMap<String, Value> = items_front
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert_eq!(dict_front["wechat"]["level"], "red");
    assert!(dict_front["wechat"]["reason"].as_str().unwrap().contains('挡'));

    // 2. 窗口在后面
    let mut inp_behind = default_input();
    inp_behind["wechat"]["windows"] = serde_json::json!([
        {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
         "bounds": {"x": 100, "y": 100, "width": 880, "height": 640}},
        {"owner": "WeChat", "pid": 8888, "layer": 0, "on_screen": true,
         "bounds": {"x": 150, "y": 150, "width": 360, "height": 124}}
    ]);
    let items_behind = run_host_cmd("status", &inp_behind).unwrap();
    let dict_behind: HashMap<String, Value> = items_behind
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert_eq!(dict_behind["wechat"]["level"], "green");
}

#[test]
fn test_status_disk_low_yellow() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["disk"]["free_bytes"] = serde_json::json!(4_i64 * 1024 * 1024 * 1024);

    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("disk"));
    assert_eq!(item_map["disk"]["level"], "yellow");
    assert!(item_map["disk"]["reason"].as_str().unwrap().contains("5GB"));
}

#[test]
fn test_config_form_rejects_invalid_without_write() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_reject");
    let target_path = td.path.join("config.json");
    let original_content = serde_json::json!({
        "schema_version": 1,
        "account_id": "original",
        "manage_window": true
    })
    .to_string();
    fs::write(&target_path, &original_content).unwrap();

    let payload = serde_json::json!({
        "op": "save_config",
        "target_path": target_path.to_str().unwrap(),
        "new_config": {
            "schema_version": 999,
            "account_id": "invalid"
        },
        "simulated_check_result": {
            "ok": false,
            "exit_code": 2,
            "error": "unsupported bot config schema or field"
        }
    });

    let res = run_host_cmd("config", &payload).unwrap();
    assert_eq!(res["ok"], false);
    assert!(res["error"].as_str().unwrap().contains("unsupported bot config"));

    assert_eq!(fs::read_to_string(&target_path).unwrap(), original_content);

    let mut bak_files = Vec::new();
    for entry in fs::read_dir(&td.path).unwrap().flatten() {
        if entry.file_name().to_string_lossy().contains(".bak") {
            bak_files.push(entry.path());
        }
    }
    assert!(bak_files.is_empty());
}

#[test]
fn test_config_save_backs_up_then_replaces() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_backup");
    let target_path = td.path.join("config.json");
    let original_content = serde_json::json!({
        "schema_version": 1,
        "account_id": "test_bot",
        "max_attachment_mb": 50
    })
    .to_string();
    fs::write(&target_path, &original_content).unwrap();

    let payload = serde_json::json!({
        "op": "save_config",
        "target_path": target_path.to_str().unwrap(),
        "new_config": {
            "schema_version": 1,
            "account_id": "test_bot",
            "max_attachment_mb": 60
        },
        "simulated_check_result": {
            "ok": true,
            "exit_code": 0
        }
    });

    let res = run_host_cmd("config", &payload).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res.get("reloaded").and_then(|v| v.as_bool()), Some(true));

    let saved: Value = serde_json::from_str(&fs::read_to_string(&target_path).unwrap()).unwrap();
    assert_eq!(saved["max_attachment_mb"], 60);

    let mut bak_files = Vec::new();
    for entry in fs::read_dir(&td.path).unwrap().flatten() {
        if entry.file_name().to_string_lossy().starts_with("config.json.bak-") {
            bak_files.push(entry.path());
        }
    }
    assert_eq!(bak_files.len(), 1);
    assert_eq!(fs::read_to_string(&bak_files[0]).unwrap(), original_content);
}

#[test]
fn test_config_form_never_shows_key() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 1. 包含 api_key
    let payload_with_key = serde_json::json!({
        "op": "mask_config",
        "config": {
            "account_id": "bot1",
            "model_config": {
                "backend": "deepseek",
                "model": "chat",
                "api_key": "mock_key"
            }
        }
    });
    let res1 = run_host_cmd("config", &payload_with_key).unwrap();
    assert!(!serde_json::to_string(&res1).unwrap().contains("mock_key"));
    assert_eq!(res1["api_key_status"], "已配置");

    // 2. 未配置 key
    let payload_no_key = serde_json::json!({
        "op": "mask_config",
        "config": {
            "account_id": "bot1",
            "model_config": {
                "backend": "deepseek",
                "model": "chat"
            }
        }
    });
    let res2 = run_host_cmd("config", &payload_no_key).unwrap();
    assert_eq!(res2["api_key_status"], "未配置");
}

#[test]
fn test_diagnostics_has_no_secrets() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let inp = default_input();
    let payload = serde_json::json!({
        "status_input": inp,
        "app_version": "1.0.0",
        "sensitive_sample": "mock_sensitive_text"
    });
    let res = run_host_cmd("diagnostics", &payload).unwrap();
    let diag_text = res["text"].as_str().unwrap_or("");
    assert!(diag_text.contains("1.0.0"));
    assert!(diag_text.contains("权限"));
    assert!(diag_text.contains("桌面服务"));
    assert!(diag_text.contains("机器人"));
    assert!(!diag_text.contains("mock_sensitive_text"));
    assert!(!diag_text.to_lowercase().contains("secret"));
}

#[test]
fn test_config_allowlist_preserves_group_and_ignorelist_deep_equal() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("allowlist_test");
    let target_path = td.path.join("config.json");
    let original_config = serde_json::json!({
        "schema_version": 1,
        "account_id": "test_bot",
        "allowlist": [
            {"recipient": "客服群1", "kind": "group", "aliases": ["客服群别名"]},
            {"recipient": "用户张三", "kind": "private"}
        ],
        "ignorelist": [
            {"recipient": "黑名单群", "kind": "group"}
        ],
        "max_attachment_mb": 50,
        "manage_window": true
    });
    fs::write(&target_path, serde_json::to_string(&original_config).unwrap()).unwrap();

    // 1. 无修改保存
    let payload_unchanged = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": original_config,
        "form_inputs": {
            "allowlist_rows": [
                {"recipient": "客服群1", "kind": "group"},
                {"recipient": "用户张三", "kind": "private"}
            ],
            "ignorelist_rows": [
                {"recipient": "黑名单群", "kind": "group"}
            ]
        },
        "simulated_check_result": {"ok": true}
    });
    let res = run_host_cmd("config", &payload_unchanged).unwrap();
    assert_eq!(res["ok"], true);
    let saved: Value = serde_json::from_str(&fs::read_to_string(&target_path).unwrap()).unwrap();
    assert_eq!(saved["allowlist"], original_config["allowlist"]);
    assert_eq!(saved["ignorelist"], original_config["ignorelist"]);

    // 2. 清空 ignorelist
    let payload_empty_ignore = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": saved,
        "form_inputs": {
            "allowlist_rows": [
                {"recipient": "用户张三", "kind": "private"}
            ],
            "ignorelist_rows": []
        },
        "simulated_check_result": {"ok": true}
    });
    let res2 = run_host_cmd("config", &payload_empty_ignore).unwrap();
    assert_eq!(res2["ok"], true);
    let saved2: Value = serde_json::from_str(&fs::read_to_string(&target_path).unwrap()).unwrap();
    assert_eq!(saved2["ignorelist"], serde_json::json!([]));
    assert_eq!(saved2["allowlist"].as_array().unwrap().len(), 1);
}

#[test]
fn test_config_integer_fields_strictly_validated() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_int_test");
    let target_path = td.path.join("config.json");
    let original_config = serde_json::json!({
        "schema_version": 1,
        "account_id": "test_bot",
        "cooldown_seconds": 15,
        "turn_quiet_seconds": 2,
        "turn_max_seconds": 180,
        "max_attachment_mb": 50,
        "max_replies_per_hour": 100,
        "context_messages": 10
    });
    let orig_str = serde_json::to_string(&original_config).unwrap();
    fs::write(&target_path, &orig_str).unwrap();

    // 1. '15.5' 必须拒绝
    let payload_float = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": original_config,
        "form_inputs": {
            "cooldown_seconds": "15.5"
        },
        "simulated_check_result": {"ok": true}
    });
    let res_float = run_host_cmd("config", &payload_float).unwrap();
    assert_eq!(res_float["ok"], false);
    assert!(res_float["error"].as_str().unwrap().contains("整数"));
    assert_eq!(fs::read_to_string(&target_path).unwrap(), orig_str);

    // 2. 'abc' 必须拒绝
    let payload_str = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": original_config,
        "form_inputs": {
            "max_replies_per_hour": "abc"
        },
        "simulated_check_result": {"ok": true}
    });
    let res_str = run_host_cmd("config", &payload_str).unwrap();
    assert_eq!(res_str["ok"], false);
    assert!(res_str["error"].as_str().unwrap().contains("整数"));

    // 3. '20' 保存为真实 int
    let payload_valid = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": original_config,
        "form_inputs": {
            "cooldown_seconds": "20",
            "turn_quiet_seconds": "3",
            "turn_max_seconds": "200",
            "max_attachment_mb": "60",
            "max_replies_per_hour": "120",
            "context_messages": "15"
        },
        "simulated_check_result": {"ok": true}
    });
    let res_valid = run_host_cmd("config", &payload_valid).unwrap();
    assert_eq!(res_valid["ok"], true);
    let saved: Value = serde_json::from_str(&fs::read_to_string(&target_path).unwrap()).unwrap();
    for field in &[
        "cooldown_seconds",
        "turn_quiet_seconds",
        "turn_max_seconds",
        "max_attachment_mb",
        "max_replies_per_hour",
        "context_messages",
    ] {
        assert!(saved[field].is_i64(), "{} must be a real int, got {:?}", field, saved[field]);
    }
}

#[test]
fn test_config_save_preserves_untouched_keys_deep_equal() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_preserve");
    let target_path = td.path.join("config.json");
    let original_config = serde_json::json!({
        "schema_version": 1,
        "host": "local",
        "state_dir": "/custom/state/dir",
        "account_id": "test_bot_99",
        "model_config": {
            "backend": "deepseek",
            "model": "deepseek-chat",
            "api_key": "custom_key",
            "temperature": 0.3
        },
        "capture_backend": "native",
        "poll_seconds": 3,
        "idle_check_seconds": 10,
        "max_attachment_mb": 50,
        "cooldown_seconds": 15
    });
    fs::write(&target_path, serde_json::to_string(&original_config).unwrap()).unwrap();

    let payload = serde_json::json!({
        "op": "update_and_save_form",
        "target_path": target_path.to_str().unwrap(),
        "original_config": original_config,
        "form_inputs": {
            "max_attachment_mb": "75"
        },
        "simulated_check_result": {"ok": true}
    });
    let res = run_host_cmd("config", &payload).unwrap();
    assert_eq!(res["ok"], true);
    let saved: Value = serde_json::from_str(&fs::read_to_string(&target_path).unwrap()).unwrap();

    let untouched = [
        "schema_version",
        "host",
        "state_dir",
        "account_id",
        "model_config",
        "capture_backend",
        "poll_seconds",
        "idle_check_seconds",
    ];
    for k in &untouched {
        assert_eq!(saved[k], original_config[k], "Key {} was modified", k);
    }
}

#[test]
fn test_config_save_fails_if_backup_fails_without_modifying_target() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_bak_fail");
    let target_path = td.path.join("config.json");
    let original_content = serde_json::json!({"schema_version": 1, "account_id": "orig"}).to_string();
    fs::write(&target_path, &original_content).unwrap();

    let payload = serde_json::json!({
        "op": "save_config",
        "target_path": target_path.to_str().unwrap(),
        "new_config": {"schema_version": 1, "account_id": "replaced"},
        "simulated_backup_failure": true,
        "simulated_check_result": {"ok": true}
    });
    let res = run_host_cmd("config", &payload).unwrap();
    assert_eq!(res["ok"], false);
    assert!(res["error"].as_str().unwrap().contains("备份"));
    assert_eq!(fs::read_to_string(&target_path).unwrap(), original_content);
}

#[test]
fn test_config_temp_file_secure_permissions_and_directory() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let payload = serde_json::json!({
        "op": "check_temp_security",
        "test_data": serde_json::to_string(&serde_json::json!({"test": "secret_key_123"})).unwrap()
    });
    let res = run_host_cmd("config", &payload).unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["dir_permissions"], 0o700);
    assert_eq!(res["file_permissions"], 0o600);
    assert_eq!(res["in_shared_tmp_root"], false, "Must not be directly in /tmp root");
}

#[test]
fn test_restore_wechat_window_pure_logic() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let display = serde_json::json!({"minX": 0, "minY": 0, "maxX": 1920, "maxY": 1080});

    // 1. 多个非最小化窗口 -> 拒绝
    let windows_multi = serde_json::json!([
        {"is_minimized": false, "is_fullscreen": false, "x": 100, "y": 100, "width": 880, "height": 640},
        {"is_minimized": false, "is_fullscreen": false, "x": 200, "y": 200, "width": 360, "height": 124}
    ]);
    let res_multi = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "decide_window_adjustment",
            "windows": windows_multi,
            "main_window": windows_multi[0],
            "display": display
        }),
    )
    .unwrap();
    assert_eq!(res_multi["status"], "rejected");
    assert!(res_multi["message"].as_str().unwrap().contains("微信有多个窗口，请先关闭其他窗口"));

    // 2. 1个非最小化窗口 + 1个最小化窗口 -> 允许并调整
    let windows_one_min = serde_json::json!([
        {"is_minimized": false, "is_fullscreen": false, "x": 2000, "y": 10, "width": 800, "height": 600},
        {"is_minimized": true, "is_fullscreen": false, "x": 0, "y": 0, "width": 360, "height": 124}
    ]);
    let res_ok = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "decide_window_adjustment",
            "windows": windows_one_min,
            "main_window": windows_one_min[0],
            "display": display
        }),
    )
    .unwrap();
    assert_eq!(res_ok["status"], "adjust");
    assert_eq!(res_ok["target_origin"]["x"], 1040);
    assert_eq!(res_ok["target_origin"]["y"], 25);
    assert_eq!(res_ok["target_size"]["width"], 880);
    assert_eq!(res_ok["target_size"]["height"], 640);
}

#[test]
fn test_status_desktop_service_idle_is_green() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["desktop_service"] = serde_json::json!({"socket_exists": true, "status": "idle"});
    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    let ds = &item_map["desktop_service"];
    assert_eq!(ds["level"], "green");
    assert!(ds["reason"].as_str().unwrap().contains("空闲"));
}

#[test]
fn test_status_desktop_service_idle_without_socket_is_red() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["desktop_service"] = serde_json::json!({"socket_exists": false, "status": "idle"});
    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert_eq!(item_map["desktop_service"]["level"], "red");
}

#[test]
fn test_mask_config_key_file_counts_as_configured() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "mask_config",
            "config": {
                "model_config": {
                    "backend": "openai-compatible",
                    "model": "m",
                    "api_key_file": "/x/model.key"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(res["api_key_status"], "已配置");
    assert!(!serde_json::to_string(&res).unwrap().contains("/x/model.key"));
}

#[test]
fn test_form_empty_integer_fields_keep_original_or_default() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("form_empty_int");
    let target = td.path.join("config.json");
    let original = serde_json::json!({"schema_version": 1, "account_id": "a", "cooldown_seconds": 3});
    fs::write(&target, serde_json::to_string(&original).unwrap()).unwrap();
    let res = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": original,
            "form_inputs": {
                "cooldown_seconds": "",
                "max_attachment_mb": "  ",
                "max_replies_per_hour": "20"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    let saved: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(saved["cooldown_seconds"], 3);
    assert!(saved.get("max_attachment_mb").is_none());
    assert_eq!(saved["max_replies_per_hour"], 20);
}

#[test]
fn test_mask_config_includes_brain_setting() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let cfg = serde_json::json!({
        "account_id": "bot1",
        "brain": {
            "mode": "external",
            "external": {
                "timeout_seconds": 180,
                "timeout_action": "handoff",
                "queue_minutes": 60
            }
        }
    });
    let res = run_host_cmd("config", &serde_json::json!({"op": "mask_config", "config": cfg})).unwrap();
    let editable = &res["editable"];
    assert!(editable.get("brain").is_some());
    assert_eq!(editable["brain"]["mode"], "external");
    assert_eq!(editable["brain"]["external"]["timeout_seconds"], 180);
}

#[test]
fn test_config_form_brain_mode_builtin_and_external() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_brain_mode");
    let target = td.path.join("config.json");
    let original = serde_json::json!({
        "schema_version": 1,
        "account_id": "bot1",
        "cooldown_seconds": 15,
        "brain": {"mode": "builtin"}
    });
    fs::write(&target, serde_json::to_string(&original).unwrap()).unwrap();

    // 1. 设置为 external
    let res = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": original,
            "form_inputs": {
                "brain_mode": "external",
                "brain_timeout_seconds": "150",
                "brain_timeout_action": "handoff",
                "brain_queue_minutes": "45"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    let saved: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(saved["brain"]["mode"], "external");
    assert_eq!(saved["brain"]["external"]["timeout_seconds"], 150);
    assert_eq!(saved["brain"]["external"]["timeout_action"], "handoff");
    assert_eq!(saved["brain"]["external"]["queue_minutes"], 45);
    assert_eq!(saved["cooldown_seconds"], 15);
    assert_eq!(saved["account_id"], "bot1");

    // 2. 切换回 builtin
    let res2 = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": saved,
            "form_inputs": {
                "brain_mode": "builtin"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res2["ok"], true);
    let saved2: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(saved2["brain"]["mode"], "builtin");
    assert!(saved2["brain"].get("external").is_none());
}

#[test]
fn test_config_form_brain_external_validation() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("cfg_brain_val");
    let target = td.path.join("config.json");
    let original = serde_json::json!({"schema_version": 1, "account_id": "bot1"});
    fs::write(&target, serde_json::to_string(&original).unwrap()).unwrap();

    // 1. 超时秒数 0 拒绝
    let res_bad_timeout = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": original,
            "form_inputs": {
                "brain_mode": "external",
                "brain_timeout_seconds": "0"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res_bad_timeout["ok"], false);

    // 2. 超时秒数 3601 拒绝
    let res_bad_timeout2 = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": original,
            "form_inputs": {
                "brain_mode": "external",
                "brain_timeout_seconds": "3601"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res_bad_timeout2["ok"], false);

    // 3. 非法 action 拒绝
    let res_bad_action = run_host_cmd(
        "config",
        &serde_json::json!({
            "op": "update_and_save_form",
            "target_path": target.to_str().unwrap(),
            "original_config": original,
            "form_inputs": {
                "brain_mode": "external",
                "brain_timeout_action": "destroy"
            },
            "simulated_check_result": {"ok": true}
        }),
    )
    .unwrap();
    assert_eq!(res_bad_action["ok"], false);
}

#[test]
fn test_status_external_brain_unconnected_yellow_and_pending_count() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["brain"] = serde_json::json!({
        "mode": "external",
        "pending": 3,
        "last_poll_at": 10000.0,
        "now": 10070.0
    });
    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("brain"));
    assert_eq!(item_map["brain"]["level"], "yellow");
    assert!(item_map["brain"]["reason"].as_str().unwrap().contains("外部大脑未连接"));
    assert!(item_map["brain"]["reason"].as_str().unwrap().contains('3'));
}

#[test]
fn test_status_external_brain_connected_green() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["brain"] = serde_json::json!({
        "mode": "external",
        "pending": 0,
        "last_poll_at": 10000.0,
        "now": 10020.0
    });
    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("brain"));
    assert_eq!(item_map["brain"]["level"], "green");
}

#[test]
fn test_status_sessions_handoff_lists_name() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_input();
    inp["sessions"]["客户张三"] = serde_json::json!({
        "state": "handoff",
        "reason": "转人工处理中"
    });
    let items = run_host_cmd("status", &inp).unwrap();
    let item_map: HashMap<String, Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().to_string(), i.clone()))
        .collect();
    assert!(item_map.contains_key("sessions"));
    assert_eq!(item_map["sessions"]["level"], "yellow");
    assert!(item_map["sessions"]["reason"].as_str().unwrap().contains("客户张三"));
    assert!(item_map["sessions"]["reason"].as_str().unwrap().contains("转人工"));
}

// ============================================================================
// HostSupervisorLogicTests, HostAppConfigurationTests, DeployScriptTests (9 tests)
// ============================================================================

#[test]
fn test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 10000.0;
    // 1. 六次正常退出
    let clean_events: Vec<Value> = (0..6)
        .map(|i| {
            serde_json::json!({
                "status": 0,
                "reason": "exit",
                "epoch": t0 + (i as f64) * 300.0
            })
        })
        .collect();
    let res = run_host_cmd("child_exit", &serde_json::json!({"events": clean_events})).unwrap();
    assert_eq!(res["is_given_up"], false);
    assert_eq!(res["accepting_connections"], true);
    assert_eq!(res["status"]["state"], "idle");

    // 2. 五次异常退出触发 give_up
    let abnormal_events: Vec<Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "status": 1,
                "reason": "exit",
                "epoch": t0 + (i as f64) * 10.0
            })
        })
        .collect();
    let res2 = run_host_cmd("child_exit", &serde_json::json!({"events": abnormal_events})).unwrap();
    assert_eq!(res2["is_given_up"], true);
    assert_eq!(res2["accepting_connections"], false);
    assert_eq!(res2["status"]["state"], "stopped");
    assert!(res2["status"]["reason"].as_str().unwrap().contains("上限"));

    // 3. reset 清除 give_up
    let mut reset_events = abnormal_events;
    reset_events.push(serde_json::json!({"op": "reset"}));
    let res_reset = run_host_cmd("child_exit", &serde_json::json!({"events": reset_events})).unwrap();
    assert_eq!(res_reset["is_given_up"], false);
    assert_eq!(res_reset["accepting_connections"], true);
}

#[test]
fn test_backoff_sequence() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 10000.0;
    let actions = serde_json::json!([
        {"op": "exit", "epoch": t0 + 0.0},
        {"op": "exit", "epoch": t0 + 10.0},
        {"op": "exit", "epoch": t0 + 30.0},
        {"op": "exit", "epoch": t0 + 100.0}
    ]);
    let res = run_host_cmd("restart_policy", &serde_json::json!({"actions": actions})).unwrap();
    let decisions = res["results"].as_array().unwrap();
    assert_eq!(decisions.len(), 4);
    assert_eq!(decisions[0]["decision"], "restart");
    assert_eq!(decisions[0]["after"].as_f64().unwrap(), 5.0);
    assert_eq!(decisions[1]["decision"], "restart");
    assert_eq!(decisions[1]["after"].as_f64().unwrap(), 15.0);
    assert_eq!(decisions[2]["decision"], "restart");
    assert_eq!(decisions[2]["after"].as_f64().unwrap(), 60.0);
    assert_eq!(decisions[3]["decision"], "restart");
    assert_eq!(decisions[3]["after"].as_f64().unwrap(), 60.0);
}

#[test]
fn test_give_up_after_five_in_an_hour() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 10000.0;
    let actions = serde_json::json!([
        {"op": "exit", "epoch": t0 + 0.0},
        {"op": "exit", "epoch": t0 + 10.0},
        {"op": "exit", "epoch": t0 + 30.0},
        {"op": "exit", "epoch": t0 + 100.0},
        {"op": "exit", "epoch": t0 + 200.0}
    ]);
    let res = run_host_cmd("restart_policy", &serde_json::json!({"actions": actions})).unwrap();
    let decisions = res["results"].as_array().unwrap();
    assert_eq!(decisions.len(), 5);
    assert_eq!(decisions[4], serde_json::json!({"decision": "give_up"}));
}

#[test]
fn test_window_rolls() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 10000.0;
    let actions = serde_json::json!([
        {"op": "exit", "epoch": t0 + 0.0},
        {"op": "exit", "epoch": t0 + 10.0},
        {"op": "exit", "epoch": t0 + 20.0},
        {"op": "exit", "epoch": t0 + 30.0},
        {"op": "exit", "epoch": t0 + 4000.0}
    ]);
    let res = run_host_cmd("restart_policy", &serde_json::json!({"actions": actions})).unwrap();
    let decisions = res["results"].as_array().unwrap();
    assert_eq!(decisions.len(), 5);
    assert_eq!(decisions[4]["decision"], "restart");
    assert_eq!(decisions[4]["after"].as_f64().unwrap(), 5.0);
}

#[test]
fn test_reset_clears() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 10000.0;
    let actions = serde_json::json!([
        {"op": "exit", "epoch": t0 + 0.0},
        {"op": "exit", "epoch": t0 + 10.0},
        {"op": "reset"},
        {"op": "exit", "epoch": t0 + 20.0}
    ]);
    let res = run_host_cmd("restart_policy", &serde_json::json!({"actions": actions})).unwrap();
    let decisions = res["results"].as_array().unwrap();
    assert_eq!(decisions.len(), 3);
    assert_eq!(decisions[0]["decision"], "restart");
    assert_eq!(decisions[0]["after"].as_f64().unwrap(), 5.0);
    assert_eq!(decisions[1]["decision"], "restart");
    assert_eq!(decisions[1]["after"].as_f64().unwrap(), 15.0);
    assert_eq!(decisions[2]["decision"], "restart");
    assert_eq!(decisions[2]["after"].as_f64().unwrap(), 5.0);
}

#[test]
fn test_info_plist_names() {
    let plist_path = repo_root().join("native").join("bootstrap").join("Info.plist");
    assert!(plist_path.exists(), "Info.plist must exist");
    let data = read_plist_as_json(&plist_path);
    assert_eq!(data["CFBundleName"], "微信客服助手");
    assert_eq!(data["CFBundleDisplayName"], "微信客服助手");
    assert_eq!(data["CFBundleIdentifier"], "local.wechat-mcp.host");
    assert_eq!(data["CFBundleExecutable"], "wechat-cs-host");
}

#[test]
fn test_launchagent_keepalive_only_on_crash() {
    let plist_path = repo_root()
        .join("native")
        .join("bootstrap")
        .join("local.wechat-mcp.host.plist");
    assert!(plist_path.exists(), "LaunchAgent plist template must exist");
    let data = read_plist_as_json(&plist_path);
    assert_eq!(data["Label"], "local.wechat-mcp.host");
    assert_eq!(data["RunAtLoad"], true);
    assert_eq!(data["ThrottleInterval"], 30);
    assert_eq!(data["KeepAlive"]["SuccessfulExit"], false);
    let args = data["ProgramArguments"].as_array().unwrap();
    assert!(!args.is_empty());
    assert!(args[0]
        .as_str()
        .unwrap()
        .ends_with("微信客服助手.app/Contents/MacOS/wechat-cs-host"));
}

#[test]
fn test_deploy_script_dry_run_by_default() {
    let script_path = repo_root().join("scripts").join("deploy_host.sh");
    assert!(script_path.exists(), "deploy_host.sh must exist");

    let fake_bin = TempDir::new("fake_bin");
    for name in &["ssh", "scp"] {
        let p = fake_bin.path.join(name);
        fs::write(&p, "#!/bin/sh\necho \"FORBIDDEN_CALL: $0 $*\" >&2\nexit 99\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
    }

    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", fake_bin.path.display(), current_path);

    let output = Command::new(&script_path)
        .arg("fake-host")
        .env("PATH", new_path)
        .output()
        .expect("failed to execute deploy_host.sh");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code().unwrap_or(-1),
        0,
        "dry-run should succeed with exit 0, got {:?}:\n{}",
        output.status.code(),
        stderr
    );
    assert!(!stderr.contains("FORBIDDEN_CALL"));
    assert!(stdout.to_lowercase().contains("dry-run"));
}

#[test]
fn test_deploy_script_has_no_pkill() {
    let script_path = repo_root().join("scripts").join("deploy_host.sh");
    assert!(script_path.exists(), "deploy_host.sh must exist");
    let content = fs::read_to_string(&script_path).unwrap();
    assert!(!content.contains("pkill"), "deploy_host.sh must never use pkill");
}

// ============================================================================
// HostCoreTests (from test_host_core.py, 17 tests)
// ============================================================================

#[test]
fn test_decision_conflict_when_both_enabled_and_loaded() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": true,
            "process_state": "not_running"
        }),
    )
    .unwrap();
    assert_eq!(res["decision"], "conflict");

    let res_running = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": true,
            "process_state": "running",
            "pid": 1234
        }),
    )
    .unwrap();
    assert_eq!(res_running["decision"], "conflict");
}

#[test]
fn test_decision_python_mode_stops_running_or_keeps_idle() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res_running = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": false,
            "python_loaded": false,
            "process_state": "running",
            "pid": 1234
        }),
    )
    .unwrap();
    assert_eq!(res_running["decision"], "stop");

    let res_idle = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": false,
            "python_loaded": true,
            "process_state": "not_running"
        }),
    )
    .unwrap();
    assert_eq!(res_idle["decision"], "keep");
}

#[test]
fn test_decision_rust_mode_starts_or_keeps() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res_not_running = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "not_running"
        }),
    )
    .unwrap();
    assert_eq!(res_not_running["decision"], "start");

    let res_running = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "running",
            "pid": 1234
        }),
    )
    .unwrap();
    assert_eq!(res_running["decision"], "keep");
}

#[test]
fn test_decision_rust_mode_restarting_backoff() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let t0 = 1000.0;
    let res_wait = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "restarting",
            "now": t0,
            "restarting_at": t0 + 10.0
        }),
    )
    .unwrap();
    assert_eq!(res_wait["decision"], "keep");

    let res_ready = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "restarting",
            "now": t0 + 15.0,
            "restarting_at": t0 + 10.0
        }),
    )
    .unwrap();
    assert_eq!(res_ready["decision"], "start");
}

#[test]
fn test_decision_rust_mode_gave_up_and_config_error() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res_gave_up = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "gave_up"
        }),
    )
    .unwrap();
    assert_eq!(res_gave_up["decision"], "keep");

    let res_config_err = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "config_error"
        }),
    )
    .unwrap();
    assert_eq!(res_config_err["decision"], "keep");
}

#[test]
fn test_decision_stopping_keeps_idle() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let res_stopping_enabled = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": true,
            "python_loaded": false,
            "process_state": "stopping"
        }),
    )
    .unwrap();
    assert_eq!(res_stopping_enabled["decision"], "keep");

    let res_stopping_disabled = run_host_cmd(
        "core_decision",
        &serde_json::json!({
            "core_enabled": false,
            "python_loaded": false,
            "process_state": "stopping"
        }),
    )
    .unwrap();
    assert_eq!(res_stopping_disabled["decision"], "keep");
}

#[test]
fn test_panel_bot_status_rust_core_running() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_status_input();
    inp["bot"]["core_enabled"] = serde_json::json!(true);
    inp["bot"]["launchd_loaded"] = serde_json::json!(false);
    inp["bot"]["core_status"] = serde_json::json!("running");
    inp["bot"]["core_pid"] = serde_json::json!(88888);

    let items = run_host_cmd("status", &inp).unwrap();
    let bot_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_item["level"], "green");
    assert!(bot_item["reason"].as_str().unwrap().contains("Rust 核心"));
    assert!(bot_item["reason"].as_str().unwrap().contains("88888"));
}

#[test]
fn test_panel_bot_status_python_mode() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_status_input();
    inp["bot"]["core_enabled"] = serde_json::json!(false);
    inp["bot"]["launchd_loaded"] = serde_json::json!(true);
    inp["bot"]["health_state"] = serde_json::json!("watching");

    let items = run_host_cmd("status", &inp).unwrap();
    let bot_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_item["level"], "green");
    assert!(bot_item["reason"].as_str().unwrap().contains("Python 模式"));
}

#[test]
fn test_panel_bot_status_mutex_conflict() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_status_input();
    inp["bot"]["core_enabled"] = serde_json::json!(true);
    inp["bot"]["launchd_loaded"] = serde_json::json!(true);

    let items = run_host_cmd("status", &inp).unwrap();
    let bot_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_item["level"], "red");
    assert!(bot_item["reason"].as_str().unwrap().contains("互斥冲突"));
}

#[test]
fn test_panel_bot_status_backoff_and_gave_up() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 1. 退避中
    let mut inp_restarting = default_status_input();
    inp_restarting["bot"]["core_enabled"] = serde_json::json!(true);
    inp_restarting["bot"]["launchd_loaded"] = serde_json::json!(false);
    inp_restarting["bot"]["core_status"] = serde_json::json!("restarting");
    let items = run_host_cmd("status", &inp_restarting).unwrap();
    let bot_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_item["level"], "yellow");
    assert!(bot_item["reason"].as_str().unwrap().contains("退避"));

    // 2. 已放弃
    let mut inp_gave_up = default_status_input();
    inp_gave_up["bot"]["core_enabled"] = serde_json::json!(true);
    inp_gave_up["bot"]["launchd_loaded"] = serde_json::json!(false);
    inp_gave_up["bot"]["core_status"] = serde_json::json!("gave_up");
    let items_gave_up = run_host_cmd("status", &inp_gave_up).unwrap();
    let bot_gave_up = items_gave_up
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_gave_up["level"], "red");
    let r = bot_gave_up["reason"].as_str().unwrap();
    assert!(r.contains("上限") || r.contains("已放弃") || r.contains("停止"));

    // 3. 配置错误
    let mut inp_cfg_err = default_status_input();
    inp_cfg_err["bot"]["core_enabled"] = serde_json::json!(true);
    inp_cfg_err["bot"]["launchd_loaded"] = serde_json::json!(false);
    inp_cfg_err["bot"]["core_status"] = serde_json::json!("config_error");
    let items_cfg_err = run_host_cmd("status", &inp_cfg_err).unwrap();
    let bot_cfg_err = items_cfg_err
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_cfg_err["level"], "red");
    assert!(bot_cfg_err["reason"].as_str().unwrap().contains("配置错误"));
}

#[test]
fn test_panel_bot_status_stopping() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_status_input();
    inp["bot"]["core_enabled"] = serde_json::json!(true);
    inp["bot"]["core_status"] = serde_json::json!("stopping");
    let items = run_host_cmd("status", &inp).unwrap();
    let bot_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "bot")
        .unwrap();
    assert_eq!(bot_item["level"], "yellow");
    assert!(bot_item["reason"]
        .as_str()
        .unwrap()
        .contains("正在停止 Rust 核心（等待当前一轮完成）"));
}

#[test]
fn test_supervisor_env_and_log_permissions() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_env");
    let env_dump_path = td.path.join("env_dump.txt");
    let log_path = td.path.join("core.log");
    let fake_script = create_fake_script(
        &td.path,
        &format!(
            "echo \"LANG=$LANG\" >> \"{}\"\necho \"LC_ALL=$LC_ALL\" >> \"{}\"\necho \"Rust Core Log Line\"\nexit 0",
            env_dump_path.display(),
            env_dump_path.display()
        ),
    );

    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "run_once",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap()
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["exit_code"], 0);

    assert!(env_dump_path.exists());
    let env_content = fs::read_to_string(&env_dump_path).unwrap();
    assert!(env_content.contains("LANG=en_US.UTF-8"));
    assert!(env_content.contains("LC_ALL=en_US.UTF-8"));

    assert!(log_path.exists());
    let log_content = fs::read_to_string(&log_path).unwrap();
    assert!(log_content.contains("Rust Core Log Line"));
    let mode = fs::metadata(&log_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "Expected 0600, got {:o}", mode);
}

#[test]
fn test_supervisor_exit_code_0_not_counted_as_crash() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_0");
    let log_path = td.path.join("core_0.log");
    let fake_script = create_fake_script(&td.path, "exit 0");
    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "run_once",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap()
        }),
    )
    .unwrap();
    assert_eq!(res["exit_code"], 0);
    assert_eq!(res["supervisor_status"], "not_running");
    assert_eq!(res["gave_up"], false);
}

#[test]
fn test_supervisor_exit_code_2_config_error_no_restart() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_2");
    let log_path = td.path.join("core_2.log");
    let fake_script = create_fake_script(&td.path, "exit 2");
    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "run_once",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap()
        }),
    )
    .unwrap();
    assert_eq!(res["exit_code"], 2);
    assert_eq!(res["supervisor_status"], "config_error");
    assert_eq!(res["gave_up"], false);
}

#[test]
fn test_supervisor_sigterm_graceful_exit() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_term");
    let log_path = td.path.join("core_term.log");
    let ready_path = td.path.join("ready_term.txt");
    let fake_script = create_fake_script(
        &td.path,
        &format!(
            "trap 'echo \"gracefully exiting on term\"; exit 0' TERM\ntouch \"{}\"\nwhile true; do sleep 0.05; done",
            ready_path.display()
        ),
    );

    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "start_and_stop",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap(),
            "ready_file": ready_path.to_str().unwrap(),
            "grace_seconds": 5.0
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["stopped_by"], "sigterm");

    std::thread::sleep(std::time::Duration::from_millis(100));
    let log_content = fs::read_to_string(&log_path).unwrap();
    assert!(log_content.contains("gracefully exiting on term"));
}

#[test]
fn test_supervisor_sigkill_on_timeout() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_kill");
    let log_path = td.path.join("core_kill.log");
    let ready_path = td.path.join("ready_kill.txt");
    let fake_script = create_fake_script(
        &td.path,
        &format!(
            "trap '' TERM\ntouch \"{}\"\nwhile true; do sleep 0.05; done",
            ready_path.display()
        ),
    );

    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "start_and_stop",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap(),
            "ready_file": ready_path.to_str().unwrap(),
            "grace_seconds": 0.3
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["stopped_by"], "sigkill");
}

#[test]
fn test_supervisor_async_stopping_non_blocking_and_recovers() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("sup_async");
    let log_path = td.path.join("core_async_stop.log");
    let ready_path = td.path.join("ready_async_stop.txt");
    let fake_script = create_fake_script(
        &td.path,
        &format!(
            "trap '' TERM\ntouch \"{}\"\nwhile true; do sleep 0.05; done",
            ready_path.display()
        ),
    );

    let res = run_host_cmd(
        "core_supervisor_run",
        &serde_json::json!({
            "action": "test_async_reconcile_stopping",
            "binary_path": fake_script.to_str().unwrap(),
            "log_path": log_path.to_str().unwrap(),
            "ready_file": ready_path.to_str().unwrap(),
            "grace_seconds": 0.3
        }),
    )
    .unwrap();
    assert_eq!(res["ok"], true);
    let elapsed = res["elapsed_seconds"].as_f64().unwrap();
    assert!(
        elapsed < 1.0,
        "checkAndReconcile must return in < 1s, took {}s",
        elapsed
    );
    assert_eq!(res["initial_status"], "running");
    assert_eq!(res["stopping_status"], "stopping");
    assert_eq!(res["second_reconcile_decision"], "keep");
    assert_eq!(res["final_status"], "not_running");
    assert_eq!(res["can_restart_decision"], "start");
}
