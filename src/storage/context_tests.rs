use super::*;

async fn fixture(store: &Arc<Store>, account: &str, customer: &str) -> AccountKey {
    let key = AccountKey {
        platform: "wechat".into(),
        account_id: account.into(),
    };
    let scoped = key.clone();
    let customer = customer.to_owned();
    store.run(move |conn| {
        conn.execute(
            "INSERT INTO customers(platform,account_id,customer_id,display_name,recipient,status,tags,notes,created_at,updated_at) VALUES(?1,?2,?3,'Client','Client','active','[]','',0,0)",
            params![scoped.platform, scoped.account_id, customer]
        )?;
        Ok(())
    }).await.unwrap();
    key
}

fn capture(key: &AccountKey, epoch: &str, sequence: u64, texts: &[&str]) -> Value {
    let lines: Vec<Value> = texts
        .iter()
        .map(|text| {
            json!({
                "text":text,"confidence":0.9,"bounds":{"x":0.1,"y":0.1,"width":0.5,"height":0.05}
            })
        })
        .collect();
    json!({"account":key,"identity_verified":false,"data":{
        "recipient":"Client","text_lines":texts,"coverage":"visible_only","source":"desktop_ocr",
        "direction":"unknown","observation":{"kind":"ocr_text_lines","source":"desktop_visual",
            "incoming_verified":false,"snapshot":{
                "epoch":epoch,"sequence":sequence,"captured_at_ms":1000+sequence,
                "window":{"window_id":42,"pid":20,"x":0,"y":0,"width":900,"height":700,"on_screen":true},
                "region":{"x":0.3,"y":0.1,"width":0.7,"height":0.7},
                "digest":format!("{sequence:064x}"),"ocr_cached":false,"lines":lines,"title_lines":[]
            }
        }
    }})
}

fn continuity(event: &Value) -> &Value {
    &event["data"]["payload"]["text_continuity"]
}

#[tokio::test]
async fn capture_resumes_from_durable_event_and_keeps_repeated_occurrences() {
    let directory = std::env::temp_dir().join(format!("wechat-context-{}", new_id()));
    let path = directory.join("context.sqlite3");
    let store = Store::open(&path).unwrap();
    let key = fixture(&store, "a", "customer").await;
    let first = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 1, &["前文", "收到"]),
        )
        .await
        .unwrap();
    assert_eq!(continuity(&first)["state"], "baseline");
    drop(store);
    let store = Store::open(&path).unwrap();
    let second = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 2, &["前文", "收到", "收到", "收到"]),
        )
        .await
        .unwrap();
    let result = continuity(&second);
    assert_eq!(result["previous_event_id"], first["data"]["event_id"]);
    assert_eq!(result["state"], "anchored");
    assert_eq!(result["new_lines"], json!(["收到", "收到"]));
    assert_eq!(result["message_continuity_verified"], false);
    assert_eq!(result["incoming_verified"], false);
    assert_eq!(second["data"]["state"], "recorded");
    assert_eq!(second["data"]["verified"], false);
    drop(store);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn capture_records_uncertain_alignment_without_inventing_new_lines() {
    for (before, after, reason, state) in [
        (
            vec!["前文", "收到", "收到"],
            vec!["收到", "新文"],
            "repeated_anchor",
            "ambiguous",
        ),
        (vec!["甲", "乙"], vec!["丙", "丁"], "anchor_missing", "gap"),
        (vec!["甲"], vec![], "empty_viewport", "gap"),
    ] {
        let store = Store::memory();
        let key = fixture(&store, "a", "customer").await;
        store
            .capture(
                key.clone(),
                "customer".into(),
                capture(&key, "epoch", 1, &before),
            )
            .await
            .unwrap();
        let event = store
            .capture(
                key.clone(),
                "customer".into(),
                capture(&key, "epoch", 2, &after),
            )
            .await
            .unwrap();
        let result = continuity(&event);
        assert_eq!(result["state"], state);
        assert_eq!(result["reason"], reason);
        assert_eq!(result["coverage_gap"], true);
        assert_eq!(result["new_lines"], json!([]));
    }
}

#[tokio::test]
async fn capture_epoch_window_region_and_driver_changes_reset_continuity() {
    for reason in [
        "epoch_changed",
        "window_changed",
        "region_changed",
        "evidence_unavailable",
    ] {
        let store = Store::memory();
        let key = fixture(&store, "a", "customer").await;
        store
            .capture(
                key.clone(),
                "customer".into(),
                capture(&key, "epoch", 1, &["甲"]),
            )
            .await
            .unwrap();
        let mut next = capture(&key, "epoch", 2, &["甲", "乙"]);
        let snapshot = &mut next["data"]["observation"]["snapshot"];
        match reason {
            "epoch_changed" => snapshot["epoch"] = json!("replacement"),
            "window_changed" => snapshot["window"]["window_id"] = json!(43),
            "region_changed" => snapshot["region"]["x"] = json!(0.2),
            "evidence_unavailable" => {
                next["data"].as_object_mut().unwrap().remove("observation");
            }
            _ => unreachable!(),
        }
        let event = store.capture(key, "customer".into(), next).await.unwrap();
        assert_eq!(continuity(&event)["reason"], reason);
        assert_eq!(continuity(&event)["new_lines"], json!([]));
    }
}

#[tokio::test]
async fn invalid_stale_or_conflicting_capture_never_replaces_the_durable_baseline() {
    let store = Store::memory();
    let key = fixture(&store, "a", "customer").await;
    let first = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 2, &["甲"]),
        )
        .await
        .unwrap();
    for failure in [
        "account",
        "recipient",
        "text",
        "bounds",
        "sequence",
        "timestamp",
        "conflict",
        "confidence",
    ] {
        let mut next = capture(&key, "epoch", 3, &["甲", "乙"]);
        match failure {
            "account" => next["account"]["account_id"] = json!("b"),
            "recipient" => next["data"]["recipient"] = json!("Other"),
            "text" => next["data"]["text_lines"] = json!(["forged"]),
            "bounds" => {
                next["data"]["observation"]["snapshot"]["lines"][0]["bounds"]["width"] = json!(2)
            }
            "sequence" => next["data"]["observation"]["snapshot"]["sequence"] = json!(1),
            "timestamp" => next["data"]["observation"]["snapshot"]["captured_at_ms"] = json!(1),
            "conflict" => next["data"]["observation"]["snapshot"]["sequence"] = json!(2),
            "confidence" => {
                next["data"]["observation"]["snapshot"]["lines"][0]["confidence"] = json!(1.1)
            }
            _ => unreachable!(),
        }
        assert!(
            store
                .capture(key.clone(), "customer".into(), next)
                .await
                .is_err(),
            "{failure}"
        );
    }
    let valid = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 4, &["甲", "乙"]),
        )
        .await
        .unwrap();
    assert_eq!(
        continuity(&valid)["previous_event_id"],
        first["data"]["event_id"]
    );
    assert_eq!(continuity(&valid)["new_lines"], json!(["乙"]));
    let count = store
        .run(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM events", [], |row| {
                row.get::<_, i64>(0)
            })?)
        })
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn capture_baselines_are_scoped_to_customer_and_account() {
    let store = Store::memory();
    let a = fixture(&store, "a", "customer").await;
    let b = fixture(&store, "b", "customer").await;
    fixture(&store, "a", "second").await;
    store
        .capture(
            a.clone(),
            "customer".into(),
            capture(&a, "epoch", 1, &["甲"]),
        )
        .await
        .unwrap();
    for (key, customer) in [(b, "customer"), (a, "second")] {
        let event = store
            .capture(
                key.clone(),
                customer.into(),
                capture(&key, "epoch", 2, &["甲", "乙"]),
            )
            .await
            .unwrap();
        assert_eq!(continuity(&event)["state"], "baseline");
        assert!(continuity(&event)["previous_event_id"].is_null());
    }
}

#[tokio::test]
async fn failed_snapshot_transaction_preserves_previous_event_reference() {
    let store = Store::memory();
    let key = fixture(&store, "a", "customer").await;
    let first = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 1, &["甲"]),
        )
        .await
        .unwrap();
    store.run(|conn| { conn.execute_batch(
        "CREATE TRIGGER reject_capture BEFORE INSERT ON events WHEN json_extract(NEW.payload,'$.data.text_lines[1]')='reject' BEGIN SELECT RAISE(ABORT,'simulated write failure'); END;"
    )?; Ok(()) }).await.unwrap();
    assert!(
        store
            .capture(
                key.clone(),
                "customer".into(),
                capture(&key, "epoch", 2, &["甲", "reject"])
            )
            .await
            .is_err()
    );
    let valid = store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 3, &["甲", "乙"]),
        )
        .await
        .unwrap();
    assert_eq!(
        continuity(&valid)["previous_event_id"],
        first["data"]["event_id"]
    );
}

#[tokio::test]
async fn cached_frame_comparison_ignores_driver_supplied_continuity() {
    let store = Store::memory();
    let key = fixture(&store, "a", "customer").await;
    let frame = capture(&key, "epoch", 1, &["甲", "收到", "收到"]);
    let first = store
        .capture(key.clone(), "customer".into(), frame.clone())
        .await
        .unwrap();
    let mut forged = frame;
    forged["text_continuity"] =
        json!({"state":"anchored","incoming_verified":true,"new_lines":["forged"]});
    let next = store.capture(key, "customer".into(), forged).await.unwrap();
    assert_eq!(
        continuity(&next)["previous_event_id"],
        first["data"]["event_id"]
    );
    assert_eq!(continuity(&next)["state"], "unchanged");
    assert_eq!(continuity(&next)["new_lines"], json!([]));
    assert_eq!(continuity(&next)["incoming_verified"], false);
}

#[tokio::test]
async fn customer_recipient_change_starts_a_gap_instead_of_reusing_old_text() {
    let store = Store::memory();
    let key = fixture(&store, "a", "customer").await;
    store
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 1, &["甲"]),
        )
        .await
        .unwrap();
    store
        .run(|conn| {
            conn.execute(
                "UPDATE customers SET recipient='Other' WHERE account_id='a'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let mut next = capture(&key, "epoch", 2, &["甲", "乙"]);
    next["data"]["recipient"] = json!("Other");
    let event = store.capture(key, "customer".into(), next).await.unwrap();
    assert_eq!(continuity(&event)["reason"], "recipient_changed");
    assert_eq!(continuity(&event)["new_lines"], json!([]));
}

#[tokio::test]
async fn independent_connections_serialize_capture_baselines_without_forks() {
    let directory = std::env::temp_dir().join(format!("wechat-context-concurrent-{}", new_id()));
    let path = directory.join("context.sqlite3");
    let a = Store::open(&path).unwrap();
    let key = fixture(&a, "a", "customer").await;
    let first = a
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 1, &["甲"]),
        )
        .await
        .unwrap();
    let b = Store::open(&path).unwrap();
    let (second, third) = tokio::join!(
        a.capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 2, &["甲", "乙"])
        ),
        b.capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 3, &["甲", "乙", "丙"])
        )
    );
    let third = third.unwrap();
    match second {
        Ok(second) => {
            assert_eq!(
                continuity(&second)["previous_event_id"],
                first["data"]["event_id"]
            );
            assert_eq!(
                continuity(&third)["previous_event_id"],
                second["data"]["event_id"]
            );
        }
        Err(error) => {
            assert!(error.to_string().contains("stale chat capture"));
            assert_eq!(
                continuity(&third)["previous_event_id"],
                first["data"]["event_id"]
            );
        }
    }
    let next = a
        .capture(
            key.clone(),
            "customer".into(),
            capture(&key, "epoch", 4, &["甲", "乙", "丙", "丁"]),
        )
        .await
        .unwrap();
    assert_eq!(
        continuity(&next)["previous_event_id"],
        third["data"]["event_id"]
    );
    assert_eq!(continuity(&next)["new_lines"], json!(["丁"]));
    drop(a);
    drop(b);
    std::fs::remove_dir_all(directory).unwrap();
}
