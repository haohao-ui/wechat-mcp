use serde_json::{Value, json};

pub fn parse_legacy_tools_env(val: Option<&str>) -> bool {
    match val {
        Some(v) => {
            let s = v.trim().to_ascii_lowercase();
            !matches!(s.as_str(), "0" | "false" | "off")
        }
        None => true,
    }
}

pub fn legacy_tools_enabled() -> bool {
    let val = std::env::var("MESSAGE_GATEWAY_LEGACY_TOOLS").ok();
    parse_legacy_tools_env(val.as_deref())
}

pub fn legacy_tool_replacement(name: &str) -> Option<&'static str> {
    match name {
        "send_wechat_message" => Some("send_message"),
        "get_recent_sessions" => Some("list_conversations"),
        "read_chat_messages" => Some("get_messages"),
        "wait_for_new_message" => Some("wait_for_message"),
        _ => None,
    }
}

pub fn is_legacy_tool(name: &str) -> bool {
    matches!(
        name,
        "send_wechat_message"
            | "focus_wechat"
            | "get_recent_sessions"
            | "read_chat_messages"
            | "wait_for_new_message"
    )
}

fn legacy_tools() -> Vec<Value> {
    let mut list = json!([
        {
            "name": "send_wechat_message",
            "description": "通过 macOS 微信桌面端触发发送操作；未验证接收人身份或送达",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "接收人昵称、备注名或群名称（例如：文件传输助手、张三、项目交流群）"
                    },
                    "message": {
                        "type": "string",
                        "description": "要发送的消息文本内容"
                    }
                },
                "required": ["recipient", "message"]
            }
        },
        {
            "name": "focus_wechat",
            "description": "激活并置顶显示微信窗口（自动校准归位至主屏标准坐标）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "get_recent_sessions",
            "description": "OCR 读取微信左侧当前可见会话、摘要与时间文字",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "read_chat_messages",
            "description": "OCR 读取当前可见聊天区域文字，不代表完整历史",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "接收人昵称、备注名或群名称（例如：文件传输助手、示例联系人甲）。若留空则读取当前激活的会话"
                    }
                }
            }
        },
        {
            "name": "wait_for_new_message",
            "description": "轮询观察微信界面变化；不保证变化是新收件。recipient 留空时扫描可见会话，不是后台订阅",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "要监听的接收人昵称或群名称（可选。留空则开启全局收件箱监听）"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "最长等待秒数（默认 60 秒）"
                    }
                }
            }
        }
    ]);
    let tools = list.as_array_mut().expect("static legacy tools");
    for tool in tools.iter_mut() {
        tool["inputSchema"]["properties"]["account_id"] =
            json!({"type":"string", "description":"微信账号绑定 ID；省略时仅允许唯一微信账号"});
    }
    for tool in tools
        .iter_mut()
        .filter(|tool| tool["name"] == "send_wechat_message")
    {
        tool["inputSchema"]["properties"]["idempotency_key"] =
            json!({"type":"string","minLength":1,"maxLength":256});
        tool["inputSchema"]["properties"]["customer_id"] = json!({"type":"string","minLength":1});
    }
    tools.clone()
}

pub fn list_tools_with_legacy(legacy_enabled: bool) -> Value {
    let mut tools = Vec::new();
    if legacy_enabled {
        tools.extend(legacy_tools());
    }
    tools.extend(gateway_tools());
    tools.extend(crate::service_tools::list());
    tools.push(crate::article::tool());
    json!({ "tools": tools })
}

fn gateway_tools() -> Vec<Value> {
    let mut tools = vec![
        json!({"name":"list_accounts", "description":"列出账号绑定、平台、适配器能力与数据边界；绑定不代表已验证微信登录身份", "inputSchema":{"type":"object","properties":{},"additionalProperties":false}}),
    ];
    for (name, description, extra, required) in [
        (
            "list_conversations",
            "读取指定账号的可见会话",
            json!({}),
            vec![],
        ),
        (
            "get_messages",
            "读取指定账号的可见聊天文字；不代表完整历史",
            json!({"recipient":{"type":"string"}}),
            vec![],
        ),
        (
            "send_message",
            "按幂等键记录并触发发送；微信送达状态未知，重复键不会再次触发",
            json!({"recipient":{"type":"string","minLength":1},"message":{"type":"string","minLength":1},"idempotency_key":{"type":"string","minLength":1,"maxLength":256},"customer_id":{"type":"string","minLength":1}}),
            vec!["recipient", "message", "idempotency_key"],
        ),
        (
            "send_file",
            "按幂等键记录并发送本机文件（仅限桌面、文稿、下载文件夹，≤10MB）；粘贴后核对输入框出现该文件才回车；送达状态未知，重复键不会再次触发",
            json!({"recipient":{"type":"string","minLength":1},"path":{"type":"string","minLength":1},"idempotency_key":{"type":"string","minLength":1,"maxLength":256},"customer_id":{"type":"string","minLength":1}}),
            vec!["recipient", "path", "idempotency_key"],
        ),
        (
            "wait_for_message",
            "在指定账号上等待界面消息变化；不是后台订阅",
            json!({"recipient":{"type":"string"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":300}}),
            vec![],
        ),
    ] {
        let mut properties =
            json!({"platform":{"type":"string"},"account_id":{"type":"string","minLength":1}});
        properties
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let mut fields = vec!["platform", "account_id"];
        fields.extend(required);
        tools.push(json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":fields,"additionalProperties":false}}));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_tools_env_respects_disable_values() {
        for disabled in ["0", "false", "off", "FALSE", "Off", " 0 ", " false "] {
            assert!(
                !parse_legacy_tools_env(Some(disabled)),
                "value {disabled:?} should disable legacy tools"
            );
        }
        for enabled in [
            None,
            Some("1"),
            Some("true"),
            Some("on"),
            Some(""),
            Some("other"),
        ] {
            assert!(
                parse_legacy_tools_env(enabled),
                "value {enabled:?} should enable legacy tools"
            );
        }
    }

    #[test]
    fn list_tools_filters_legacy_tools_when_disabled() {
        let legacy_names = [
            "send_wechat_message",
            "focus_wechat",
            "get_recent_sessions",
            "read_chat_messages",
            "wait_for_new_message",
        ];

        let enabled_list = list_tools_with_legacy(true);
        let enabled_tools: Vec<&str> = enabled_list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in legacy_names {
            assert!(
                enabled_tools.contains(&name),
                "legacy enabled list should contain {name}"
            );
        }

        let disabled_list = list_tools_with_legacy(false);
        let disabled_tools: Vec<&str> = disabled_list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in legacy_names {
            assert!(
                !disabled_tools.contains(&name),
                "legacy disabled list must not contain {name}"
            );
        }
        for unified_name in [
            "list_accounts",
            "send_message",
            "list_conversations",
            "get_messages",
            "wait_for_message",
        ] {
            assert!(
                disabled_tools.contains(&unified_name),
                "legacy disabled list must keep unified tool {unified_name}"
            );
        }
    }

    #[test]
    fn legacy_tool_helpers_map_replacements_correctly() {
        assert_eq!(
            legacy_tool_replacement("send_wechat_message"),
            Some("send_message")
        );
        assert_eq!(
            legacy_tool_replacement("get_recent_sessions"),
            Some("list_conversations")
        );
        assert_eq!(
            legacy_tool_replacement("read_chat_messages"),
            Some("get_messages")
        );
        assert_eq!(
            legacy_tool_replacement("wait_for_new_message"),
            Some("wait_for_message")
        );
        assert_eq!(legacy_tool_replacement("focus_wechat"), None);
        assert_eq!(legacy_tool_replacement("send_message"), None);

        for name in [
            "send_wechat_message",
            "focus_wechat",
            "get_recent_sessions",
            "read_chat_messages",
            "wait_for_new_message",
        ] {
            assert!(is_legacy_tool(name));
        }
        assert!(!is_legacy_tool("send_message"));
    }
}
