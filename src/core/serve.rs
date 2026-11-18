use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::Duration;

use crate::core::config::{BotConfig, BrainMode};
use crate::core::external_brain::{ExternalBrain, bind_core_socket, start_socket_server};
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

/// 纯函数决策：对齐原 std::env::var("LANG").is_err() 语义，仅缺失 (None) 补默认，所有 Some 保持不覆盖。
pub(crate) fn required_locale_defaults(lang: Option<&str>) -> Option<(&'static str, &'static str)> {
    if lang.is_none() {
        Some(("en_US.UTF-8", "en_US.UTF-8"))
    } else {
        None
    }
}

pub fn ensure_locale() {
    let current_lang = std::env::var("LANG").ok();
    if let Some((lang, lc_all)) = required_locale_defaults(current_lang.as_deref()) {
        unsafe {
            std::env::set_var("LANG", lang);
            std::env::set_var("LC_ALL", lc_all);
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
    // 同一进程刚释放 core.lock 后立刻再取（连续执行历史维护命令、测试并行跑）时，
    // flock 的释放存在极短的可见性窗口，直接 try_lock 会误报「无法获取 core.lock」。
    // 维护命令本来就是「等服务停下来再执行」，给一个有界重试；真有服务占着锁时
    // 仍然会在 deadline 后如实报错。
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    loop {
        match acquire_core_lock(state_dir) {
            Ok(file) => return Ok(file),
            Err(err) => {
                if std::time::Instant::now() >= deadline {
                    return Err(err).context("先停止服务：无法获取 core.lock");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
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

    file.try_lock_exclusive().with_context(|| {
        format!(
            "failed to acquire core.lock at {}: another instance is running",
            lock_path.display()
        )
    })?;

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

#[cfg(test)]
#[derive(Debug)]
struct OwnedSocketDir {
    dir: PathBuf,
    dev: u64,
    ino: u64,
}

#[cfg(test)]
impl Drop for OwnedSocketDir {
    fn drop(&mut self) {
        // 安全清理：
        // 1. 获取 symlink_metadata，绝不跟随符号链接 (follow symlink)
        // 2. 如果是符号链接，只删除该符号链接本身，绝不删除外部目标目录中的内容
        // 3. 如果是真实目录，严格比对 dev 与 ino，防止目录被 rename 并被外部非自有目录替换后误删
        if let Ok(meta) = fs::symlink_metadata(&self.dir) {
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                let _ = fs::remove_file(&self.dir);
            } else if file_type.is_dir() {
                use std::os::unix::fs::MetadataExt;
                if meta.dev() == self.dev && meta.ino() == self.ino {
                    let _ = fs::remove_dir_all(&self.dir);
                } else {
                    eprintln!(
                        "[WARN] OwnedSocketDir drop skipped: directory identity mismatch for {} (expected dev={}, ino={}, actual dev={}, ino={})",
                        self.dir.display(),
                        self.dev,
                        self.ino,
                        meta.dev(),
                        meta.ino()
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServePaths {
    pub home: PathBuf,
    pub core_sock: PathBuf,
    pub worker_sock: PathBuf,
    pub bot_worker_sock: PathBuf,
    pub gateway_db: Option<Option<PathBuf>>,
    /// 仅测试 isolated 环境独占持有的临时目录 guard。
    /// 生产环境 from_env() 始终为 None，永不清理任何目录。
    /// 使用 Arc 保证 Clone 共享所有权，只有所有 Clone 均 Drop 后才触发清理。
    #[cfg(test)]
    temp_dir_guard: Option<std::sync::Arc<OwnedSocketDir>>,
}

impl ServePaths {
    /// 纯函数解析器：根据传入的 HOME 与 WECHAT_CORE_SOCK 解析路径。
    /// 严格保持与 1e120b5 原生产行为完全一致：
    /// - 当 HOME 缺失时，fallback 到 "." (而不是 ControlPaths 的 "/")
    /// - 当 HOME 为空时，保持 ""
    /// - worker_sock 与 bot_worker_sock 挂在 home.join(".wechat-rpa") 下
    /// - core_sock 若未提供覆盖，则按原 external_brain::default_socket_path 规则回退
    pub(crate) fn from_values(home: Option<&str>, core_sock: Option<&str>) -> Self {
        let home_path = home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let base_dir = home_path.join(".wechat-rpa");
        let worker_sock = base_dir.join("worker.sock");
        let bot_worker_sock = base_dir.join("bot-worker.sock");

        let resolved_core_sock = match core_sock {
            Some(sock) if !sock.trim().is_empty() => PathBuf::from(sock),
            _ => {
                let default_home = home.unwrap_or("/");
                PathBuf::from(default_home)
                    .join(".wechat-rpa")
                    .join("core.sock")
            }
        };

        Self {
            home: home_path,
            core_sock: resolved_core_sock,
            worker_sock,
            bot_worker_sock,
            gateway_db: None,
            #[cfg(test)]
            temp_dir_guard: None,
        }
    }

    pub fn from_env() -> Self {
        let home = std::env::var("HOME").ok();
        let core_sock = std::env::var("WECHAT_CORE_SOCK").ok();
        Self::from_values(home.as_deref(), core_sock.as_deref())
    }

    /// 尝试创建隔离的 ServePaths 环境，若目录创建或权限设置失败则返回错误，绝不静默吞错。
    #[cfg(test)]
    pub(crate) fn try_isolated(dir: &Path) -> Result<Self> {
        let base_dir = dir.join(".wechat-rpa");
        fs::create_dir_all(&base_dir)
            .with_context(|| format!("failed to create base dir {}", base_dir.display()))?;
        fs::set_permissions(&base_dir, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to set permissions for base dir {}",
                base_dir.display()
            )
        })?;

        // Unix domain socket 路径受 sockaddr_un.sun_path 限制（macOS 上为 104 字节）。
        // 测试临时目录路径通常较深，因此 socket 统一挂在短前缀 /tmp/wc_{uuid} 下。
        let short_id = uuid::Uuid::new_v4().simple();
        let sock_dir = PathBuf::from(format!("/tmp/wc_{short_id}"));
        fs::create_dir(&sock_dir).with_context(|| {
            format!("failed to create isolated sock dir {}", sock_dir.display())
        })?;
        fs::set_permissions(&sock_dir, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to set permissions for sock dir {}",
                sock_dir.display()
            )
        })?;

        let meta = fs::symlink_metadata(&sock_dir)
            .with_context(|| format!("failed to stat isolated sock dir {}", sock_dir.display()))?;
        use std::os::unix::fs::MetadataExt;
        let dev = meta.dev();
        let ino = meta.ino();

        Ok(Self {
            home: dir.to_path_buf(),
            core_sock: sock_dir.join("c.sock"),
            worker_sock: sock_dir.join("w.sock"),
            bot_worker_sock: sock_dir.join("bw.sock"),
            gateway_db: Some(Some(dir.join("gateway.sqlite3"))),
            temp_dir_guard: Some(std::sync::Arc::new(OwnedSocketDir {
                dir: sock_dir,
                dev,
                ino,
            })),
        })
    }

    #[cfg(test)]
    pub(crate) fn isolated(dir: &Path) -> Self {
        Self::try_isolated(dir).expect("failed to create isolated ServePaths")
    }

    #[cfg(test)]
    pub(crate) fn socket_dir(&self) -> Option<PathBuf> {
        self.temp_dir_guard.as_ref().map(|g| g.dir.clone())
    }
}

pub type AccountDetector = std::sync::Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// 启动守卫路径定义（严格区分状态根与安装根）
#[derive(Debug, Clone)]
pub(crate) struct StartupGuardPaths {
    pub state_dir: PathBuf,
    pub install_root: Option<PathBuf>,
    pub releases_dir: Option<PathBuf>,
    pub current_link: Option<PathBuf>,
    pub legacy_dir: Option<PathBuf>,
}

impl StartupGuardPaths {
    pub(crate) fn from_config(config: &BotConfig) -> Self {
        Self::from_config_with_legacy(config, None)
    }

    pub(crate) fn from_config_with_legacy(
        config: &BotConfig,
        legacy_override: Option<&Path>,
    ) -> Self {
        let (releases_dir, current_link) = match &config.install_root {
            Some(root) => (Some(root.join("releases")), Some(root.join("current"))),
            None => (None, None),
        };
        let legacy_dir = legacy_override
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".wechat-rpa")));
        Self {
            state_dir: config.state_dir.clone(),
            install_root: config.install_root.clone(),
            releases_dir,
            current_link,
            legacy_dir,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_state_dir(state_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            install_root: None,
            releases_dir: None,
            current_link: None,
            legacy_dir: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_install_root(
        state_dir: &Path,
        releases_dir: &Path,
        current_link: &Path,
    ) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            install_root: releases_dir.parent().map(Path::to_path_buf),
            releases_dir: Some(releases_dir.to_path_buf()),
            current_link: Some(current_link.to_path_buf()),
            legacy_dir: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_legacy_dir(state_dir: &Path, legacy_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            install_root: None,
            releases_dir: None,
            current_link: None,
            legacy_dir: Some(legacy_dir.to_path_buf()),
        }
    }
}

pub(crate) fn verify_serve_core_startup_guard_in_lock(
    paths: &StartupGuardPaths,
    core_lock: File,
) -> Result<File> {
    let decision = crate::updater::journal::check_startup_guard_readonly_with_root(
        &paths.state_dir,
        paths.releases_dir.as_deref(),
        paths.current_link.as_deref(),
        paths.install_root.as_deref(),
        paths.legacy_dir.as_deref(),
    );

    match decision {
        crate::updater::journal::StartupDualCheckDecision::AllowStartup => Ok(core_lock),
        crate::updater::journal::StartupDualCheckDecision::MaintenanceSuppressed {
            tx_id,
            stage,
        } => {
            drop(core_lock);
            eprintln!(
                "serve-core startup suppressed by maintenance: tx_id={}, stage={:?}",
                tx_id, stage
            );
            bail!(
                "startup_suppressed_by_maintenance: tx_id={}, stage={:?}",
                tx_id,
                stage
            );
        }
        crate::updater::journal::StartupDualCheckDecision::RecoveryRequired { reason } => {
            drop(core_lock);
            eprintln!("serve-core startup blocked: recovery required: {}", reason);
            bail!("startup_blocked_recovery_required: {}", reason);
        }
    }
}

pub async fn run_serve_core(
    config_path: &Path,
    stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<i32> {
    run_serve_core_with(config_path, stop_rx, AccountSource::Config).await
}

pub async fn run_serve_core_with(
    config_path: &Path,
    stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
) -> Result<i32> {
    let paths = ServePaths::from_env();
    run_serve_core_with_paths(config_path, stop_rx, source, &paths).await
}

pub async fn run_serve_core_with_paths(
    config_path: &Path,
    stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
    paths: &ServePaths,
) -> Result<i32> {
    run_serve_core_internal(config_path, stop_rx, source, paths, None, None).await
}

#[cfg(test)]
pub(crate) async fn run_serve_core_for_test(
    config_path: &Path,
    stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
    paths: &ServePaths,
    guard_paths: Option<StartupGuardPaths>,
    custom_detector: Option<AccountDetector>,
) -> Result<i32> {
    run_serve_core_internal(
        config_path,
        stop_rx,
        source,
        paths,
        guard_paths,
        custom_detector,
    )
    .await
}

async fn run_serve_core_internal(
    config_path: &Path,
    mut stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
    source: AccountSource,
    paths: &ServePaths,
    guard_paths: Option<StartupGuardPaths>,
    custom_detector: Option<AccountDetector>,
) -> Result<i32> {
    ensure_locale();
    install_signals();

    // 1. 加载配置（失败直接向上抛出，由外层捕获并以退出码 2 退出）
    let config = BotConfig::load(config_path)
        .with_context(|| format!("configuration error in {}", config_path.display()))?;

    // 2. 获取文件锁（保证同一 state_dir 只有一个实例，拿不到退出 2 并打印原因）
    let core_lock = acquire_core_lock(&config.state_dir)
        .with_context(|| format!("state lock error in {}", config.state_dir.display()))?;

    // 3. 锁内核验启动双检查 Guard（在任何微信探测、Worker 或业务 DB 副作用之前）
    let gp = guard_paths.unwrap_or_else(|| StartupGuardPaths::from_config(&config));
    let _core_lock = verify_serve_core_startup_guard_in_lock(&gp, core_lock)?;

    let detector: AccountDetector = custom_detector.unwrap_or_else(|| {
        std::sync::Arc::new(crate::core::wechat_account::detect_logged_in_account)
    });

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
                    if let Some(acc) = detector() {
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
                if crate::core::wechat_account::start_fresh_once(
                    &config.state_dir,
                    &config.account_id,
                    &acc,
                    now_secs(),
                )
                .context("account start-fresh failed")?
                {
                    eprintln!("wechat account bound: previous data backed up and cleared");
                }
                crate::core::wechat_account::record_current_account(
                    &config.state_dir,
                    &acc,
                    now_secs(),
                )?;
                Some(acc)
            }
        };
        match run_core_for_account(
            config_path,
            &config,
            &mut stop_rx,
            source,
            account,
            paths,
            Some(detector.clone()),
        )
        .await?
        {
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
    paths: &ServePaths,
    custom_detector: Option<AccountDetector>,
) -> Result<CoreRunEnd> {
    // 3. 构造外部大脑并启动 core.sock
    let external_brain = ExternalBrain::new_with_socket(
        &config.brain,
        &config.model_config,
        config.state_dir.clone(),
        paths.core_sock.clone(),
    );
    let sock_path = external_brain.socket_path.clone();
    let listener = bind_core_socket(&sock_path)?;
    let (server_stop_tx, server_stop_rx) = tokio::sync::watch::channel(false);
    let socket_server_handle =
        start_socket_server(external_brain.clone(), listener, Some(server_stop_rx));

    let open_api_server = if config.open_api.enabled {
        Some(
            crate::core::open_api::http::OpenApiServer::start(
                &config.open_api,
                external_brain.clone(),
                std::sync::Arc::new(config.clone()),
            )
            .await?,
        )
    } else {
        None
    };

    // 4. 运行调度器
    let mut coordinator = BotCoordinator::new_with_account(config_path, account.clone())?;
    if source == AccountSource::WeChat {
        coordinator.account_detector = custom_detector.or_else(|| {
            Some(std::sync::Arc::new(
                crate::core::wechat_account::detect_logged_in_account,
            ))
        });
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

    let base_dir = paths.home.join(".wechat-rpa");
    let worker_sock = paths.worker_sock.clone();
    let bot_worker_sock = paths.bot_worker_sock.clone();

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
        let http_token =
            crate::core::history::serve::ensure_token(&history_dir).with_context(|| {
                format!(
                    "failed to ensure history http token in {}",
                    history_dir.display()
                )
            })?;
        let history_db_path = history_dir.join("history.sqlite3");
        let history_query = std::sync::Arc::new(std::sync::Mutex::new(
            crate::core::history::query::HistoryQuery::open(
                &history_db_path,
                Some(config.context_messages as i64),
            )
            .with_context(|| {
                format!(
                    "failed to open history database for query at {}",
                    history_db_path.display()
                )
            })?,
        ));
        crate::core::history::serve::HttpServer::start(
            history_query,
            http_token,
            &config.history_http.bind,
            config.history_http.port,
        )
        .with_context(|| {
            format!(
                "failed to start history http server on {}:{}",
                config.history_http.bind, config.history_http.port
            )
        })
    };
    let history_server = match start_history() {
        Ok(server) => {
            coordinator.log_event(
                "history_http_started",
                json!({"bind": config.history_http.bind, "port": server.port()}),
            );
            Some(server)
        }
        Err(error) => {
            coordinator.log_event(
                "history_http_failed",
                json!({"error": format!("{error:#}").chars().take(300).collect::<String>()}),
            );
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
        if source == AccountSource::WeChat
            && now_secs() - last_account_check >= ACCOUNT_CHECK_SECONDS
        {
            last_account_check = now_secs();
            if let Some(now_acc) = crate::core::wechat_account::detect_logged_in_account() {
                if Some(&now_acc) != account.as_ref() {
                    coordinator.log_event(
                        "wechat_account_changed",
                        json!({"from": account, "to": now_acc}),
                    );
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
        let is_idle = coordinator.pending.is_empty()
            && !coordinator.sessions.values().any(|s| s.turn.is_some());
        if crate::core::backup::should_auto_backup(is_idle, now, last_backup_at) {
            match crate::core::backup::run_backup_with_gateway_db(
                &coordinator.config,
                None,
                paths.gateway_db.clone(),
            ) {
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
    if let Some(server) = open_api_server {
        server.shutdown();
    }
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
        // 纯函数隔离验证：仅缺失 (None) 触发回退，所有 Some（含空串、空白与有效值）严格保留原语义不覆盖
        assert_eq!(
            required_locale_defaults(None),
            Some(("en_US.UTF-8", "en_US.UTF-8"))
        );
        assert_eq!(required_locale_defaults(Some("")), None);
        assert_eq!(required_locale_defaults(Some("   ")), None);
        assert_eq!(required_locale_defaults(Some("C.UTF-8")), None);
        assert_eq!(required_locale_defaults(Some("zh_CN.UTF-8")), None);
        assert_eq!(required_locale_defaults(Some("en_US.UTF-8")), None);
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
        assert!(
            acquire_core_lock_for_maintenance(&state)
                .unwrap_err()
                .to_string()
                .contains("先停止服务")
        );
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

    struct StopOnDrop(Option<tokio::sync::watch::Sender<bool>>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(true);
            }
        }
    }

    #[tokio::test]
    async fn test_run_serve_core_graceful_shutdown() {
        let dir = TempDir::new();
        let paths = ServePaths::isolated(dir.path());
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let (tx, rx) = tokio::sync::watch::channel(false);
        let tx_clone = tx.clone();

        // 启动后台任务在 100ms 后触发停止
        let client_task = tokio::spawn(async move {
            let _guard = StopOnDrop(Some(tx_clone));
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        });

        let code = tokio::time::timeout(
            Duration::from_secs(10),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &paths),
        )
        .await
        .expect("serve-core should exit within bounded timeout")
        .unwrap();
        assert_eq!(code, 0);

        client_task.await.unwrap();

        // 检查退出后的 health.json
        let health_file = state_dir.join("health.json");
        assert!(health_file.exists());
        let content = fs::read_to_string(&health_file).unwrap();
        let health: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(health["state"], "stopped");
        assert!(health.get("brain").is_some());
        assert_eq!(health["brain"]["mode"], "builtin");

        assert!(!paths.core_sock.exists());
    }

    #[tokio::test]
    async fn test_run_serve_core_external_brain_socket_and_health() {
        let dir = TempDir::new();
        let paths = ServePaths::isolated(dir.path());
        let state_dir = dir.path().join("state");
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
        let s_path = paths.core_sock.clone();
        let tx_clone = tx.clone();

        let client_task = tokio::spawn(async move {
            let _guard = StopOnDrop(Some(tx_clone));
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
        });

        let code = tokio::time::timeout(
            Duration::from_secs(10),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &paths),
        )
        .await
        .expect("serve-core should exit within bounded timeout")
        .unwrap();
        assert_eq!(code, 0);

        client_task.await.unwrap();

        // 验证退出后 socket 文件被删除
        assert!(
            !paths.core_sock.exists(),
            "core.sock should be deleted on exit"
        );

        // 验证 health.json 记录的 brain 状态
        let health_file = state_dir.join("health.json");
        let health: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&health_file).unwrap()).unwrap();
        assert_eq!(health["brain"]["mode"], "external");
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
        assert!(
            err_str.contains("state lock error") || err_str.contains("failed to acquire core.lock")
        );
    }

    #[tokio::test]
    async fn test_run_serve_core_blocked_by_startup_guard_maintenance() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        // 写入 36 字节合法 UUID 维护标志
        let maint_file = state_dir.join("core.maintenance");
        fs::write(&maint_file, b"12345678-1234-4234-8234-123456789abc").unwrap();
        fs::set_permissions(&maint_file, fs::Permissions::from_mode(0o600)).unwrap();

        // 真实调用 run_serve_core_with_paths 入口（必须在进入业务前被启动守卫阻断返回 Err，且非 0）
        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &serve_paths),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_err(),
            "run_serve_core must fail when maintenance is active"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("startup_suppressed_by_maintenance"));

        // 验证零业务副作用：未初始化 core.sock、未生成 health.json、未写事件
        assert!(
            !serve_paths.core_sock.exists(),
            "core.sock must NOT be bound when maintenance blocks startup"
        );
        assert!(
            !state_dir.join("health.json").exists(),
            "health.json must NOT exist when maintenance blocks startup"
        );
        assert!(
            !state_dir.join("events.jsonl").exists(),
            "events.jsonl must NOT exist"
        );

        // 验证 core.lock 必须被干净释放，其他进程可以立即获取
        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after guard suppression"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_run_serve_core_blocked_by_maintenance_proves_zero_wechat_probe() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let maint_file = state_dir.join("core.maintenance");
        fs::write(&maint_file, b"12345678-1234-4234-8234-123456789abc").unwrap();
        fs::set_permissions(&maint_file, fs::Permissions::from_mode(0o600)).unwrap();

        // 注入带原子计数器的测试探测器，证明 AccountSource::WeChat 阻断时零微信探测调用（不触真机）
        let probe_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probe_counter = probe_count.clone();
        let detector: AccountDetector = std::sync::Arc::new(move || {
            probe_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            None
        });

        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_for_test(
                &cfg_path,
                Some(rx),
                AccountSource::WeChat,
                &serve_paths,
                None,
                Some(detector),
            ),
        )
        .await
        .expect("test timed out");

        assert!(res.is_err());
        assert_eq!(
            probe_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "WeChat account probe must NOT be executed when startup guard blocks"
        );
        assert!(!serve_paths.core_sock.exists());
        assert!(!state_dir.join("health.json").exists());

        let lock_check = acquire_core_lock(&state_dir);
        assert!(lock_check.is_ok(), "core.lock must be released");
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_run_serve_core_blocked_by_recovery_required_on_corrupt_journal() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let releases_dir = dir.path().join("releases");
        let release_1 = releases_dir.join("1.0.0");
        fs::create_dir_all(&release_1).unwrap();
        let current_link = dir.path().join("current");
        std::os::unix::fs::symlink(&release_1, &current_link).unwrap();
        let guard_paths =
            StartupGuardPaths::with_install_root(&state_dir, &releases_dir, &current_link);

        // 写入损坏格式的 update-journal.json
        let journal_file = state_dir.join("update-journal.json");
        fs::write(&journal_file, b"{ invalid json content").unwrap();
        fs::set_permissions(&journal_file, fs::Permissions::from_mode(0o600)).unwrap();

        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_for_test(
                &cfg_path,
                Some(rx),
                AccountSource::Config,
                &serve_paths,
                Some(guard_paths),
                None,
            ),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_err(),
            "run_serve_core must fail when journal is corrupt"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("startup_blocked_recovery_required"));

        // 零业务副作用验证
        assert!(
            !serve_paths.core_sock.exists(),
            "core.sock must NOT be bound on corrupt journal"
        );
        assert!(
            !state_dir.join("health.json").exists(),
            "health.json must NOT exist on corrupt journal"
        );

        // 验证 core.lock 必须被干净释放
        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after recovery required"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_run_serve_core_blocked_by_rolled_back_stage() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let releases_dir = dir.path().join("releases");
        let release_1 = releases_dir.join("1.0.0");
        fs::create_dir_all(&release_1).unwrap();
        let current_link = dir.path().join("current");
        std::os::unix::fs::symlink(&release_1, &current_link).unwrap();
        let guard_paths =
            StartupGuardPaths::with_install_root(&state_dir, &releases_dir, &current_link);

        let mut journal = crate::updater::journal::UpdateJournal::new(
            "00000000-0000-4000-8000-000000000001",
            "1.0.0",
            "1.0.1",
            2,
            1000,
        );
        journal.stage = crate::updater::journal::TransactionStage::RolledBack;
        journal.persist_atomic(&state_dir).unwrap();

        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_for_test(
                &cfg_path,
                Some(rx),
                AccountSource::Config,
                &serve_paths,
                Some(guard_paths),
                None,
            ),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_err(),
            "run_serve_core must fail when stage is RolledBack"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("startup_blocked_recovery_required"));

        // 零业务副作用验证
        assert!(!serve_paths.core_sock.exists());
        assert!(!state_dir.join("health.json").exists());

        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after guard suppression"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_run_serve_core_committed_with_unknown_install_root_fails_closed() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let mut journal = crate::updater::journal::UpdateJournal::new(
            "00000000-0000-4000-8000-000000000002",
            "1.0.0",
            "1.0.1",
            2,
            1000,
        );
        journal.stage = crate::updater::journal::TransactionStage::Committed;
        journal.manifest_digest =
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());
        journal.persist_atomic(&state_dir).unwrap();

        // 默认生产路径（未知安装根，releases_dir: None）直接调用
        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &serve_paths),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_err(),
            "committed release with unknown install root must fail closed"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(
            err_msg.contains("unconfigured_production_install_root")
                || err_msg.contains("startup_blocked_recovery_required"),
            "unexpected error message: {}",
            err_msg
        );

        // 零业务副作用验证
        assert!(!serve_paths.core_sock.exists());
        assert!(!state_dir.join("health.json").exists());

        let lock_check = acquire_core_lock(&state_dir);
        assert!(lock_check.is_ok(), "core.lock must be released");
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_run_serve_core_empty_state_allows_startup() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let (tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });

        // 纯净空状态（无 maintenance，无 journal），正常放行启动并优雅停机
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &serve_paths),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_ok(),
            "empty state must allow startup: {:?}",
            res.err()
        );
        assert_eq!(res.unwrap(), 0);

        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after normal shutdown"
        );
    }

    #[tokio::test]
    async fn test_run_serve_core_allows_startup_with_committed_release() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        let releases_dir = dir.path().join("releases");
        let release_1 = releases_dir.join("1.0.1");
        fs::create_dir_all(&release_1).unwrap();
        let current_link = dir.path().join("current");
        std::os::unix::fs::symlink(&release_1, &current_link).unwrap();

        let tx_id = "00000000-0000-4000-8000-000000000003";
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut journal =
            crate::updater::journal::UpdateJournal::new(tx_id, "1.0.0", "1.0.1", 2, 1000);
        journal.stage = crate::updater::journal::TransactionStage::Committed;
        journal.manifest_digest = Some(digest.to_string());
        journal.persist_atomic(&state_dir).unwrap();

        let record = crate::updater::journal::CurrentReleaseCommitRecord {
            record_schema: 1,
            highest_accepted_sequence: None,
            tx_id: tx_id.to_string(),
            release_id: "1.0.1".to_string(),
            sequence: 2,
            channel: "stable".to_string(),
            manifest_digest: digest.to_string(),
            installed_at: 1000,
        };
        crate::updater::journal::atomic_commit_release_record(&state_dir, &record).unwrap();

        let guard_paths =
            StartupGuardPaths::with_install_root(&state_dir, &releases_dir, &current_link);

        let (tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });

        // 注入合法全要素安装根，启动双检查通过并优雅停机
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_for_test(
                &cfg_path,
                Some(rx),
                AccountSource::Config,
                &serve_paths,
                Some(guard_paths),
                None,
            ),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_ok(),
            "committed release with verified install root must succeed: {:?}",
            res.err()
        );
        assert_eq!(res.unwrap(), 0);

        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after normal shutdown"
        );
    }

    #[tokio::test]
    async fn test_run_serve_core_with_configured_install_root_in_config_allows_startup() {
        let dir = TempDir::new();
        let install_root = dir.path().join("install");
        let releases_dir = install_root.join("releases");
        let release_1 = releases_dir.join("1.0.1");
        fs::create_dir_all(&release_1).unwrap();
        let current_link = install_root.join("current");
        std::os::unix::fs::symlink(&release_1, &current_link).unwrap();

        let state_dir = dir.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let port = get_free_port();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "test_user", "kind": "private"}],
            "state_dir": state_dir.to_string_lossy().to_string(),
            "install_root": install_root.to_string_lossy().to_string(),
            "poll_seconds": 1,
            "cooldown_seconds": 15,
            "max_replies_per_hour": 20,
            "history_http": {
                "bind": "127.0.0.1",
                "port": port,
            }
        });
        let cfg_path = dir.path().join("config.json");
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let serve_paths = ServePaths::isolated(dir.path());

        let tx_id = "00000000-0000-4000-8000-000000000004";
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut journal =
            crate::updater::journal::UpdateJournal::new(tx_id, "1.0.0", "1.0.1", 2, 1000);
        journal.stage = crate::updater::journal::TransactionStage::Committed;
        journal.manifest_digest = Some(digest.to_string());
        journal.persist_atomic(&state_dir).unwrap();

        let record = crate::updater::journal::CurrentReleaseCommitRecord {
            record_schema: 1,
            highest_accepted_sequence: None,
            tx_id: tx_id.to_string(),
            release_id: "1.0.1".to_string(),
            sequence: 2,
            channel: "stable".to_string(),
            manifest_digest: digest.to_string(),
            installed_at: 1000,
        };
        crate::updater::journal::atomic_commit_release_record(&state_dir, &record).unwrap();

        let (tx, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });

        // 调用生产 run_serve_core_with_paths（不传 guard_paths，自动从 config.install_root 派生）
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &serve_paths),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_ok(),
            "committed release with configured install_root in config must succeed: {:?}",
            res.err()
        );
        assert_eq!(res.unwrap(), 0);

        let lock_check = acquire_core_lock(&state_dir);
        assert!(
            lock_check.is_ok(),
            "core.lock must be released after normal shutdown"
        );
    }

    #[tokio::test]
    async fn test_run_serve_core_blocked_when_legacy_dir_has_unsettled_maintenance() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());
        let serve_paths = ServePaths::isolated(dir.path());

        // 构造独立的 legacy 目录，写入未完成维护标记
        let legacy_dir = dir.path().join("legacy_rpa");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_maint = legacy_dir.join("core.maintenance");
        fs::write(&legacy_maint, b"12345678-1234-4234-8234-123456789abc").unwrap();
        fs::set_permissions(&legacy_maint, fs::Permissions::from_mode(0o600)).unwrap();

        let guard_paths = StartupGuardPaths::with_legacy_dir(&state_dir, &legacy_dir);

        let (tx, rx) = tokio::sync::watch::channel(false);
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            run_serve_core_for_test(
                &cfg_path,
                Some(rx),
                AccountSource::Config,
                &serve_paths,
                Some(guard_paths),
                None,
            ),
        )
        .await
        .expect("test timed out");

        assert!(
            res.is_err(),
            "serve-core must be blocked when legacy dir has unsettled maintenance"
        );
        let err_msg = res.err().unwrap().to_string();
        assert!(
            err_msg.contains("legacy_maintenance_marker_detected")
                || err_msg.contains("startup_blocked_recovery_required"),
            "unexpected error message: {}",
            err_msg
        );

        // 零业务副作用验证
        assert!(!serve_paths.core_sock.exists());
        assert!(!state_dir.join("health.json").exists());
        // 关键契约：严格只读，旧文件必须完好无损存在
        assert!(
            legacy_maint.exists(),
            "legacy maintenance marker must NOT be deleted"
        );

        let lock_check = acquire_core_lock(&state_dir);
        assert!(lock_check.is_ok(), "core.lock must be released");
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn test_serve_core_auto_backup_on_idle() {
        let dir = TempDir::new();
        let paths = ServePaths::isolated(dir.path());
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 创建并写入初始历史库
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let db_path = history_dir.join("history.sqlite3");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE auto_test (id INT PRIMARY KEY, name TEXT);
                 INSERT INTO auto_test VALUES (1, 'hello');",
            )
            .unwrap();
        }

        let (tx, rx) = tokio::sync::watch::channel(false);
        let tx_clone = tx.clone();

        let client_task = tokio::spawn(async move {
            let _guard = StopOnDrop(Some(tx_clone));
            // 等待一轮 sweep 完成，并触发自动备份
            tokio::time::sleep(Duration::from_millis(500)).await;
        });

        let code = tokio::time::timeout(
            Duration::from_secs(10),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &paths),
        )
        .await
        .expect("serve-core should exit within bounded timeout")
        .unwrap();
        assert_eq!(code, 0);

        client_task.await.unwrap();

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
                    let check: String = conn
                        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                        .unwrap();
                    assert_eq!(check, "ok");
                    found_backup = true;
                    break;
                }
            }
        }
        assert!(
            found_backup,
            "Automatic backup of history.sqlite3 should exist in backups dir"
        );
    }

    #[tokio::test]
    async fn test_history_http_launchd_plist() {
        let dir = TempDir::new();
        let paths = ServePaths::isolated(dir.path());
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let config = BotConfig::load(&cfg_path).unwrap();
        assert_eq!(config.history_http.bind, "127.0.0.1");
        let port = config.history_http.port;

        let (tx, rx) = tokio::sync::watch::channel(false);
        let tx_clone = tx.clone();

        let http_client_task = tokio::spawn(async move {
            let _guard = StopOnDrop(Some(tx_clone));
            let mut connected = false;
            let mut perm_denied = false;
            for _ in 0..50 {
                match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                    Ok(mut stream) => {
                        connected = true;
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        // 1. 无 token 请求返回 401
                        let req = b"GET /v1/contacts HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
                        let _ = stream.write_all(req).await;
                        let mut buf = vec![0u8; 1024];
                        let n = stream.read(&mut buf).await.unwrap_or(0);
                        let resp = String::from_utf8_lossy(&buf[..n]);
                        assert!(
                            resp.starts_with("HTTP/1.1 401"),
                            "Expected 401, got: {resp}"
                        );
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        perm_denied = true;
                        break;
                    }
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }

            if perm_denied {
                // 精准记录受限原因：macOS 沙箱策略拦截本地网络回环 connect
                panic!("sandbox network restriction: TcpStream::connect returned PermissionDenied");
            }
            assert!(connected, "History HTTP server did not start in time");

            // 2. 带 token 请求返回 200
            let token_file = state_dir.join("history").join("http-token");
            assert!(token_file.exists());
            let token = fs::read_to_string(&token_file).unwrap().trim().to_string();

            if let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let req = format!(
                    "GET /v1/contacts HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n"
                );
                let _ = stream.write_all(req.as_bytes()).await;
                let mut buf = vec![0u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let resp = String::from_utf8_lossy(&buf[..n]);
                assert!(
                    resp.starts_with("HTTP/1.1 200"),
                    "Expected 200, got: {resp}"
                );
            }
        });

        let serve_res = tokio::time::timeout(
            Duration::from_secs(10),
            run_serve_core_with_paths(&cfg_path, Some(rx), AccountSource::Config, &paths),
        )
        .await;

        let code = serve_res
            .expect("serve-core should exit within bounded timeout when client task settles")
            .unwrap();
        assert_eq!(code, 0);

        http_client_task.await.unwrap();

        // 验证停止后端口关闭
        let after = std::net::TcpStream::connect(("127.0.0.1", port));
        assert!(after.is_err(), "History HTTP server should be stopped");
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

        assert!(
            STOP_SIGNAL.load(Ordering::SeqCst),
            "STOP_SIGNAL 必须被置为 true"
        );
        assert!(
            coordinator_stop.load(Ordering::SeqCst),
            "OS 信号到达时必须实时通过共享指针将 coordinator.stop 设为 true"
        );

        SHARED_STOP.store(std::ptr::null_mut(), Ordering::SeqCst);
        STOP_SIGNAL.store(false, Ordering::SeqCst);
    }

    #[test]
    fn test_serve_paths_from_env_never_deletes_directory() {
        let temp = TempDir::new();
        let custom_sock_dir = temp.path().join("custom_socks");
        fs::create_dir_all(&custom_sock_dir).unwrap();
        let sock_file = custom_sock_dir.join("core.sock");
        fs::write(&sock_file, b"socket placeholder").unwrap();

        let paths = ServePaths {
            home: temp.path().to_path_buf(),
            core_sock: sock_file.clone(),
            worker_sock: custom_sock_dir.join("w.sock"),
            bot_worker_sock: custom_sock_dir.join("bw.sock"),
            gateway_db: None,
            temp_dir_guard: None,
        };

        drop(paths);
        assert!(
            custom_sock_dir.exists(),
            "from_env paths must never delete custom directory"
        );
        assert!(
            sock_file.exists(),
            "from_env paths must never delete custom socket file"
        );
    }

    #[test]
    fn test_serve_paths_isolated_cleans_up_on_drop() {
        let temp = TempDir::new();
        let sock_dir;
        {
            let paths = ServePaths::isolated(temp.path());
            sock_dir = paths
                .socket_dir()
                .expect("isolated paths must have socket_dir");
            assert!(
                sock_dir.exists(),
                "socket directory must exist while paths is alive"
            );
        }
        assert!(
            !sock_dir.exists(),
            "isolated socket directory must be removed on drop"
        );
    }

    #[test]
    fn test_serve_paths_clone_lifecycle_shares_ownership_without_early_deletion() {
        let temp = TempDir::new();
        let p1 = ServePaths::isolated(temp.path());
        let sock_dir = p1.socket_dir().unwrap();
        assert!(sock_dir.exists());

        let p2 = p1.clone();
        let p3 = p2.clone();

        drop(p1);
        assert!(
            sock_dir.exists(),
            "directory must still exist after dropping p1"
        );

        drop(p2);
        assert!(
            sock_dir.exists(),
            "directory must still exist after dropping p2"
        );

        drop(p3);
        assert!(
            !sock_dir.exists(),
            "directory must be removed after dropping final clone p3"
        );
    }

    #[test]
    fn test_serve_paths_try_isolated_directory_creation_failure() {
        let temp = TempDir::new();
        let file_as_dir = temp.path().join("not_a_dir");
        fs::write(&file_as_dir, b"regular file").unwrap();

        let res = ServePaths::try_isolated(&file_as_dir);
        assert!(
            res.is_err(),
            "try_isolated must return Err when base dir creation fails"
        );
        let err_str = res.err().unwrap().to_string();
        assert!(
            err_str.contains("failed to create base dir")
                || err_str.contains("failed to set permissions"),
            "Error should describe failure context: {err_str}"
        );
    }

    #[test]
    fn test_serve_paths_symlink_replacement_safe_does_not_delete_external_target() {
        let temp = TempDir::new();
        let external_target_dir = temp.path().join("external_important_data");
        fs::create_dir_all(&external_target_dir).unwrap();
        let canary_file = external_target_dir.join("must_survive.txt");
        fs::write(&canary_file, b"precious contents").unwrap();

        let paths = ServePaths::isolated(temp.path());
        let owned_sock_dir = paths.socket_dir().unwrap();
        assert!(owned_sock_dir.exists());

        fs::remove_dir_all(&owned_sock_dir).unwrap();
        std::os::unix::fs::symlink(&external_target_dir, &owned_sock_dir).unwrap();
        assert!(owned_sock_dir.is_symlink());

        drop(paths);

        assert!(
            external_target_dir.exists(),
            "external target must remain intact"
        );
        assert!(
            canary_file.exists(),
            "canary file in external target must not be deleted"
        );
        assert_eq!(fs::read(&canary_file).unwrap(), b"precious contents");
        assert!(!owned_sock_dir.exists(), "symlink itself must be removed");
    }

    #[test]
    fn test_serve_paths_owned_socket_dir_identity_mismatch_does_not_delete_external_dir() {
        let temp = TempDir::new();
        let external_dir = temp.path().join("external_replacement_dir");
        fs::create_dir_all(&external_dir).unwrap();
        let marker_file = external_dir.join("must_not_be_deleted.txt");
        fs::write(&marker_file, b"valuable marker content").unwrap();

        let paths = ServePaths::isolated(temp.path());
        let owned_sock_dir = paths
            .socket_dir()
            .expect("isolated paths must have socket_dir");
        assert!(owned_sock_dir.exists());

        // 模拟攻击或竞态替换：
        // 1. 将原目录重命名为 backup 路径
        let backup_dir = temp.path().join("original_sock_dir_backup");
        fs::rename(&owned_sock_dir, &backup_dir).unwrap();

        // 2. 将 external_dir 移动或重命名到 owned_sock_dir 路径上
        // 此时 owned_sock_dir 路径存在且是目录，但其 inode/dev 属于 external_dir，与 guard 创建时的 dev/ino 不匹配
        fs::rename(&external_dir, &owned_sock_dir).unwrap();
        let current_marker = owned_sock_dir.join("must_not_be_deleted.txt");
        assert!(current_marker.exists());

        // 3. drop paths，触发 OwnedSocketDir::drop
        drop(paths);

        // 4. 断言：由于 dev/ino 身份不匹配，drop 绝不递归删除该外部目录！
        assert!(
            owned_sock_dir.exists(),
            "mismatched directory must not be deleted"
        );
        assert!(
            current_marker.exists(),
            "marker file in mismatched directory must remain intact"
        );
        assert_eq!(
            fs::read(&current_marker).unwrap(),
            b"valuable marker content"
        );

        // 收尾清理
        let _ = fs::remove_dir_all(&owned_sock_dir);
        let _ = fs::remove_dir_all(&backup_dir);
    }

    #[test]
    fn test_serve_paths_from_values_semantics_and_comparison_with_baseline() {
        // 1. HOME 缺失 (None) 且 core_sock 缺失 (None)
        // 必须精确恢复 1e120b5 原生产语义：
        // home 为 "."，worker_sock/bot_worker_sock 挂在 "./.wechat-rpa" 下
        // core_sock 则按原 default_socket_path 规则回退到 "/.wechat-rpa/core.sock"
        {
            let paths = ServePaths::from_values(None, None);
            assert_eq!(paths.home, PathBuf::from("."));
            assert_eq!(
                paths.worker_sock,
                PathBuf::from("./.wechat-rpa/worker.sock")
            );
            assert_eq!(
                paths.bot_worker_sock,
                PathBuf::from("./.wechat-rpa/bot-worker.sock")
            );
            assert_eq!(paths.core_sock, PathBuf::from("/.wechat-rpa/core.sock"));
        }

        // 2. HOME 为空字符串 (Some("")) 且 core_sock 缺失 (None)
        // 1e120b5 中 std::env::var("HOME") 返回 Ok("")，因此 home 为 ""
        {
            let paths = ServePaths::from_values(Some(""), None);
            assert_eq!(paths.home, PathBuf::from(""));
            assert_eq!(paths.worker_sock, PathBuf::from(".wechat-rpa/worker.sock"));
            assert_eq!(
                paths.bot_worker_sock,
                PathBuf::from(".wechat-rpa/bot-worker.sock")
            );
            assert_eq!(paths.core_sock, PathBuf::from(".wechat-rpa/core.sock"));
        }

        // 3. HOME 有效设置且 core_sock 缺失 (None)
        {
            let paths = ServePaths::from_values(Some("/opt/fake_user_dir"), None);
            assert_eq!(paths.home, PathBuf::from("/opt/fake_user_dir"));
            assert_eq!(
                paths.worker_sock,
                PathBuf::from("/opt/fake_user_dir/.wechat-rpa/worker.sock")
            );
            assert_eq!(
                paths.bot_worker_sock,
                PathBuf::from("/opt/fake_user_dir/.wechat-rpa/bot-worker.sock")
            );
            assert_eq!(
                paths.core_sock,
                PathBuf::from("/opt/fake_user_dir/.wechat-rpa/core.sock")
            );
        }

        // 4. WECHAT_CORE_SOCK 覆盖存在
        {
            let paths =
                ServePaths::from_values(Some("/opt/fake_user_dir"), Some("/tmp/custom.sock"));
            assert_eq!(paths.core_sock, PathBuf::from("/tmp/custom.sock"));
        }

        // 5. WECHAT_CORE_SOCK 为空或纯空白，应忽略并回退到默认路径
        {
            let paths_empty = ServePaths::from_values(Some("/opt/fake_user_dir"), Some(""));
            assert_eq!(
                paths_empty.core_sock,
                PathBuf::from("/opt/fake_user_dir/.wechat-rpa/core.sock")
            );

            let paths_spaces = ServePaths::from_values(Some("/opt/fake_user_dir"), Some("   "));
            assert_eq!(
                paths_spaces.core_sock,
                PathBuf::from("/opt/fake_user_dir/.wechat-rpa/core.sock")
            );
        }
    }
}
