mod accounts;
mod adapters;
mod article;
pub mod brain;
mod clipboard;
pub mod core;
mod crm;
mod desktop;
mod domain;
mod gateway;
mod listeners;
mod mcp;
mod messaging;
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

fn main() -> Result<()> {
    // Environment changes must happen before the multi-threaded runtime starts its worker threads.
    core::serve::ensure_locale();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--desktop-diagnostics" => {
                let result = desktop::runtime::shared().diagnostics().await;
                desktop::runtime::shared().shutdown().await;
                println!("{}", serde_json::to_string(&result?)?);
                return Ok(());
            }
            "serve-core" => {
                let mut config_path: Option<std::path::PathBuf> = None;
                let mut i = 2;
                while i < args.len() {
                    if args[i] == "--config" && i + 1 < args.len() {
                        config_path = Some(std::path::PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                let Some(cfg_path) = config_path else {
                    eprintln!("error: serve-core requires --config <path>");
                    std::process::exit(2);
                };
                match core::serve::run_serve_core(&cfg_path, None).await {
                    Ok(code) => std::process::exit(code),
                    Err(err) => {
                        eprintln!("{}", err);
                        std::process::exit(2);
                    }
                }
            }
            "control" => {
                let code = core::control::run_control(&args[2..])?;
                std::process::exit(code);
            }
            "backup" => {
                let mut config_path: Option<std::path::PathBuf> = None;
                let mut out_dir: Option<std::path::PathBuf> = None;
                let mut i = 2;
                while i < args.len() {
                    if args[i] == "--config" && i + 1 < args.len() {
                        config_path = Some(std::path::PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else if args[i] == "--out" && i + 1 < args.len() {
                        out_dir = Some(std::path::PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                let Some(cfg_path) = config_path else {
                    eprintln!("error: backup requires --config <path>");
                    std::process::exit(2);
                };
                let config = match core::config::BotConfig::load(&cfg_path) {
                    Ok(cfg) => cfg,
                    Err(err) => {
                        eprintln!("configuration error in {}: {}", cfg_path.display(), err);
                        std::process::exit(2);
                    }
                };
                match core::backup::run_backup(&config, out_dir.as_deref()) {
                    Ok(out) => {
                        println!("Backup completed: {}", out.display());
                        std::process::exit(0);
                    }
                    Err(err) => {
                        eprintln!("Backup failed: {}", err);
                        std::process::exit(1);
                    }
                }
            }
            "replay" => {
                match core::replay::run_replay_cli(&args[2..]).await {
                    Ok(code) => std::process::exit(code),
                    Err(err) => {
                        eprintln!("{}", err);
                        std::process::exit(2);
                    }
                }
            }
            _ => {}
        }
    }
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
                    let client_version = request
                        .params
                        .as_ref()
                        .and_then(|params| params.get("protocolVersion"))
                        .and_then(Value::as_str);
                    let (_, enabled) = negotiate_protocol_version(client_version);
                    structured_content = enabled;
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
    desktop::runtime::shared().shutdown().await;
    Ok(())
}

const SUPPORTED_PROTOCOL_VERSION: &str = "2025-06-18";
const FALLBACK_PROTOCOL_VERSION: &str = "2024-11-05";

fn is_valid_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits_valid = bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit());
    if !digits_valid {
        return false;
    }
    let month: u32 = match s[5..7].parse() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let day: u32 = match s[8..10].parse() {
        Ok(d) => d,
        Err(_) => return false,
    };
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn negotiate_protocol_version(client_version: Option<&str>) -> (&'static str, bool) {
    match client_version {
        Some(v) if is_valid_date(v) && v >= SUPPORTED_PROTOCOL_VERSION => {
            (SUPPORTED_PROTOCOL_VERSION, true)
        }
        _ => (FALLBACK_PROTOCOL_VERSION, false),
    }
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
        "list_conversations" | "get_recent_sessions" | "wait_for_events" | article::TOOL
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
        "initialize" => {
            let client_version = request
                .params
                .as_ref()
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str);
            let (protocol_version, _) = negotiate_protocol_version(client_version);
            json!({
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wechat-mcp", "version": env!("CARGO_PKG_VERSION")},
            })
        }
        "ping" => json!({}),
        "tools/list" => tools::list_tools_with_legacy(gateway.legacy_tools),
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
            ("fetch_article", json!({"url":"x"}), true),
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

    #[test]
    fn protocol_version_negotiation_handles_versions_and_invalid_strings() {
        // >= "2025-06-18" 返回 ("2025-06-18", true)
        assert_eq!(
            negotiate_protocol_version(Some("2025-11-25")),
            ("2025-06-18", true)
        );
        assert_eq!(
            negotiate_protocol_version(Some("2025-06-18")),
            ("2025-06-18", true)
        );

        // < "2025-06-18" 返回 ("2024-11-05", false)
        assert_eq!(
            negotiate_protocol_version(Some("2024-11-05")),
            ("2024-11-05", false)
        );
        assert_eq!(
            negotiate_protocol_version(Some("2025-01-01")),
            ("2024-11-05", false)
        );

        // 非法字符串校验
        for invalid in [
            "",
            "invalid",
            "2025-6-18",
            "2025-13-01",
            "2025-00-10",
            "2025-06-00",
            "2025-06-32",
            "2025/06/18",
            "latest",
            "2026-99-99",
            "2025-06-18-extra",
            "prefix-2025-06-18",
        ] {
            assert_eq!(
                negotiate_protocol_version(Some(invalid)),
                ("2024-11-05", false),
                "testing invalid: {invalid}"
            );
        }

        // None
        assert_eq!(negotiate_protocol_version(None), ("2024-11-05", false));
    }
}
