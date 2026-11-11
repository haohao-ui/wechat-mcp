use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use wechat_mcp::core::config::BotConfig;
use wechat_mcp::core::control::{ControlPaths, execute_control_with_paths};
use wechat_mcp::updater::journal;

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

fn write_config_json(dir: &Path, install_root: Option<&Path>) -> (PathBuf, PathBuf) {
    let state_dir = dir.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let cfg_path = dir.join("config.json");

    let mut map = serde_json::json!({
        "schema_version": 1,
        "host": "local",
        "account_id": "test_account",
        "state_dir": state_dir.to_str().unwrap(),
        "allowlist": [
            {
                "recipient": "test_user",
                "kind": "private"
            }
        ]
    });

    if let Some(ir) = install_root {
        map["install_root"] = serde_json::json!(ir.to_str().unwrap());
    }

    fs::write(&cfg_path, serde_json::to_vec_pretty(&map).unwrap()).unwrap();
    (cfg_path, state_dir)
}

#[test]
fn test_bot_config_install_root_absolute_validation() {
    let dir = TempDir::new("cfg_root_test");

    // 1. 未配置 install_root: 正常解析为 None
    let (cfg_none, _) = write_config_json(&dir.path, None);
    let loaded_none = BotConfig::load(&cfg_none).expect("config without install_root should load");
    assert!(loaded_none.install_root.is_none());

    // 2. 相对路径 install_root: 必须被严格拒绝
    let cfg_rel = dir.path.join("config_rel.json");
    let mut raw_map: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cfg_none).unwrap()).unwrap();
    raw_map["install_root"] = serde_json::json!("relative/path/to/install");
    fs::write(&cfg_rel, serde_json::to_vec_pretty(&raw_map).unwrap()).unwrap();

    let err_rel = BotConfig::load(&cfg_rel);
    assert!(err_rel.is_err(), "relative install_root must be rejected");
    let err_msg = err_rel.unwrap_err().to_string();
    assert!(
        err_msg.contains("absolute install_root required"),
        "unexpected error message: {err_msg}"
    );

    // 3. 绝对路径 install_root: 正常解析为 Some(PathBuf)
    let abs_install = dir.path.join("app_root");
    fs::create_dir_all(&abs_install).unwrap();
    let (cfg_abs, _) = write_config_json(&dir.path, Some(&abs_install));
    let loaded_abs =
        BotConfig::load(&cfg_abs).expect("config with absolute install_root should load");
    assert_eq!(loaded_abs.install_root, Some(abs_install));
}

#[test]
fn test_control_update_status_invalid_config_fails_closed() {
    let dir = TempDir::new("ctrl_status_bad_cfg");
    let bad_cfg = dir.path.join("nonexistent_or_bad_config.json");
    let ctrl_paths = ControlPaths::isolated(&dir.path);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = execute_control_with_paths(
        &[
            "wechat-control".into(),
            "update-status".into(),
            "--config".into(),
            bad_cfg.to_str().unwrap().into(),
        ],
        &mut stdout,
        &mut stderr,
        None,
        &ctrl_paths,
    );

    // 必须失败退出（退出码非 0 或标准错误非空）
    assert!(
        code != 0 || !stderr.is_empty(),
        "invalid config must fail closed in update-status without falling back to state dir (code: {code})"
    );
    // 验证未向假想的 base.join("state") 写入任何文件或 fallback 成功
    assert!(
        !dir.path.join("state").exists(),
        "fallback state directory must never be created"
    );
}

#[test]
fn test_legacy_state_maintenance_detected_without_deletion() {
    let dir = TempDir::new("legacy_maint_test");
    let legacy_dir = dir.path.join("legacy_rpa");
    fs::create_dir_all(&legacy_dir).unwrap();

    // 在 legacy 目录写入未完成的维护标记
    let raw_uuid = uuid::Uuid::new_v4().to_string();
    let legacy_maint = legacy_dir.join("core.maintenance");
    fs::write(&legacy_maint, raw_uuid.as_bytes()).unwrap();
    fs::set_permissions(&legacy_maint, fs::Permissions::from_mode(0o600)).unwrap();

    // 探测 legacy 维护标记
    let has_unsettled = journal::detect_legacy_maintenance_marker(&legacy_dir);
    assert!(
        has_unsettled,
        "legacy unresolved maintenance must be detected"
    );
    // 关键契约：严格只读探测，绝不删除旧目录中的任何文件
    assert!(
        legacy_maint.exists(),
        "legacy maintenance marker must be preserved without deletion"
    );
}
