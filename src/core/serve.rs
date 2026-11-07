use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::json;

use crate::core::config::BotConfig;
use crate::core::scheduler::BotCoordinator;
use crate::core::state::write_private;

static STOP_SIGNAL: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_: i32) {
    STOP_SIGNAL.store(true, Ordering::SeqCst);
}

fn install_signals() {
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    unsafe {
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
    }
}

pub fn ensure_locale() {
    if std::env::var("LANG").is_err() {
        unsafe {
            std::env::set_var("LANG", "en_US.UTF-8");
            std::env::set_var("LC_ALL", "en_US.UTF-8");
        }
    }
}

pub fn acquire_core_lock(state_dir: &Path) -> Result<File> {
    if !state_dir.exists() {
        fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;
        let _ = fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700));
    }
    let lock_path = state_dir.join("core.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;
    let _ = fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600));

    file.try_lock_exclusive()
        .with_context(|| format!("failed to acquire core.lock at {}: another instance is running", lock_path.display()))?;

    Ok(file)
}

pub async fn run_serve_core(
    config_path: &Path,
    mut stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<i32> {
    ensure_locale();
    install_signals();

    // 1. 加载配置（失败直接向上抛出，由外层捕获并以退出码 2 退出）
    let config = BotConfig::load(config_path)
        .with_context(|| format!("configuration error in {}", config_path.display()))?;

    // 2. 获取文件锁（保证同一 state_dir 只有一个实例，拿不到退出 2 并打印原因）
    let _core_lock = acquire_core_lock(&config.state_dir)
        .with_context(|| format!("state lock error in {}", config.state_dir.display()))?;

    // 3. 运行调度器
    let mut coordinator = BotCoordinator::new(config_path)?;
    coordinator.reconcile_journals()?;
    coordinator.log_event(
        "service_started",
        json!({
            "pid": std::process::id(),
            "mode": "multi-private",
            "model_backend": coordinator.config.model_config.backend,
        }),
    );
    coordinator.health("connecting", serde_json::Map::new())?;

    let poll_seconds = coordinator.config.poll_seconds.max(1);

    loop {
        if STOP_SIGNAL.load(Ordering::SeqCst) || coordinator.stop.load(Ordering::SeqCst) {
            coordinator.stop.store(true, Ordering::SeqCst);
            break;
        }

        // 运行本轮调度
        if let Err(e) = coordinator.sweep().await {
            let _ = coordinator.close_connection().await;
            coordinator.log_event("routing_config_failed", json!({"error_type": "Error"}));
            let health_path = coordinator.root.join("health.json");
            let _ = write_private(
                &health_path,
                &json!({
                    "pid": std::process::id(),
                    "state": "paused",
                    "reason": e.to_string(),
                    "updated_at": (coordinator.clock)(),
                }),
            );
        }

        // 处理校准请求
        if let Err(e) = coordinator.serve_calibration().await {
            coordinator.log_event(
                "serve_calibration_failed",
                json!({
                    "error_type": "Error",
                    "error": e.to_string(),
                }),
            );
        }

        if STOP_SIGNAL.load(Ordering::SeqCst) || coordinator.stop.load(Ordering::SeqCst) {
            coordinator.stop.store(true, Ordering::SeqCst);
            break;
        }

        // 等待下一轮轮询，同时响应退出信号（细粒度 100ms 检查，最多 poll_seconds 秒）
        let check_steps = (poll_seconds * 10).max(1);
        for _ in 0..check_steps {
            if STOP_SIGNAL.load(Ordering::SeqCst) || coordinator.stop.load(Ordering::SeqCst) {
                break;
            }
            if let Some(rx) = stop_rx.as_mut() {
                if *rx.borrow() {
                    coordinator.stop.store(true, Ordering::SeqCst);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // 退出前完成连接关闭与健康状态落盘
    let _ = coordinator.close_connection().await;
    for child in coordinator.sessions.values_mut() {
        let _ = child.health("stopped", serde_json::Map::new());
    }
    let health_path = coordinator.root.join("health.json");
    let _ = write_private(
        &health_path,
        &json!({
            "pid": std::process::id(),
            "state": "stopped",
            "updated_at": (coordinator.clock)(),
        }),
    );

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("test-serve-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_test_config(dir: &Path) -> (PathBuf, PathBuf) {
        let state_dir = dir.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "test_user", "kind": "private"}],
            "state_dir": state_dir.to_string_lossy().to_string(),
            "poll_seconds": 1,
            "cooldown_seconds": 15,
            "max_replies_per_hour": 20,
        });
        let cfg_path = dir.join("config.json");
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        (cfg_path, state_dir)
    }

    #[test]
    fn test_ensure_locale_sets_vars() {
        ensure_locale();
        assert_eq!(std::env::var("LANG").unwrap(), "en_US.UTF-8");
        assert_eq!(std::env::var("LC_ALL").unwrap(), "en_US.UTF-8");
    }

    #[test]
    fn test_acquire_core_lock_mutual_exclusion() {
        let dir = TempDir::new();
        let state_dir = dir.path().join("state");

        // 第一次成功
        let lock1 = acquire_core_lock(&state_dir).unwrap();
        assert!(state_dir.join("core.lock").exists());

        // 第二次在同目录应失败
        let lock2_res = acquire_core_lock(&state_dir);
        assert!(lock2_res.is_err());
        let err_str = lock2_res.err().unwrap().to_string();
        assert!(err_str.contains("failed to acquire core.lock"));

        // 释放锁1后，再次获取应成功
        drop(lock1);
        let lock3 = acquire_core_lock(&state_dir);
        assert!(lock3.is_ok());
    }

    #[tokio::test]
    async fn test_run_serve_core_graceful_shutdown() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let (tx, rx) = tokio::sync::watch::channel(false);

        // 启动后台任务在 100ms 后触发停止
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });

        let code = run_serve_core(&cfg_path, Some(rx)).await.unwrap();
        assert_eq!(code, 0);

        // 检查退出后的 health.json
        let health_file = state_dir.join("health.json");
        assert!(health_file.exists());
        let content = fs::read_to_string(&health_file).unwrap();
        let health: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(health["state"], "stopped");
    }

    #[tokio::test]
    async fn test_run_serve_core_invalid_config_fails() {
        let dir = TempDir::new();
        let bad_cfg = dir.path().join("bad.json");
        fs::write(&bad_cfg, r#"{"schema_version": 99}"#).unwrap();

        let res = run_serve_core(&bad_cfg, None).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_run_serve_core_lock_collision_fails() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 预先占有 core.lock
        let _pre_lock = acquire_core_lock(&state_dir).unwrap();

        let res = run_serve_core(&cfg_path, None).await;
        assert!(res.is_err());
        let err_str = res.err().unwrap().to_string();
        assert!(err_str.contains("state lock error") || err_str.contains("failed to acquire core.lock"));
    }
}
