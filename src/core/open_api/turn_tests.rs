use super::tests::{make_test_brain, sample_turn};
use super::*;
use crate::core::open_api::access::BrainAccess;
use crate::core::open_api::ledger::{TurnLedger, now};

fn access() -> BrainAccess {
    BrainAccess::for_test("test-acc", &["c1"], &[("午夜咖啡", "private", "c1")])
}
fn request(id: &str, token: &str, action: &str, text: Option<&str>) -> StrictReply {
    StrictReply {
        turn_id: id.into(),
        claim_token: token.into(),
        action: action.into(),
        text: text.map(str::to_string),
    }
}
async fn queued(
    brain: &Arc<ExternalBrain>,
    id: &str,
) -> tokio::task::JoinHandle<Result<BrainReply>> {
    let b = brain.clone();
    let turn = sample_turn(id);
    let task = tokio::spawn(async move { b.handle_turn(turn).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if brain.state.lock().await.pending.contains_key(id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task
}
async fn claim(brain: &ExternalBrain) -> ClaimedTurn {
    brain
        .claim_next("session", Duration::ZERO, &access())
        .await
        .unwrap()
        .unwrap()
}
async fn setup() -> (
    Arc<ExternalBrain>,
    PathBuf,
    tokio::task::JoinHandle<Result<BrainReply>>,
    ClaimedTurn,
) {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    let task = queued(&b, "t").await;
    let c = claim(&b).await;
    (b, dir, task, c)
}
#[tokio::test]
async fn claim_requires_external_mode() {
    let (b, dir) = make_test_brain(BrainMode::Builtin, TimeoutAction::Skip, 60);
    assert!(matches!(
        b.claim_next("s", Duration::ZERO, &access()).await,
        Err(OpenApiError::BrainModeNotExternal)
    ));
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn reply_requires_claim() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    let task = queued(&b, "t").await;
    assert_eq!(
        b.reply_strict(request("t", "", "skip", None), &access())
            .await,
        Err(OpenApiError::InvalidClaimToken)
    );
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn reply_wrong_token() {
    let (b, dir, task, _) = setup().await;
    for token in ["", "wrong"] {
        assert_eq!(
            b.reply_strict(request("t", token, "skip", None), &access())
                .await,
            Err(OpenApiError::InvalidClaimToken)
        );
    }
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn reply_after_deadline() {
    let (b, dir, task, c) = setup().await;
    b.state
        .lock()
        .await
        .pending
        .get_mut("t")
        .unwrap()
        .deadline_at = now();
    assert_eq!(
        b.reply_strict(
            request("t", &c.claim.claim_token, "send", Some("有效回复")),
            &access()
        )
        .await,
        Err(OpenApiError::TurnExpired)
    );
    assert!(!task.is_finished());
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}
async fn revoked(new_access: BrainAccess) {
    let (b, dir, task, c) = setup().await;
    assert_eq!(
        b.reply_strict(
            request("t", &c.claim.claim_token, "skip", None),
            &new_access
        )
        .await,
        Err(OpenApiError::TurnRevoked)
    );
    let reply = task.await.unwrap().unwrap();
    assert_eq!(reply.metadata["action"], "skip");
    assert_eq!(reply.metadata["reason"], "revoked");
    assert_eq!(
        TurnLedger::open(&dir).unwrap().get("t").unwrap().outcome,
        "revoked"
    );
    assert!(!b.state.lock().await.pending.contains_key("t"));
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn account_switch_revokes_claim() {
    revoked(BrainAccess::for_test("other", &["c1"], &[])).await;
}
#[tokio::test]
async fn permit_lost_revokes_claim() {
    revoked(BrainAccess::for_test("test-acc", &[], &[])).await;
}
#[tokio::test]
async fn duplicate_reply_same_content() {
    let (b, dir, task, c) = setup().await;
    let req = || request("t", &c.claim.claim_token, "send", Some("相同回复"));
    assert_eq!(
        b.reply_strict(req(), &access()).await.unwrap(),
        ReplyOutcome {
            accepted: true,
            duplicate: false
        }
    );
    assert_eq!(
        b.reply_strict(req(), &access()).await.unwrap(),
        ReplyOutcome {
            accepted: true,
            duplicate: true
        }
    );
    assert_eq!(task.await.unwrap().unwrap().reply, "相同回复");
    assert!(b.state.lock().await.pending.is_empty());
    let ledger: Value =
        serde_json::from_slice(&fs::read(dir.join("open-api/turns.json")).unwrap()).unwrap();
    assert_eq!(ledger["terminal"].as_array().unwrap().len(), 1);
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn conflicting_second_reply() {
    let (b, dir, task, c) = setup().await;
    b.reply_strict(
        request("t", &c.claim.claim_token, "send", Some("第一次")),
        &access(),
    )
    .await
    .unwrap();
    for (action, text) in [("send", Some("第二次")), ("skip", None)] {
        assert_eq!(
            b.reply_strict(request("t", &c.claim.claim_token, action, text), &access())
                .await,
            Err(OpenApiError::AlreadyAnswered {
                action: "send".into()
            })
        );
    }
    task.await.unwrap().unwrap();
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn invalid_text_keeps_claim() {
    let (b, dir, task, c) = setup().await;
    for text in [None, Some("  "), Some(&"x".repeat(201)), Some("x\0")] {
        assert_eq!(
            b.reply_strict(request("t", &c.claim.claim_token, "send", text), &access())
                .await,
            Err(OpenApiError::InvalidText)
        );
    }
    b.reply_strict(
        request("t", &c.claim.claim_token, "send", Some("合法内容")),
        &access(),
    )
    .await
    .unwrap();
    assert_eq!(task.await.unwrap().unwrap().reply, "合法内容");
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn claim_lease_expiry_requeues() {
    let (b, dir, task, _) = setup().await;
    b.release_session("session").await;
    b.claim_lease.store(1, Ordering::Relaxed);
    let first = claim(&b).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        b.reply_strict(
            request("t", &first.claim.claim_token, "skip", None),
            &access()
        )
        .await,
        Err(OpenApiError::InvalidClaimToken)
    );
    let second = claim(&b).await;
    assert_ne!(first.claim.claim_token, second.claim.claim_token);
    assert_eq!(
        b.reply_strict(
            request("t", &first.claim.claim_token, "skip", None),
            &access()
        )
        .await,
        Err(OpenApiError::InvalidClaimToken)
    );
    b.reply_strict(
        request("t", &second.claim.claim_token, "skip", None),
        &access(),
    )
    .await
    .unwrap();
    task.await.unwrap().unwrap();
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn release_session_requeues_claims() {
    let (b, dir, task, c) = setup().await;
    b.release_session("different").await;
    assert!(
        b.claim_next("other", Duration::ZERO, &access())
            .await
            .unwrap()
            .is_none()
    );
    b.release_session("session").await;
    assert_eq!(b.turn_status("t").await.unwrap()["state"], "queued");
    let new = b
        .claim_next("other", Duration::ZERO, &access())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(new.claim.claim_token, c.claim.claim_token);
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn reply_racing_deadline_yields_single_outcome() {
    let mut races = Vec::new();
    for i in 0..100 {
        races.push(tokio::spawn(async move {
            let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 1);
            let task = queued(&b, "race").await;
            let c = claim(&b).await;
            let delay = if i % 2 == 0 { 990 } else { 1000 };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let response = b
                .reply_strict(
                    request("race", &c.claim.claim_token, "send", Some("竞争回复")),
                    &access(),
                )
                .await;
            let result = task.await.unwrap().unwrap();
            let ledger = TurnLedger::open(&dir).unwrap();
            let record = ledger.get("race").unwrap();
            match response {
                Ok(_) => {
                    assert_eq!(record.outcome, "accepted");
                    assert_eq!(result.reply, "竞争回复");
                }
                Err(OpenApiError::TurnExpired) => {
                    assert_eq!(record.outcome, "expired");
                    assert_eq!(result.metadata["action"], "skip");
                }
                other => panic!("unexpected race result: {other:?}"),
            }
            assert!(b.state.lock().await.pending.is_empty());
            let file: Value =
                serde_json::from_slice(&fs::read(dir.join("open-api/turns.json")).unwrap())
                    .unwrap();
            assert_eq!(file["terminal"].as_array().unwrap().len(), 1);
            fs::remove_dir_all(dir).unwrap();
        }));
    }
    for race in races {
        race.await.unwrap();
    }
}
fn restart(dir: &Path) -> Arc<ExternalBrain> {
    let cfg = BrainConfig {
        mode: BrainMode::External,
        ..BrainConfig::default()
    };
    ExternalBrain::new_with_socket(
        &cfg,
        &serde_json::from_value::<ModelConfig>(json!({"backend":"codex-exec"})).unwrap(),
        dir.to_path_buf(),
        dir.join("core.sock"),
    )
}
#[tokio::test]
async fn restart_replays_accepted_reply() {
    let (b, dir, task, c) = setup().await;
    b.reply_strict(
        request("t", &c.claim.claim_token, "send", Some("重启回复")),
        &access(),
    )
    .await
    .unwrap();
    let first = task.await.unwrap().unwrap();
    drop(b);
    let b = restart(&dir);
    let replay = tokio::time::timeout(Duration::from_millis(100), b.handle_turn(sample_turn("t")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, replay);
    assert!(b.state.lock().await.pending.is_empty());
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn restart_with_expired_record() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 0);
    b.handle_turn(sample_turn("t")).await.unwrap();
    drop(b);
    let b = restart(&dir);
    let reply = tokio::time::timeout(Duration::from_millis(100), b.handle_turn(sample_turn("t")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.metadata["action"], "skip");
    assert_eq!(reply.metadata["reason"], "expired");
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn restart_with_unclaimed_turn_requeues() {
    for claimed in [false, true] {
        let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
        let task = queued(&b, "t").await;
        let old_token = if claimed {
            claim(&b).await.claim.claim_token
        } else {
            "old".into()
        };
        task.abort();
        task.await.unwrap_err();
        drop(b);
        let b = restart(&dir);
        assert_eq!(
            b.reply_strict(request("t", &old_token, "skip", None), &access())
                .await,
            Err(OpenApiError::TurnNotFound)
        );
        let task = queued(&b, "t").await;
        assert_eq!(
            b.reply_strict(request("t", &old_token, "skip", None), &access())
                .await,
            Err(OpenApiError::InvalidClaimToken)
        );
        let c = claim(&b).await;
        assert_ne!(old_token, c.claim.claim_token);
        task.abort();
        fs::remove_dir_all(dir).unwrap();
    }
}
#[tokio::test]
async fn turn_status_reports_send_status() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    fs::create_dir_all(dir.join("sessions/contact")).unwrap();
    for (status, expected) in [
        ("triggered", "triggered"),
        ("not_sent", "not_sent"),
        ("skipped_by_rule", "not_sent"),
        ("skipped_by_brain", "not_sent"),
        ("stopped_before_send", "not_sent"),
        ("attempting", "pending"),
        ("preparing_media", "pending"),
        ("generating", "pending"),
        ("generated", "pending"),
        ("other", "unknown"),
        ("missing", "unknown"),
    ] {
        let id = status;
        let task = queued(&b, id).await;
        let c = claim(&b).await;
        assert_eq!(b.turn_status(id).await.unwrap()["state"], "claimed");
        b.reply_strict(request(id, &c.claim.claim_token, "skip", None), &access())
            .await
            .unwrap();
        task.await.unwrap().unwrap();
        if status != "missing" {
            fs::write(
                dir.join(format!("sessions/contact/send-{id}.json")),
                json!({"status":status}).to_string(),
            )
            .unwrap();
        }
        assert_eq!(b.turn_status(id).await.unwrap()["send"]["status"], expected);
    }
    assert_eq!(
        b.turn_status("absent").await,
        Err(OpenApiError::TurnNotFound)
    );
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn legacy_core_sock_reply_unchanged() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    let task = queued(&b, "t").await;
    let poll: Value = serde_json::from_str(
        &b.dispatch_request(123, r#"{"id":1,"op":"next_turn","wait_seconds":0}"#)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(poll["result"]["turn"]["turn_id"], "t");
    assert!(poll["result"]["turn"]["claim_token"].as_str().is_some());
    assert!(
        b.claim_next("strict", Duration::ZERO, &access())
            .await
            .unwrap()
            .is_none()
    );
    let response: Value = serde_json::from_str(
        &b.dispatch_request(
            124,
            r#"{"id":2,"op":"reply","turn_id":"t","action":"send","text":"旧入口回复"}"#,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        response,
        json!({"id":2,"ok":true,"result":{"accepted":true}})
    );
    assert_eq!(task.await.unwrap().unwrap().reply, "旧入口回复");
    fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn ledger_unavailable_blocks_strict_only() {
    let (original, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    drop(original);
    fs::remove_dir(dir.join("open-api")).unwrap();
    fs::write(dir.join("open-api"), "blocks directory creation").unwrap();
    let b = restart(&dir);
    let task = queued(&b, "t").await;
    assert!(
        matches!(b.claim_next("s", Duration::ZERO, &access()).await, Err(OpenApiError::Internal(s)) if s.starts_with("turn_ledger_unavailable:"))
    );
    assert!(
        matches!(b.reply_strict(request("t", "", "skip", None), &access()).await, Err(OpenApiError::Internal(s)) if s.starts_with("turn_ledger_unavailable:"))
    );
    assert!(matches!(
        b.turn_status("t").await,
        Err(OpenApiError::Internal(_))
    ));
    assert!(!task.is_finished());
    let response: Value = serde_json::from_str(
        &b.dispatch_request(1, r#"{"op":"reply","turn_id":"t","action":"skip"}"#)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["result"]["accepted"], true);
    assert_eq!(task.await.unwrap().unwrap().metadata["action"], "skip");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn ledger_write_failure_keeps_claim_and_does_not_deliver() {
    let (b, dir, task, c) = setup().await;
    fs::rename(dir.join("open-api"), dir.join("saved-ledger")).unwrap();
    fs::write(dir.join("open-api"), "not a directory").unwrap();
    let req = || request("t", &c.claim.claim_token, "send", Some("持久后才能接受"));
    assert!(matches!(
        b.reply_strict(req(), &access()).await,
        Err(OpenApiError::Internal(_))
    ));
    assert!(!task.is_finished());
    assert_eq!(b.turn_status("t").await.unwrap()["state"], "claimed");
    fs::remove_file(dir.join("open-api")).unwrap();
    fs::rename(dir.join("saved-ledger"), dir.join("open-api")).unwrap();
    assert_eq!(
        b.reply_strict(req(), &access()).await.unwrap(),
        ReplyOutcome {
            accepted: true,
            duplicate: false
        }
    );
    assert_eq!(task.await.unwrap().unwrap().reply, "持久后才能接受");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn claim_filters_account_and_ambiguous_or_forbidden_contacts() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    let task = queued(&b, "t").await;
    for access in [
        BrainAccess::for_test("other", &["c1"], &[("午夜咖啡", "private", "c1")]),
        BrainAccess::for_test("test-acc", &[], &[("午夜咖啡", "private", "c1")]),
        BrainAccess::for_test(
            "test-acc",
            &["c1", "c2"],
            &[("午夜咖啡", "private", "c1"), ("午夜咖啡", "private", "c2")],
        ),
    ] {
        assert!(
            b.claim_next("s", Duration::ZERO, &access)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(b.turn_status("t").await.unwrap()["state"], "queued");
    assert_eq!(claim(&b).await.contact_id, "c1");
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn renew_session_extends_only_its_live_claims() {
    let (b, dir, task, _) = setup().await;
    let original = b.state.lock().await.pending["t"]
        .strict_claim
        .as_ref()
        .unwrap()
        .lease_until;
    b.claim_lease.store(120, Ordering::Relaxed);
    b.renew_session("other").await;
    assert_eq!(
        b.state.lock().await.pending["t"]
            .strict_claim
            .as_ref()
            .unwrap()
            .lease_until,
        original
    );
    b.renew_session("session").await;
    assert!(
        b.state.lock().await.pending["t"]
            .strict_claim
            .as_ref()
            .unwrap()
            .lease_until
            > original
    );
    b.state
        .lock()
        .await
        .pending
        .get_mut("t")
        .unwrap()
        .strict_claim
        .as_mut()
        .unwrap()
        .lease_until = now();
    b.renew_session("session").await;
    assert_eq!(b.turn_status("t").await.unwrap()["state"], "queued");
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn long_poll_wakes_for_enqueue_and_lease_expiry() {
    let (b, dir) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
    let poll_brain = b.clone();
    let poll = tokio::spawn(async move {
        poll_brain
            .claim_next("first", Duration::from_secs(2), &access())
            .await
            .unwrap()
            .unwrap()
    });
    let task = queued(&b, "t").await;
    b.claim_lease.store(1, Ordering::Relaxed);
    let c = poll.await.unwrap();
    // The first claim may predate changing the lease length; release and claim again.
    b.release_session("first").await;
    let c2 = claim(&b).await;
    assert_ne!(c.claim.claim_token, c2.claim.claim_token);
    let next = tokio::time::timeout(
        Duration::from_secs(2),
        b.claim_next("next", Duration::from_secs(2), &access()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_ne!(c2.claim.claim_token, next.claim.claim_token);
    task.abort();
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn restart_with_revoked_record_returns_skip() {
    let (b, dir, task, c) = setup().await;
    let no_access = BrainAccess::for_test("test-acc", &[], &[]);
    assert_eq!(
        b.reply_strict(request("t", &c.claim.claim_token, "skip", None), &no_access)
            .await,
        Err(OpenApiError::TurnRevoked)
    );
    task.await.unwrap().unwrap();
    drop(b);
    let b = restart(&dir);
    let reply = tokio::time::timeout(Duration::from_millis(100), b.handle_turn(sample_turn("t")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.metadata["action"], "skip");
    assert_eq!(reply.metadata["reason"], "revoked");
    fs::remove_dir_all(dir).unwrap();
}
