use std::{collections::HashSet, sync::Arc};

use anyhow::{Result, bail};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    accounts::AccountKey,
    storage::{EVENT_SELECT, OUTBOX_SELECT, Store, new_id, now, one, query, require_customer},
};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    CreateCustomer {
        idempotency_key: String,
        display_name: String,
        recipient: Option<String>,
        external_id: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        notes: String,
    },
    UpdateCustomer {
        customer_id: String,
        expected_version: i64,
        display_name: Option<String>,
        recipient: Option<String>,
        status: Option<CustomerStatus>,
        tags: Option<Vec<String>>,
        notes: Option<String>,
    },
    ListCustomers {
        status: Option<CustomerStatus>,
        #[serde(default = "page_limit")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    GetCustomer {
        customer_id: String,
        #[serde(default = "page_limit")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    RecordInteraction {
        customer_id: String,
        idempotency_key: String,
        text: String,
        #[serde(default)]
        direction: Direction,
        occurred_at: Option<i64>,
        #[serde(default)]
        enqueue: bool,
    },
    LinkEventCustomer {
        event_id: String,
        customer_id: String,
    },
    ClaimEvents {
        consumer: String,
        #[serde(default = "page_limit")]
        limit: u32,
        #[serde(default = "lease_seconds")]
        lease_seconds: u32,
    },
    AckEvent {
        event_id: String,
        consumer: String,
        lease_token: String,
    },
    ReleaseEvent {
        event_id: String,
        consumer: String,
        lease_token: String,
    },
    RenewEventLease {
        event_id: String,
        consumer: String,
        lease_token: String,
        #[serde(default = "lease_seconds")]
        lease_seconds: u32,
    },
    CreateFollowup {
        idempotency_key: String,
        customer_id: String,
        title: String,
        due_at: i64,
        owner: String,
    },
    ListFollowups {
        customer_id: Option<String>,
        #[serde(default)]
        status: FollowupStatus,
        due_before: Option<i64>,
        #[serde(default = "page_limit")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    CompleteFollowup {
        task_id: String,
        outcome: String,
    },
    CancelFollowup {
        task_id: String,
        outcome: String,
    },
    RecordCustomerAnalysis {
        idempotency_key: String,
        customer_id: String,
        summary: String,
        evidence_event_ids: Vec<String>,
    },
    GetCustomerAnalyses {
        customer_id: String,
        #[serde(default = "page_limit")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    GetServiceMetrics {
        from: Option<i64>,
        to: Option<i64>,
    },
    GetSendStatus {
        idempotency_key: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerStatus {
    Lead,
    Active,
    Dormant,
    Closed,
}
impl CustomerStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Active => "active",
            Self::Dormant => "dormant",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
    #[default]
    Unknown,
}
impl Direction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowupStatus {
    #[default]
    Pending,
    Done,
    Cancelled,
}
impl FollowupStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

fn page_limit() -> u32 {
    50
}
fn lease_seconds() -> u32 {
    60
}
fn text(value: &str, field: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max {
        bail!("{field} 必须是非空文本且不超过 {max} 字节");
    }
    Ok(())
}
fn page(limit: u32, offset: u32) -> Result<()> {
    if !(1..=100).contains(&limit) || offset > 1_000_000 {
        bail!("limit 必须为 1–100，offset 不超过 1000000");
    }
    Ok(())
}
fn tags_valid(tags: &[String]) -> Result<()> {
    if tags.len() > 50 {
        bail!("标签最多 50 个");
    }
    for tag in tags {
        text(tag, "tag", 128)?;
    }
    Ok(())
}
fn timestamp(time: i64) -> Result<()> {
    if !(0..=253402300799).contains(&time) {
        bail!("时间必须是有效 UTC Unix 秒");
    }
    Ok(())
}

pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "create_customer"
            | "update_customer"
            | "list_customers"
            | "get_customer"
            | "record_interaction"
            | "link_event_customer"
            | "claim_events"
            | "ack_event"
            | "release_event"
            | "renew_event_lease"
            | "create_followup"
            | "list_followups"
            | "complete_followup"
            | "cancel_followup"
            | "record_customer_analysis"
            | "get_customer_analyses"
            | "get_service_metrics"
            | "get_send_status"
    )
}

pub async fn execute(
    store: Arc<Store>,
    key: AccountKey,
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
    let command: Command = serde_json::from_value(args)?;
    store.run(move |conn| apply(conn, &key, command)).await
}

const CUSTOMER_SELECT: &str = "SELECT json_object('customer_id',customer_id,'display_name',display_name,'recipient',recipient,'external_id',external_id,'status',status,'tags',json(tags),'notes',notes,'version',version,'created_at',created_at,'updated_at',updated_at) FROM customers WHERE platform=?1 AND account_id=?2";
const FOLLOWUP_SELECT: &str = "SELECT json_object('task_id',task_id,'customer_id',customer_id,'title',title,'due_at',due_at,'owner',owner,'status',status,'outcome',outcome,'created_at',created_at,'completed_at',completed_at) FROM followups WHERE platform=?1 AND account_id=?2";

fn prior_receipt(
    conn: &Connection,
    key: &AccountKey,
    operation: &str,
    idempotency_key: &str,
    request: &Value,
) -> Result<Option<Value>> {
    text(idempotency_key, "idempotency_key", 256)?;
    let prior=query(conn,"SELECT json_object('request',json(request),'result',json(result)) FROM operation_receipts WHERE platform=?1 AND account_id=?2 AND operation=?3 AND idempotency_key=?4",params![key.platform,key.account_id,operation,idempotency_key])?.into_iter().next();
    if let Some(prior) = prior {
        if prior["request"] != *request {
            bail!("幂等键已用于不同的操作参数");
        }
        return Ok(Some(prior["result"].clone()));
    }
    Ok(None)
}

fn save_receipt(
    conn: &Connection,
    key: &AccountKey,
    operation: &str,
    idempotency_key: &str,
    request: &Value,
    result: &Value,
) -> Result<()> {
    conn.execute("INSERT INTO operation_receipts(platform,account_id,operation,idempotency_key,request,result,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![key.platform,key.account_id,operation,idempotency_key,request.to_string(),result.to_string(),now()])?;
    Ok(())
}

fn apply(conn: &mut Connection, key: &AccountKey, command: Command) -> Result<Value> {
    let time = now();
    let data = match command {
        Command::CreateCustomer {
            idempotency_key,
            display_name,
            recipient,
            external_id,
            tags,
            notes,
        } => {
            text(&display_name, "display_name", 512)?;
            tags_valid(&tags)?;
            if notes.len() > 32_768 {
                bail!("notes 太长");
            }
            if let Some(ref target) = recipient {
                text(target, "recipient", 512)?;
            }
            if let Some(ref id) = external_id {
                text(id, "external_id", 512)?;
            }
            let request = json!({"display_name":display_name,"recipient":recipient,"external_id":external_id,"tags":tags,"notes":notes});
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(prior) =
                prior_receipt(&tx, key, "create_customer", &idempotency_key, &request)?
            {
                return Ok(json!({"account":key,"data":prior}));
            }
            let id = new_id();
            tx.execute("INSERT INTO customers(platform,account_id,customer_id,display_name,recipient,external_id,status,tags,notes,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'lead',?7,?8,?9,?9)",params![key.platform,key.account_id,id,display_name,recipient,external_id,json!(tags).to_string(),notes,time])?;
            let value = one(
                &tx,
                &format!("{CUSTOMER_SELECT} AND customer_id=?3"),
                params![key.platform, key.account_id, id],
            )?;
            save_receipt(
                &tx,
                key,
                "create_customer",
                &idempotency_key,
                &request,
                &value,
            )?;
            tx.commit()?;
            value
        }
        Command::UpdateCustomer {
            customer_id,
            expected_version,
            display_name,
            recipient,
            status,
            tags,
            notes,
        } => {
            if let Some(ref name) = display_name {
                text(name, "display_name", 512)?;
            }
            if let Some(ref recipient) = recipient {
                text(recipient, "recipient", 512)?;
            }
            if let Some(ref tags) = tags {
                tags_valid(tags)?;
            }
            if notes.as_ref().is_some_and(|v| v.len() > 32_768) {
                bail!("notes 太长");
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let updated=tx.execute("UPDATE customers SET display_name=COALESCE(?4,display_name),recipient=COALESCE(?5,recipient),status=COALESCE(?6,status),tags=COALESCE(?7,tags),notes=COALESCE(?8,notes),version=version+1,updated_at=?9 WHERE platform=?1 AND account_id=?2 AND customer_id=?3 AND version=?10",params![key.platform,key.account_id,customer_id,display_name,recipient,status.map(|s|s.as_str()),tags.map(|t|json!(t).to_string()),notes,time,expected_version])?;
            if updated != 1 {
                bail!("客户不存在、账号不匹配或 version 冲突；请重新读取后更新");
            }
            let value = one(
                &tx,
                &format!("{CUSTOMER_SELECT} AND customer_id=?3"),
                params![key.platform, key.account_id, customer_id],
            )?;
            tx.commit()?;
            value
        }
        Command::ListCustomers {
            status,
            limit,
            offset,
        } => {
            page(limit, offset)?;
            json!({"customers":query(conn,&format!("{CUSTOMER_SELECT} AND (?3 IS NULL OR status=?3) ORDER BY created_at,customer_id LIMIT ?4 OFFSET ?5"),params![key.platform,key.account_id,status.map(|s|s.as_str()),limit,offset])?,"limit":limit,"offset":offset})
        }
        Command::GetCustomer {
            customer_id,
            limit,
            offset,
        } => {
            page(limit, offset)?;
            let tx = conn.transaction()?;
            let customer = one(
                &tx,
                &format!("{CUSTOMER_SELECT} AND customer_id=?3"),
                params![key.platform, key.account_id, customer_id],
            )?;
            let events = query(
                &tx,
                &format!(
                    "{EVENT_SELECT} AND customer_id=?3 ORDER BY observed_at,event_id LIMIT ?4 OFFSET ?5"
                ),
                params![key.platform, key.account_id, customer_id, limit, offset],
            )?;
            let sends = query(
                &tx,
                &format!(
                    "{OUTBOX_SELECT} AND customer_id=?3 ORDER BY created_at,send_id LIMIT ?4 OFFSET ?5"
                ),
                params![key.platform, key.account_id, customer_id, limit, offset],
            )?;
            tx.commit()?;
            json!({"customer":customer,"events":events,"sends":sends,"coverage":"locally_recorded_only","limit":limit,"offset":offset})
        }
        Command::RecordInteraction {
            customer_id,
            idempotency_key,
            text: content,
            direction,
            occurred_at,
            enqueue,
        } => {
            text(&content, "text", 65_536)?;
            text(&idempotency_key, "idempotency_key", 256)?;
            let occurred = occurred_at.unwrap_or(time);
            timestamp(occurred)?;
            let payload = json!({"text":content,"direction":direction.as_str(),"enqueue":enqueue});
            let event_key = format!("manual:{idempotency_key}");
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_customer(&tx, key, &customer_id)?;
            let existing = query(
                &tx,
                &format!("{EVENT_SELECT} AND event_key=?3"),
                params![key.platform, key.account_id, event_key],
            )?
            .into_iter()
            .next();
            if let Some(existing) = existing {
                if existing["customer_id"] != customer_id
                    || existing["payload"] != payload
                    || occurred_at.is_some_and(|at| existing["occurred_at"] != at)
                {
                    bail!("幂等键已用于不同的交互记录");
                }
                existing
            } else {
                let id = new_id();
                tx.execute("INSERT INTO events(event_id,platform,account_id,customer_id,event_key,kind,source,verified,occurred_at,observed_at,payload,state) VALUES(?1,?2,?3,?4,?5,'manual_interaction','agent_record',0,?6,?7,?8,?9)",params![id,key.platform,key.account_id,customer_id,event_key,occurred,time,payload.to_string(),if enqueue {"pending"} else {"recorded"}])?;
                let value = one(
                    &tx,
                    &format!("{EVENT_SELECT} AND event_id=?3"),
                    params![key.platform, key.account_id, id],
                )?;
                tx.commit()?;
                value
            }
        }
        Command::LinkEventCustomer {
            event_id,
            customer_id,
        } => {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_customer(&tx, key, &customer_id)?;
            let updated=tx.execute("UPDATE events SET customer_id=?4 WHERE platform=?1 AND account_id=?2 AND event_id=?3 AND (customer_id IS NULL OR customer_id=?4)",params![key.platform,key.account_id,event_id,customer_id])?;
            if updated != 1 {
                bail!("事件不存在、账号不匹配或已关联其他客户");
            }
            let value = one(
                &tx,
                &format!("{EVENT_SELECT} AND event_id=?3"),
                params![key.platform, key.account_id, event_id],
            )?;
            tx.commit()?;
            value
        }
        Command::ClaimEvents {
            consumer,
            limit,
            lease_seconds,
        } => {
            text(&consumer, "consumer", 128)?;
            page(limit, 0)?;
            if !(5..=300).contains(&lease_seconds) {
                bail!("lease_seconds 必须为 5–300");
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("UPDATE events SET state='pending',consumer=NULL,lease_token=NULL,lease_until=NULL WHERE platform=?1 AND account_id=?2 AND state='leased' AND lease_until<=?3",params![key.platform,key.account_id,time])?;
            let ids = query(
                &tx,
                "SELECT json_object('id',event_id) FROM events WHERE platform=?1 AND account_id=?2 AND state='pending' ORDER BY observed_at,event_id LIMIT ?3",
                params![key.platform, key.account_id, limit],
            )?;
            let mut events = Vec::new();
            for id in ids {
                let id = id["id"].as_str().unwrap();
                let token = new_id();
                tx.execute("UPDATE events SET state='leased',consumer=?4,lease_token=?5,lease_until=?6 WHERE platform=?1 AND account_id=?2 AND event_id=?3 AND state='pending'",params![key.platform,key.account_id,id,consumer,token,time+i64::from(lease_seconds)])?;
                let mut event = one(
                    &tx,
                    &format!("{EVENT_SELECT} AND event_id=?3"),
                    params![key.platform, key.account_id, id],
                )?;
                event["lease_token"] = json!(token);
                events.push(event);
            }
            tx.commit()?;
            json!({"events":events,"lease_seconds":lease_seconds})
        }
        Command::AckEvent {
            event_id,
            consumer,
            lease_token,
        } => {
            let updated=conn.execute("UPDATE events SET state='acknowledged' WHERE platform=?1 AND account_id=?2 AND event_id=?3 AND consumer=?4 AND lease_token=?5 AND ((state='leased' AND lease_until>?6) OR state='acknowledged')",params![key.platform,key.account_id,event_id,consumer,lease_token,time])?;
            if updated != 1 {
                bail!("事件租约已过期、令牌不匹配或账号不匹配");
            }
            json!({"event_id":event_id,"state":"acknowledged"})
        }
        Command::ReleaseEvent {
            event_id,
            consumer,
            lease_token,
        } => {
            let updated=conn.execute("UPDATE events SET state='pending',consumer=NULL,lease_token=NULL,lease_until=NULL WHERE platform=?1 AND account_id=?2 AND event_id=?3 AND state='leased' AND consumer=?4 AND lease_token=?5 AND lease_until>?6",params![key.platform,key.account_id,event_id,consumer,lease_token,time])?;
            if updated != 1 {
                bail!("事件租约已过期、令牌不匹配或账号不匹配");
            }
            json!({"event_id":event_id,"state":"pending"})
        }
        Command::RenewEventLease {
            event_id,
            consumer,
            lease_token,
            lease_seconds,
        } => {
            if !(5..=300).contains(&lease_seconds) {
                bail!("lease_seconds 必须为 5–300");
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let updated=tx.execute("UPDATE events SET lease_until=MAX(lease_until,?6) WHERE platform=?1 AND account_id=?2 AND event_id=?3 AND state='leased' AND consumer=?4 AND lease_token=?5 AND lease_until>?7",params![key.platform,key.account_id,event_id,consumer,lease_token,time+i64::from(lease_seconds),time])?;
            if updated != 1 {
                bail!("事件租约已过期、令牌不匹配或账号不匹配");
            }
            let value = one(
                &tx,
                &format!("{EVENT_SELECT} AND event_id=?3"),
                params![key.platform, key.account_id, event_id],
            )?;
            tx.commit()?;
            value
        }
        Command::CreateFollowup {
            idempotency_key,
            customer_id,
            title,
            due_at,
            owner,
        } => {
            text(&title, "title", 2048)?;
            text(&owner, "owner", 128)?;
            timestamp(due_at)?;
            let request =
                json!({"customer_id":customer_id,"title":title,"due_at":due_at,"owner":owner});
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(prior) =
                prior_receipt(&tx, key, "create_followup", &idempotency_key, &request)?
            {
                return Ok(json!({"account":key,"data":prior}));
            }
            require_customer(&tx, key, &customer_id)?;
            let id = new_id();
            tx.execute("INSERT INTO followups(task_id,platform,account_id,customer_id,title,due_at,owner,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8)",params![id,key.platform,key.account_id,customer_id,title,due_at,owner,time])?;
            let value = one(
                &tx,
                &format!("{FOLLOWUP_SELECT} AND task_id=?3"),
                params![key.platform, key.account_id, id],
            )?;
            save_receipt(
                &tx,
                key,
                "create_followup",
                &idempotency_key,
                &request,
                &value,
            )?;
            tx.commit()?;
            value
        }
        Command::ListFollowups {
            customer_id,
            status,
            due_before,
            limit,
            offset,
        } => {
            page(limit, offset)?;
            if let Some(at) = due_before {
                timestamp(at)?;
            }
            json!({"tasks":query(conn,&format!("{FOLLOWUP_SELECT} AND status=?3 AND (?4 IS NULL OR customer_id=?4) AND (?5 IS NULL OR due_at<=?5) ORDER BY due_at,task_id LIMIT ?6 OFFSET ?7"),params![key.platform,key.account_id,status.as_str(),customer_id,due_before,limit,offset])?,"limit":limit,"offset":offset})
        }
        Command::CompleteFollowup { task_id, outcome } => {
            finish_followup(conn, key, &task_id, &outcome, "done")?
        }
        Command::CancelFollowup { task_id, outcome } => {
            finish_followup(conn, key, &task_id, &outcome, "cancelled")?
        }
        Command::RecordCustomerAnalysis {
            idempotency_key,
            customer_id,
            summary,
            evidence_event_ids,
        } => {
            text(&summary, "summary", 32_768)?;
            if evidence_event_ids.is_empty()
                || evidence_event_ids.len() > 100
                || evidence_event_ids.iter().collect::<HashSet<_>>().len()
                    != evidence_event_ids.len()
            {
                bail!("需要 1–100 个不重复的 evidence_event_ids");
            }
            let request = json!({"customer_id":customer_id,"summary":summary,"evidence_event_ids":evidence_event_ids});
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(prior) = prior_receipt(
                &tx,
                key,
                "record_customer_analysis",
                &idempotency_key,
                &request,
            )? {
                return Ok(json!({"account":key,"data":prior}));
            }
            require_customer(&tx, key, &customer_id)?;
            for event in &evidence_event_ids {
                let valid: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE platform=?1 AND account_id=?2 AND customer_id=?3 AND event_id=?4)",params![key.platform,key.account_id,customer_id,event],|r|r.get(0))?;
                if !valid {
                    bail!("分析证据不存在或不属于该账号和客户");
                }
            }
            let id = new_id();
            tx.execute("INSERT INTO analyses(analysis_id,platform,account_id,customer_id,summary,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,key.platform,key.account_id,customer_id,summary,time])?;
            for event in &evidence_event_ids {
                tx.execute(
                    "INSERT INTO analysis_evidence(analysis_id,event_id) VALUES(?1,?2)",
                    params![id, event],
                )?;
            }
            let value = json!({"analysis_id":id,"customer_id":customer_id,"summary":summary,"evidence_event_ids":evidence_event_ids,"classification":"agent_inference","created_at":time});
            save_receipt(
                &tx,
                key,
                "record_customer_analysis",
                &idempotency_key,
                &request,
                &value,
            )?;
            tx.commit()?;
            value
        }
        Command::GetCustomerAnalyses {
            customer_id,
            limit,
            offset,
        } => {
            page(limit, offset)?;
            require_customer(conn, key, &customer_id)?;
            json!({"analyses":query(conn,"SELECT json_object('analysis_id',a.analysis_id,'summary',a.summary,'classification','agent_inference','created_at',a.created_at,'evidence_event_ids',json((SELECT json_group_array(event_id) FROM analysis_evidence WHERE analysis_id=a.analysis_id))) FROM analyses a WHERE a.platform=?1 AND a.account_id=?2 AND a.customer_id=?3 ORDER BY a.created_at,a.analysis_id LIMIT ?4 OFFSET ?5",params![key.platform,key.account_id,customer_id,limit,offset])?,"limit":limit,"offset":offset})
        }
        Command::GetServiceMetrics { from, to } => {
            let to = to.unwrap_or(time + 1);
            let from = from.unwrap_or(to - 7 * 86400);
            timestamp(from)?;
            timestamp(to)?;
            if from >= to {
                bail!("from 必须小于 to，区间为 [from,to)");
            }
            let tx = conn.transaction()?;
            let counts = one(
                &tx,
                "SELECT json_object('interface_observations',COALESCE(SUM(CASE WHEN kind='interface_change' THEN 1 ELSE 0 END),0),'agent_records',COALESCE(SUM(CASE WHEN source='agent_record' THEN 1 ELSE 0 END),0),'verified_incoming',COALESCE(SUM(CASE WHEN verified=1 AND source IN ('provider_event','wecom_kf_api') AND json_extract(payload,'$.direction')='inbound' THEN 1 ELSE 0 END),0),'pending_events',COALESCE(SUM(CASE WHEN state IN ('pending','leased') THEN 1 ELSE 0 END),0)) FROM events WHERE platform=?1 AND account_id=?2 AND observed_at>=?3 AND observed_at<?4",
                params![key.platform, key.account_id, from, to],
            )?;
            let sends = one(
                &tx,
                "SELECT json_object('attempting',COALESCE(SUM(CASE WHEN status='attempting' THEN 1 ELSE 0 END),0),'triggered',COALESCE(SUM(CASE WHEN status='triggered' THEN 1 ELSE 0 END),0),'unknown',COALESCE(SUM(CASE WHEN status='unknown' THEN 1 ELSE 0 END),0),'rejected',COALESCE(SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END),0)) FROM outbox WHERE platform=?1 AND account_id=?2 AND created_at>=?3 AND created_at<?4",
                params![key.platform, key.account_id, from, to],
            )?;
            let snapshot = one(
                &tx,
                "SELECT json_object('customers',(SELECT COUNT(*) FROM customers WHERE platform=?1 AND account_id=?2),'overdue_followups',(SELECT COUNT(*) FROM followups WHERE platform=?1 AND account_id=?2 AND status='pending' AND due_at<?3),'as_of',?3)",
                params![key.platform, key.account_id, time],
            )?;
            tx.commit()?;
            json!({"window":{"from":from,"to":to,"basis":"local_observed_at"},"events":counts,"send_attempts":sends,"current_snapshot":snapshot,"coverage":"locally_recorded_only","delivery_rate":null,"response_time":null,"limitations":["OCR变化不是已验证新收件","发送触发不是送达","未计算无可靠收件与回复关联的响应时间"]})
        }
        Command::GetSendStatus { idempotency_key } => one(
            conn,
            &format!("{OUTBOX_SELECT} AND idempotency_key=?3"),
            params![key.platform, key.account_id, idempotency_key],
        )?,
    };
    Ok(json!({"account":key,"data":data}))
}

fn finish_followup(
    conn: &mut Connection,
    key: &AccountKey,
    task_id: &str,
    outcome: &str,
    status: &str,
) -> Result<Value> {
    text(outcome, "outcome", 8192)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = one(
        &tx,
        &format!("{FOLLOWUP_SELECT} AND task_id=?3"),
        params![key.platform, key.account_id, task_id],
    )?;
    if existing["status"] == status {
        if existing["outcome"] != outcome {
            bail!("已结束任务不能修改结果");
        }
        return Ok(existing);
    }
    if existing["status"] != "pending" {
        bail!("任务状态不允许此操作");
    }
    tx.execute("UPDATE followups SET status=?4,outcome=?5,completed_at=?6 WHERE platform=?1 AND account_id=?2 AND task_id=?3 AND status='pending'",params![key.platform,key.account_id,task_id,status,outcome,now()])?;
    let value = one(
        &tx,
        &format!("{FOLLOWUP_SELECT} AND task_id=?3"),
        params![key.platform, key.account_id, task_id],
    )?;
    tx.commit()?;
    Ok(value)
}
