use anyhow::Result;
use serde_json::Value;
use crate::core::model::Frame;

pub trait ToolClient: Send + Sync {
    fn tool(&self, name: &str, args: Value) -> impl std::future::Future<Output = Result<Value>> + Send;
}

pub fn is_not_sent(err_msg: Option<&str>, receipt: Option<&Value>) -> bool {
    if let Some(rec) = receipt {
        if let Some(delivery) = rec.get("delivery").and_then(|v| v.as_str()) {
            if delivery == "not_sent" {
                return true;
            }
        }
        if let Some(status) = rec.get("status").and_then(|v| v.as_str()) {
            if status == "not_sent" {
                return true;
            }
        }
    }
    if let Some(msg) = err_msg {
        if msg.contains("未触发发送") || msg.contains("not_sent") {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq)]
pub enum EchoOutcome {
    Seen(Frame),
    Unverifiable(String),
    NotSeen(Option<Frame>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_not_sent_conditions_match_python_golden() {
        // 1. receipt with delivery == not_sent
        let rec1 = json!({"delivery": "not_sent", "status": "unknown"});
        assert!(is_not_sent(None, Some(&rec1)));

        // 2. receipt with status == not_sent
        let rec2 = json!({"delivery": "unknown", "status": "not_sent"});
        assert!(is_not_sent(None, Some(&rec2)));

        // 3. error contains "未触发发送"
        let err1 = "发送尝试 send-1 未触发发送: wechat_click 退出状态异常: Some(1)";
        assert!(is_not_sent(Some(err1), None));

        // 4. error contains "not_sent"
        let err2 = "some error with not_sent detail";
        assert!(is_not_sent(Some(err2), None));

        // 5. error without not_sent and receipt with triggered
        let err3 = "结果未知: timeout";
        let rec3 = json!({"delivery": "unknown", "status": "unknown"});
        assert!(!is_not_sent(Some(err3), Some(&rec3)));
        assert!(!is_not_sent(None, Some(&rec3)));
    }
}
