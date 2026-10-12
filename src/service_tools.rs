use serde_json::{Value, json};

pub fn list() -> Vec<Value> {
    let mut tools = Vec::new();
    let customer = json!({"type":"string","minLength":1});
    let key = json!({"type":"string","minLength":1,"maxLength":256});
    let limit = json!({"type":"integer","minimum":1,"maximum":100,"default":50});
    let offset = json!({"type":"integer","minimum":0,"maximum":1000000,"default":0});
    let time = json!({"type":"integer","minimum":0,"maximum":253402300799_i64,"description":"UTC Unix 秒"});
    let status = json!({"type":"string","enum":["lead","active","dormant","closed"]});
    let tags = json!({"type":"array","maxItems":50,"items":{"type":"string","minLength":1,"maxLength":128}});
    for (name, description, properties, required, read_only) in [
        (
            "create_customer",
            "创建账号内的客户档案；本地 ID 不表示已验证平台身份",
            json!({"idempotency_key":key,"display_name":{"type":"string","minLength":1},"recipient":{"type":"string","minLength":1},"external_id":{"type":"string","minLength":1},"tags":tags,"notes":{"type":"string"}}),
            vec!["idempotency_key", "display_name"],
            false,
        ),
        (
            "update_customer",
            "按版本更新客户，版本冲突时需重新读取",
            json!({"customer_id":customer,"expected_version":{"type":"integer","minimum":1},"display_name":{"type":"string","minLength":1},"recipient":{"type":"string","minLength":1},"status":status,"tags":tags,"notes":{"type":"string"}}),
            vec!["customer_id", "expected_version"],
            false,
        ),
        (
            "list_customers",
            "分页查询指定账号客户",
            json!({"status":status,"limit":limit,"offset":offset}),
            vec![],
            true,
        ),
        (
            "get_customer",
            "读取客户档案及本地积累事件和发送记录；不是完整平台历史",
            json!({"customer_id":customer,"limit":limit,"offset":offset}),
            vec!["customer_id"],
            true,
        ),
        (
            "capture_customer_context",
            "读取客户目标的可见聊天并持久存为未验证 OCR 快照，不代表完整历史",
            json!({"customer_id":customer}),
            vec!["customer_id"],
            false,
        ),
        (
            "record_interaction",
            "保存 Agent 记录的交互，来源为 agent_record，不作为已验证平台消息",
            json!({"customer_id":customer,"idempotency_key":key,"text":{"type":"string","minLength":1},"direction":{"type":"string","enum":["inbound","outbound","unknown"],"default":"unknown"},"occurred_at":time,"enqueue":{"type":"boolean","default":false}}),
            vec!["customer_id", "idempotency_key", "text"],
            false,
        ),
        (
            "link_event_customer",
            "将账号内事件关联到客户，不提升事件可信度",
            json!({"customer_id":customer,"event_id":customer}),
            vec!["customer_id", "event_id"],
            false,
        ),
        (
            "claim_events",
            "事务领取待处理事件；过期租约可重新领取。处理副作用应使用幂等键",
            json!({"consumer":{"type":"string","minLength":1},"limit":limit,"lease_seconds":{"type":"integer","minimum":5,"maximum":300,"default":60}}),
            vec!["consumer"],
            false,
        ),
        (
            "ack_event",
            "用领取时的 consumer 和租约令牌确认处理完成",
            json!({"event_id":customer,"consumer":customer,"lease_token":customer}),
            vec!["event_id", "consumer", "lease_token"],
            false,
        ),
        (
            "release_event",
            "在有效租约内退回事件，允许重试处理",
            json!({"event_id":customer,"consumer":customer,"lease_token":customer}),
            vec!["event_id", "consumer", "lease_token"],
            false,
        ),
        (
            "renew_event_lease",
            "在有效租约内延长事件处理时间，过期令牌不能恢复所有权",
            json!({"event_id":customer,"consumer":customer,"lease_token":customer,"lease_seconds":{"type":"integer","minimum":5,"maximum":300,"default":60}}),
            vec!["event_id", "consumer", "lease_token"],
            false,
        ),
        (
            "create_followup",
            "创建客户跟进任务；到期查询由 Agent 调度，不自动发送消息",
            json!({"idempotency_key":key,"customer_id":customer,"title":{"type":"string","minLength":1},"due_at":time,"owner":customer}),
            vec!["idempotency_key", "customer_id", "title", "due_at", "owner"],
            false,
        ),
        (
            "list_followups",
            "分页查询跟进任务，可筛选到期时间",
            json!({"customer_id":customer,"status":{"type":"string","enum":["pending","done","cancelled"],"default":"pending"},"due_before":time,"limit":limit,"offset":offset}),
            vec![],
            true,
        ),
        (
            "complete_followup",
            "记录跟进结果，重复完成必须保持相同结果",
            json!({"task_id":customer,"outcome":{"type":"string","minLength":1}}),
            vec!["task_id", "outcome"],
            false,
        ),
        (
            "cancel_followup",
            "取消待办跟进并记录原因，结束后不可再次完成",
            json!({"task_id":customer,"outcome":{"type":"string","minLength":1}}),
            vec!["task_id", "outcome"],
            false,
        ),
        (
            "record_customer_analysis",
            "保存 Agent 推断及同账号同客户的证据事件引用",
            json!({"idempotency_key":key,"customer_id":customer,"summary":{"type":"string","minLength":1},"evidence_event_ids":{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":customer}}),
            vec![
                "idempotency_key",
                "customer_id",
                "summary",
                "evidence_event_ids",
            ],
            false,
        ),
        (
            "get_customer_analyses",
            "分页读取带证据引用的客户分析",
            json!({"customer_id":customer,"limit":limit,"offset":offset}),
            vec!["customer_id"],
            true,
        ),
        (
            "get_service_metrics",
            "按本地观察时间统计，区分 OCR 观察、Agent 记录和发送尝试；无可靠依据的指标为空",
            json!({"from":time,"to":time}),
            vec![],
            true,
        ),
        (
            "get_send_status",
            "查询幂等键对应的发送尝试；attempting/unknown 不允许盲目重发",
            json!({"idempotency_key":key}),
            vec!["idempotency_key"],
            true,
        ),
        (
            "start_listener",
            "启用并持久保存后台观察，重启恢复；变化入收件箱，不保证是真实收件，会操作微信窗口",
            json!({"interval_seconds":{"type":"integer","minimum":2,"maximum":60,"default":5}}),
            vec![],
            false,
        ),
        (
            "stop_listener",
            "持久关闭指定账号的后台观察；其他进程负责时返回 stop_requested",
            json!({}),
            vec![],
            false,
        ),
        (
            "get_listener_status",
            "读取账号监听状态、持久启用状态、其他进程占用及最近错误",
            json!({}),
            vec![],
            true,
        ),
        (
            "wait_for_events",
            "等待持久收件箱事件；返回后仍需 claim_events 领取，不阻塞桌面操作",
            json!({"timeout_seconds":{"type":"integer","minimum":0,"maximum":300,"default":60},"limit":limit}),
            vec![],
            true,
        ),
    ] {
        let mut props = json!({"platform":{"type":"string","minLength":1},"account_id":{"type":"string","minLength":1}});
        props
            .as_object_mut()
            .unwrap()
            .extend(properties.as_object().unwrap().clone());
        let mut fields = vec!["platform", "account_id"];
        fields.extend(required);
        tools.push(json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":props,"required":fields,"additionalProperties":false},"annotations":{"readOnlyHint":read_only,"openWorldHint":matches!(name,"start_listener"|"stop_listener")}}));
    }
    tools
}
