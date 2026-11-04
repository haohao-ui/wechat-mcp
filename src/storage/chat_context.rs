//! Snapshot and continuity evidence commit together in the existing event journal.
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use super::{EVENT_SELECT, Store, new_id, now, one, require_customer};
use crate::{
    accounts::AccountKey,
    domain::observation::{ChatCapture, Snapshot},
    messaging::history,
};

fn parse_capture(payload: &Value, key: &AccountKey) -> Result<ChatCapture> {
    if payload["account"] != json!(key) {
        bail!("chat snapshot account mismatch");
    }
    let data = payload.get("data").context("chat snapshot data missing")?;
    let text_lines = serde_json::from_value::<Vec<String>>(
        data.get("text_lines")
            .context("chat snapshot text missing")?
            .clone(),
    )
    .context("invalid chat snapshot text")?;
    let recipient = match data.get("recipient") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.clone()),
        _ => bail!("invalid chat snapshot recipient"),
    };
    let evidence = match data.get("observation") {
        None => None,
        Some(observation) => {
            if observation["kind"] != "ocr_text_lines"
                || observation["source"] != "desktop_visual"
                || observation["incoming_verified"] != false
            {
                bail!("unsupported chat snapshot evidence");
            }
            Some(
                serde_json::from_value::<Snapshot>(observation["snapshot"].clone())
                    .context("invalid chat snapshot evidence")?,
            )
        }
    };
    let capture = ChatCapture {
        recipient,
        text_lines,
        evidence,
    };
    capture.validate()?;
    Ok(capture)
}

impl Store {
    pub async fn capture(
        self: &Arc<Self>,
        key: AccountKey,
        customer: String,
        mut payload: Value,
    ) -> Result<Value> {
        // Parse before beginning a transaction; no desktop or model work holds the DB lock.
        let current = parse_capture(&payload, &key)?;
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_customer(&tx, &key, &customer)?;
            let expected_recipient: Option<String> = tx.query_row(
                "SELECT recipient FROM customers WHERE platform=?1 AND account_id=?2 AND customer_id=?3",
                params![key.platform, key.account_id, customer], |row| row.get(0)
            )?;
            if current.recipient.is_some() && current.recipient != expected_recipient {
                bail!("chat snapshot recipient no longer matches the customer");
            }
            // Rowid orders commits even when observed_at is identical to the second.
            let previous: Option<(String, String)> = tx.query_row(
                "SELECT event_id,payload FROM events WHERE platform=?1 AND account_id=?2 AND customer_id=?3 AND kind='chat_snapshot' ORDER BY rowid DESC LIMIT 1",
                params![key.platform, key.account_id, customer], |row| Ok((row.get(0)?, row.get(1)?))
            ).optional()?;
            let old = previous.as_ref().map(|(_, text)| {
                let value: Value = serde_json::from_str(text)?;
                parse_capture(&value, &key)
            }).transpose().context("invalid persisted chat snapshot; baseline unchanged")?;
            let comparison = history::compare(old.as_ref(), &current)?;
            // Never trust a driver's proposed reconciliation or promote OCR to messages.
            payload["text_continuity"] = json!({
                "schema_version":1,"basis":"ordered_visible_text",
                "previous_event_id":previous.as_ref().map(|(id, _)| id),
                "state":comparison.state,"reason":comparison.reason,"new_lines":comparison.new_lines,
                "coverage":"visible_only","message_continuity_verified":false,"incoming_verified":false,
                "direction":"unknown","coverage_gap":matches!(comparison.state,history::State::Gap|history::State::Ambiguous),
            });
            let event_id = new_id();
            let time = now();
            tx.execute(
                "INSERT INTO events(event_id,platform,account_id,customer_id,kind,source,verified,occurred_at,observed_at,payload,state) VALUES(?1,?2,?3,?4,'chat_snapshot','desktop_ocr',0,?5,?5,?6,'recorded')",
                params![event_id, key.platform, key.account_id, customer, time, payload.to_string()]
            )?;
            let event = one(&tx, &format!("{EVENT_SELECT} AND event_id=?3"), params![key.platform, key.account_id, event_id])?;
            tx.commit()?;
            Ok(json!({"account":key,"data":event}))
        }).await
    }
}
