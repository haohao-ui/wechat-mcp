mod clipboard;
mod mcp;
mod wechat;

use anyhow::Result;
use mcp::{make_tool_result, JsonRpcRequest, JsonRpcResponse};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // 关键：所有日志必须输出到 stderr，绝不能污染 stdout（stdout 专用于 MCP JSON-RPC 消息）
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    info!("WeChat MCP Server 正在启动...");

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(e) => {
                error!("解析 JSON-RPC 请求失败: {:?}, 原文: {}", e, trimmed);
                let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e));
                send_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        // 处理没有 id 的通知类消息（如 notifications/initialized）
        if request.id.is_none() {
            info!("收到客户端通知: {}", request.method);
            continue;
        }

        let id = request.id.clone();
        let resp = handle_request(request).await;
        let final_resp = match resp {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                error!("处理方法出错: {:?}", e);
                JsonRpcResponse::error(id, -32603, e.to_string())
            }
        };

        send_response(&mut stdout, &final_resp).await?;
    }

    info!("WeChat MCP Server 正常退出");
    Ok(())
}

async fn send_response<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &JsonRpcResponse,
) -> Result<()> {
    let json_str = serde_json::to_string(response)?;
    writer.write_all(json_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn handle_request(req: JsonRpcRequest) -> Result<serde_json::Value> {
    match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "wechat-mcp",
                "version": "0.1.0"
            }
        })),

        "ping" => Ok(json!({})),

        "tools/list" => Ok(json!({
            "tools": [
                {
                    "name": "send_wechat_message",
                    "description": "通过 macOS 微信桌面端向指定联系人或群聊发送消息（利用系统级安全 UI 模拟）",
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
                    "description": "获取微信左侧最近会话列表及各联系人/群聊的最新一条消息摘要与时间",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "read_chat_messages",
                    "description": "读取指定联系人/群聊（或当前打开窗口）的最近聊天记录文本（基于 macOS 原生离线 Vision OCR）",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "recipient": {
                                "type": "string",
                                "description": "接收人昵称、备注名或群名称（例如：文件传输助手、午夜咖啡）。若留空则读取当前激活的会话"
                            }
                        }
                    }
                },
                {
                    "name": "wait_for_new_message",
                    "description": "长轮询监听微信新消息。若提供 recipient，则只监听该指定好友/群聊；若留空（不提供 recipient），则开启全局收件箱监听模式，自动感知任意好友/群聊发来的新消息，并自动提取最新聊天内容，用于驱动 Agent 自动回复闭环",
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
        })),

        "tools/call" => {
            let params = req.params.unwrap_or(serde_json::Value::Null);
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match tool_name {
                "send_wechat_message" => {
                    let recipient = arguments.get("recipient").and_then(|v| v.as_str()).unwrap_or("");
                    let message = arguments.get("message").and_then(|v| v.as_str()).unwrap_or("");

                    if recipient.is_empty() || message.is_empty() {
                        return Ok(make_tool_result("参数缺失：必须提供 recipient 和 message", true));
                    }

                    match wechat::send_message(recipient, message).await {
                        Ok(()) => Ok(make_tool_result(&format!("已成功触发发送给 [{}] 的微信消息", recipient), false)),
                        Err(e) => Ok(make_tool_result(&format!("发送微信消息失败: {:#}", e), true)),
                    }
                }

                "focus_wechat" => {
                    match wechat::focus_wechat().await {
                        Ok(_) => Ok(make_tool_result("已激活微信窗口", false)),
                        Err(e) => Ok(make_tool_result(&format!("激活微信窗口失败: {:#}", e), true)),
                    }
                }

                "read_chat_messages" => {
                    let recipient = arguments.get("recipient").and_then(|v| v.as_str());
                    match wechat::read_chat_messages(recipient).await {
                        Ok(lines) => {
                            if lines.is_empty() {
                                Ok(make_tool_result("未读取到任何聊天记录或聊天区域为空", false))
                            } else {
                                let content = lines.join("\n");
                                Ok(make_tool_result(&format!("成功读取聊天记录:\n{}", content), false))
                            }
                        }
                        Err(e) => Ok(make_tool_result(&format!("读取聊天记录失败: {:#}", e), true)),
                    }
                }

                "get_recent_sessions" => {
                    match wechat::get_recent_sessions().await {
                        Ok(sessions) => {
                            if sessions.is_empty() {
                                Ok(make_tool_result("未读取到任何会话列表项", false))
                            } else {
                                let mut lines = Vec::new();
                                for (idx, s) in sessions.iter().enumerate() {
                                    lines.push(format!("{}. [{}] ({}): {}", idx + 1, s.name, s.time, s.preview));
                                }
                                Ok(make_tool_result(&format!("当前最近会话列表:\n{}", lines.join("\n")), false))
                            }
                        }
                        Err(e) => Ok(make_tool_result(&format!("获取会话列表失败: {:#}", e), true)),
                    }
                }

                "wait_for_new_message" => {
                    let recipient = arguments.get("recipient").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
                    let timeout = arguments.get("timeout_seconds").and_then(|v| v.as_u64()).unwrap_or(60);

                    match wechat::wait_for_new_message(recipient, timeout).await {
                        Ok(Some(msg)) => {
                            let label = recipient.unwrap_or("任意好友/群聊");
                            Ok(make_tool_result(&format!("监听到来自 [{}] 的新消息:\n{}", label, msg), false))
                        }
                        Ok(None) => {
                            let label = recipient.unwrap_or("任意好友/群聊");
                            Ok(make_tool_result(&format!("在 {} 秒内未检测到 [{}] 的新消息", timeout, label), false))
                        }
                        Err(e) => Ok(make_tool_result(&format!("监听新消息出错: {:#}", e), true)),
                    }
                }

                _ => Ok(make_tool_result(&format!("未知的工具名称: {}", tool_name), true)),
            }
        }

        _ => anyhow::bail!("未支持的方法: {}", req.method),
    }
}
