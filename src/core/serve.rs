use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::Duration;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::json;

use crate::core::config::{BotConfig, BrainMode};
use crate::core::external_brain::{bind_core_socket, start_socket_server, ExternalBrain};
use crate::core::mcp_client::McpSocketClient;
use crate::core::scheduler::BotCoordinator;
use crate::core::state::{read_json, write_private};
use crate::core::worker_client::WorkerClient;

static STOP_SIGNAL: AtomicBool = AtomicBool::new(false);
static SHARED_STOP: AtomicPtr<AtomicBool> = AtomicPtr::new(std::ptr::null_mut());

extern "C" fn handle_signal(_: i32) {
    STOP_SIGNAL.store(true, Ordering::SeqCst);
    let ptr = SHARED_STOP.load(Ordering::SeqCst);
    if !ptr.is_null() {
        unsafe {
            (*ptr).store(true, Ordering::SeqCst);
        }
    }
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

pub fn core_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join("core.lock")
}

/// Conservatively treat an inaccessible lock as running, so callers never
/// perform destructive maintenance when the service state cannot be checked.
pub fn core_is_running(state_dir: &Path) -> bool {
    let path = core_lock_path(state_dir);
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    file.try_lock_exclusive().is_err()
}

/// The returned file must remain alive for the entire maintenance operation.
pub fn acquire_core_lock_for_maintenance(state_dir: &Path) -> Result<File> {
    if core_is_running(state_dir) {
        anyhow::bail!("先停止服务 (core.lock is held)");
    }
    acquire_core_lock(state_dir).context("先停止服务：无法获取 core.lock")
}

pub fn acquire_core_lock(state_dir: &Path) -> Result<File> {
    if !state_dir.exists() {
        fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;
        let _ = fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700));
    }
    let lock_path = core_lock_path(state_dir);
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

async fn update_health_brain(coordinator_root: &Path, brain: &ExternalBrain) {
    let health_path = coordinator_root.join("health.json");
    if let Ok(mut health_data) = read_json::<serde_json::Value>(&health_path) {
        if let Some(map) = health_data.as_object_mut() {
            let brain_summary = brain.status_summary().await;
            map.insert("brain".to_string(), brain_summary);
            let _ = write_private(&health_path, &health_data);
        }
    }
}

/// 数据归属用哪个账号
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSource {
    /// 配置文件里的账号名（测试用）
    Config,
    /// 当前登录的微信账号：聊天记录和联系人都挂在它下面，换号自动切换（各账号数据分别保留）
    WeChat,
}

enum CoreRunEnd {
    Exit(i32),
    AccountChanged,
}

const ACCOUNT_CHECK_SECONDS: f64 = 30.0;

fn stop_requested(stop_rx: &Option<tokio::sync::watch::Receiver<bool>>) -> bool {
    STOP_SIGNAL.load(Ordering::SeqCst) || stop_rx.as_ref().map_or(false, |rx| *rx.borrow())
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub async fn run_serve_core(
    config_path: &Path,
    stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<i32> {
    run_serve_core_with(config_path, stop_rx, AccountSource::Config).await
}

pub async fn run_serve_core_with(
    config_path: &Path,
    mut stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
) -> Result<i32> {
    ensure_locale();
    install_signals();

    // 1. 加载配置（失败直接向上抛出，由外层捕获并以退出码 2 退出）
    let config = BotConfig::load(config_path)
        .with_context(|| format!("configuration error in {}", config_path.display()))?;

    // 2. 获取文件锁（保证同一 state_dir 只有一个实例，拿不到退出 2 并打印原因）
    let _core_lock = acquire_core_lock(&config.state_dir)
        .with_context(|| format!("state lock error in {}", config.state_dir.display()))?;

    loop {
        let account = match source {
            AccountSource::Config => None,
            AccountSource::WeChat => {
                // 微信没运行或没登录：暂停等待，不往任何账号下写数据
                let mut waited = false;
                let acc = loop {
                    if stop_requested(&stop_rx) {
                        return Ok(0);
                    }
                    if let Some(acc) = crate::core::wechat_account::detect_logged_in_account() {
                        break acc;
                    }
                    if !waited {
                        waited = true;
                        let _ = write_private(
                            &config.state_dir.join("health.json"),
                            &json!({"pid": std::process::id(), "state": "paused", "reason": "wechat_not_logged_in", "updated_at": now_secs()}),
                        );
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                };
                // 第一次识别到账号：旧数据分不清属于哪个号，备份后清空，当前号从零开始（只做一次）
                if crate::core::wechat_account::start_fresh_once(&config.state_dir, &config.account_id, &acc, now_secs())
                    .context("account start-fresh failed")?
                {
                    eprintln!("wechat account bound: previous data backed up and cleared");
                }
                crate::core::wechat_account::record_current_account(&config.state_dir, &acc, now_secs())?;
                Some(acc)
            }
        };
        match run_core_for_account(config_path, &config, &mut stop_rx, source, account).await? {
            CoreRunEnd::Exit(code) => return Ok(code),
            CoreRunEnd::AccountChanged => continue,
        }
    }
}

async fn run_core_for_account(
    config_path: &Path,
    config: &BotConfig,
    stop_rx: &mut Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
    account: Option<String>,
) -> Result<CoreRunEnd> {
    // 3. 构造外部大脑并启动 core.sock
    let external_brain = ExternalBrain::new(
        &config.brain,
        &config.model_config,
        config.state_dir.clone(),
    );
    let sock_path = external_brain.socket_path.clone();
    let listener = bind_core_socket(&sock_path)?;
    let (server_stop_tx, server_stop_rx) = tokio::sync::watch::channel(false);
    let socket_server_handle = start_socket_server(
        external_brain.clone(),
        listener,
        Some(server_stop_rx),
    );

    let open_api_server = if config.open_api.enabled {
        Some(crate::core::open_api::http::OpenApiServer::start(
            &config.open_api, external_brain.clone(), std::sync::Arc::new(config.clone()),
        ).await?)
    } else { None };

    // 4. 运行调度器
    let mut coordinator = BotCoordinator::new_with_account(config_path, account.clone())?;
    if source == AccountSource::WeChat {
        coordinator.account_detector = Some(std::sync::Arc::new(crate::core::wechat_account::detect_logged_in_account));
    }
    if let Some(acc) = &account {
        coordinator.log_event("wechat_account", json!({"account_id": acc}));
    }
    let mut last_account_check = now_secs();
    let mut account_changed = false;
    coordinator.reconcile_journals()?;

    // 有意泄漏一个 Arc 引用：信号处理器随时可能触发，指针必须在进程余生内有效，
    // 即使 run_serve_core 因错误提前返回也不能悬空。
    let stop_ptr = std::sync::Arc::into_raw(coordinator.stop.clone()) as *mut AtomicBool;
    SHARED_STOP.store(stop_ptr, Ordering::SeqCst);

    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let base_dir = home.join(".wechat-rpa");
    let worker_sock = base_dir.join("worker.sock");
    let bot_worker_sock = base_dir.join("bot-worker.sock");

    // 网关里登记的是配置里的账号名（设备绑定），不是数据归属的微信账号；用 wxid 会"未注册账号"连不上（2026-11-08 实机）
    let mcp_client = std::sync::Arc::new(McpSocketClient::new(
        worker_sock,
        coordinator.configured_account_id.clone(),
    ));

    let max_bytes = (coordinator.config.max_attachment_mb * 1024 * 1024) as usize;
    let worker_client = WorkerClient::new(
        base_dir.clone(),
        bot_worker_sock,
        coordinator.ownership.clone(),
        String::new(),
        base_dir.clone(),
        None,
        coordinator.config.manage_window,
        false,
        max_bytes,
        Some(mcp_client.clone()),
    );

    coordinator.tool_client = Some(mcp_client);
    let mut worker_client = worker_client;
    worker_client.stop = Some(coordinator.stop.clone());
    coordinator.capture = Some(Box::new(worker_client));

    if config.brain.mode == BrainMode::External {
        let brain_fn = external_brain.brain_fn();
        coordinator.default_brain = Some(brain_fn.clone());
        let targets = coordinator.config.allowlist.clone();
        for target in &targets {
            if let Ok(child) = coordinator.session(target) {
                child.brain = Some(brain_fn.clone());
            }
        }
    }

    coordinator.log_event(
        "service_started",
        json!({
            "pid": std::process::id(),
            "mode": "multi-private",
            "model_backend": coordinator.config.model_config.backend,
        }),
    );
    coordinator.health("connecting", serde_json::Map::new())?;
    update_health_brain(&coordinator.root, &external_brain).await;

    // 5. 按配置启动历史 HTTP 服务（只监听配置的 bind，默认 127.0.0.1）。它只是辅助查询：启动失败（例如端口
    //    仍被旧的 Python 历史服务占用）只记录事件，不能让核心退出、停止回复客户。
    let history_dir = config.state_dir.join("history");
    let start_history = || -> Result<crate::core::history::serve::HttpServer> {
        let http_token = crate::core::history::serve::ensure_token(&history_dir)
            .with_context(|| format!("failed to ensure history http token in {}", history_dir.display()))?;
        let history_db_path = history_dir.join("history.sqlite3");
        let history_query = std::sync::Arc::new(std::sync::Mutex::new(
            crate::core::history::query::HistoryQuery::open(&history_db_path, Some(config.context_messages as i64))
                .with_context(|| format!("failed to open history database for query at {}", history_db_path.display()))?,
        ));
        crate::core::history::serve::HttpServer::start(
            history_query,
            http_token,
            &config.history_http.bind,
            config.history_http.port,
        )
        .with_context(|| format!("failed to start history http server on {}:{}", config.history_http.bind, config.history_http.port))
    };
    let history_server = match start_history() {
        Ok(server) => {
            coordinator.log_event("history_http_started", json!({"bind": config.history_http.bind, "port": server.port()}));
            Some(server)
        }
        Err(error) => {
            coordinator.log_event("history_http_failed", json!({"error": format!("{error:#}").chars().take(300).collect::<String>()}));
            None
        }
    };

    let backups_dir = coordinator.config.state_dir.join("backups");
    let mut last_backup_at = crate::core::backup::latest_backup_timestamp(&backups_dir);

    let poll_seconds = coordinator.config.poll_seconds.max(1);

    loop {
        if STOP_SIGNAL.load(Ordering::SeqCst) || coordinator.stop.load(Ordering::SeqCst) {
            coordinator.stop.store(true, Ordering::SeqCst);
            break;
        }

        // 定期看当前登录的是不是同一个微信号；换号了就按新账号重建（旧号数据原样保留）
        if source == AccountSource::WeChat && now_secs() - last_account_check >= ACCOUNT_CHECK_SECONDS {
            last_account_check = now_secs();
            if let Some(now_acc) = crate::core::wechat_account::detect_logged_in_account() {
                if Some(&now_acc) != account.as_ref() {
                    coordinator.log_event("wechat_account_changed", json!({"from": account, "to": now_acc}));
                    account_changed = true;
                    break;
                }
            }
        }

        if config.brain.mode == BrainMode::External {
            let brain_fn = external_brain.brain_fn();
            for child in coordinator.sessions.values_mut() {
                if child.brain.is_none() {
                    child.brain = Some(brain_fn.clone());
                }
            }
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

        update_health_brain(&coordinator.root, &external_brain).await;

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

        // 检查自动备份：空闲且距离上次备份满 24 小时
        let now = (coordinator.clock)();
        let is_idle = coordinator.pending.is_empty() && !coordinator.sessions.values().any(|s| s.turn.is_some());
        if crate::core::backup::should_auto_backup(is_idle, now, last_backup_at) {
            match crate::core::backup::run_backup(&coordinator.config, None) {
                Ok(out_path) => {
                    last_backup_at = now;
                    coordinator.log_event(
                        "backup_succeeded",
                        json!({
                            "path": out_path.display().to_string(),
                            "timestamp": now,
                        }),
                    );
                }
                Err(e) => {
                    coordinator.log_event(
                        "backup_failed",
                        json!({
                            "error": e.to_string(),
                            "timestamp": now,
                        }),
                    );
                }
            }
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
    update_health_brain(&coordinator.root, &external_brain).await;

    // 停止并清理 core.sock
    let _ = server_stop_tx.send(true);
    socket_server_handle.abort();
    if let Some(server) = open_api_server { server.shutdown(); }
    let _ = fs::remove_file(&sock_path);

    // 停止历史 HTTP 服务
    if let Some(server) = history_server {
        server.shutdown();
    }

    SHARED_STOP.store(std::ptr::null_mut(), Ordering::SeqCst);

    if account_changed {
        return Ok(CoreRunEnd::AccountChanged);
    }
    Ok(CoreRunEnd::Exit(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 实机 2026-11-08：连网关时用了当前微信账号（wxid），网关报"未注册账号"，机器人连不上。
    /// 网关客户端必须用配置里的账号名。
    #[test]
    fn test_gateway_client_uses_configured_account_not_wechat_account() {
        let src = include_str!("serve.rs");
        let start = src.find("McpSocketClient::new(").expect("mcp client");
        let call = &src[start..start + 200];
        assert!(call.contains("configured_account_id"), "{call}");
        assert!(!call.contains("config.account_id"), "{call}");
    }


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

    fn get_free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn write_test_config(dir: &Path) -> (PathBuf, PathBuf) {
        let state_dir = dir.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let port = get_free_port();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "test_user", "kind": "private"}],
            "state_dir": state_dir.to_string_lossy().to_string(),
            "poll_seconds": 1,
            "cooldown_seconds": 15,
            "max_replies_per_hour": 20,
            "history_http": {
                "bind": "127.0.0.1",
                "port": port,
            }
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
    fn maintenance_holds_core_lock() {
        let dir = TempDir::new();
        let state = dir.path().join("state");
        assert!(!core_is_running(&state));
        let maintenance = acquire_core_lock_for_maintenance(&state).unwrap();
        assert_eq!(core_lock_path(&state), state.join("core.lock"));
        assert!(core_is_running(&state));
        assert!(acquire_core_lock(&state).is_err());
        assert!(acquire_core_lock_for_maintenance(&state).unwrap_err().to_string().contains("先停止服务"));
        drop(maintenance);
        assert!(!core_is_running(&state));
        let core = acquire_core_lock(&state).unwrap();
        assert!(core_is_running(&state));
        assert!(acquire_core_lock_for_maintenance(&state).is_err());
        drop(core);
        assert!(acquire_core_lock_for_maintenance(&state).is_ok());
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
        let _lock = crate::core::env_lock();
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let pid = std::process::id();
        let count = uuid::Uuid::new_v4().simple();
        let sock_dir = PathBuf::from(format!("/tmp/sg_{pid}_{count}"));
        fs::create_dir_all(&sock_dir).unwrap();
        fs::set_permissions(&sock_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = sock_dir.join("core.sock");
        unsafe {
            std::env::set_var("WECHAT_CORE_SOCK", &sock_path);
        }

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
        assert!(health.get("brain").is_some());
        assert_eq!(health["brain"]["mode"], "builtin");

        assert!(!sock_path.exists());
        let _ = fs::remove_dir_all(&sock_dir);
        unsafe {
            std::env::remove_var("WECHAT_CORE_SOCK");
        }
    }

    #[tokio::test]
    async fn test_run_serve_core_external_brain_socket_and_health() {
        let _lock = crate::core::env_lock();
        let dir = TempDir::new();
        let state_dir = dir.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let pid = std::process::id();
        let count = uuid::Uuid::new_v4().simple();
        let sock_dir = PathBuf::from(format!("/tmp/se_{pid}_{count}"));
        fs::create_dir_all(&sock_dir).unwrap();
        fs::set_permissions(&sock_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = sock_dir.join("core.sock");

        unsafe {
            std::env::set_var("WECHAT_CORE_SOCK", &sock_path);
        }

        let port = get_free_port();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "test_user", "kind": "private"}],
            "state_dir": state_dir.to_string_lossy().to_string(),
            "poll_seconds": 1,
            "cooldown_seconds": 15,
            "max_replies_per_hour": 20,
            "history_http": {
                "bind": "127.0.0.1",
                "port": port,
            },
            "brain": {
                "mode": "external",
                "external": {
                    "timeout_seconds": 30,
                    "timeout_action": "skip"
                }
            }
        });
        let cfg_path = dir.path().join("config.json");
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let (tx, rx) = tokio::sync::watch::channel(false);
        let s_path = sock_path.clone();

        tokio::spawn(async move {
            // 等待 core.sock 出现并连接验证 status
            for _ in 0..50 {
                if s_path.exists() {
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            if let Ok(stream) = tokio::net::UnixStream::connect(&s_path).await {
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
                let (reader, mut writer) = stream.into_split();
                let req = json!({"id": "probe", "op": "status"}).to_string() + "\n";
                let _ = writer.write_all(req.as_bytes()).await;
                let _ = writer.flush().await;

                let mut lines = BufReader::new(reader).lines();
                if let Ok(Some(line)) = lines.next_line().await {
                    let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(resp["result"]["mode"], "external");
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });

        let code = run_serve_core(&cfg_path, Some(rx)).await.unwrap();
        assert_eq!(code, 0);

        // 验证退出后 socket 文件被删除
        assert!(!sock_path.exists(), "core.sock should be deleted on exit");

        // 验证 health.json 记录的 brain 状态
        let health_file = state_dir.join("health.json");
        let health: serde_json::Value = serde_json::from_str(&fs::read_to_string(&health_file).unwrap()).unwrap();
        assert_eq!(health["brain"]["mode"], "external");

        let _ = fs::remove_dir_all(&sock_dir);
        unsafe {
            std::env::remove_var("WECHAT_CORE_SOCK");
        }
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

    #[tokio::test]
    async fn test_serve_core_auto_backup_on_idle() {
        let _guard = crate::core::env_lock();
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 创建并写入初始历史库
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let db_path = history_dir.join("history.sqlite3");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE auto_test (id INT PRIMARY KEY, name TEXT);
                 INSERT INTO auto_test VALUES (1, 'hello');"
            ).unwrap();
        }

        let (tx, rx) = tokio::sync::watch::channel(false);

        tokio::spawn(async move {
            // 等待一轮 sweep 完成，并触发自动备份
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = tx.send(true);
        });

        let code = run_serve_core(&cfg_path, Some(rx)).await.unwrap();
        assert_eq!(code, 0);

        // 验证 backups 目录下生成了备份
        let backups_dir = state_dir.join("backups");
        assert!(backups_dir.exists(), "backups dir should be created");

        let mut found_backup = false;
        for entry in fs::read_dir(&backups_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                let backup_db = entry.path().join("history.sqlite3");
                if backup_db.exists() {
                    let conn = rusqlite::Connection::open(&backup_db).unwrap();
                    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
                    assert_eq!(check, "ok");
                    found_backup = true;
                    break;
                }
            }
        }
        assert!(found_backup, "Automatic backup of history.sqlite3 should exist in backups dir");
    }

    #[tokio::test]
    async fn test_history_http_launchd_plist() {
        let _lock = crate::core::env_lock();
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let config = BotConfig::load(&cfg_path).unwrap();
        assert_eq!(config.history_http.bind, "127.0.0.1");
        let port = config.history_http.port;

        let pid = std::process::id();
        let count = uuid::Uuid::new_v4().simple();
        let sock_dir = PathBuf::from(format!("/tmp/sh_{pid}_{count}"));
        fs::create_dir_all(&sock_dir).unwrap();
        fs::set_permissions(&sock_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = sock_dir.join("core.sock");
        unsafe {
            std::env::set_var("WECHAT_CORE_SOCK", &sock_path);
        }

        let (tx, rx) = tokio::sync::watch::channel(false);

        let http_client_task = tokio::spawn(async move {
            let mut connected = false;
            for _ in 0..50 {
                if let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                    connected = true;
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    // 1. 无 token 请求返回 401
                    let req = b"GET /v1/contacts HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
                    let _ = stream.write_all(req).await;
                    let mut buf = vec![0u8; 1024];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let resp = String::from_utf8_lossy(&buf[..n]);
                    assert!(resp.starts_with("HTTP/1.1 401"), "Expected 401, got: {resp}");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(connected, "History HTTP server did not start in time");

            // 2. 带 token 请求返回 200
            let token_file = state_dir.join("history").join("http-token");
            assert!(token_file.exists());
            let token = fs::read_to_string(&token_file).unwrap().trim().to_string();

            if let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let req = format!("GET /v1/contacts HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n");
                let _ = stream.write_all(req.as_bytes()).await;
                let mut buf = vec![0u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let resp = String::from_utf8_lossy(&buf[..n]);
                assert!(resp.starts_with("HTTP/1.1 200"), "Expected 200, got: {resp}");
            }

            let _ = tx.send(true);
        });

        let code = run_serve_core(&cfg_path, Some(rx)).await.unwrap();
        assert_eq!(code, 0);

        http_client_task.await.unwrap();

        // 验证停止后端口关闭
        let after = std::net::TcpStream::connect(("127.0.0.1", port));
        assert!(after.is_err(), "History HTTP server should be stopped");

        let _ = fs::remove_dir_all(&sock_dir);
        unsafe {
            std::env::remove_var("WECHAT_CORE_SOCK");
        }
    }

    #[tokio::test]
    async fn test_os_signal_immediately_sets_shared_stop() {
        let _guard = crate::core::env_lock();
        const SIGTERM: i32 = 15;

        install_signals();
        let coordinator_stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop_ptr = &*coordinator_stop as *const AtomicBool as *mut AtomicBool;
        SHARED_STOP.store(stop_ptr, Ordering::SeqCst);
        STOP_SIGNAL.store(false, Ordering::SeqCst);

        assert!(!coordinator_stop.load(Ordering::SeqCst));
        // 执行信号处理器
        handle_signal(SIGTERM);

        assert!(STOP_SIGNAL.load(Ordering::SeqCst), "STOP_SIGNAL 必须被置为 true");
        assert!(coordinator_stop.load(Ordering::SeqCst), "OS 信号到达时必须实时通过共享指针将 coordinator.stop 设为 true");

        SHARED_STOP.store(std::ptr::null_mut(), Ordering::SeqCst);
        STOP_SIGNAL.store(false, Ordering::SeqCst);
    }
}
