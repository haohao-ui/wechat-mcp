pub mod backup;
pub mod contacts;
pub mod brain_tools;
pub mod config;
pub mod control;
pub mod delivery;
pub mod external_brain;
pub mod history;
pub mod mcp_client;
pub mod model;
pub mod policy;
pub mod replay;
pub mod scheduler;
pub mod serve;
pub mod state;
pub mod turns;
pub mod worker_client;


/// 测试专用：所有修改进程环境变量（HOME、WECHAT_CORE_SOCK 等）的测试共用这一把锁，
/// 否则不同模块的测试并行时互相改环境变量会串扰。中毒时继续使用，避免一个失败连锁成一片。
#[cfg(test)]
static PROCESS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
