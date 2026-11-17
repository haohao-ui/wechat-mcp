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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Foundation resolves `FileManager.homeDirectoryForCurrentUser` from
/// `CFFIXED_USER_HOME`, not `HOME`. Point every spawned helper at a temporary
/// home so config staging never touches the developer's `~/.wechat-rpa`.
fn swift_test_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("wechat-mcp-swift-home-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create swift test home");
        dir
    })
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
            root.join("native")
                .join("bootstrap")
                .join("Supervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("CoreSupervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("SocketWorker.swift"),
            root.join("native")
                .join("bootstrap")
                .join("StatusModel.swift"),
            root.join("native")
                .join("bootstrap")
                .join("ConfigForm.swift"),
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
        .env("CFFIXED_USER_HOME", swift_test_home())
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
    assert!(
        item_map["sessions"]["reason"]
            .as_str()
            .unwrap()
            .contains("user_vip")
    );
    assert!(
        item_map["sessions"]["reason"]
            .as_str()
            .unwrap()
            .contains("screen_not_main:other_wechat_window")
    );
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
    assert!(
        dict_front["wechat"]["reason"]
            .as_str()
            .unwrap()
            .contains('挡')
    );

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
    assert!(
        res["error"]
            .as_str()
            .unwrap()
            .contains("unsupported bot config")
    );

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
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("config.json.bak-")
        {
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
                "backend": "openai-compatible",
                "model": "chat",
                "api_key_env": "WECHAT_TEST_INTENTIONALLY_MISSING_KEY",
                "api_key": "mock_key"
            }
        }
    });
    let res1 = run_host_cmd("config", &payload_with_key).unwrap();
    assert!(!serde_json::to_string(&res1).unwrap().contains("mock_key"));
    assert_eq!(res1["api_key_status"], "未配置");

    // 2. 未配置 key
    let payload_no_key = serde_json::json!({
        "op": "mask_config",
        "config": {
            "account_id": "bot1",
            "model_config": {
                "backend": "openai-compatible",
                "model": "chat",
                "api_key_env": "WECHAT_TEST_INTENTIONALLY_MISSING_KEY"
            }
        }
    });
    let res2 = run_host_cmd("config", &payload_no_key).unwrap();
    assert_eq!(res2["api_key_status"], "未配置");
}

#[test]
fn test_model_form_replaces_backend_and_validates() {
    if !has_swiftc() {
        return;
    }
    let td = TempDir::new("model_form");
    let path = td.path.join("config.json");
    let original = serde_json::json!({"schema_version":1,"model_config":{"backend":"openai-compatible","base_url":"https://example.com/v1","model":"chat","api_key_env":"TEST_KEY"}});
    for (form, valid) in [
        (
            serde_json::json!({"backend":"codex-exec","model":"stale","supports_images":true,"images_per_model_call":16}),
            true,
        ),
        (
            serde_json::json!({"backend":"openai-compatible","base_url":"https://example.com/v1","model":"chat","key_source":"env","key_env_name":"TEST_KEY","timeout_seconds":180}),
            true,
        ),
        (
            serde_json::json!({"backend":"openai-compatible","base_url":"http://example.com","model":"chat"}),
            false,
        ),
        (
            serde_json::json!({"backend":"openai-compatible","base_url":"https://example.com","model":"chat","timeout_seconds":181}),
            false,
        ),
    ] {
        fs::write(&path, original.to_string()).unwrap();
        let res = run_host_cmd("config", &serde_json::json!({"op":"update_and_save_form","target_path":path,"original_config":original,"form_inputs":{"model_config_form":form},"simulated_check_result":{"ok":true}})).unwrap();
        assert_eq!(res["ok"], valid);
        if valid {
            let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            if form["backend"] == "codex-exec" {
                assert_eq!(
                    saved["model_config"],
                    serde_json::json!({"backend":"codex-exec"})
                );
                assert_eq!(saved["images_per_model_call"], 16);
            } else {
                assert_eq!(saved["model_config"]["api_key_env"], "TEST_KEY");
                assert!(saved["model_config"].get("api_key_file").is_none());
            }
        }
    }
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
    fs::write(
        &target_path,
        serde_json::to_string(&original_config).unwrap(),
    )
    .unwrap();

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
        assert!(
            saved[field].is_i64(),
            "{} must be a real int, got {:?}",
            field,
            saved[field]
        );
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
            "backend": "openai-compatible",
            "model": "deepseek-chat",
            "api_key_env": "CUSTOM_KEY"
        },
        "capture_backend": "native",
        "poll_seconds": 3,
        "idle_check_seconds": 10,
        "max_attachment_mb": 50,
        "cooldown_seconds": 15
    });
    fs::write(
        &target_path,
        serde_json::to_string(&original_config).unwrap(),
    )
    .unwrap();

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
    let original_content =
        serde_json::json!({"schema_version": 1, "account_id": "orig"}).to_string();
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
    assert_eq!(
        res["in_shared_tmp_root"], false,
        "Must not be directly in /tmp root"
    );
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
    assert!(
        res_multi["message"]
            .as_str()
            .unwrap()
            .contains("微信有多个窗口，请先关闭其他窗口")
    );

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
    assert!(
        !serde_json::to_string(&res)
            .unwrap()
            .contains("/x/model.key")
    );
}

#[test]
fn test_form_empty_integer_fields_keep_original_or_default() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("form_empty_int");
    let target = td.path.join("config.json");
    let original =
        serde_json::json!({"schema_version": 1, "account_id": "a", "cooldown_seconds": 3});
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
    let res = run_host_cmd(
        "config",
        &serde_json::json!({"op": "mask_config", "config": cfg}),
    )
    .unwrap();
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
    assert!(
        item_map["brain"]["reason"]
            .as_str()
            .unwrap()
            .contains("外部大脑未连接")
    );
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
    assert!(
        item_map["sessions"]["reason"]
            .as_str()
            .unwrap()
            .contains("客户张三")
    );
    assert!(
        item_map["sessions"]["reason"]
            .as_str()
            .unwrap()
            .contains("转人工")
    );
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
    let res2 = run_host_cmd(
        "child_exit",
        &serde_json::json!({"events": abnormal_events}),
    )
    .unwrap();
    assert_eq!(res2["is_given_up"], true);
    assert_eq!(res2["accepting_connections"], false);
    assert_eq!(res2["status"]["state"], "stopped");
    assert!(res2["status"]["reason"].as_str().unwrap().contains("上限"));

    // 3. reset 清除 give_up
    let mut reset_events = abnormal_events;
    reset_events.push(serde_json::json!({"op": "reset"}));
    let res_reset =
        run_host_cmd("child_exit", &serde_json::json!({"events": reset_events})).unwrap();
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
    let plist_path = repo_root()
        .join("native")
        .join("bootstrap")
        .join("Info.plist");
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
    assert!(
        args[0]
            .as_str()
            .unwrap()
            .ends_with("微信客服助手.app/Contents/MacOS/wechat-cs-host")
    );
}

#[test]
fn test_deploy_script_dry_run_by_default() {
    let script_path = repo_root().join("scripts").join("deploy_host.sh");
    assert!(script_path.exists(), "deploy_host.sh must exist");

    let fake_bin = TempDir::new("fake_bin");
    for name in &["ssh", "scp"] {
        let p = fake_bin.path.join(name);
        fs::write(
            &p,
            "#!/bin/sh\necho \"FORBIDDEN_CALL: $0 $*\" >&2\nexit 99\n",
        )
        .unwrap();
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
    assert!(
        !content.contains("pkill"),
        "deploy_host.sh must never use pkill"
    );
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
    assert!(
        bot_item["reason"]
            .as_str()
            .unwrap()
            .contains("正在停止 Rust 核心（等待当前一轮完成）")
    );
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

/// 附件保存的文件夹未授权（核心写了 folder-permission-needed.json）：权限项必须变红并告诉人去哪里允许。
#[test]
fn test_panel_shows_folder_permission_needed() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let mut inp = default_status_input();
    inp["permissions"]["folder_permission_blocked"] =
        serde_json::json!("folder_access_blocked:Desktop");
    let items = run_host_cmd("status", &inp).unwrap();
    let item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "permissions")
        .unwrap();
    assert_eq!(item["level"], "red");
    assert!(
        item["reason"].as_str().unwrap().contains("Desktop"),
        "{item}"
    );
    assert!(
        item["fix"].as_str().unwrap().contains("文件和文件夹"),
        "{item}"
    );

    // 没有标记时保持原样（绿色）
    let ok = run_host_cmd("status", &default_status_input()).unwrap();
    let item = ok
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "permissions")
        .unwrap();
    assert_eq!(item["level"], "green");
}

/// launchd 停止/重启宿主时发 SIGTERM：AppKit 默认直接退出、不走 applicationWillTerminate，
/// 核心子进程会变成无人管理的孤儿（2026-11-08 实机）。宿主必须把 SIGTERM 转成正常退出。
#[test]
fn test_host_turns_sigterm_into_orderly_terminate() {
    let src = fs::read_to_string("native/bootstrap/main.swift").unwrap();
    assert!(
        src.contains("signal(SIGTERM, SIG_IGN)"),
        "必须先忽略默认 SIGTERM 处理"
    );
    let at = src
        .find("makeSignalSource(signal: SIGTERM")
        .expect("必须为 SIGTERM 建立 DispatchSource");
    let handler = &src[at..(at + 400).min(src.len())];
    assert!(
        handler.contains("terminate("),
        "SIGTERM 处理必须调用 NSApplication terminate，以执行 applicationWillTerminate"
    );
    let plist = fs::read_to_string("native/bootstrap/local.wechat-mcp.host.plist").unwrap();
    assert!(
        plist.contains("<key>ExitTimeOut</key>"),
        "LaunchAgent 要给宿主足够时间停止核心"
    );
}

fn host_task5_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("host_task5_selftest");

        let files_to_compile = [
            root.join("native").join("bootstrap").join("Probe.swift"),
            root.join("native")
                .join("bootstrap")
                .join("Supervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("CoreSupervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("SocketWorker.swift"),
            root.join("native").join("bootstrap").join("Actions.swift"),
            root.join("native")
                .join("bootstrap")
                .join("StatusModel.swift"),
            root.join("native")
                .join("bootstrap")
                .join("ConfigForm.swift"),
            root.join("native")
                .join("bootstrap")
                .join("StatusPanel.swift"),
            root.join("native").join("bootstrap").join("LogTail.swift"),
            root.join("native")
                .join("bootstrap")
                .join("Permissions.swift"),
            root.join("native").join("bootstrap").join("Shell.swift"),
            root.join("native")
                .join("bootstrap")
                .join("SettingsPage.swift"),
            root.join("native").join("eyes").join("Screens.swift"),
            root.join("tests")
                .join("host_task5_selftest")
                .join("main.swift"),
        ];

        for f in &files_to_compile {
            if !f.exists() {
                eprintln!("required Swift file does not exist: {}", f.display());
                return None;
            }
        }

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        for f in &files_to_compile {
            cmd.arg(f);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of host_task5_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_task5_cmd(command: &str, payload: &Value) -> Result<Value, String> {
    let bin = host_task5_selftest_bin()
        .ok_or_else(|| "host_task5_selftest binary not available".to_string())?;
    let payload_str = serde_json::to_string(payload).unwrap();
    let output = Command::new(bin)
        .env("CFFIXED_USER_HOME", swift_test_home())
        .arg(command)
        .arg(&payload_str)
        .output()
        .map_err(|e| format!("failed to execute host_task5_selftest: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "host_task5_selftest {} failed (code {:?}): {}",
            command,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid json output: {}", e))
}

#[test]
fn test_log_tail_reads_only_new_lines_and_caps() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let td = TempDir::new("log_tail_test");
    let log_file = td.path.join("test.log");

    // 初始写 3 行
    fs::write(&log_file, "line 1\nline 2\nline 3\n").unwrap();

    let ops = serde_json::json!({
        "file_path": log_file.to_str().unwrap(),
        "max_lines": 500,
        "operations": [
            // 第一次 poll：应得 3 行
            {"op": "poll"},
            // 追加 2 行
            {"op": "append", "lines": ["line 4", "line 5"]},
            // 第二次 poll：应只增量读到 2 行
            {"op": "poll"},
            // 超过 500 行测试（写入 600 行），截断后 poll 只保留最近 500 行
            {"op": "truncate", "lines": (1..=600).map(|i| format!("bulk line {}", i)).collect::<Vec<_>>()},
            // 第三次 poll：截断后从头读，且 cap 到 500
            {"op": "poll"}
        ]
    });

    let res = run_task5_cmd("log_tail", &ops).expect("log_tail command failed");
    assert_eq!(res["ok"], true);
    let polls = res["polls"].as_array().expect("polls array");
    assert_eq!(polls.len(), 3);

    // 第一次 poll
    let p1 = polls[0][0].as_array().unwrap();
    assert_eq!(p1.len(), 3);
    assert_eq!(p1[0], "line 1");
    assert_eq!(p1[2], "line 3");

    // 第二次 poll：只读新增的 2 行
    let p2 = polls[1][0].as_array().unwrap();
    assert_eq!(p2.len(), 2);
    assert_eq!(p2[0], "line 4");
    assert_eq!(p2[1], "line 5");

    // 第三次 poll：截断后读，最多保留 500 行
    let p3 = polls[2][0].as_array().unwrap();
    assert_eq!(p3.len(), 500);
    assert_eq!(p3[0], "bulk line 101");
    assert_eq!(p3[499], "bulk line 600");
}

#[test]
fn test_session_state_copy_for_missing_row() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 1. 通过 StatusModel.evaluate 测试
    let mut inp = default_status_input();
    inp["sessions"] = serde_json::json!({
        "示例群": {
            "state": "waiting_for_wechat_window",
            "reason": "session_row_not_unique_or_missing",
            "code": "session_row_not_unique_or_missing"
        }
    });
    let items = run_host_cmd("status", &inp).unwrap();
    let session_item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "sessions")
        .unwrap();
    let reason_text = session_item["reason"].as_str().unwrap();
    assert!(
        reason_text.contains("会话不在列表可见范围"),
        "evaluate() sessions reason 必须含人话文案，实际为: {}",
        reason_text
    );

    // 2. 通过 sessionStateHumanReadable 纯逻辑命令测试
    let payload = serde_json::json!({
        "state": "waiting_for_wechat_window",
        "reason": "session_row_not_unique_or_missing",
        "code": "session_row_not_unique_or_missing"
    });
    let res = run_task5_cmd("session_copy", &payload).expect("session_copy failed");
    assert_eq!(res["ok"], true);
    let text = res["text"].as_str().unwrap();
    assert!(
        text.contains("会话不在列表可见范围"),
        "人话状态文案必须包含「会话不在列表可见范围」，实际为: {}",
        text
    );
}

#[test]
fn test_every_legacy_panel_control_has_new_home() {
    // 源码检查——spec §8.2 每一项（保存配置、重启桌面服务、重启机器人、放回标准位置、恢复自动回复、打开附件文件夹、打开日志文件夹、复制诊断信息、请求屏幕录制权限、请求辅助功能权限、自动管理微信主窗口、各配置字段键名）在新页面文件里都有对应调用。
    let overview_src = fs::read_to_string("native/bootstrap/OverviewPage.swift")
        .expect("OverviewPage.swift 必须存在");
    let sessions_src = fs::read_to_string("native/bootstrap/SessionsPage.swift")
        .expect("SessionsPage.swift 必须存在");
    let settings_src = fs::read_to_string("native/bootstrap/SettingsPage.swift")
        .expect("SettingsPage.swift 必须存在");
    let logs_src =
        fs::read_to_string("native/bootstrap/LogsPage.swift").expect("LogsPage.swift 必须存在");
    let perm_src = fs::read_to_string("native/bootstrap/Permissions.swift")
        .expect("Permissions.swift 必须存在");
    let shell_src =
        fs::read_to_string("native/bootstrap/Shell.swift").expect("Shell.swift 必须存在");

    let all_new_src = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        overview_src, sessions_src, settings_src, logs_src, perm_src, shell_src
    );

    // 1. 保存配置
    assert!(
        all_new_src.contains("保存配置") || all_new_src.contains("saveConfig"),
        "必须包含保存配置操作"
    );
    // 2. 重启桌面服务
    assert!(
        all_new_src.contains("重启桌面服务") || all_new_src.contains("restartDesktopService"),
        "必须包含重启桌面服务操作"
    );
    // 3. 重启机器人
    assert!(
        all_new_src.contains("重启机器人") || all_new_src.contains("restartBot"),
        "必须包含重启机器人操作"
    );
    // 4. 放回标准位置
    assert!(
        all_new_src.contains("放回标准位置") || all_new_src.contains("repositionWechat"),
        "必须包含微信放回标准位置操作"
    );
    // 5. 恢复自动回复
    assert!(
        all_new_src.contains("恢复自动回复") || all_new_src.contains("resumeHandoff"),
        "必须包含恢复自动回复操作"
    );
    // 6. 打开附件文件夹
    assert!(
        all_new_src.contains("打开附件文件夹") || all_new_src.contains("openAttachments"),
        "必须包含打开附件文件夹操作"
    );
    // 7. 打开日志文件夹
    assert!(
        all_new_src.contains("打开日志文件夹") || all_new_src.contains("openLogs"),
        "必须包含打开日志文件夹操作"
    );
    // 8. 复制诊断信息
    assert!(
        all_new_src.contains("复制诊断信息") || all_new_src.contains("copyDiagnostics"),
        "必须包含复制诊断信息操作"
    );
    // 9. 请求屏幕录制权限
    assert!(
        all_new_src.contains("screen")
            && (all_new_src.contains("requestPermission")
                || all_new_src.contains("requestScreenRecording")),
        "必须包含请求屏幕录制权限"
    );
    // 10. 请求辅助功能权限
    assert!(
        all_new_src.contains("accessibility")
            && (all_new_src.contains("requestPermission")
                || all_new_src.contains("requestAccessibility")),
        "必须包含请求辅助功能权限"
    );
    // 11. 自动管理微信主窗口
    assert!(
        all_new_src.contains("manage_window") || all_new_src.contains("自动管理微信主窗口"),
        "必须包含自动管理微信主窗口控件"
    );

    // 12. 各配置字段键名
    for key in &[
        "cooldown_seconds",
        "turn_quiet_seconds",
        "turn_max_seconds",
        "max_attachment_mb",
        "max_replies_per_hour",
        "context_messages",
    ] {
        assert!(
            settings_src.contains(key),
            "SettingsPage 必须包含配置字段键名: {}",
            key
        );
    }
}

#[test]
fn test_settings_fields_keep_validation_ranges() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 设置页保存走 ConfigFormLogic，越界值（冷却 301）返回错误提示。
    let payload = serde_json::json!({
        "original_config": {
            "schema_version": 1,
            "account_id": "test_acc",
            "cooldown_seconds": 15
        },
        "form_inputs": {
            "cooldown_seconds": "301"
        }
    });
    let res = run_task5_cmd("settings_validate", &payload).expect("settings_validate failed");
    assert_eq!(res["ok"], false, "冷却 301 必须校验失败");
    let err = res["error"].as_str().unwrap();
    assert!(
        err.contains("cooldown_seconds") || err.contains("冷却") || err.contains("300"),
        "错误提示必须说明范围，实际: {}",
        err
    );
}

/// 新界面不再显示旧面板，但旧面板控制器的 viewDidLoad 负责读取配置并启动 5 秒定时器
/// （定时器里调用 CoreSupervisor.checkAndReconcile：启动核心、崩溃后重启）。
/// 不加载它，宿主启动后核心永远不会被拉起（2026-11-08 审查 Task 5 时发现）。
#[test]
fn test_host_loads_status_controller_so_core_supervision_runs() {
    let src = fs::read_to_string("native/bootstrap/main.swift").unwrap();
    let created = src
        .find("StatusPanelController(worker:")
        .expect("controller created");
    let shell = src.find("ShellViewController()").expect("shell created");
    let load = src
        .find("_ = controller.view")
        .expect("main.swift 必须访问 controller.view 以触发 viewDidLoad");
    assert!(
        created < load && load < shell,
        "必须在创建页面之前加载控制器（页面需要它已读入的配置）"
    );
}

fn host_task6_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("host_task6_selftest");

        let files_to_compile = [
            root.join("native").join("bootstrap").join("Probe.swift"),
            root.join("native")
                .join("bootstrap")
                .join("Supervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("CoreSupervisor.swift"),
            root.join("native")
                .join("bootstrap")
                .join("SocketWorker.swift"),
            root.join("native").join("bootstrap").join("Actions.swift"),
            root.join("native")
                .join("bootstrap")
                .join("StatusModel.swift"),
            root.join("native")
                .join("bootstrap")
                .join("ConfigForm.swift"),
            root.join("native")
                .join("bootstrap")
                .join("StatusPanel.swift"),
            root.join("native").join("bootstrap").join("LogTail.swift"),
            root.join("native")
                .join("bootstrap")
                .join("Permissions.swift"),
            root.join("native").join("bootstrap").join("Shell.swift"),
            root.join("native")
                .join("bootstrap")
                .join("SettingsPage.swift"),
            root.join("native")
                .join("bootstrap")
                .join("ContactsPage.swift"),
            root.join("native").join("bootstrap").join("McpPage.swift"),
            root.join("native").join("eyes").join("Screens.swift"),
            root.join("tests")
                .join("host_task6_selftest")
                .join("main.swift"),
        ];

        for f in &files_to_compile {
            if !f.exists() {
                eprintln!("required Swift file does not exist: {}", f.display());
                return None;
            }
        }

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        for f in &files_to_compile {
            cmd.arg(f);
        }
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of host_task6_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn run_task6_cmd(command: &str, payload: &Value) -> Result<Value, String> {
    let bin = host_task6_selftest_bin()
        .ok_or_else(|| "host_task6_selftest binary not available".to_string())?;
    let payload_str = serde_json::to_string(payload).unwrap();
    let output = Command::new(bin)
        .env("CFFIXED_USER_HOME", swift_test_home())
        .arg(command)
        .arg(&payload_str)
        .output()
        .map_err(|e| format!("failed to execute host_task6_selftest: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "host_task6_selftest {} failed (code {:?}): {}",
            command,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid json output: {}", e))
}

#[test]
fn test_apply_handling_ignore_moves_to_ignorelist() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let payload = serde_json::json!({
        "config": {
            "schema_version": 1,
            "account_id": "test_acc",
            "allowlist": [{"recipient": "张三", "kind": "private"}],
            "ignorelist": []
        },
        "name": "李四",
        "kind": "private",
        "category": "personal",
        "handling": "ignored"
    });
    let res = run_task6_cmd("apply_handling", &payload).expect("apply_handling failed");
    assert_eq!(res["ok"], true);
    let updated = &res["updated"];
    let ignorelist = updated["ignorelist"].as_array().expect("ignorelist array");
    assert!(
        ignorelist.iter().any(|item| item["recipient"] == "李四"),
        "李四 必须在 ignorelist 中"
    );
    let allowlist = updated["allowlist"].as_array().expect("allowlist array");
    assert!(
        allowlist.iter().any(|item| item["recipient"] == "张三"),
        "张三 必须仍保留在 allowlist 中"
    );
}

#[test]
fn test_apply_handling_auto_reply_refuses_official() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let payload = serde_json::json!({
        "config": {
            "schema_version": 1,
            "account_id": "test_acc",
            "allowlist": [],
            "ignorelist": []
        },
        "name": "微信运动",
        "kind": "private",
        "category": "official",
        "handling": "auto_reply"
    });
    let res = run_task6_cmd("apply_handling", &payload).expect("apply_handling failed");
    assert_eq!(res["ok"], false, "公众号不可加入自动回复");
    let err = res["error"].as_str().unwrap();
    assert!(
        err.contains("公众号") || err.contains("official") || err.contains("服务号"),
        "错误信息必须说明公众号不可加入自动回复，实际: {}",
        err
    );
}

#[test]
fn test_apply_handling_unignore_restores_previous() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 初始配置：张三在 allowlist
    let initial_config = serde_json::json!({
        "schema_version": 1,
        "account_id": "test_acc",
        "allowlist": [{"recipient": "张三", "kind": "private"}],
        "ignorelist": []
    });

    // 1. 张三设为 ignored
    let p1 = serde_json::json!({
        "config": initial_config,
        "name": "张三",
        "kind": "private",
        "category": "personal",
        "handling": "ignored"
    });
    let r1 = run_task6_cmd("apply_handling", &p1).expect("apply_handling ignored failed");
    assert_eq!(r1["ok"], true);
    let conf_ignored = &r1["updated"];
    let ignores = conf_ignored["ignorelist"].as_array().unwrap();
    assert!(ignores.iter().any(|i| i["recipient"] == "张三"));

    // 2. 张三取消忽略 (unignore)
    let p2 = serde_json::json!({
        "config": conf_ignored,
        "name": "张三",
        "kind": "private",
        "category": "personal",
        "handling": "unignore"
    });
    let r2 = run_task6_cmd("apply_handling", &p2).expect("apply_handling unignore failed");
    assert_eq!(r2["ok"], true);
    let conf_restored = &r2["updated"];
    let restored_ignores = conf_restored["ignorelist"].as_array().unwrap();
    assert!(
        !restored_ignores.iter().any(|i| i["recipient"] == "张三"),
        "取消忽略后不在 ignorelist"
    );
    let restored_allows = conf_restored["allowlist"].as_array().unwrap();
    assert!(
        restored_allows.iter().any(|i| i["recipient"] == "张三"),
        "取消忽略后自动恢复在 allowlist 中"
    );
}

#[test]
fn test_contacts_page_parses_contacts_list_json() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    // 合成符合契约的 contacts-list JSON
    let sample_json = serde_json::json!({
        "account_id": "test-desktop",
        "sync": {
            "finished_at_ms": 1794115000000i64,
            "complete": true,
            "per_category": [
                {"category": "personal", "expected": 2, "synced": 2},
                {"category": "work", "expected": 1, "synced": 1},
                {"category": "official", "expected": 1, "synced": 1}
            ],
            "unreadable_rows": 0,
            "needs_review": 0
        },
        "contacts": [
            {
                "contact_id": "c_1",
                "category": "personal",
                "display_name": "个人好友甲",
                "nickname": "甲",
                "remark": "个人好友甲",
                "wechat_id": "wxid_p1",
                "identity": "strong",
                "region": "北京",
                "source": "扫一扫",
                "added_on": "2023-01-01",
                "tags": ["好友"],
                "enterprise": null,
                "real_name": null,
                "handling": "auto_reply",
                "removed": false,
                "last_message_at_ms": 1794115000000i64,
                "message_count": 5
            },
            {
                "contact_id": "c_2",
                "category": "personal",
                "display_name": "被忽略的朋友",
                "nickname": "朋友乙",
                "remark": "被忽略的朋友",
                "wechat_id": "wxid_p2",
                "identity": "strong",
                "region": "上海",
                "source": "名片",
                "added_on": "2023-02-01",
                "tags": [],
                "enterprise": null,
                "real_name": null,
                "handling": "ignored",
                "removed": false,
                "last_message_at_ms": null,
                "message_count": 0
            },
            {
                "contact_id": "c_3",
                "category": "work",
                "display_name": "企微联系人丙",
                "nickname": "丙",
                "remark": null,
                "wechat_id": null,
                "identity": "weak",
                "region": null,
                "source": null,
                "added_on": null,
                "tags": ["客户"],
                "enterprise": "某某企业",
                "real_name": "李丙",
                "handling": "record_only",
                "removed": false,
                "last_message_at_ms": null,
                "message_count": 0
            },
            {
                "contact_id": "c_4",
                "category": "official",
                "display_name": "服务号丁",
                "nickname": "服务号丁",
                "remark": null,
                "wechat_id": "gh_ding",
                "identity": "strong",
                "region": null,
                "source": null,
                "added_on": null,
                "tags": [],
                "enterprise": null,
                "real_name": null,
                "handling": "record_only",
                "removed": false,
                "last_message_at_ms": null,
                "message_count": 0
            },
            {
                "contact_id": "c_5",
                "category": "group",
                "display_name": "技术交流群",
                "nickname": null,
                "remark": null,
                "wechat_id": null,
                "identity": "strong",
                "region": null,
                "source": null,
                "added_on": null,
                "tags": [],
                "enterprise": null,
                "real_name": null,
                "handling": "auto_reply",
                "removed": false,
                "last_message_at_ms": 1794115100000i64,
                "message_count": 100
            }
        ],
        "reviews": []
    });

    let payload = serde_json::json!({
        "json_str": sample_json.to_string()
    });

    let res = run_task6_cmd("parse_contacts_list", &payload).expect("parse_contacts_list failed");
    assert_eq!(res["ok"], true);
    assert_eq!(res["contact_count"], 5);

    let counts = &res["counts"];
    assert_eq!(counts["personal"], 2, "personal 计数必须为 2");
    assert_eq!(counts["work"], 1, "work 计数必须为 1");
    assert_eq!(counts["group"], 1, "group 计数必须为 1");
    assert_eq!(counts["official"], 1, "official 计数必须为 1");
    assert_eq!(counts["ignored"], 1, "ignored 计数必须为 1");
}

/// 用户 2026-11-08 截图：微信号一列对没有微信号的人一律写"（企微无微信号）"，联系人分组里也会这样显示。
/// 列表里没有微信号只显示 —，原因放在详情里；同步状态用中文分组名。
#[test]
fn test_contacts_page_wechat_id_text_and_titles() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let sample = serde_json::json!({
        "account_id": "wxid_a",
        "contacts": [
            {"contact_id": "c1", "category": "work", "display_name": "企微甲", "identity": "weak", "handling": "record_only"},
            {"contact_id": "c2", "category": "personal", "display_name": "个人乙", "identity": "weak", "handling": "record_only"},
            {"contact_id": "c3", "category": "personal", "display_name": "个人丙", "wechat_id": "wxid_c", "identity": "strong", "handling": "record_only"},
            {"contact_id": "c4", "category": "personal", "display_name": "聊天丁", "handling": "auto_reply"}
        ]
    });
    let res = run_task6_cmd(
        "parse_contacts_list",
        &serde_json::json!({"json_str": sample.to_string()}),
    )
    .expect("parse");
    assert_eq!(
        res["list_wechat"],
        serde_json::json!(["—", "—", "wxid_c", "—"])
    );
    let detail = res["detail_wechat"].as_array().unwrap();
    assert!(detail[0].as_str().unwrap().contains("企业微信"));
    assert!(
        !detail[1].as_str().unwrap().contains("企"),
        "联系人分组不能显示成企微: {}",
        detail[1]
    );
    assert_eq!(
        res["titles"],
        serde_json::json!(["联系人", "企业微信", "群聊", "公众号"])
    );
}

/// 用户 2026-11-08 截图：企业微信 12 人列表只显示 7 行（列表没占满剩余高度、被截断）；详情面板一直显示。
/// 列表必须占满剩余高度；详情面板默认隐藏，选中联系人才出现。
#[test]
fn test_contacts_page_list_fills_height_and_detail_on_select() {
    let src =
        std::fs::read_to_string(repo_root().join("native/bootstrap/ContactsPage.swift")).unwrap();
    assert!(
        src.contains("mainStack.distribution = .fill"),
        "主区域要把剩余高度分给列表"
    );
    assert!(src.contains(
        "contentRow.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)"
    ));
    assert!(
        !src.contains("NSSplitView()"),
        "详情面板隐藏再显示时分栏会把它留成 0 宽"
    );
    let setup = &src[src.find("private func setupUI()").unwrap()
        ..src.find("private func setupDetailBox()").unwrap()];
    assert!(
        setup.contains("detailBox.isHidden = true"),
        "详情面板默认隐藏"
    );
    let show = &src[src.find("private func showDetail(").unwrap()..];
    assert!(
        show[..400].contains("detailBox.isHidden = false"),
        "选中联系人才显示详情"
    );
    let clear = &src[src.find("private func clearDetail()").unwrap()..];
    assert!(
        clear[..200].contains("detailBox.isHidden = true"),
        "取消选择收起详情"
    );
    assert!(!src.contains("企微无微信号"));
    // 用户 2026-11-08 截图：名字被直接裁掉，看起来不完整。放不下要显示"…"并可悬停看全名
    let name_cell = &src[src.find("case \"NameCol\":").unwrap()..];
    assert!(name_cell[..500].contains(".byTruncatingTail") && name_cell[..500].contains("toolTip"));
}

#[test]
fn test_mcp_page_brain_settings_round_trip() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let initial_config = serde_json::json!({
        "schema_version": 1,
        "account_id": "test_acc",
        "brain": {
            "mode": "builtin"
        }
    });

    let payload = serde_json::json!({
        "config": initial_config,
        "timeout_seconds": 120,
        "timeout_action": "handoff",
        "queue_minutes": 30
    });

    let res = run_task6_cmd("mcp_brain_settings", &payload).expect("mcp_brain_settings failed");
    assert_eq!(res["ok"], true);
    let updated = &res["updated"];
    let brain = &updated["brain"];
    assert_eq!(brain["mode"], "external");
    let ext = &brain["external"];
    assert_eq!(ext["timeout_seconds"], 120);
    assert_eq!(ext["timeout_action"], "handoff");
    assert_eq!(ext["queue_minutes"], 30);
}

#[test]
fn test_contacts_and_mcp_pages_registered_in_shell_order() {
    let src = fs::read_to_string("native/bootstrap/main.swift").expect("main.swift exists");
    let overview_idx = src.find("\"overview\"").expect("overview registered");
    let sessions_idx = src.find("\"sessions\"").expect("sessions registered");
    let contacts_idx = src.find("\"contacts\"").expect("contacts registered");
    let mcp_idx = src.find("\"mcp\"").expect("mcp registered");
    let settings_idx = src.find("\"settings\"").expect("settings registered");
    let logs_idx = src.find("\"logs\"").expect("logs registered");

    assert!(overview_idx < sessions_idx, "总览在会话之前");
    assert!(sessions_idx < contacts_idx, "会话在联系人之前");
    assert!(contacts_idx < mcp_idx, "联系人在MCP之前");
    assert!(mcp_idx < settings_idx, "MCP在设置之前");
    assert!(settings_idx < logs_idx, "设置在日志之前");
}

#[test]
fn test_settings_page_brain_and_allowlist_pointers() {
    let settings_src = fs::read_to_string("native/bootstrap/SettingsPage.swift")
        .expect("SettingsPage.swift exists");
    assert!(
        settings_src.contains("联系人") && settings_src.contains("MCP"),
        "SettingsPage 必须包含指向联系人页和 MCP 页的说明"
    );
    assert!(
        settings_src.contains("允许名单") || settings_src.contains("忽略名单"),
        "SettingsPage 说明中必须包含允许名单/忽略名单"
    );
    assert!(
        settings_src.contains("外部大脑") || settings_src.contains("大脑配置"),
        "SettingsPage 说明中必须包含大脑配置"
    );
}

/// 页面读取 control 命令输出时不能先 waitUntilExit 再读管道：输出超过管道缓冲（约 64KB，
/// 几百个联系人的 contacts-list 就会超过）时子进程卡在写、父进程卡在等，页面永远加载不出来。
#[test]
fn test_run_control_command_reads_large_output_without_deadlock() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let big = run_task6_cmd("run_control", &serde_json::json!({
        "executable": "/bin/sh", "args": ["-c", "head -c 300000 /dev/zero | tr '\\0' x"], "timeout": 10
    })).unwrap();
    assert_eq!(big["status"], 0, "{big}");
    assert_eq!(big["bytes"], 300000, "{big}");
    assert_eq!(big["timed_out"], false, "{big}");

    let hung = run_task6_cmd(
        "run_control",
        &serde_json::json!({
            "executable": "/bin/sh", "args": ["-c", "sleep 60"], "timeout": 1
        }),
    )
    .unwrap();
    assert_eq!(hung["timed_out"], true, "{hung}");
    assert!(hung["seconds"].as_f64().unwrap() < 5.0, "{hung}");
}

/// 专用 Mac 上是 Swift 5.7（开发机更新）：部署时在设备上编译，较新的语法在开发机能过、到设备就失败
/// （2026-11-08：`extension String: @retroactive Error` 让宿主应用在设备上编译失败）。
/// 禁止已知的 5.7 不支持的写法，并禁止把 String 当 Error 用。
#[test]
fn test_native_sources_avoid_syntax_newer_than_swift_5_7() {
    let mut offenders = Vec::new();
    let mut stack = vec![repo_root().join("native")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map_or(false, |e| e == "swift") {
                let src = fs::read_to_string(&path).unwrap();
                for (i, line) in src.lines().enumerate() {
                    let t = line.trim_start();
                    if t.starts_with("//") {
                        continue;
                    }
                    for bad in [
                        "@retroactive",
                        "extension String: Error",
                        "throws(",
                        "#Preview",
                        "package func ",
                        "consuming ",
                        "borrowing ",
                    ] {
                        if line.contains(bad) {
                            offenders.push(format!(
                                "{}:{}: {}",
                                path.display(),
                                i + 1,
                                line.trim()
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "设备上的 Swift 5.7 不支持这些写法:\n{}",
        offenders.join("\n")
    );
}

/// NSBox 直接替换 contentView 时，框的高度不会随内容撑开：内容溢出、互相重叠（2026-11-08 设备上
/// 所有页面右侧"挤在一起"）。页面里的分组框必须用 setContent(_:)（内容加进框自带的 contentView 并四边约束）。
#[test]
fn test_pages_never_replace_nsbox_content_view() {
    let mut offenders = Vec::new();
    for entry in fs::read_dir(repo_root().join("native/bootstrap")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(true, |e| e != "swift") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//")
                || t.contains("scrollView")
                || t.contains("window.")
                || t.contains("guard let host")
                || t.contains("self.contentView = content")
            {
                continue;
            }
            if t.contains(".contentView = ") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1,
                    t
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "分组框内容必须用 setContent(_:)：\n{}",
        offenders.join("\n")
    );
}

#[test]
fn config_check_reports_rust_errors() {
    if !has_swiftc() {
        return;
    }
    for (status, output, ok, error) in [
        (0, r#"{"ok":true}"#, true, ""),
        (0, r#"{"ok":false,"error":"配置无效"}"#, false, "配置无效"),
        (2, r#"{"ok":true}"#, false, "config-check 校验失败 (exit 2)"),
        (2, "stderr diagnostic", false, "stderr diagnostic"),
    ] {
        let result = run_task6_cmd(
            "config_check_result",
            &serde_json::json!({
                "status": status, "output": output
            }),
        )
        .unwrap();
        assert_eq!(result["ok"], ok);
        assert_eq!(result["error"], error);
    }
}

#[test]
fn test_main_window_prefers_on_screen() {
    if !has_swiftc() {
        return;
    }
    let payload = serde_json::json!({
        "windows": [
            { "id": 23647, "x": 290.0, "y": 176.0, "width": 700.0, "height": 640.0, "on_screen": false },
            { "id": 23645, "x": 10.0, "y": 183.0, "width": 880.0, "height": 640.0, "on_screen": true }
        ]
    });
    let res = run_host_cmd("select_main_window", &payload).unwrap();
    assert_eq!(res["width"], 880.0);
    assert_eq!(res["height"], 640.0);
    assert_eq!(res["x"], 10.0);
    assert_eq!(res["y"], 183.0);
}

#[test]
fn test_main_window_ignores_offscreen_only() {
    if !has_swiftc() {
        return;
    }
    let payload = serde_json::json!({
        "windows": [
            { "id": 23647, "x": 290.0, "y": 176.0, "width": 700.0, "height": 640.0, "on_screen": false }
        ]
    });
    let res = run_host_cmd("select_main_window", &payload).unwrap();
    assert!(res.is_null(), "expected null but got: {:?}", res);
}

#[test]
fn test_main_window_largest_on_screen() {
    if !has_swiftc() {
        return;
    }
    let payload = serde_json::json!({
        "windows": [
            { "id": 23647, "x": 290.0, "y": 176.0, "width": 700.0, "height": 640.0, "on_screen": true },
            { "id": 23645, "x": 10.0, "y": 183.0, "width": 880.0, "height": 640.0, "on_screen": true }
        ]
    });
    let res = run_host_cmd("select_main_window", &payload).unwrap();
    assert_eq!(res["width"], 880.0);
    assert_eq!(res["height"], 640.0);
    assert_eq!(res["x"], 10.0);
    assert_eq!(res["y"], 183.0);
}

#[test]
fn test_swift_core_supervisor_dual_check_guard_and_uuid_contract() {
    if !has_swiftc() {
        eprintln!("swiftc not available, skipping test");
        return;
    }
    let temp = TempDir::new("swift_dual_guard_test");
    let state_dir = temp.path.join("state");
    let releases_dir = temp.path.join("releases");
    let current_link = temp.path.join("current");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&releases_dir).unwrap();
    fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&releases_dir, fs::Permissions::from_mode(0o700)).unwrap();

    let root = repo_root();
    let swift_test_source = temp.path.join("main.swift");
    let swift_test_bin = temp.path.join("dual_guard_bin");

    let swift_code = r#"
import Foundation

func checkMode() -> String {
    let args = CommandLine.arguments
    guard args.count >= 5 else { return "ERR_ARGS" }
    let statePath = args[1]
    let releasesPath = args[2]
    let currentPath = args[3]
    let expectedMode = args[4]
    let legacyPath = args.count > 5 ? args[5] : ""
    let legacyURL = legacyPath.isEmpty ? nil : URL(fileURLWithPath: legacyPath)

    let stateURL = URL(fileURLWithPath: statePath)
    let releasesURL = URL(fileURLWithPath: releasesPath)
    let currentURL = URL(fileURLWithPath: currentPath)
    let maintURL = stateURL.appendingPathComponent("core.maintenance")
    let journalURL = stateURL.appendingPathComponent("update-journal.json")
    let configURL = stateURL.appendingPathComponent("isolated_config.json")

    let supervisor = CoreSupervisor(
        coreEnabledURL: stateURL.appendingPathComponent("core.enabled"),
        maintenanceURL: maintURL,
        configURL: configURL,
        journalURL: journalURL,
        releasesDirURL: releasesURL,
        currentLinkURL: currentURL,
        legacyStateDirURL: legacyURL,
        allowMissingConfigForTesting: true
    )

    let status = supervisor.maintenanceStatus
    switch status {
    case .inactive:
        return "INACTIVE"
    case .active(let txID):
        return "ACTIVE:\(txID)"
    case .recoveryRequired:
        return "RECOVERY_REQUIRED"
    }
}

print(checkMode())
"#;

    fs::write(&swift_test_source, swift_code).unwrap();

    let files_to_compile = [
        root.join("native")
            .join("bootstrap")
            .join("Supervisor.swift"),
        root.join("native")
            .join("bootstrap")
            .join("CoreSupervisor.swift"),
        swift_test_source.clone(),
    ];

    let mut compile_cmd = Command::new("swiftc");
    compile_cmd.arg("-O");
    for f in &files_to_compile {
        compile_cmd.arg(f);
    }
    compile_cmd.arg("-o").arg(&swift_test_bin);
    let comp_res = compile_cmd.output().expect("failed to run swiftc");
    assert!(
        comp_res.status.success(),
        "swiftc compilation failed: {}",
        String::from_utf8_lossy(&comp_res.stderr)
    );

    let empty_legacy = temp.path.join("empty_legacy");
    fs::create_dir_all(&empty_legacy).unwrap();
    fs::set_permissions(&empty_legacy, fs::Permissions::from_mode(0o700)).unwrap();

    let run_guard = |expected: &str| -> String {
        let output = Command::new(&swift_test_bin)
            .arg(state_dir.to_str().unwrap())
            .arg(releases_dir.to_str().unwrap())
            .arg(current_link.to_str().unwrap())
            .arg(expected)
            .arg(empty_legacy.to_str().unwrap())
            .output()
            .expect("failed to run dual_guard_bin");
        assert!(
            output.status.success(),
            "run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    // 场景 A: 初始空目录 -> INACTIVE
    assert_eq!(run_guard("INACTIVE"), "INACTIVE");

    // 场景 B: 写入 Rust 规范的 36 字节 UUID -> ACTIVE:<uuid>
    let raw_maint = state_dir.join("core.maintenance");
    let test_uuid = "12345678-1234-4234-8234-123456789abc";
    fs::write(&raw_maint, test_uuid.as_bytes()).unwrap();
    fs::set_permissions(&raw_maint, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run_guard("ACTIVE"), format!("ACTIVE:{}", test_uuid));

    // 场景 C: 损坏的 core.maintenance (小于 36 字节) -> RECOVERY_REQUIRED
    fs::write(&raw_maint, b"too_short").unwrap();
    fs::set_permissions(&raw_maint, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 清理 core.maintenance
    fs::remove_file(&raw_maint).unwrap();
    assert_eq!(run_guard("INACTIVE"), "INACTIVE");

    // 场景 D: 存在中间状态 update-journal.json (Draining) -> ACTIVE:<tx_id>
    let journal_path = state_dir.join("update-journal.json");
    let valid_sha256 = "6b3a2f1c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a";
    let journal_draining = serde_json::json!({
        "journal_schema": 1,
        "transaction_id": "tx-88d447d2-749e-4e89-a5c3-1d89851148e6",
        "stage": "Draining",
        "source_release_id": "0.1.0",
        "target_release_id": "0.2.0-test",
        "target_sequence": 1002,
        "timestamp": 1794300000,
        "maintenance_marker_confirmed": true
    });
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_draining).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        run_guard("ACTIVE"),
        "ACTIVE:tx-88d447d2-749e-4e89-a5c3-1d89851148e6"
    );

    // 场景 E: journal 为 Committed 但缺少 current_release.json -> RECOVERY_REQUIRED (严格拒绝盲猜放行)
    let journal_committed = serde_json::json!({
        "journal_schema": 1,
        "transaction_id": "tx-88d447d2-749e-4e89-a5c3-1d89851148e6",
        "stage": "Committed",
        "source_release_id": "0.1.0",
        "target_release_id": "0.2.0-test",
        "target_sequence": 1002,
        "manifest_digest": valid_sha256,
        "timestamp": 1794300000,
        "maintenance_marker_confirmed": true
    });
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_committed).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 场景 F: 提供 current_release.json 但要素不一致 (sequence 篡改) -> RECOVERY_REQUIRED
    let record_path = state_dir.join("current_release.json");
    let commit_record_tampered = serde_json::json!({
        "record_schema": 1,
        "tx_id": "tx-88d447d2-749e-4e89-a5c3-1d89851148e6",
        "release_id": "0.2.0-test",
        "sequence": 9999,
        "channel": "stable",
        "manifest_digest": valid_sha256,
        "installed_at": 1794300000
    });
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&commit_record_tampered).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 场景 G: 修正 current_release.json 要素，建立正确的 releases 目录与 current 软链接 -> INACTIVE 放行
    let commit_record_correct = serde_json::json!({
        "record_schema": 1,
        "tx_id": "tx-88d447d2-749e-4e89-a5c3-1d89851148e6",
        "release_id": "0.2.0-test",
        "sequence": 1002,
        "channel": "stable",
        "manifest_digest": valid_sha256,
        "installed_at": 1794300000
    });
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&commit_record_correct).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600)).unwrap();

    let target_release_dir = releases_dir.join("0.2.0-test");
    fs::create_dir_all(&target_release_dir).unwrap();
    std::os::unix::fs::symlink(&target_release_dir, &current_link).unwrap();

    assert_eq!(run_guard("INACTIVE"), "INACTIVE");

    // 场景 H: 篡改 current 软链接指向非预期目录 -> RECOVERY_REQUIRED
    fs::remove_file(&current_link).unwrap();
    let foreign_dir = releases_dir.join("0.9.9-foreign");
    fs::create_dir_all(&foreign_dir).unwrap();
    std::os::unix::fs::symlink(&foreign_dir, &current_link).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 场景 I: 防 /tmp 外部目录逃逸：若 current 指向 /tmp 下同名目录，必须 Fail-Closed
    fs::remove_file(&current_link).unwrap();
    let tmp_escape_dir = temp.path.join("external_tmp").join("0.2.0-test");
    fs::create_dir_all(&tmp_escape_dir).unwrap();
    std::os::unix::fs::symlink(&tmp_escape_dir, &current_link).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 场景 J: releases/id 自身为 symlink 指向外部且 current 同指外部：必须拦截！
    fs::remove_file(&current_link).unwrap();
    fs::remove_dir(&target_release_dir).unwrap();
    std::os::unix::fs::symlink(&tmp_escape_dir, &target_release_dir).unwrap();
    std::os::unix::fs::symlink(&tmp_escape_dir, &current_link).unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");
    // 恢复正常物理目录与链接
    fs::remove_file(&current_link).unwrap();
    fs::remove_file(&target_release_dir).unwrap();
    fs::create_dir_all(&target_release_dir).unwrap();
    std::os::unix::fs::symlink(&target_release_dir, &current_link).unwrap();

    // 场景 K: RolledBack 终态一律拦截为 RECOVERY_REQUIRED（在缺乏独立恢复协议前严禁放行）
    let journal_rolled_back = serde_json::json!({
        "journal_schema": 1,
        "transaction_id": "tx-88d447d2-749e-4e89-a5c3-1d89851148e6",
        "stage": "RolledBack",
        "source_release_id": "0.1.0",
        "target_release_id": "0.2.0-test",
        "target_sequence": 1002,
        "timestamp": 1794300000,
        "maintenance_marker_confirmed": true
    });
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_rolled_back).unwrap(),
    )
    .unwrap();
    assert_eq!(run_guard("RECOVERY_REQUIRED"), "RECOVERY_REQUIRED");

    // 恢复为合法 Committed 终态全要素 -> INACTIVE
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_committed).unwrap(),
    )
    .unwrap();
    assert_eq!(run_guard("INACTIVE"), "INACTIVE");

    // 场景 L: 注入 legacy 目录且存在未完成 core.maintenance -> 严格 RECOVERY_REQUIRED 且不删除现场
    let legacy_dir = temp.path.join("legacy_state");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_maint = legacy_dir.join("core.maintenance");
    let legacy_uuid = "00000000-0000-4000-8000-000000000099";
    fs::write(&legacy_maint, legacy_uuid.as_bytes()).unwrap();
    fs::set_permissions(&legacy_maint, fs::Permissions::from_mode(0o600)).unwrap();

    let run_guard_legacy = |expected: &str, leg: &Path| -> String {
        let output = Command::new(&swift_test_bin)
            .arg(state_dir.to_str().unwrap())
            .arg(releases_dir.to_str().unwrap())
            .arg(current_link.to_str().unwrap())
            .arg(expected)
            .arg(leg.to_str().unwrap())
            .output()
            .expect("failed to run dual_guard_bin with legacy");
        assert!(
            output.status.success(),
            "run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    assert_eq!(
        run_guard_legacy("RECOVERY_REQUIRED", &legacy_dir),
        "RECOVERY_REQUIRED"
    );
    // 验证严格只读零删除现场
    assert!(
        legacy_maint.exists(),
        "legacy maintenance marker must NOT be deleted by probe"
    );

    // 场景 M: legacy 损坏（非 36 字节）同样 Fail-Closed 判定 RECOVERY_REQUIRED 且不删除
    fs::write(&legacy_maint, b"bad_content").unwrap();
    assert_eq!(
        run_guard_legacy("RECOVERY_REQUIRED", &legacy_dir),
        "RECOVERY_REQUIRED"
    );
    assert!(
        legacy_maint.exists(),
        "corrupt legacy file must remain untouched"
    );
    fs::remove_file(&legacy_maint).unwrap();
    let old_journal = legacy_dir.join("update-journal.json");
    let committed_label = br#"{"journal_schema":1,"stage":"Committed"}"#;
    fs::write(&old_journal, committed_label).unwrap();
    fs::set_permissions(&old_journal, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        run_guard_legacy("RECOVERY_REQUIRED", &legacy_dir),
        "RECOVERY_REQUIRED"
    );
    assert_eq!(fs::read(&old_journal).unwrap(), committed_label);
}

#[test]
fn test_swift_core_supervisor_reload_config_and_conflict_fail_closed() {
    let temp = TempDir::new("swift_cfg_reload");
    let root = repo_root();
    let bin_path = temp.path.join("cfg_reload_bin");
    let swift_src = temp.path.join("main.swift");

    let cfg_file = temp.path.join("config.json");
    let state_dir = temp.path.join("dyn_state");
    fs::create_dir_all(&state_dir).unwrap();

    let swift_code = r#"
import Foundation

func main() -> String {
    let args = CommandLine.arguments
    guard args.count >= 2 else { return "ERR_ARGS" }
    let cfgPath = args[1]
    let cfgURL = URL(fileURLWithPath: cfgPath)

    let supervisor = CoreSupervisor(configURL: cfgURL)
    let status = supervisor.maintenanceStatus
    switch status {
    case .inactive:
        return "INACTIVE"
    case .active(let id):
        return "ACTIVE:\(id)"
    case .recoveryRequired:
        return "RECOVERY_REQUIRED"
    }
}

print(main())
"#;
    fs::write(&swift_src, swift_code).unwrap();

    let files = [
        root.join("native")
            .join("bootstrap")
            .join("Supervisor.swift"),
        root.join("native")
            .join("bootstrap")
            .join("CoreSupervisor.swift"),
        swift_src,
    ];

    let mut cmd = Command::new("swiftc");
    cmd.arg("-O");
    for f in &files {
        cmd.arg(f);
    }
    cmd.arg("-o").arg(&bin_path);
    let comp = cmd.output().expect("compile swiftc");
    assert!(
        comp.status.success(),
        "swiftc failed: {}",
        String::from_utf8_lossy(&comp.stderr)
    );

    let run_bin = || -> String {
        let out = Command::new(&bin_path)
            .env("CFFIXED_USER_HOME", swift_test_home())
            .arg(cfg_file.to_str().unwrap())
            .output()
            .expect("run cfg_reload_bin");
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // 1. 配置文件尚不存在 -> 生产必须 Fail-Closed 判定 RECOVERY_REQUIRED
    assert_eq!(run_bin(), "RECOVERY_REQUIRED");

    // 2. 写入非法相对路径 state_dir -> 严格判定 RECOVERY_REQUIRED
    let bad_cfg1 = serde_json::json!({
        "schema_version": 1,
        "state_dir": "relative/state"
    });
    fs::write(&cfg_file, serde_json::to_vec_pretty(&bad_cfg1).unwrap()).unwrap();
    fs::set_permissions(&cfg_file, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(run_bin(), "RECOVERY_REQUIRED");

    // 3. 写入非法类型 install_root (数字/布尔/null/数组) -> 必须严格判定 RECOVERY_REQUIRED，严禁静默吞掉
    let bad_types = vec![
        serde_json::json!(12345),
        serde_json::json!(true),
        serde_json::json!(null),
        serde_json::json!(["/some/path"]),
    ];
    for bt in bad_types {
        let bad_cfg_type = serde_json::json!({
            "schema_version": 1,
            "state_dir": state_dir.to_str().unwrap(),
            "install_root": bt
        });
        fs::write(&cfg_file, serde_json::to_vec_pretty(&bad_cfg_type).unwrap()).unwrap();
        assert_eq!(run_bin(), "RECOVERY_REQUIRED");
    }

    // 4. 写入非法相对路径 install_root -> 严格判定 RECOVERY_REQUIRED
    let bad_cfg2 = serde_json::json!({
        "schema_version": 1,
        "state_dir": state_dir.to_str().unwrap(),
        "install_root": "relative/install"
    });
    fs::write(&cfg_file, serde_json::to_vec_pretty(&bad_cfg2).unwrap()).unwrap();
    assert_eq!(run_bin(), "RECOVERY_REQUIRED");

    // 5. 修复为合法绝对路径 state_dir 与 install_root -> 成功解析并判定 INACTIVE
    let install_dir = temp.path.join("dyn_install");
    fs::create_dir_all(&install_dir).unwrap();
    let good_cfg = serde_json::json!({
        "schema_version": 1,
        "state_dir": state_dir.to_str().unwrap(),
        "install_root": install_dir.to_str().unwrap()
    });
    fs::write(&cfg_file, serde_json::to_vec_pretty(&good_cfg).unwrap()).unwrap();
    assert_eq!(run_bin(), "INACTIVE");

    // 6. 原配置 Some(root) 改 None 后必须显式重置为未配置状态，不能残留旧 releases/current
    let no_root_cfg = serde_json::json!({
        "schema_version": 1,
        "state_dir": state_dir.to_str().unwrap()
    });
    fs::write(&cfg_file, serde_json::to_vec_pretty(&no_root_cfg).unwrap()).unwrap();

    // 在 state_dir 写入 Committed 的 journal
    let journal_path = state_dir.join("update-journal.json");
    let valid_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let journal_committed = serde_json::json!({
        "journal_schema": 1,
        "transaction_id": "tx-00000000-0000-4000-8000-000000000001",
        "stage": "Committed",
        "source_release_id": "1.0.0",
        "target_release_id": "1.0.1",
        "target_sequence": 2,
        "manifest_digest": valid_sha,
        "timestamp": 1000,
        "maintenance_marker_confirmed": true
    });
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_committed).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();

    // 因为 install_root 已被移除，处于未配置状态，Committed 终态全要素检查必须严格拒绝（RECOVERY_REQUIRED）！
    assert_eq!(run_bin(), "RECOVERY_REQUIRED");
}
