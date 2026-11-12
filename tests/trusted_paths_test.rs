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

#[test]
fn test_bot_config_install_root_invalid_types_rejected() {
    let dir = TempDir::new("cfg_types_test");
    let (cfg_base, _) = write_config_json(&dir.path, None);
    let base_content = fs::read_to_string(&cfg_base).unwrap();

    let invalid_values = vec![
        serde_json::json!(12345),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!(["/absolute/path"]),
        serde_json::json!({"path": "/absolute/path"}),
    ];

    for (idx, bad_val) in invalid_values.into_iter().enumerate() {
        let mut map: serde_json::Value = serde_json::from_str(&base_content).unwrap();
        map["install_root"] = bad_val;
        let bad_cfg_path = dir.path.join(format!("bad_cfg_{}.json", idx));
        fs::write(&bad_cfg_path, serde_json::to_vec_pretty(&map).unwrap()).unwrap();

        let res = BotConfig::load(&bad_cfg_path);
        assert!(
            res.is_err(),
            "invalid install_root type must be rejected: {:?}",
            map["install_root"]
        );
    }
}

#[test]
fn test_legacy_state_corrupt_json_and_incomplete_journal_detected() {
    let dir = TempDir::new("legacy_corrupt_test");
    let legacy_dir = dir.path.join("legacy");
    fs::create_dir_all(&legacy_dir).unwrap();

    // 1. core.maintenance.json 损坏 (格式非 JSON) 必须判定为需阻断，绝不能被吞掉
    let json_maint = legacy_dir.join("core.maintenance.json");
    fs::write(&json_maint, b"not_valid_json_content").unwrap();
    fs::set_permissions(&json_maint, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        journal::detect_legacy_maintenance_marker(&legacy_dir),
        "corrupt legacy core.maintenance.json must be detected as requiring maintenance block"
    );
    assert!(json_maint.exists(), "corrupt json must not be deleted");
    fs::remove_file(&json_maint).unwrap();

    // 2. legacy 目录存在未完成事务的 update-journal.json (Draining) 必须被探测并阻断
    let journal_path = legacy_dir.join("update-journal.json");
    let journal_draining = serde_json::json!({
        "journal_schema": 1,
        "transaction_id": "00000000-0000-4000-8000-000000000001",
        "stage": "Draining",
        "source_release_id": "1.0.0",
        "target_release_id": "1.0.1",
        "target_sequence": 2,
        "timestamp": 1000
    });
    fs::write(
        &journal_path,
        serde_json::to_vec_pretty(&journal_draining).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        journal::detect_legacy_maintenance_marker(&legacy_dir),
        "legacy incomplete update-journal.json must be detected"
    );
    assert!(journal_path.exists(), "legacy journal must not be deleted");
}

#[test]
fn test_install_root_symlink_rejected() {
    let dir = TempDir::new("ir_symlink_test");
    let real_install = dir.path.join("real_install");
    fs::create_dir_all(&real_install).unwrap();
    let releases_dir = real_install.join("releases");
    let release_1 = releases_dir.join("1.0.0");
    fs::create_dir_all(&release_1).unwrap();
    let current_link = real_install.join("current");
    std::os::unix::fs::symlink(&release_1, &current_link).unwrap();

    // 创建指向 real_install 的符号链接 install_symlink
    let install_symlink = dir.path.join("install_symlink");
    std::os::unix::fs::symlink(&real_install, &install_symlink).unwrap();

    // 当 install_root 自身是符号链接时，必须被严格拒绝
    let verify_res = journal::verify_current_link_target_with_root(
        &install_symlink.join("current"),
        &install_symlink.join("releases"),
        "1.0.0",
        Some(&install_symlink),
    );
    assert!(
        verify_res.is_err(),
        "install_root as symlink must be strictly rejected"
    );
    let err_str = verify_res.err().unwrap().to_string();
    assert!(
        err_str.contains("install_root_must_not_be_symlink") || err_str.contains("symlink"),
        "unexpected error: {err_str}"
    );
}
