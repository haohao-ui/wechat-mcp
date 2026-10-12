use serde_json::{Value, json};

pub fn list_tools() -> Value {
    let mut list = json!({
        "tools": [
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
        ]
    });
    let tools = list["tools"].as_array_mut().expect("static tool list");
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
    tools.extend(gateway_tools());
    tools.extend(crate::service_tools::list());
    list
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
