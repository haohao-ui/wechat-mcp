use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Params, TransactionBehavior, params};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::accounts::AccountKey;

pub struct Store {
    connection: Mutex<Connection>,
    lock_prefix: std::path::PathBuf,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock predates Unix epoch")
        .as_secs() as i64
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn query(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

pub fn one(conn: &Connection, sql: &str, params: impl Params) -> Result<Value> {
    query(conn, sql, params)?
        .into_iter()
        .next()
        .context("记录不存在或不属于指定账号")
}

pub fn require_customer(conn: &Connection, key: &AccountKey, customer: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM customers WHERE platform=?1 AND account_id=?2 AND customer_id=?3)",
        params![key.platform, key.account_id, customer], |row| row.get(0),
    )?;
    if !exists {
        bail!("客户不存在或不属于指定账号");
    }
    Ok(())
}

pub const EVENT_SELECT: &str = "SELECT json_object('event_id',event_id,'customer_id',customer_id,'kind',kind,'source',source,'verified',json(CASE verified WHEN 1 THEN 'true' ELSE 'false' END),'occurred_at',occurred_at,'observed_at',observed_at,'payload',json(payload),'state',state,'consumer',consumer,'lease_until',lease_until) FROM events WHERE platform=?1 AND account_id=?2";
pub const OUTBOX_SELECT: &str = "SELECT json_object('send_id',send_id,'customer_id',customer_id,'idempotency_key',idempotency_key,'recipient',recipient,'message',message,'status',status,'result',CASE WHEN result IS NULL THEN NULL ELSE json(result) END,'created_at',created_at,'updated_at',updated_at,'delivery','unknown') FROM outbox WHERE platform=?1 AND account_id=?2";

impl Store {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if !path.is_absolute() {
            bail!("数据库路径必须是绝对路径");
        }
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true).mode(0o700).create(parent)?;
            }
            #[cfg(not(unix))]
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(path)?;
        }
        let conn = Connection::open(path).context("打开客服数据库失败")?;
        Self::initialize(conn)
    }

    #[cfg(test)]
    pub fn memory() -> Arc<Self> {
        Self::initialize(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn initialize(mut conn: Connection) -> Result<Arc<Self>> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                transaction.execute_batch(include_str!("../migrations/001_service_core.sql"))?;
                transaction
                    .execute_batch(include_str!("../migrations/002_listener_settings.sql"))?;
            }
            1 => transaction
                .execute_batch(include_str!("../migrations/002_listener_settings.sql"))?,
            2 => {}
            _ => bail!("数据库版本 {version} 高于支持版本，拒绝打开"),
        }
        transaction.commit()?;
        let lock_prefix = match conn.path().filter(|path| !path.is_empty()) {
            Some(path) => std::fs::canonicalize(path)?,
            None => std::env::temp_dir().join(format!("gateway-memory-{}", new_id())),
        };
        Ok(Arc::new(Self {
            connection: Mutex::new(conn),
            lock_prefix,
        }))
    }

    pub async fn run<T, F>(self: &Arc<Self>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = store
                .connection
                .lock()
                .map_err(|_| anyhow::anyhow!("数据库锁已损坏"))?;
            operation(&mut conn)
        })
        .await
        .context("数据库任务失败")?
    }

    pub async fn begin_send(
        self: &Arc<Self>,
        key: AccountKey,
        request_key: String,
        recipient: String,
        message: String,
        customer: Option<String>,
    ) -> Result<(bool, Value)> {
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(customer) = customer.as_ref() {
                require_customer(&tx, &key, customer)?;
                let target: Option<String> = tx.query_row("SELECT recipient FROM customers WHERE platform=?1 AND account_id=?2 AND customer_id=?3", params![key.platform,key.account_id,customer], |r| r.get(0))?;
                if target.as_deref() != Some(&recipient) { bail!("recipient 与客户档案的发送目标不一致"); }
            }
            let existing = query(&tx, &format!("{OUTBOX_SELECT} AND idempotency_key=?3"), params![key.platform,key.account_id,request_key])?.into_iter().next();
            if let Some(existing) = existing {
                if existing["recipient"] != recipient || existing["message"] != message || existing["customer_id"] != json!(customer) {
                    bail!("幂等键已用于不同的发送请求");
                }
                return Ok((false, existing));
            }
            let id = new_id(); let time = now();
            tx.execute("INSERT INTO outbox(send_id,platform,account_id,customer_id,idempotency_key,recipient,message,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'attempting',?8,?8)",params![id,key.platform,key.account_id,customer,request_key,recipient,message,time])?;
            let receipt = one(&tx,&format!("{OUTBOX_SELECT} AND send_id=?3"),params![key.platform,key.account_id,id])?;
            tx.commit()?;
            Ok((true,receipt))
        }).await
    }

    pub async fn finish_send(
        self: &Arc<Self>,
        key: AccountKey,
        send_id: String,
        result: Value,
        status: &'static str,
    ) -> Result<Value> {
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = tx.execute("UPDATE outbox SET status=?4,result=?5,updated_at=?6 WHERE platform=?1 AND account_id=?2 AND send_id=?3 AND status='attempting'",params![key.platform,key.account_id,send_id,status,result.to_string(),now()])?;
            if changed != 1 { bail!("发送状态不允许变更"); }
            let receipt = one(&tx,&format!("{OUTBOX_SELECT} AND send_id=?3"),params![key.platform,key.account_id,send_id])?;
            tx.commit()?;
            Ok(receipt)
        }).await
    }

    pub async fn observe(self: &Arc<Self>, key: AccountKey, snapshot: Value) -> Result<usize> {
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let previous: Option<String> = tx.query_row("SELECT snapshot FROM listener_snapshots WHERE platform=?1 AND account_id=?2",params![key.platform,key.account_id],|r|r.get(0)).optional()?;
            let current = snapshot.as_array().context("适配器会话结果必须是数组")?;
            let mut names = std::collections::HashSet::new();
            for conversation in current {
                let name = conversation.get("display_name").and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty()).context("会话缺少有效 display_name")?;
                if !names.insert(name) { bail!("可见会话存在同名项，无法可靠建立观察基线"); }
                if !conversation["preview"].is_string() || !conversation["time_text"].is_string() {
                    bail!("会话预览或时间字段格式错误");
                }
            }

            let old: Option<Value> = previous.as_deref().map(serde_json::from_str).transpose()?;
            let mut count = 0;
            if let Some(old) = old {
                let old = old.as_array().context("持久会话快照格式错误")?;
                for conversation in current {
                    let name = conversation.get("display_name").and_then(Value::as_str).context("会话缺少 display_name")?;
                    let matched = old.iter().find(|item| item["display_name"] == name);
                    if matched != Some(conversation) {
                        let payload = json!({"conversation":conversation,"incoming_verified":false,"coverage":"visible_only"});
                        tx.execute("INSERT INTO events(event_id,platform,account_id,kind,source,verified,occurred_at,observed_at,payload,state) VALUES(?1,?2,?3,'interface_change','desktop_ocr',0,?4,?4,?5,'pending')",params![new_id(),key.platform,key.account_id,now(),payload.to_string()])?;
                        count += 1;
                    }
                }
            }
            // Snapshot and inbox writes commit together so a restart cannot drop
            // a detected change between advancing the baseline and queuing it.
            tx.execute("INSERT INTO listener_snapshots(platform,account_id,snapshot,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(platform,account_id) DO UPDATE SET snapshot=excluded.snapshot,updated_at=excluded.updated_at",params![key.platform,key.account_id,snapshot.to_string(),now()])?;
            tx.commit()?;
            Ok(count)
        }).await
    }
}

impl Store {
    pub async fn capture(
        self: &Arc<Self>,
        key: AccountKey,
        customer: String,
        payload: Value,
    ) -> Result<Value> {
        self.run(move |conn| {
            let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_customer(&tx,&key,&customer)?;
            let event_id=new_id();let time=now();
            tx.execute("INSERT INTO events(event_id,platform,account_id,customer_id,kind,source,verified,occurred_at,observed_at,payload,state) VALUES(?1,?2,?3,?4,'chat_snapshot','desktop_ocr',0,?5,?5,?6,'recorded')",params![event_id,key.platform,key.account_id,customer,time,payload.to_string()])?;
            let event=one(&tx,&format!("{EVENT_SELECT} AND event_id=?3"),params![key.platform,key.account_id,event_id])?;
            tx.commit()?;
            Ok(json!({"account":key,"data":event}))
        }).await
    }
}

impl Store {
    pub fn listener_lock(&self, key: &AccountKey) -> std::path::PathBuf {
        let mut path = self.lock_prefix.as_os_str().to_os_string();
        path.push(format!(
            ".listener-{}-{}.lock",
            key.platform, key.account_id
        ));
        path.into()
    }
    pub async fn listener_settings(self: &Arc<Self>) -> Result<Vec<Value>> {
        self.run(|conn|query(conn,"SELECT json_object('platform',platform,'account_id',account_id,'enabled',json(CASE enabled WHEN 1 THEN 'true' ELSE 'false' END),'interval_seconds',interval_seconds,'updated_at',updated_at) FROM listener_settings",[])).await
    }
    pub async fn set_listener(
        self: &Arc<Self>,
        key: AccountKey,
        enabled: bool,
        interval: u64,
    ) -> Result<()> {
        if !(2..=60).contains(&interval) {
            bail!("监听间隔必须为 2–60 秒");
        }
        self.run(move |conn| {
            conn.execute("INSERT INTO listener_settings(platform,account_id,enabled,interval_seconds,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(platform,account_id) DO UPDATE SET enabled=excluded.enabled,interval_seconds=excluded.interval_seconds,updated_at=excluded.updated_at",params![key.platform,key.account_id,enabled,interval as i64,now()])?;
            Ok(())
        }).await
    }
    pub async fn listener_enabled(self: &Arc<Self>, key: AccountKey) -> Result<bool> {
        self.run(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT enabled FROM listener_settings WHERE platform=?1 AND account_id=?2",
                    params![key.platform, key.account_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .await
    }
}

impl Store {
    pub async fn disable_listener(self: &Arc<Self>, key: AccountKey) -> Result<()> {
        self.run(move |conn| {
            conn.execute("INSERT INTO listener_settings(platform,account_id,enabled,interval_seconds,updated_at) VALUES(?1,?2,0,5,?3) ON CONFLICT(platform,account_id) DO UPDATE SET enabled=0,updated_at=excluded.updated_at",params![key.platform,key.account_id,now()])?;
            Ok(())
        }).await
    }
}
