mod accounts;
mod adapters;
mod clipboard;
mod crm;
mod gateway;
mod listeners;
mod mcp;
mod resource_lock;
#[cfg(test)]
mod service_tests;
mod service_tools;
mod storage;
mod tools;
mod wechat;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use gateway::Gateway;
use mcp::{JsonRpcRequest, JsonRpcResponse};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    task::{AbortHandle, JoinSet},
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let gateway = Arc::new(Gateway::load()?);
    gateway
        .listeners
        .restore(&gateway.accounts, gateway.store.clone())
        .await?;
    info!(
        "消息网关启动，已注册 {} 个账号绑定",
        gateway.accounts.list().len()
    );
    let mut reader = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    let mut tasks = JoinSet::new();
    let mut pending: HashMap<String, Option<AbortHandle>> = HashMap::new();
    let mut input_open = true;
    let mut structured_content = false;

    while input_open || !tasks.is_empty() {
        tokio::select! {
            completed = tasks.join_next(), if !tasks.is_empty() => {
                match completed {
                    Some(Ok((key, response))) => {
                        if pending.remove(&key).is_some() {
                            send_response(&mut stdout, &response).await?;
                        }
                    }
                    Some(Err(error)) if error.is_cancelled() => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => {}
                }
            }
            line = reader.next_line(), if input_open => {
                let Some(line) = line? else {
                    input_open = false;
                    for handle in pending.values().flatten() { handle.abort(); }
                    pending.retain(|_, handle| handle.is_none());
                    gateway.listeners.shutdown().await;
                    continue;
                };
                if line.trim().is_empty() { continue; }
                let value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => {
                        send_response(&mut stdout, &JsonRpcResponse::error(None, -32700, "Parse error")).await?;
                        continue;
                    }
                };
                let id = value.get("id").cloned();
                let valid_id = id.as_ref().is_none_or(|id| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some());
                let request: JsonRpcRequest = match serde_json::from_value::<JsonRpcRequest>(value) {
                    Ok(request) if valid_id && request.jsonrpc == "2.0" => request,
                    _ => {
                        send_response(&mut stdout, &JsonRpcResponse::error(id.filter(|_| valid_id), -32600, "Invalid Request")).await?;
                        continue;
                    }
                };
                let Some(id) = request.id.clone() else {
                    if request.method == "notifications/cancelled"
                        && let Some(request_id) = request.params.as_ref().and_then(|params| params.get("requestId"))
                    {
                        let key = request_id.to_string();
                        if let Some(Some(handle)) = pending.get(&key) {
                            handle.abort();
                            pending.remove(&key);
                        }
                        // A send already admitted to the UI must finish clipboard cleanup.
                    }
                    continue;
                };
                let key = id.to_string();
                if pending.contains_key(&key) {
                    send_response(&mut stdout, &JsonRpcResponse::error(Some(id), -32600, "请求 ID 正在使用")).await?;
                    continue;
                }
                if tasks.len() >= 64 {
                    send_response(&mut stdout, &JsonRpcResponse::error(Some(id), -32000, "请求队列已满")).await?;
                    continue;
                }
                if request.method == "initialize" {
                    structured_content = request.params.as_ref()
                        .and_then(|params| params.get("protocolVersion"))
                        .and_then(Value::as_str) == Some("2025-06-18");
                }
                let can_cancel = is_cancellable(&request);
                let gateway = gateway.clone();
                let response_key = key.clone();
                let abort = tasks.spawn(async move {
                    let response = handle_request(request, &gateway, structured_content).await;
                    (response_key, response)
                });
                pending.insert(key, can_cancel.then_some(abort));
            }
        }
    }
    gateway.listeners.shutdown().await;
    Ok(())
}

fn is_cancellable(request: &JsonRpcRequest) -> bool {
    if request.method != "tools/call" {
        return false;
    }
    let Some(params) = request.params.as_ref() else {
        return false;
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return false;
    };
    if matches!(
        name,
        "list_conversations" | "get_recent_sessions" | "wait_for_events"
    ) {
        return true;
    }
    let navigates = params
        .get("arguments")
        .and_then(|args| args.get("recipient"))
        .and_then(Value::as_str)
        .is_some_and(|recipient| !recipient.trim().is_empty());
    // Contact search temporarily changes the clipboard and must reach cleanup.
    !navigates
        && matches!(
            name,
            "wait_for_message" | "wait_for_new_message" | "get_messages" | "read_chat_messages"
        )
}

async fn send_response<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &JsonRpcResponse,
) -> Result<()> {
    let body = serde_json::to_vec(response)?;
    writer.write_all(&body).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn handle_request(
    request: JsonRpcRequest,
    gateway: &Gateway,
    structured_content: bool,
) -> JsonRpcResponse {
    let id = request.id;
    let result = match request.method.as_str() {
        "initialize" => json!({
            "protocolVersion":if structured_content { "2025-06-18" } else { "2024-11-05" }, "capabilities":{"tools":{}},
            "serverInfo":{"name":"wechat-mcp","version":env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => tools::list_tools(),
        "tools/call" => {
            let Some(params) = request.params.as_ref().and_then(Value::as_object) else {
                return JsonRpcResponse::error(id, -32602, "tools/call params 必须是对象");
            };
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return JsonRpcResponse::error(id, -32602, "必须提供工具名称");
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut result = gateway.call(name, args).await;
            if !structured_content {
                result.as_object_mut().unwrap().remove("structuredContent");
            }
            result
        }
        _ => return JsonRpcResponse::error(id, -32601, "Method not found"),
    };
    JsonRpcResponse::success(id, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_does_not_interrupt_clipboard_search_or_send() {
        for (name, args, cancellable) in [
            ("wait_for_message", json!({}), true),
            ("wait_for_message", json!({"recipient":"someone"}), false),
            ("get_messages", json!({}), true),
            ("get_messages", json!({"recipient":"someone"}), false),
            ("list_conversations", json!({}), true),
            ("send_message", json!({}), false),
            ("focus_wechat", json!({}), false),
        ] {
            let request = JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/call".into(),
                params: Some(json!({"name":name,"arguments":args})),
            };
            assert_eq!(is_cancellable(&request), cancellable, "{name}");
        }
    }
}
