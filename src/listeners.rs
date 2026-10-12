use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use rusqlite::params;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::{
    accounts::{AccountKey, AccountRegistry},
    adapters::{MessageAdapter, Operation},
    resource_lock::ResourceLock,
    storage::{EVENT_SELECT, Store, now, query},
};

#[derive(Clone, Serialize)]
struct Health {
    state: &'static str,
    interval_seconds: u64,
    last_success_at: Option<i64>,
    last_error: Option<String>,
    queued_observations: u64,
    backoff_seconds: u64,
}
struct Worker {
    task: JoinHandle<()>,
    health: Arc<Mutex<Health>>,
    official_events: bool,
}
#[derive(Default)]
pub struct Listeners {
    workers: Mutex<BTreeMap<AccountKey, Worker>>,
    control: tokio::sync::Mutex<()>,
}

impl Listeners {
    fn spawn_worker(
        &self,
        adapter: Arc<dyn MessageAdapter>,
        store: Arc<Store>,
        interval: u64,
        ownership: ResourceLock,
    ) -> Result<Value> {
        if !(2..=60).contains(&interval) {
            bail!("interval_seconds 必须为 2–60");
        }
        if !adapter.account().capabilities.background_events
            && (adapter.account().capabilities.source != "desktop_ocr"
                || !adapter.account().capabilities.visible_conversations)
        {
            bail!("此适配器不支持桌面会话观察；API 适配器应通过已验证事件接入");
        }
        let key = adapter.account().key.clone();
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| anyhow::anyhow!("监听管理锁损坏"))?;
        if workers
            .get(&key)
            .is_some_and(|worker| !worker.task.is_finished())
        {
            bail!("账号已有运行中的监听，请先停止再调整");
        }
        let health = Arc::new(Mutex::new(Health {
            state: "starting",
            interval_seconds: interval,
            last_success_at: None,
            last_error: None,
            queued_observations: 0,
            backoff_seconds: 0,
        }));
        let live = health.clone();
        let account = key.clone();
        let official_events = adapter.account().capabilities.background_events;
        let source = adapter.account().capabilities.source.clone();
        let task = tokio::spawn(async move {
            let _ownership = ownership;
            let mut rate_failures = 0u32;
            loop {
                let mut delay = interval;
                let capture = async {
                    if !store.listener_enabled(account.clone()).await? {
                        return Ok(None);
                    }
                    if official_events {
                        let cursor = store.provider_cursor(account.clone()).await?;
                        let output = adapter.execute(Operation::SyncEvents { cursor }).await?;
                        if output["account"] != json!(account) {
                            bail!("适配器返回了不同账号，拒绝入库");
                        }
                        let page: crate::wecom_kf::SyncPage =
                            serde_json::from_value(output["data"].clone())
                                .map_err(|_| anyhow::anyhow!("平台同步页格式无效"))?;
                        let has_more = page.has_more;
                        let count = store.commit_provider_page(account.clone(), page).await?;
                        Ok(Some((count, has_more)))
                    } else {
                        let output = adapter.execute(Operation::ListConversations).await?;
                        if output["account"] != json!(account) {
                            bail!("适配器返回了不同账号，拒绝入库");
                        }
                        let snapshot = output["data"]["conversations"].clone();
                        store
                            .observe(account.clone(), snapshot)
                            .await
                            .map(|count| Some((count, false)))
                    }
                };
                let outcome = tokio::time::timeout(Duration::from_secs(30), capture).await;
                {
                    let mut status = live.lock().expect("listener health lock");
                    match outcome {
                        Ok(Ok(None)) => {
                            status.state = "stopped";
                            break;
                        }
                        Ok(Ok(Some((count, has_more)))) => {
                            rate_failures = 0;
                            status.backoff_seconds = 0;
                            if has_more {
                                delay = 0;
                            }
                            status.state = "running";
                            status.last_success_at = Some(now());
                            status.last_error = None;
                            status.queued_observations += count as u64;
                        }
                        Ok(Err(error)) => {
                            if official_events
                                && error
                                    .downcast_ref::<crate::wecom_kf::ProviderError>()
                                    .is_some_and(|e| e.rate_limited())
                            {
                                rate_failures = rate_failures.saturating_add(1);
                                delay = sync_backoff(interval, rate_failures);
                                status.backoff_seconds = delay;
                            } else {
                                rate_failures = 0;
                                status.backoff_seconds = 0;
                            }
                            status.state = "degraded";
                            status.last_error = Some(format!("{error:#}"));
                        }
                        Err(_) => {
                            status.state = "degraded";
                            status.last_error = Some("监听操作超过 30 秒，已终止本轮".into());
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        });
        workers.insert(
            key.clone(),
            Worker {
                task,
                health,
                official_events,
            },
        );
        Ok(
            json!({"account":key,"data":{"state":"starting","source":source,"incoming_verified":official_events,"agent_wakeup":"consumer_must_wait_or_poll_inbox"}}),
        )
    }

    pub fn status(&self, key: &AccountKey) -> Result<Value> {
        let workers = self
            .workers
            .lock()
            .map_err(|_| anyhow::anyhow!("监听管理锁损坏"))?;
        let data = match workers.get(key) {
            Some(worker) => {
                let health = worker
                    .health
                    .lock()
                    .map_err(|_| anyhow::anyhow!("监听健康锁损坏"))?;
                json!({"health":*health,"worker_finished":worker.task.is_finished()})
            }
            None => json!({"health":{"state":"stopped"}}),
        };
        let incoming_verified = workers
            .get(key)
            .is_some_and(|worker| worker.official_events);
        Ok(json!({"account":key,"data":data,"incoming_verified":incoming_verified}))
    }

    pub async fn stop(&self, key: &AccountKey, store: Arc<Store>) -> Result<Value> {
        let _control = self.control.lock().await;
        store.disable_listener(key.clone()).await?;
        let worker = self
            .workers
            .lock()
            .map_err(|_| anyhow::anyhow!("监听管理锁损坏"))?
            .remove(key);
        if let Some(worker) = worker {
            worker.task.abort();
            let _ = worker.task.await;
        }
        let state = if ResourceLock::try_acquire(&store.listener_lock(key))?.is_some() {
            "stopped"
        } else {
            "stop_requested"
        };
        Ok(json!({"account":key,"data":{"state":state,"enabled":false}}))
    }

    pub async fn shutdown(&self) {
        let workers = std::mem::take(&mut *self.workers.lock().expect("listener manager lock"));
        for worker in workers.into_values() {
            worker.task.abort();
            let _ = worker.task.await;
        }
    }
}

pub async fn wait_events(
    store: Arc<Store>,
    key: AccountKey,
    timeout: u64,
    limit: u32,
) -> Result<Value> {
    if timeout > 300 || !(1..=100).contains(&limit) {
        bail!("timeout_seconds 必须为 0–300，limit 为 1–100");
    }
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let account = key.clone();
        let events=store.run(move |conn|query(conn,&format!("{EVENT_SELECT} AND (state='pending' OR (state='leased' AND lease_until<=?3)) ORDER BY observed_at,event_id LIMIT ?4"),params![account.platform,account.account_id,now(),limit])).await?;
        if !events.is_empty() || Instant::now() >= deadline {
            return Ok(
                json!({"account":key,"data":{"status":if events.is_empty(){"timeout"}else{"available"},"events":events,"claimed":false}}),
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(remaining.min(Duration::from_millis(200))).await;
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    StartListener {
        #[serde(default = "interval")]
        interval_seconds: u64,
    },
    StopListener,
    GetListenerStatus,
    WaitForEvents {
        #[serde(default = "timeout")]
        timeout_seconds: u64,
        #[serde(default = "limit")]
        limit: u32,
    },
}
fn interval() -> u64 {
    5
}
fn timeout() -> u64 {
    60
}
fn limit() -> u32 {
    50
}
pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "start_listener" | "stop_listener" | "get_listener_status" | "wait_for_events"
    )
}
impl Listeners {
    pub async fn execute(
        &self,
        adapter: Arc<dyn MessageAdapter>,
        store: Arc<Store>,
        name: &str,
        mut args: Value,
    ) -> Result<Value> {
        let object = args
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("arguments 必须是对象"))?;
        if object.values().any(Value::is_null) {
            bail!("参数不接受 null；未设置的可选字段应省略");
        }
        object.remove("platform");
        object.remove("account_id");
        if object.contains_key("operation") {
            bail!("operation 不是工具参数");
        }
        object.insert("operation".into(), json!(name));
        let request: Request = serde_json::from_value(args)?;
        let key = adapter.account().key.clone();
        match request {
            Request::StartListener { interval_seconds } => {
                self.start(adapter, store, interval_seconds).await
            }
            Request::StopListener => self.stop(&key, store).await,
            Request::GetListenerStatus => {
                let mut status = self.status(&key)?;
                status["data"]["enabled"] = json!(store.listener_enabled(key.clone()).await?);
                let local_running = self
                    .workers
                    .lock()
                    .map_err(|_| anyhow::anyhow!("监听管理锁损坏"))?
                    .get(&key)
                    .is_some_and(|worker| !worker.task.is_finished());
                if !local_running
                    && ResourceLock::try_acquire(&store.listener_lock(&key))?.is_none()
                {
                    status["data"]["health"]["state"] = json!("owned_by_another_process");
                }
                Ok(status)
            }
            Request::WaitForEvents {
                timeout_seconds,
                limit,
            } => wait_events(store, key, timeout_seconds, limit).await,
        }
    }
}

impl Listeners {
    pub async fn start(
        &self,
        adapter: Arc<dyn MessageAdapter>,
        store: Arc<Store>,
        interval: u64,
    ) -> Result<Value> {
        if !(2..=60).contains(&interval) {
            bail!("interval_seconds 必须为 2–60");
        }
        if !adapter.account().capabilities.background_events
            && (adapter.account().capabilities.source != "desktop_ocr"
                || !adapter.account().capabilities.visible_conversations)
        {
            bail!("适配器不支持桌面观察");
        }
        let _control = self.control.lock().await;
        let key = adapter.account().key.clone();
        let existing = {
            let workers = self
                .workers
                .lock()
                .map_err(|_| anyhow::anyhow!("监听管理锁损坏"))?;
            workers
                .get(&key)
                .filter(|worker| !worker.task.is_finished())
                .map(|worker| worker.health.lock().expect("health lock").interval_seconds)
        };
        if let Some(current) = existing {
            if current != interval {
                bail!("监听已运行，调整间隔前请先停止");
            }
            return self.status(&key);
        }
        let ownership = ResourceLock::try_acquire(&store.listener_lock(&key))?
            .ok_or_else(|| anyhow::anyhow!("同一数据库的该账号已由另一个进程监听"))?;
        store.set_listener(key, true, interval).await?;
        self.spawn_worker(adapter, store, interval, ownership)
    }
    pub async fn restore(&self, accounts: &AccountRegistry, store: Arc<Store>) -> Result<()> {
        let _control = self.control.lock().await;
        for setting in store.listener_settings().await? {
            if setting["enabled"] != true {
                continue;
            }
            let key = AccountKey {
                platform: setting["platform"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("监听配置平台无效"))?
                    .into(),
                account_id: setting["account_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("监听配置账号无效"))?
                    .into(),
            };
            let adapter = match accounts.resolve(&key.platform, Some(&key.account_id)) {
                Ok(adapter) => adapter,
                Err(_) => {
                    tracing::warn!("跳过未注册账号的历史监听配置");
                    continue;
                }
            };
            let Some(ownership) = ResourceLock::try_acquire(&store.listener_lock(&key))? else {
                continue;
            };
            if !store.listener_enabled(key).await? {
                continue;
            }
            let interval = setting["interval_seconds"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("监听间隔无效"))?;
            self.spawn_worker(adapter, store.clone(), interval, ownership)?;
        }
        Ok(())
    }
}

/// 无回调 token 的轮询发生限流时逐级退避，每页成功后复位。
pub(crate) fn sync_backoff(interval: u64, failures: u32) -> u64 {
    interval.saturating_mul(1u64 << failures.min(6)).min(60)
}
