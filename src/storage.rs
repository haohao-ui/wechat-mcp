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

            let current_sessions = parse_and_normalize_sessions(&snapshot)?;

            let old: Option<Value> = previous.as_deref().map(serde_json::from_str).transpose()?;
            let old_sessions = match old {
                Some(old_val) => Some(parse_and_normalize_sessions(&old_val).context("持久会话快照格式错误")?),
                None => None,
            };

            let mut count = 0;
            if let Some(old_sessions) = old_sessions {
                let pending_events = compute_session_diffs(&old_sessions, &current_sessions, FULL_SCREEN_THRESHOLD);
                for event in pending_events {
                    tx.execute(
                        "INSERT INTO events(event_id,platform,account_id,kind,source,verified,occurred_at,observed_at,payload,state) VALUES(?1,?2,?3,'interface_change','desktop_ocr',0,?4,?4,?5,'pending')",
                        params![new_id(), key.platform, key.account_id, now(), event.payload.to_string()],
                    )?;
                    count += 1;
                }
            }
            // 快照与事件在同一事务提交，保证重启或崩溃不会在推进基线与入队事件之间丢失差异。
            tx.execute("INSERT INTO listener_snapshots(platform,account_id,snapshot,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(platform,account_id) DO UPDATE SET snapshot=excluded.snapshot,updated_at=excluded.updated_at",params![key.platform,key.account_id,snapshot.to_string(),now()])?;
            tx.commit()?;
            Ok(count)
        }).await
    }
}

/// 满屏会话数量阈值（启发式）。
/// 微信桌面端标准分辨率下会话列表一屏通常可见 8~10 个会话项。
/// 考虑到窗口高度缩放与截取区域差异，当旧快照数量达到该阈值时认为列表已满屏，可能存在滚动行为。
pub const FULL_SCREEN_THRESHOLD: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedSession {
    pub key: String,
    pub is_ambiguous: bool,
    pub display_name: String,
    pub preview: String,
    pub time_text: String,
    pub raw: Value,
}

/// 解析会话列表并进行规范化处理：
/// 1. 严格检查顶层是否为数组及每项是否为对象；
/// 2. 跳过缺少有效 display_name 的项（容错 OCR 截断或碎片），而非整体失败；
/// 3. 对有效会话项严格校验 preview 和 time_text 为字符串；
/// 4. 对同名组按出现顺序生成比较键（如 name#1、name#2），并标记 is_ambiguous 为 true。
pub(crate) fn parse_and_normalize_sessions(snapshot: &Value) -> Result<Vec<NormalizedSession>> {
    let items = snapshot.as_array().context("适配器会话结果必须是数组")?;

    let mut valid_items = Vec::new();
    for conversation in items {
        if !conversation.is_object() {
            bail!("会话项必须是对象");
        }
        let display_name_opt = conversation
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty());

        let Some(name) = display_name_opt else {
            // 会话缺少 display_name 的项跳过而非整体失败
            continue;
        };

        let preview = conversation.get("preview").and_then(Value::as_str);
        let time_text = conversation.get("time_text").and_then(Value::as_str);
        if preview.is_none() || time_text.is_none() {
            bail!("会话预览或时间字段格式错误");
        }

        valid_items.push((
            name.to_string(),
            preview.unwrap().to_string(),
            time_text.unwrap().to_string(),
            conversation.clone(),
        ));
    }

    let mut name_counts = std::collections::HashMap::new();
    for (name, _, _, _) in &valid_items {
        *name_counts.entry(name.clone()).or_insert(0usize) += 1;
    }

    let mut seen_counts = std::collections::HashMap::new();
    let mut normalized = Vec::with_capacity(valid_items.len());
    for (name, preview, time_text, raw) in valid_items {
        let total = name_counts.get(&name).copied().unwrap_or(0);
        let (key, is_ambiguous) = if total > 1 {
            let count = seen_counts.entry(name.clone()).or_insert(0usize);
            *count += 1;
            (format!("{name}#{count}"), true)
        } else {
            (name.clone(), false)
        };
        normalized.push(NormalizedSession {
            key,
            is_ambiguous,
            display_name: name,
            preview,
            time_text,
            raw,
        });
    }

    Ok(normalized)
}

/// 判定在旧快照中不存在的会话是否作为新消息入队。
///
/// 【启发式规则说明】
/// 1. 微信桌面端 OCR 观察仅覆盖当前可见区域（visible_only）。
/// 2. 旧快照未满屏时（old_count < threshold），不可能有会话被滚出，因此任何新出现的会话都应入队。
/// 3. 满屏时（old_count >= threshold），用户滚动可能导致历史会话滚出或滚回视口；
///    当新消息到来时，微信会将该会话置顶至列表第一项（即索引 0）。
///    因此满屏时只有索引 0 的新会话入队，非顶部位置视为滚动带出的历史会话不入队。
/// 4. 已知限制：满屏时向上滚动露出的会话位于索引 0，与真实新消息无法区分。
pub fn is_unseen_conversation_new_incoming(
    index: usize,
    old_count: usize,
    threshold: usize,
) -> bool {
    index == 0 || old_count < threshold
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingEvent {
    pub payload: Value,
}

/// 对比新旧规范化快照并生成待入队事件列表（纯函数）。
pub(crate) fn compute_session_diffs(
    old_sessions: &[NormalizedSession],
    current_sessions: &[NormalizedSession],
    full_screen_threshold: usize,
) -> Vec<PendingEvent> {
    let mut events = Vec::new();
    for (index, current) in current_sessions.iter().enumerate() {
        let matched = old_sessions.iter().find(|old| old.key == current.key);
        match matched {
            Some(old) => {
                // 只有 preview 发生变化时才入队；仅 time_text 变化不入队（但快照照常推进）。preview 不变且 time 变化，不入队。
                if old.preview != current.preview {
                    let mut payload = json!({
                        "conversation": current.raw,
                        "incoming_verified": false,
                        "coverage": "visible_only",
                        "changed": ["preview"],
                    });
                    if current.is_ambiguous {
                        payload["ambiguous_name"] = json!(true);
                    }
                    events.push(PendingEvent { payload });
                }
            }
            None => {
                // 旧快照中不存在的会话：
                // 旧快照未满屏时任何新出现的会话均入队；满屏时仅顶部（索引 0）的新会话入队。
                if is_unseen_conversation_new_incoming(
                    index,
                    old_sessions.len(),
                    full_screen_threshold,
                ) {
                    let mut payload = json!({
                        "conversation": current.raw,
                        "incoming_verified": false,
                        "coverage": "visible_only",
                        "new_conversation": true,
                    });
                    if current.is_ambiguous {
                        payload["ambiguous_name"] = json!(true);
                    }
                    events.push(PendingEvent { payload });
                }
            }
        }
    }
    events
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_normalize_sessions_handles_duplicates_and_missing_names() {
        let input = json!([
            {"display_name": "Alice", "preview": "p1", "time_text": "t1"},
            {"display_name": "Bob", "preview": "p2", "time_text": "t2"},
            {"display_name": "", "preview": "ignored", "time_text": "t_ignored"},
            {"preview": "no_name", "time_text": "t_no_name"},
            {"display_name": "Alice", "preview": "p3", "time_text": "t3"}
        ]);
        let normalized = parse_and_normalize_sessions(&input).unwrap();
        assert_eq!(normalized.len(), 3);

        assert_eq!(normalized[0].key, "Alice#1");
        assert!(normalized[0].is_ambiguous);
        assert_eq!(normalized[0].preview, "p1");

        assert_eq!(normalized[1].key, "Bob");
        assert!(!normalized[1].is_ambiguous);

        assert_eq!(normalized[2].key, "Alice#2");
        assert!(normalized[2].is_ambiguous);
        assert_eq!(normalized[2].preview, "p3");
    }

    #[test]
    fn parse_and_normalize_sessions_rejects_malformed_formats() {
        assert!(parse_and_normalize_sessions(&json!("not an array")).is_err());
        assert!(parse_and_normalize_sessions(&json!([123])).is_err());
        assert!(
            parse_and_normalize_sessions(
                &json!([{"display_name": "Alice", "preview": 123, "time_text": "t"}])
            )
            .is_err()
        );
        assert!(
            parse_and_normalize_sessions(
                &json!([{"display_name": "Alice", "preview": "p", "time_text": false}])
            )
            .is_err()
        );
    }

    #[test]
    fn unseen_conversation_heuristic_boundary_check() {
        // 满屏（old_count >= threshold）：仅顶部索引 0 入队，非顶部不入队
        assert!(is_unseen_conversation_new_incoming(0, 5, 5));
        assert!(is_unseen_conversation_new_incoming(0, 10, 5));
        assert!(!is_unseen_conversation_new_incoming(1, 5, 5));
        assert!(!is_unseen_conversation_new_incoming(2, 8, 5));

        // 未满屏（old_count < threshold）：任何新出现的会话均入队（不可能由于滚动被滚出）
        assert!(is_unseen_conversation_new_incoming(0, 4, 5));
        assert!(is_unseen_conversation_new_incoming(1, 4, 5));
        assert!(is_unseen_conversation_new_incoming(3, 2, 5));
        assert!(is_unseen_conversation_new_incoming(0, 0, 5));
        assert!(is_unseen_conversation_new_incoming(1, 0, 5));
    }

    #[test]
    fn diff_computation_enqueues_only_on_preview_change_or_top_unseen() {
        let old = parse_and_normalize_sessions(&json!([
            {"display_name": "Alice", "preview": "hello", "time_text": "10:00"},
            {"display_name": "Bob", "preview": "stay", "time_text": "10:01"},
            {"display_name": "Cathy", "preview": "c", "time_text": "10:02"},
            {"display_name": "David", "preview": "d", "time_text": "10:03"},
            {"display_name": "Eva", "preview": "e", "time_text": "10:04"}
        ]))
        .unwrap();

        // 1. 仅 time_text 变化：不入队
        let cur_time_only = parse_and_normalize_sessions(&json!([
            {"display_name": "Alice", "preview": "hello", "time_text": "昨天"},
            {"display_name": "Bob", "preview": "stay", "time_text": "昨天"},
            {"display_name": "Cathy", "preview": "c", "time_text": "昨天"},
            {"display_name": "David", "preview": "d", "time_text": "昨天"},
            {"display_name": "Eva", "preview": "e", "time_text": "昨天"}
        ]))
        .unwrap();
        let diffs = compute_session_diffs(&old, &cur_time_only, 5);
        assert_eq!(diffs.len(), 0);

        // 2. preview 变化：入队且 changed 为 ["preview"]
        let cur_preview = parse_and_normalize_sessions(&json!([
            {"display_name": "Alice", "preview": "hello world", "time_text": "昨天"},
            {"display_name": "Bob", "preview": "stay", "time_text": "昨天"},
            {"display_name": "Cathy", "preview": "c", "time_text": "昨天"},
            {"display_name": "David", "preview": "d", "time_text": "昨天"},
            {"display_name": "Eva", "preview": "e", "time_text": "昨天"}
        ]))
        .unwrap();
        let diffs = compute_session_diffs(&old, &cur_preview, 5);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].payload["changed"], json!(["preview"]));
        assert_eq!(diffs[0].payload["conversation"]["preview"], "hello world");

        // 3. 滚动出现的会话（非顶部索引 0）：不入队
        let cur_scrolled = parse_and_normalize_sessions(&json!([
            {"display_name": "Bob", "preview": "stay", "time_text": "10:01"},
            {"display_name": "Cathy", "preview": "c", "time_text": "10:02"},
            {"display_name": "David", "preview": "d", "time_text": "10:03"},
            {"display_name": "Eva", "preview": "e", "time_text": "10:04"},
            {"display_name": "Frank", "preview": "f", "time_text": "10:05"}
        ]))
        .unwrap();
        let diffs = compute_session_diffs(&old, &cur_scrolled, 5);
        assert_eq!(diffs.len(), 0);

        // 4. 满屏时顶部新会话（索引 0）：入队且 new_conversation 为 true
        let cur_top_new = parse_and_normalize_sessions(&json!([
            {"display_name": "Zoe", "preview": "new message", "time_text": "刚刚"},
            {"display_name": "Alice", "preview": "hello", "time_text": "10:00"},
            {"display_name": "Bob", "preview": "stay", "time_text": "10:01"},
            {"display_name": "Cathy", "preview": "c", "time_text": "10:02"},
            {"display_name": "David", "preview": "d", "time_text": "10:03"}
        ]))
        .unwrap();
        let diffs = compute_session_diffs(&old, &cur_top_new, 5);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].payload["new_conversation"], json!(true));
        assert_eq!(diffs[0].payload["conversation"]["display_name"], "Zoe");

        // 5. 旧快照未满屏时新会话：无论位置均入队
        let old_not_full = parse_and_normalize_sessions(&json!([
            {"display_name": "Alice", "preview": "hello", "time_text": "10:00"},
            {"display_name": "Bob", "preview": "stay", "time_text": "10:01"}
        ]))
        .unwrap();
        let cur_not_full = parse_and_normalize_sessions(&json!([
            {"display_name": "Alice", "preview": "hello", "time_text": "10:00"},
            {"display_name": "Bob", "preview": "stay", "time_text": "10:01"},
            {"display_name": "Charlie", "preview": "c", "time_text": "10:02"}
        ]))
        .unwrap();
        let diffs = compute_session_diffs(&old_not_full, &cur_not_full, 5);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].payload["new_conversation"], json!(true));
        assert_eq!(diffs[0].payload["conversation"]["display_name"], "Charlie");
    }
}
