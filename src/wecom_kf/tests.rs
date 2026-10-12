use super::*;
use crate::{
    accounts::AccountRegistry,
    gateway::Gateway,
    listeners::Listeners,
    storage::{EVENT_SELECT, new_id, query},
};
use rusqlite::OptionalExtension;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

const SECRET: &str = "test-secret-never-print";
const TOKEN: &str = "test-access-token-never-print";

#[derive(Clone)]
struct Request {
    path: String,
    body: Value,
}
enum Reply {
    Json(Value),
    Raw(&'static str),
    Close,
    Delay(Duration, Value),
}
struct Mock {
    base: String,
    requests: Arc<StdMutex<Vec<Request>>>,
    task: JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    async fn new(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let log = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                let (head_end, length) = loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break (0, 0);
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..pos]);
                        let len = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                if head_end == 0 {
                    continue;
                }
                while bytes.len() < head_end + length {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let head = String::from_utf8_lossy(&bytes[..head_end]);
                let path = head.split_whitespace().nth(1).unwrap().to_owned();
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[head_end..head_end + length]).unwrap()
                };
                let request = Request { path, body };
                let reply = handler(&request);
                log.lock().unwrap().push(request);
                let body = match reply {
                    Reply::Json(v) => v.to_string(),
                    Reply::Raw(v) => v.to_owned(),
                    Reply::Close => continue,
                    Reply::Delay(delay, v) => {
                        tokio::time::sleep(delay).await;
                        v.to_string()
                    }
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self {
            base,
            requests,
            task,
        }
    }
    async fn scripted(replies: Vec<Reply>) -> Self {
        let replies = StdMutex::new(VecDeque::from(replies));
        Self::new(move |_| {
            replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Reply::Json(empty_page("end")))
        })
        .await
    }
    fn paths(&self, endpoint: &str) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path.contains(endpoint))
            .cloned()
            .collect()
    }
}
fn token(expiry: u64) -> Reply {
    Reply::Json(json!({"errcode":0,"access_token":TOKEN,"expires_in":expiry}))
}
fn empty_page(cursor: &str) -> Value {
    json!({"errcode":0,"next_cursor":cursor,"has_more":0,"msg_list":[]})
}
fn message(id: &str, origin: u64, time: i64) -> Value {
    json!({"msgid":id,"open_kfid":"kf","external_userid":"user","send_time":time,"origin":origin,"msgtype":"text","text":{"content":"hello"}})
}
fn adapter(mock: &Mock, store: Arc<Store>) -> Arc<WecomKf> {
    Arc::new(
        WecomKf::new(
            "a".into(),
            WecomKfConfig {
                corp_id: "corp".into(),
                open_kfid: "kf".into(),
                secret_env: "unused".into(),
                base_url: Some(mock.base.clone()),
            },
            SECRET.into(),
            store,
        )
        .unwrap(),
    )
}
fn gateway(adapter: Arc<WecomKf>, store: Arc<Store>) -> Gateway {
    let mut accounts = AccountRegistry::default();
    accounts.register(adapter).unwrap();
    Gateway {
        accounts,
        store,
        listeners: Listeners::default(),
        legacy_tools: true,
    }
}
fn args(key: &str) -> Value {
    json!({"platform":"wechat_kf","account_id":"a","recipient":"user","message":"reply","idempotency_key":key})
}
async fn sync(adapter: &WecomKf) -> Result<Value> {
    adapter
        .execute(Operation::SyncEvents { cursor: None })
        .await
}
async fn inbound(store: &Arc<Store>, key: AccountKey, time: i64) {
    store
        .commit_provider_page(
            key,
            SyncPage {
                messages: vec![normalize_message(&message("in", 3, time), "kf").unwrap()],
                next_cursor: "initial".into(),
                has_more: false,
            },
        )
        .await
        .unwrap();
}
async fn status(g: &Gateway, key: &str) -> Value {
    let result = g
        .call(
            "get_send_status",
            json!({"platform":"wechat_kf","account_id":"a","idempotency_key":key}),
        )
        .await;
    assert_eq!(result["isError"], false);
    result["structuredContent"]["data"].clone()
}
fn assert_redacted(text: &str) {
    assert!(!text.contains(SECRET));
    assert!(!text.contains(TOKEN));
}

#[tokio::test]
async fn token_cache_refreshes_five_minutes_early_and_serializes_concurrent_refresh() {
    let mock = Mock::new(|r| {
        if r.path.contains("gettoken") {
            token(7200)
        } else {
            Reply::Json(empty_page("end"))
        }
    })
    .await;
    let a = adapter(&mock, Store::memory());
    let (one, two, three) = tokio::join!(sync(&a), sync(&a), sync(&a));
    one.unwrap();
    two.unwrap();
    three.unwrap();
    assert_eq!(mock.paths("gettoken").len(), 1);
    let remaining = a
        .token
        .lock()
        .await
        .as_ref()
        .unwrap()
        .refresh_at
        .duration_since(Instant::now());
    assert!((6890..=6900).contains(&remaining.as_secs()));
    a.token.lock().await.as_mut().unwrap().refresh_at = Instant::now();
    let (one, two) = tokio::join!(sync(&a), sync(&a));
    one.unwrap();
    two.unwrap();
    assert_eq!(mock.paths("gettoken").len(), 2);
}

#[tokio::test]
async fn token_invalid_retries_once_and_never_loops() {
    for code in [40014, 42001, 40001] {
        let mock = Mock::scripted(vec![
            token(7200),
            Reply::Json(json!({"errcode":code})),
            token(7200),
            Reply::Json(empty_page("ok")),
        ])
        .await;
        sync(&adapter(&mock, Store::memory())).await.unwrap();
        assert_eq!(mock.paths("gettoken").len(), 2);
        assert_eq!(mock.paths("sync_msg").len(), 2);
    }
    let mock = Mock::scripted(vec![
        token(7200),
        Reply::Json(json!({"errcode":42001})),
        token(7200),
        Reply::Json(json!({"errcode":42001,"errmsg":format!("{SECRET} {TOKEN}")})),
    ])
    .await;
    let error = sync(&adapter(&mock, Store::memory())).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<ProviderError>().unwrap().code,
        Some(42001)
    );
    assert_redacted(&format!("{error:#}"));
    assert_eq!(mock.paths("sync_msg").len(), 2);
}

#[tokio::test]
async fn concurrent_failures_of_old_token_refresh_only_once() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let calls = tokens.clone();
    let mock = Mock::new(move |r| {
        if r.path.contains("gettoken") {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            Reply::Json(
                json!({"errcode":0,"access_token":if n==0 {"old"} else {"new"},"expires_in":7200}),
            )
        } else if r.path.contains("access_token=old") {
            Reply::Json(json!({"errcode":40014}))
        } else {
            Reply::Json(empty_page("end"))
        }
    })
    .await;
    let a = adapter(&mock, Store::memory());
    a.access_token(None).await.unwrap();
    let (one, two, three) = tokio::join!(sync(&a), sync(&a), sync(&a));
    one.unwrap();
    two.unwrap();
    three.unwrap();
    assert_eq!(tokens.load(Ordering::SeqCst), 2);
}

#[test]
fn pure_window_and_msgid_rules() {
    assert!(!within_reply_window(None, 200000));
    assert!(within_reply_window(Some(200000 - 172799), 200000));
    assert!(!within_reply_window(Some(200000 - 172800), 200000));
    assert!(!within_reply_window(Some(200001), 200000));
    assert_eq!(platform_msgid("hello_-12"), "hello_-12");
    for key in [
        "",
        "中文",
        "more-than-32-bytes-for-a-platform-message-id",
        "bad.value",
    ] {
        let id = platform_msgid(key);
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, platform_msgid(key));
    }
    assert_ne!(platform_msgid("中文"), platform_msgid("另一个"));
}

#[tokio::test]
async fn origins_deduplicate_and_only_customer_messages_enter_inbox() {
    let store = Store::memory();
    let mock = Mock::scripted(vec![]).await;
    let a = adapter(&mock, store.clone());
    let mut raw = vec![
        message("customer", 3, now()),
        message("staff", 5, now()),
        message("unknown", 9, now()),
    ];
    raw.push(json!({"msgid":"event","send_time":now(),"origin":4,"msgtype":"event","event":{"event_type":"msg_send_fail","open_kfid":"kf","external_userid":"user","fail_type":6}}));
    raw.push(json!({"msgid":"image","open_kfid":"kf","external_userid":"user","send_time":now(),"origin":3,"msgtype":"image","image":{"media_id":"media"}}));
    for expected in [5, 0] {
        let messages = raw
            .iter()
            .map(|r| normalize_message(r, "kf").unwrap())
            .collect();
        assert_eq!(
            store
                .commit_provider_page(
                    a.account.key.clone(),
                    SyncPage {
                        messages,
                        next_cursor: "next".into(),
                        has_more: false
                    }
                )
                .await
                .unwrap(),
            expected
        );
    }
    let key = a.account.key.clone();
    let rows = store
        .run(move |conn| {
            query(
                conn,
                EVENT_SELECT,
                rusqlite::params![key.platform, key.account_id],
            )
        })
        .await
        .unwrap();
    for row in &rows {
        let payload = &row["payload"];
        assert_eq!(row["kind"], "message");
        assert_eq!(row["source"], "wecom_kf_api");
        if payload["direction"] == "inbound" {
            assert_eq!(row["state"], "pending");
        } else {
            assert_eq!(row["state"], "recorded");
        }
        assert_eq!(row["verified"], payload["origin"] != 9);
    }
    assert_eq!(
        rows.iter()
            .find(|r| r["payload"]["msgid"] == "image")
            .unwrap()["payload"]["media_id"],
        "media"
    );
    assert_eq!(
        rows.iter()
            .find(|r| r["payload"]["msgid"] == "staff")
            .unwrap()["payload"]["direction"],
        "outbound"
    );
    assert_eq!(
        rows.iter()
            .find(|r| r["payload"]["msgid"] == "event")
            .unwrap()["payload"]["event"]["fail_type"],
        6
    );
    let mut wrong = message("bad", 3, now());
    wrong["open_kfid"] = json!("other");
    assert!(normalize_message(&wrong, "kf").is_err());
}

struct FileDb {
    path: PathBuf,
}
impl FileDb {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("wecom-tests-{}.sqlite3", new_id())),
        }
    }
}
impl Drop for FileDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

async fn wait_cursor(store: &Arc<Store>, key: &AccountKey, cursor: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store.provider_cursor(key.clone()).await.unwrap().as_deref() == Some(cursor) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn listener_syncs_empty_and_nonempty_pages_and_restores_durable_cursor() {
    let file = FileDb::new();
    let store = Store::open(&file.path).unwrap();
    let mock = Mock::scripted(vec![token(7200),Reply::Json(json!({"errcode":0,"next_cursor":"p1","has_more":1,"msg_list":[]})),Reply::Json(json!({"errcode":0,"next_cursor":"p2","has_more":1,"msg_list":[message("one",3,now())]})),Reply::Json(json!({"errcode":0,"next_cursor":"p3","has_more":0,"msg_list":[message("one",3,now()),message("two",5,now())]}))]).await;
    let a = adapter(&mock, store.clone());
    let key = a.account.key.clone();
    let g = gateway(a, store.clone());
    g.listeners
        .start(
            g.accounts.resolve("wechat_kf", Some("a")).unwrap(),
            store.clone(),
            2,
        )
        .await
        .unwrap();
    wait_cursor(&store, &key, "p3").await;
    let requests = mock.paths("sync_msg");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].body.get("cursor").is_none());
    assert_eq!(requests[1].body["cursor"], "p1");
    assert_eq!(requests[2].body["cursor"], "p2");
    assert!(
        requests
            .iter()
            .all(|r| r.body["open_kfid"] == "kf" && r.body.get("token").is_none())
    );
    let claimed = g
        .call(
            "claim_events",
            json!({"platform":"wechat_kf","account_id":"a","consumer":"agent"}),
        )
        .await;
    assert_eq!(
        claimed["structuredContent"]["data"]["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let metrics = g
        .call(
            "get_service_metrics",
            json!({"platform":"wechat_kf","account_id":"a"}),
        )
        .await;
    assert_eq!(
        metrics["structuredContent"]["data"]["events"]["verified_incoming"],
        1
    );
    g.listeners.shutdown().await;
    drop(g);
    drop(store);
    let restored = Store::open(&file.path).unwrap();
    let second = Mock::scripted(vec![token(7200), Reply::Json(empty_page("p4"))]).await;
    let a = adapter(&second, restored.clone());
    let g = gateway(a, restored.clone());
    g.listeners
        .restore(&g.accounts, restored.clone())
        .await
        .unwrap();
    wait_cursor(&restored, &key, "p4").await;
    assert_eq!(second.paths("sync_msg")[0].body["cursor"], "p3");
    g.listeners.stop(&key, restored.clone()).await.unwrap();
}

#[tokio::test]
async fn page_failure_rolls_back_both_events_and_cursor() {
    let store = Store::memory();
    let key = AccountKey {
        platform: "wechat_kf".into(),
        account_id: "a".into(),
    };
    inbound(&store, key.clone(), now()).await;
    let good = normalize_message(&message("good", 3, now()), "kf").unwrap();
    let mut bad = good.clone();
    bad.msgid = "mismatch".into();
    assert!(
        store
            .commit_provider_page(
                key.clone(),
                SyncPage {
                    messages: vec![good, bad],
                    next_cursor: "lost".into(),
                    has_more: false
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        store.provider_cursor(key.clone()).await.unwrap().as_deref(),
        Some("initial")
    );
    assert_eq!(
        store
            .run(|conn| Ok(
                conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?
            ))
            .await
            .unwrap(),
        1
    );
    let other = AccountKey {
        account_id: "b".into(),
        ..key.clone()
    };
    assert!(
        store
            .provider_cursor(other.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .latest_provider_inbound(other, "user".into())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn rate_limit_worker_reports_exponential_backoff_and_recovers() {
    let mock = Mock::scripted(vec![
        token(7200),
        Reply::Json(json!({"errcode":45009,"errmsg":format!("{SECRET} {TOKEN}")})),
        Reply::Json(json!({"errcode":45011})),
        Reply::Json(empty_page("ok")),
    ])
    .await;
    let store = Store::memory();
    let a = adapter(&mock, store.clone());
    let key = a.account.key.clone();
    let listeners = Listeners::default();
    listeners.start(a, store.clone(), 2).await.unwrap();
    for expected in [4, 8] {
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let health = listeners.status(&key).unwrap()["data"]["health"].clone();
                if health["backoff_seconds"] == expected {
                    assert_eq!(health["state"], "degraded");
                    assert_redacted(&health.to_string());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if expected == 4 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(mock.paths("sync_msg").len(), 1);
        }
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store.provider_cursor(key.clone()).await.unwrap().as_deref() == Some("ok") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let health = listeners.status(&key).unwrap()["data"]["health"].clone();
    assert_eq!(health["state"], "running");
    assert_eq!(health["backoff_seconds"], 0);
    assert!(health["last_error"].is_null());
    listeners.stop(&key, store).await.unwrap();
    assert_eq!(crate::listeners::sync_backoff(2, 1), 4);
    assert_eq!(crate::listeners::sync_backoff(2, 2), 8);
    assert_eq!(crate::listeners::sync_backoff(2, u32::MAX), 60);
    assert_eq!(crate::listeners::sync_backoff(60, 1), 60);
}

#[tokio::test]
async fn accepted_send_maps_outbox_send_id_and_idempotent_replay() {
    let mock = Mock::new(|r| {
        if r.path.contains("gettoken") {
            token(7200)
        } else {
            Reply::Json(json!({"errcode":0,"msgid":r.body["msgid"]}))
        }
    })
    .await;
    let store = Store::memory();
    let a = adapter(&mock, store.clone());
    inbound(&store, a.account.key.clone(), now()).await;
    let g = gateway(a, store);
    let response = g.call("send_message", args("reply")).await;
    assert_eq!(response["isError"], false);
    let receipt = &response["structuredContent"]["data"];
    assert_eq!(receipt["status"], "triggered");
    assert_eq!(receipt["result"]["data"]["status"], "accepted_by_platform");
    assert_eq!(receipt["delivery"], "unknown");
    let requests = mock.paths("send_msg");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body["msgid"],
        platform_msgid(receipt["send_id"].as_str().unwrap())
    );
    assert_eq!(requests[0].body["touser"], "user");
    assert_eq!(requests[0].body["open_kfid"], "kf");
    assert_eq!(
        receipt["result"]["data"]["platform_msgid"],
        requests[0].body["msgid"]
    );
    assert_redacted(&response.to_string());
    let replay = g.call("send_message", args("reply")).await;
    assert_eq!(replay["structuredContent"]["data"]["replayed"], true);
    assert_eq!(mock.paths("send_msg").len(), 1);
}

#[tokio::test]
async fn send_passes_legal_msgid_unchanged_and_token_retry_uses_same_body() {
    let mock = Mock::scripted(vec![
        token(7200),
        Reply::Json(json!({"errcode":42001})),
        token(7200),
        Reply::Json(json!({"errcode":0,"msgid":"legal_id"})),
    ])
    .await;
    let store = Store::memory();
    let a = adapter(&mock, store.clone());
    inbound(&store, a.account.key.clone(), now()).await;
    a.execute(Operation::SendMessage {
        recipient: "user".into(),
        message: "hi".into(),
        idempotency: Some("legal_id".into()),
    })
    .await
    .unwrap();
    let requests = mock.paths("send_msg");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["msgid"], "legal_id");
    assert_eq!(requests[0].body, requests[1].body);
}

#[tokio::test]
async fn missing_expired_or_other_account_window_is_rejected_without_http() {
    for latest in [None, Some(now() - 172801)] {
        let mock = Mock::scripted(vec![]).await;
        let store = Store::memory();
        let a = adapter(&mock, store.clone());
        if let Some(time) = latest {
            inbound(&store, a.account.key.clone(), time).await;
        }
        inbound(
            &store,
            AccountKey {
                platform: "wechat_kf".into(),
                account_id: "other".into(),
            },
            now(),
        )
        .await;
        let g = gateway(a, store);
        assert_eq!(
            g.call("send_message", args("blocked")).await["isError"],
            true
        );
        let receipt = status(&g, "blocked").await;
        assert_eq!(receipt["status"], "rejected");
        assert_eq!(receipt["delivery"], "not_sent");
        assert_eq!(
            g.call("send_message", args("blocked")).await["structuredContent"]["data"]["status"],
            "rejected"
        );
        assert!(mock.requests.lock().unwrap().is_empty());
        let metrics = g
            .call(
                "get_service_metrics",
                json!({"platform":"wechat_kf","account_id":"a"}),
            )
            .await;
        assert_eq!(
            metrics["structuredContent"]["data"]["send_attempts"]["rejected"],
            1
        );
    }
}

#[tokio::test]
async fn explicit_platform_rejections_are_rejected_but_duplicate_and_busy_are_unknown() {
    for (code, expected) in [
        (95001, "rejected"),
        (95002, "rejected"),
        (95018, "rejected"),
        (95031, "rejected"),
        (95033, "unknown"),
        (-1, "unknown"),
    ] {
        let mock = Mock::scripted(vec![
            token(7200),
            Reply::Json(json!({"errcode":code,"errmsg":format!("{SECRET} {TOKEN}")})),
        ])
        .await;
        let store = Store::memory();
        let a = adapter(&mock, store.clone());
        inbound(&store, a.account.key.clone(), now()).await;
        let g = gateway(a, store);
        let error = g.call("send_message", args("send")).await;
        assert_eq!(error["isError"], true);
        assert_redacted(&error.to_string());
        let receipt = status(&g, "send").await;
        assert_eq!(receipt["status"], expected);
        assert_redacted(&receipt.to_string());
    }
}

#[tokio::test]
async fn dropped_connection_or_malformed_response_is_unknown_and_redacted() {
    for reply in [
        Reply::Close,
        Reply::Raw("test-secret-never-print test-access-token-never-print"),
        Reply::Json(json!({"errcode":0})),
    ] {
        let mock = Mock::scripted(vec![token(7200), reply]).await;
        let store = Store::memory();
        let a = adapter(&mock, store.clone());
        inbound(&store, a.account.key.clone(), now()).await;
        let g = gateway(a, store);
        let error = g.call("send_message", args("send")).await;
        assert_eq!(error["isError"], true);
        assert_redacted(&error.to_string());
        let receipt = status(&g, "send").await;
        assert_eq!(receipt["status"], "unknown");
        assert_redacted(&receipt.to_string());
        g.call("send_message", args("send")).await;
        assert_eq!(mock.paths("send_msg").len(), 1);
    }
}

#[tokio::test]
async fn v2_migration_preserves_every_outbox_state_data_and_foreign_keys() {
    let file = FileDb::new();
    {
        let conn = rusqlite::Connection::open(&file.path).unwrap();
        conn.execute_batch(include_str!("../../migrations/001_service_core.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/002_listener_settings.sql"))
            .unwrap();
        conn.execute("INSERT INTO customers(platform,account_id,customer_id,display_name,status,tags,notes,created_at,updated_at) VALUES('wechat_kf','a','c','Existing','lead','[]','',1,2)",[]).unwrap();
        for state in ["attempting", "triggered", "unknown"] {
            conn.execute("INSERT INTO outbox VALUES(?1,'wechat_kf','a','c',?1,'user','old',?1,'{\"old\":true}',1,2)",[state]).unwrap();
        }
        conn.execute("INSERT INTO events(event_id,platform,account_id,customer_id,event_key,kind,source,verified,occurred_at,observed_at,payload,state) VALUES('e','wechat_kf','a','c','message','message','wecom_kf_api',1,1,1,'{}','leased')",[]).unwrap();
        conn.execute(
            "INSERT INTO listener_settings VALUES('wechat_kf','a',1,5,2)",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&file.path).unwrap();
    store.run(|conn| {
        assert_eq!(conn.query_row("PRAGMA user_version",[],|r| r.get::<_,i64>(0))?,3);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM outbox WHERE message='old' AND customer_id='c' AND result='{\"old\":true}' AND created_at=1 AND updated_at=2",[],|r| r.get::<_,i64>(0))?,3);
        assert!(conn.execute("DELETE FROM customers WHERE customer_id='c'",[]).is_err());
        assert!(conn.query_row("PRAGMA foreign_key_check",[],|_| Ok(())).optional()?.is_none());
        conn.execute("INSERT INTO outbox VALUES('new','wechat_kf','a','c','new','user','no','rejected',NULL,1,2)",[])?;
        assert!(conn.execute("INSERT INTO outbox VALUES('duplicate','wechat_kf','a','c','new','user','no','rejected',NULL,1,2)",[]).is_err());
        assert_eq!(conn.query_row("SELECT state FROM events WHERE event_id='e'",[],|r| r.get::<_,String>(0))?,"leased");
        Ok(())
    }).await.unwrap();
    drop(store);
    assert!(Store::open(&file.path).is_ok());
}

#[tokio::test]
async fn migration_handles_all_supported_versions_and_rejects_future_versions() {
    for version in [0, 1, 2, 3, 4] {
        let file = FileDb::new();
        {
            let conn = rusqlite::Connection::open(&file.path).unwrap();
            if version >= 1 {
                conn.execute_batch(include_str!("../../migrations/001_service_core.sql"))
                    .unwrap();
            }
            if version >= 2 {
                conn.execute_batch(include_str!("../../migrations/002_listener_settings.sql"))
                    .unwrap();
            }
            if version >= 3 {
                conn.execute_batch(include_str!("../../migrations/003_provider_sync.sql"))
                    .unwrap();
            }
            if version == 4 {
                conn.pragma_update(None, "user_version", 4).unwrap();
            }
        }
        let opened = Store::open(&file.path);
        if version == 4 {
            assert!(opened.is_err());
        } else {
            let store = opened.unwrap();
            assert_eq!(
                store
                    .run(|conn| Ok(
                        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?
                    ))
                    .await
                    .unwrap(),
                3
            );
        }
    }
}

#[test]
fn config_requires_env_secret_and_rejects_unknown_fields() {
    use crate::accounts::GatewayConfig;
    let config: GatewayConfig = serde_json::from_value(json!({"accounts":[{"platform":"wechat_kf","account_id":"a","adapter":"wecom_kf","wecom_kf":{"corp_id":"corp","open_kfid":"kf","secret_env":"WECOM_KF_MISSING_TEST_ENV_42"}}]})).unwrap();
    let error = Gateway::from_config(config, Store::memory()).err().unwrap();
    assert!(error.to_string().contains("凭据环境变量缺失"));
    for extra in [json!({"secret":"bad"}), json!({"typo":1})] {
        let mut options = json!({"corp_id":"corp","open_kfid":"kf","secret_env":"unused"});
        options
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(serde_json::from_value::<WecomKfConfig>(options).is_err());
    }
}

#[tokio::test]
async fn official_capabilities_and_unsupported_operations_are_honest() {
    let mock = Mock::scripted(vec![]).await;
    let a = adapter(&mock, Store::memory());
    let descriptor = serde_json::to_value(a.account()).unwrap();
    assert_eq!(descriptor["binding_id"], "wecom_kf:corp:kf");
    assert_eq!(descriptor["identity_verified"], true);
    let caps = &descriptor["capabilities"];
    for field in ["send_text", "background_events"] {
        assert_eq!(caps[field], true);
    }
    for field in [
        "visible_conversations",
        "visible_messages",
        "full_history",
        "delivery_receipts",
        "background_observations",
    ] {
        assert_eq!(caps[field], false);
    }
    assert_eq!(caps["source"], "wecom_kf_api");
    assert!(a.execute(Operation::ListConversations).await.is_err());
    let desktop = crate::adapters::WechatDesktop::new("local".into(), Arc::new(Mutex::new(())));
    assert!(
        desktop
            .execute(Operation::SyncEvents { cursor: None })
            .await
            .unwrap_err()
            .to_string()
            .contains("不支持")
    );
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sync_rejects_stuck_cursor_and_bad_page_without_advancing_store() {
    let mock = Mock::scripted(vec![token(7200),Reply::Json(json!({"errcode":0,"next_cursor":"same","has_more":1,"msg_list":[]})),Reply::Json(json!({"errcode":0,"next_cursor":"new","has_more":0,"msg_list":[message("good",3,now()),{"msgid":"bad"}]}))]).await;
    let store = Store::memory();
    let a = adapter(&mock, store.clone());
    assert!(
        a.execute(Operation::SyncEvents {
            cursor: Some("same".into())
        })
        .await
        .is_err()
    );
    assert!(sync(&a).await.is_err());
    assert!(
        store
            .provider_cursor(a.account.key.clone())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn send_timeout_is_unknown_and_never_retried_automatically() {
    let mock = Mock::scripted(vec![
        token(7200),
        Reply::Delay(
            Duration::from_millis(200),
            json!({"errcode":0,"msgid":"late"}),
        ),
    ])
    .await;
    let store = Store::memory();
    let mut a = adapter(&mock, store.clone());
    Arc::get_mut(&mut a).unwrap().client = reqwest::Client::builder()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    inbound(&store, a.account.key.clone(), now()).await;
    let g = gateway(a, store);
    let error = g.call("send_message", args("timeout")).await;
    assert_eq!(error["isError"], true);
    assert_redacted(&error.to_string());
    assert_eq!(status(&g, "timeout").await["status"], "unknown");
    g.call("send_message", args("timeout")).await;
    assert_eq!(mock.paths("send_msg").len(), 1);
}

#[tokio::test]
async fn short_lived_token_is_not_cached_past_its_refresh_deadline() {
    let mock = Mock::new(|r| {
        if r.path.contains("gettoken") {
            token(1)
        } else {
            Reply::Json(empty_page("end"))
        }
    })
    .await;
    let a = adapter(&mock, Store::memory());
    sync(&a).await.unwrap();
    sync(&a).await.unwrap();
    assert_eq!(mock.paths("gettoken").len(), 2);
}

#[tokio::test]
async fn invalid_text_is_definitely_rejected_before_request() {
    let mock = Mock::scripted(vec![]).await;
    let store = Store::memory();
    let a = adapter(&mock, store.clone());
    inbound(&store, a.account.key.clone(), now()).await;
    let g = gateway(a, store);
    let mut input = args("too-long");
    input["message"] = json!("字".repeat(683));
    assert_eq!(g.call("send_message", input).await["isError"], true);
    assert_eq!(status(&g, "too-long").await["status"], "rejected");
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn successful_response_cannot_leak_credentials_into_result_or_storage() {
    for credential in [SECRET, TOKEN] {
        let mock = Mock::scripted(vec![
            token(7200),
            Reply::Json(json!({"errcode":0,"msgid":credential})),
        ])
        .await;
        let store = Store::memory();
        let a = adapter(&mock, store.clone());
        inbound(&store, a.account.key.clone(), now()).await;
        let g = gateway(a, store);
        let response = g.call("send_message", args("echo")).await;
        assert_eq!(response["isError"], true);
        assert_redacted(&response.to_string());
        let receipt = status(&g, "echo").await;
        assert_eq!(receipt["status"], "unknown");
        assert_redacted(&receipt.to_string());
    }
}
