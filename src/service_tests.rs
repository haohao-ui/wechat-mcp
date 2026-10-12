use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use serde_json::{Value, json};

use crate::{
    accounts::{AccountDescriptor, AccountKey, AccountRegistry, Capabilities},
    adapters::{AdapterFuture, MessageAdapter, Operation},
    gateway::Gateway,
    listeners::Listeners,
    storage::{Store, new_id, now},
};

struct TestAdapter {
    account: AccountDescriptor,
    sends: AtomicUsize,
    fail: AtomicBool,
    snapshot: Mutex<Value>,
}
impl TestAdapter {
    fn new(id: &str) -> Arc<Self> {
        Arc::new(Self {
            account: AccountDescriptor {
                key: AccountKey {
                    platform: "wechat".into(),
                    account_id: id.into(),
                },
                adapter: "test".into(),
                binding_id: format!("test:{id}"),
                identity_verified: false,
                capabilities: Capabilities {
                    send_text: true,
                    visible_conversations: true,
                    visible_messages: true,
                    full_history: false,
                    delivery_receipts: false,
                    background_events: false,
                    background_observations: true,
                    source: "desktop_ocr".into(),
                },
            },
            sends: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            snapshot: Mutex::new(json!([])),
        })
    }
}
impl MessageAdapter for TestAdapter {
    fn account(&self) -> &AccountDescriptor {
        &self.account
    }
    fn execute(&self, operation: Operation) -> AdapterFuture<'_> {
        Box::pin(async move {
            let data = match operation {
                Operation::SendMessage { .. } => {
                    self.sends.fetch_add(1, Ordering::SeqCst);
                    if self.fail.load(Ordering::SeqCst) {
                        anyhow::bail!("simulated uncertain send");
                    }
                    json!({"status":"triggered","delivery":"unknown"})
                }
                Operation::ListConversations => {
                    if self.fail.load(Ordering::SeqCst) {
                        anyhow::bail!("simulated screenshot failure");
                    }
                    json!({"conversations":self.snapshot.lock().unwrap().clone()})
                }
                Operation::ReadMessages { .. } => {
                    json!({"text_lines":["simulated visible text"],"coverage":"visible_only","source":"desktop_ocr"})
                }
                _ => json!({}),
            };
            Ok(json!({"account":self.account.key,"data":data}))
        })
    }
}
fn gateway(store: Arc<Store>) -> (Gateway, Arc<TestAdapter>, Arc<TestAdapter>) {
    let a = TestAdapter::new("a");
    let b = TestAdapter::new("b");
    let mut accounts = AccountRegistry::default();
    accounts.register(a.clone()).unwrap();
    accounts.register(b.clone()).unwrap();
    (
        Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
        },
        a,
        b,
    )
}
fn args(id: &str, mut args: Value) -> Value {
    args["platform"] = json!("wechat");
    args["account_id"] = json!(id);
    args
}
async fn call(g: &Gateway, id: &str, name: &str, input: Value) -> Value {
    let result = g.call(name, args(id, input)).await;
    assert_eq!(result["isError"], false, "{name}: {result}");
    result["structuredContent"]["data"].clone()
}
async fn customer(g: &Gateway, id: &str) -> String {
    call(g,id,"create_customer",json!({"idempotency_key":"customer-1","display_name":"Client","recipient":"Client","external_id":"external-1"})).await["customer_id"].as_str().unwrap().into()
}
async fn event(g: &Gateway, id: &str, customer: &str, key: &str) -> Value {
    call(g,id,"record_interaction",json!({"customer_id":customer,"idempotency_key":key,"text":"manual contact note","direction":"inbound","enqueue":true})).await
}
struct FileDb {
    directory: PathBuf,
}
impl FileDb {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("service-db-test-{}", new_id()));
        std::fs::create_dir(&directory).unwrap();
        Self { directory }
    }
    fn path(&self) -> PathBuf {
        self.directory.join("service.sqlite3")
    }
}
impl Drop for FileDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
async fn customers_are_scoped_and_updates_detect_lost_writes() {
    let (g, _, _) = gateway(Store::memory());
    let a = customer(&g, "a").await;
    let b = customer(&g, "b").await;
    assert_ne!(a, b);
    assert_eq!(customer(&g, "a").await, a);
    assert_eq!(
        g.call("get_customer", args("b", json!({"customer_id":a})))
            .await["isError"],
        true
    );
    let result = call(
        &g,
        "a",
        "update_customer",
        json!({"customer_id":a,"expected_version":1,"tags":["priority"],"status":"active"}),
    )
    .await;
    assert_eq!(result["version"], 2);
    assert_eq!(result["status"], "active");
    assert_eq!(
        g.call(
            "update_customer",
            args(
                "a",
                json!({"customer_id":a,"expected_version":1,"notes":"stale write"})
            )
        )
        .await["isError"],
        true
    );
    let current = call(&g, "a", "get_customer", json!({"customer_id":a})).await;
    assert_eq!(current["customer"]["tags"], json!(["priority"]));
    assert_eq!(current["customer"]["notes"], "");
    assert_eq!(
        g.call(
            "create_customer",
            args(
                "a",
                json!({"idempotency_key":"customer-1","display_name":"different"})
            )
        )
        .await["isError"],
        true
    );
}

#[tokio::test]
async fn inbox_claims_are_exclusive_and_stale_tokens_cannot_acknowledge() {
    let (g, _, _) = gateway(Store::memory());
    let customer = customer(&g, "a").await;
    let first = event(&g, "a", &customer, "message-1").await;
    assert_eq!(
        event(&g, "a", &customer, "message-1").await["event_id"],
        first["event_id"]
    );
    let (a, b) = tokio::join!(
        call(&g, "a", "claim_events", json!({"consumer":"agent-a"})),
        call(&g, "a", "claim_events", json!({"consumer":"agent-b"}))
    );
    let claimed = if a["events"].as_array().unwrap().is_empty() {
        b["events"][0].clone()
    } else {
        a["events"][0].clone()
    };
    assert_eq!(
        a["events"].as_array().unwrap().len() + b["events"].as_array().unwrap().len(),
        1
    );
    let timeline = call(&g, "a", "get_customer", json!({"customer_id":customer})).await;
    assert!(timeline["events"][0].get("lease_token").is_none());
    let id = claimed["event_id"].as_str().unwrap().to_owned();
    let expire = id.clone();
    g.store
        .run(move |conn| {
            conn.execute(
                "UPDATE events SET lease_until=?2 WHERE event_id=?1",
                rusqlite::params![expire, now() - 1],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let renewed =
        call(&g, "a", "claim_events", json!({"consumer":"retry"})).await["events"][0].clone();
    let stale =
        json!({"event_id":id,"consumer":claimed["consumer"],"lease_token":claimed["lease_token"]});
    assert_eq!(g.call("ack_event", args("a", stale)).await["isError"], true);
    let ack = json!({"event_id":id,"consumer":"retry","lease_token":renewed["lease_token"]});
    call(&g, "a", "ack_event", ack.clone()).await;
    call(&g, "a", "ack_event", ack).await;
    assert_eq!(
        call(&g, "a", "wait_for_events", json!({"timeout_seconds":0})).await["status"],
        "timeout"
    );
}

#[tokio::test]
async fn followups_and_analysis_are_idempotent_and_require_customer_evidence() {
    let (g, _, _) = gateway(Store::memory());
    let a = customer(&g, "a").await;
    let b = customer(&g, "b").await;
    let local = event(&g, "a", &a, "m").await;
    let foreign = event(&g, "b", &b, "m").await;
    let input = json!({"idempotency_key":"followup-1","customer_id":a,"title":"check shipment","due_at":now()-1,"owner":"service-agent"});
    let task = call(&g, "a", "create_followup", input.clone()).await;
    assert_eq!(
        call(&g, "a", "create_followup", input).await["task_id"],
        task["task_id"]
    );
    assert_eq!(
        call(&g, "a", "list_followups", json!({"due_before":now()})).await["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let complete = json!({"task_id":task["task_id"],"outcome":"customer confirmed"});
    call(&g, "a", "complete_followup", complete.clone()).await;
    call(&g, "a", "complete_followup", complete).await;
    let wrong = json!({"idempotency_key":"analysis-1","customer_id":a,"summary":"may need support","evidence_event_ids":[foreign["event_id"]]});
    assert_eq!(
        g.call("record_customer_analysis", args("a", wrong)).await["isError"],
        true
    );
    let input = json!({"idempotency_key":"analysis-1","customer_id":a,"summary":"may need support","evidence_event_ids":[local["event_id"]]});
    let analysis = call(&g, "a", "record_customer_analysis", input.clone()).await;
    assert_eq!(analysis["classification"], "agent_inference");
    assert_eq!(
        call(&g, "a", "record_customer_analysis", input).await["analysis_id"],
        analysis["analysis_id"]
    );
    let metrics = call(&g, "a", "get_service_metrics", json!({})).await;
    assert_eq!(metrics["events"]["agent_records"], 1);
    assert_eq!(metrics["events"]["verified_incoming"], 0);
    assert!(metrics["delivery_rate"].is_null());
    assert!(metrics["response_time"].is_null());
}

#[tokio::test]
async fn analysis_cannot_use_another_customer_in_the_same_account() {
    let (g, _, _) = gateway(Store::memory());
    let a = customer(&g, "a").await;
    let other = call(
        &g,
        "a",
        "create_customer",
        json!({"idempotency_key":"other","display_name":"Other"}),
    )
    .await["customer_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let foreign = event(&g, "a", &other, "other").await;
    assert_eq!(g.call("record_customer_analysis",args("a",json!({"idempotency_key":"bad","customer_id":a,"summary":"bad","evidence_event_ids":[foreign["event_id"]]}))).await["isError"],true);
}

#[tokio::test]
async fn uncertain_sends_are_persisted_and_retries_never_redispatch() {
    let (g, a, b) = gateway(Store::memory());
    let input = json!({"recipient":"Client","message":"hello","idempotency_key":"reply-1"});
    let sent = call(&g, "a", "send_message", input.clone()).await;
    assert_eq!(sent["status"], "triggered");
    assert_eq!(sent["delivery"], "unknown");
    assert_eq!(
        call(&g, "a", "send_message", input.clone()).await["replayed"],
        true
    );
    assert_eq!(a.sends.load(Ordering::SeqCst), 1);
    call(&g, "b", "send_message", input).await;
    assert_eq!(b.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        g.call(
            "send_message",
            args(
                "a",
                json!({"recipient":"Other","message":"hello","idempotency_key":"reply-1"})
            )
        )
        .await["isError"],
        true
    );
    a.fail.store(true, Ordering::SeqCst);
    let failure = json!({"recipient":"Client","message":"uncertain","idempotency_key":"reply-2"});
    assert_eq!(
        g.call("send_message", args("a", failure.clone())).await["isError"],
        true
    );
    assert_eq!(
        call(
            &g,
            "a",
            "get_send_status",
            json!({"idempotency_key":"reply-2"})
        )
        .await["status"],
        "unknown"
    );
    assert_eq!(
        call(&g, "a", "send_message", failure).await["status"],
        "unknown"
    );
    assert_eq!(a.sends.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn customer_bound_send_rejects_a_different_target_before_dispatch() {
    let (g, a, _) = gateway(Store::memory());
    let customer = customer(&g, "a").await;
    assert_eq!(g.call("send_message",args("a",json!({"recipient":"Wrong","message":"hello","idempotency_key":"bad","customer_id":customer}))).await["isError"],true);
    assert_eq!(a.sends.load(Ordering::SeqCst), 0);
    let event = call(
        &g,
        "a",
        "capture_customer_context",
        json!({"customer_id":customer}),
    )
    .await;
    assert_eq!(event["verified"], false);
    assert_eq!(event["kind"], "chat_snapshot");
}

#[tokio::test]
async fn restarting_retains_events_baselines_customers_and_uncertain_outbox() {
    let file = FileDb::new();
    let key = AccountKey {
        platform: "wechat".into(),
        account_id: "a".into(),
    };
    let store = Store::open(&file.path()).unwrap();
    let (g, _, _) = gateway(store.clone());
    let customer = customer(&g, "a").await;
    let pending = event(&g, "a", &customer, "restart").await;
    store
        .observe(
            key.clone(),
            json!([{"display_name":"Client","preview":"old","time_text":"today"}]),
        )
        .await
        .unwrap();
    store
        .begin_send(
            key.clone(),
            "crashed".into(),
            "Client".into(),
            "pending".into(),
            Some(customer.clone()),
        )
        .await
        .unwrap();
    drop(g);
    drop(store);
    let store = Store::open(&file.path()).unwrap();
    let (g, adapter, _) = gateway(store.clone());
    assert_eq!(
        call(&g, "a", "get_customer", json!({"customer_id":customer})).await["events"][0]["event_id"],
        pending["event_id"]
    );
    assert_eq!(
        store
            .observe(
                key,
                json!([{"display_name":"Client","preview":"new","time_text":"today"}])
            )
            .await
            .unwrap(),
        1
    );
    let receipt=call(&g,"a","send_message",json!({"recipient":"Client","message":"pending","idempotency_key":"crashed","customer_id":customer})).await;
    assert_eq!(receipt["status"], "attempting");
    assert_eq!(adapter.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&g, "a", "claim_events", json!({"consumer":"after-restart"})).await["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn independent_database_connections_cannot_claim_the_same_event() {
    let file = FileDb::new();
    let (first, _, _) = gateway(Store::open(&file.path()).unwrap());
    let customer = customer(&first, "a").await;
    event(&first, "a", &customer, "concurrent").await;
    let (second, _, _) = gateway(Store::open(&file.path()).unwrap());
    let (a, b) = tokio::join!(
        call(&first, "a", "claim_events", json!({"consumer":"first"})),
        call(&second, "a", "claim_events", json!({"consumer":"second"}))
    );
    assert_eq!(
        a["events"].as_array().unwrap().len() + b["events"].as_array().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn observations_do_not_reenqueue_unchanged_snapshots_or_claim_incoming_messages() {
    let (g, _, _) = gateway(Store::memory());
    let key = AccountKey {
        platform: "wechat".into(),
        account_id: "a".into(),
    };
    let snapshot = json!([{"display_name":"Client","preview":"initial","time_text":"now"}]);
    assert_eq!(
        g.store
            .observe(key.clone(), snapshot.clone())
            .await
            .unwrap(),
        0
    );
    assert_eq!(g.store.observe(key.clone(), snapshot).await.unwrap(), 0);
    let next = json!([{"display_name":"Client","preview":"changed","time_text":"now"}]);
    assert_eq!(g.store.observe(key.clone(), next.clone()).await.unwrap(), 1);
    assert_eq!(g.store.observe(key, next).await.unwrap(), 0);
    let queue = call(&g, "a", "wait_for_events", json!({"timeout_seconds":0})).await;
    assert_eq!(queue["events"][0]["verified"], false);
    assert_eq!(queue["events"][0]["payload"]["incoming_verified"], false);
}

#[tokio::test]
async fn background_observer_reports_errors_and_stops_without_desktop_access() {
    let (g, adapter, _) = gateway(Store::memory());
    adapter.fail.store(true, Ordering::SeqCst);
    let result = g
        .call("start_listener", args("a", json!({"interval_seconds":2})))
        .await;
    assert_eq!(result["isError"], false);
    for _ in 0..50 {
        let result = g.call("get_listener_status", args("a", json!({}))).await;
        if result["structuredContent"]["data"]["health"]["state"] == "degraded" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let result = g.call("get_listener_status", args("a", json!({}))).await;
    assert_eq!(
        result["structuredContent"]["data"]["health"]["state"],
        "degraded"
    );
    assert_eq!(
        g.call("start_listener", args("a", json!({"interval_seconds":2})))
            .await["isError"],
        false
    );
    g.call("stop_listener", args("a", json!({}))).await;
    assert_eq!(
        g.call("get_listener_status", args("a", json!({}))).await["structuredContent"]["data"]["health"]
            ["state"],
        "stopped"
    );
}

#[tokio::test]
async fn manual_records_cannot_forge_provider_verification_or_override_operation() {
    let (g, _, _) = gateway(Store::memory());
    let customer = customer(&g, "a").await;
    for extra in [
        json!({"verified":true}),
        json!({"source":"provider_event"}),
        json!({"operation":"claim_events"}),
    ] {
        let mut input = json!({"customer_id":customer,"idempotency_key":"forged","text":"claim"});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            g.call("record_interaction", args("a", input)).await["isError"],
            true
        );
    }
}

#[tokio::test]
async fn released_and_renewed_leases_preserve_account_ownership() {
    let (g, _, _) = gateway(Store::memory());
    let customer = customer(&g, "a").await;
    event(&g, "a", &customer, "lease").await;
    let claimed = call(
        &g,
        "a",
        "claim_events",
        json!({"consumer":"one","lease_seconds":5}),
    )
    .await["events"][0]
        .clone();
    let lease = json!({"event_id":claimed["event_id"],"consumer":"one","lease_token":claimed["lease_token"]});
    assert_eq!(
        g.call("ack_event", args("b", lease.clone())).await["isError"],
        true
    );
    let renewed=call(&g,"a","renew_event_lease",json!({"event_id":claimed["event_id"],"consumer":"one","lease_token":claimed["lease_token"],"lease_seconds":120})).await;
    assert!(renewed["lease_until"].as_i64().unwrap() > claimed["lease_until"].as_i64().unwrap());
    call(&g, "a", "release_event", lease.clone()).await;
    assert_eq!(g.call("ack_event", args("a", lease)).await["isError"], true);
    let next = call(&g, "a", "claim_events", json!({"consumer":"two"})).await;
    assert_ne!(next["events"][0]["lease_token"], claimed["lease_token"]);
}

#[tokio::test]
async fn cancelled_followups_cannot_be_completed_and_bad_snapshots_do_not_advance_baselines() {
    let (g, _, _) = gateway(Store::memory());
    let customer = customer(&g, "a").await;
    let task=call(&g,"a","create_followup",json!({"idempotency_key":"cancel","customer_id":customer,"title":"obsolete","owner":"agent","due_at":now()})).await;
    call(
        &g,
        "a",
        "cancel_followup",
        json!({"task_id":task["task_id"],"outcome":"customer no longer needs this"}),
    )
    .await;
    assert_eq!(
        g.call(
            "complete_followup",
            args("a", json!({"task_id":task["task_id"],"outcome":"wrong"}))
        )
        .await["isError"],
        true
    );
    let key = AccountKey {
        platform: "wechat".into(),
        account_id: "a".into(),
    };
    g.store
        .observe(
            key.clone(),
            json!([{"display_name":"Client","preview":"old","time_text":"now"}]),
        )
        .await
        .unwrap();
    assert!(g.store.observe(key.clone(),json!([{"display_name":"Client","preview":"bad","time_text":"now"},{"display_name":"Client","preview":"duplicate","time_text":"now"}])).await.is_err());
    assert_eq!(
        g.store
            .observe(
                key,
                json!([{"display_name":"Client","preview":"new","time_text":"now"}])
            )
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn listener_intent_and_baseline_resume_after_restart_but_explicit_stop_persists() {
    let file = FileDb::new();
    let (g, adapter, _) = gateway(Store::open(&file.path()).unwrap());
    *adapter.snapshot.lock().unwrap() =
        json!([{"display_name":"Client","preview":"old","time_text":"now"}]);
    call(&g, "a", "start_listener", json!({"interval_seconds":2})).await;
    for _ in 0..100 {
        if call(&g, "a", "get_listener_status", json!({})).await["health"]["state"] == "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    g.listeners.shutdown().await;
    assert!(
        g.store
            .listener_enabled(adapter.account.key.clone())
            .await
            .unwrap()
    );
    drop(g);
    let (restored, new_adapter, _) = gateway(Store::open(&file.path()).unwrap());
    *new_adapter.snapshot.lock().unwrap() =
        json!([{"display_name":"Client","preview":"new","time_text":"now"}]);
    restored
        .listeners
        .restore(&restored.accounts, restored.store.clone())
        .await
        .unwrap();
    let queued = call(
        &restored,
        "a",
        "wait_for_events",
        json!({"timeout_seconds":2}),
    )
    .await;
    assert_eq!(queued["events"].as_array().unwrap().len(), 1);
    call(&restored, "a", "stop_listener", json!({})).await;
    assert!(
        !restored
            .store
            .listener_enabled(new_adapter.account.key.clone())
            .await
            .unwrap()
    );
    restored
        .listeners
        .restore(&restored.accounts, restored.store.clone())
        .await
        .unwrap();
    assert_eq!(
        call(&restored, "a", "get_listener_status", json!({})).await["health"]["state"],
        "stopped"
    );
}

#[tokio::test]
async fn one_listener_owns_an_account_and_other_processes_can_request_stop() {
    let file = FileDb::new();
    let (first, _, _) = gateway(Store::open(&file.path()).unwrap());
    let (second, _, _) = gateway(Store::open(&file.path()).unwrap());
    call(&first, "a", "start_listener", json!({"interval_seconds":2})).await;
    for _ in 0..100 {
        if call(&first, "a", "get_listener_status", json!({})).await["health"]["state"] == "running"
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        second
            .call("start_listener", args("a", json!({"interval_seconds":2})))
            .await["isError"],
        true
    );
    assert_eq!(
        call(&second, "a", "get_listener_status", json!({})).await["health"]["state"],
        "owned_by_another_process"
    );
    let stopped = call(&second, "a", "stop_listener", json!({})).await;
    assert_eq!(stopped["enabled"], false);
    for _ in 0..300 {
        if call(&first, "a", "get_listener_status", json!({})).await["health"]["state"] == "stopped"
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        call(&first, "a", "get_listener_status", json!({})).await["health"]["state"],
        "stopped"
    );
    first.listeners.shutdown().await;
}

#[tokio::test]
async fn listener_migration_preserves_existing_customer_data() {
    let file = FileDb::new();
    {
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        conn.execute_batch(include_str!("../migrations/001_service_core.sql"))
            .unwrap();
        conn.execute("INSERT INTO customers(platform,account_id,customer_id,display_name,status,tags,notes,created_at,updated_at) VALUES('wechat','a','existing','Existing','lead','[]','',1,1)",[]).unwrap();
    }
    let (g, _, _) = gateway(Store::open(&file.path()).unwrap());
    assert_eq!(
        call(&g, "a", "get_customer", json!({"customer_id":"existing"})).await["customer"]["display_name"],
        "Existing"
    );
    let version = g
        .store
        .run(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?))
        .await
        .unwrap();
    assert_eq!(version, 2);
}

#[tokio::test]
async fn persistent_observer_mode_does_not_allow_ui_long_poll_to_starve_it() {
    let (g, adapter, _) = gateway(Store::memory());
    adapter.fail.store(true, Ordering::SeqCst);
    call(&g, "a", "start_listener", json!({"interval_seconds":2})).await;
    let result = g
        .call("wait_for_message", args("a", json!({"timeout_seconds":60})))
        .await;
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("wait_for_events")
    );
    call(&g, "a", "stop_listener", json!({})).await;
}
