//! 微信外部大脑对外开放接口 Rust 示例
//!
//! 运行方式：
//! OPEN_API_URL=http://127.0.0.1:8770/mcp OPEN_API_TOKEN=<64位十六进制令牌> cargo run --example open_api_brain

use std::{sync::Arc, time::Duration};
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Debug)]
pub enum BrainError {
    SessionNotFound,          // HTTP 404
    Conflict,                 // HTTP 409
    Unauthorized,             // HTTP 401
    Business(String, String), // isError: true, (code, message)
    Network(String),
    Other(String),
}

impl std::fmt::Display for BrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => write!(f, "session not found or expired (404)"),
            Self::Conflict => write!(f, "session conflict (409)"),
            Self::Unauthorized => write!(f, "unauthorized (401)"),
            Self::Business(code, msg) => write!(f, "business error {code}: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Other(msg) => write!(f, "error: {msg}"),
        }
    }
}

impl std::error::Error for BrainError {}

#[derive(Debug, Clone)]
pub struct ReplyOutcome {
    pub accepted: bool,
    pub duplicate: bool,
}

pub struct BrainClient {
    pub base_url: String,
    pub token: String,
    pub http: reqwest::Client,
    pub session_id: Arc<Mutex<Option<String>>>,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl BrainClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            base_url,
            token,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(65))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            session_id: Arc::new(Mutex::new(None)),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
        }
    }

    pub fn with_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = max;
        self
    }

    pub async fn current_session(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    pub async fn clear_session(&self) {
        let mut guard = self.session_id.lock().await;
        *guard = None;
    }

    pub async fn initialize(&self) -> Result<String, BrainError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "open-api-brain",
                    "version": "0.1.0"
                }
            }
        });

        let resp = self
            .http
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| BrainError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(BrainError::Unauthorized);
        }
        if status == 409 {
            return Err(BrainError::Conflict);
        }
        if status != 200 {
            return Err(BrainError::Other(format!("initialize returned status {status}")));
        }

        let sid = resp
            .headers()
            .get("mcp-session-id")
            .ok_or_else(|| BrainError::Other("missing Mcp-Session-Id header".into()))?
            .to_str()
            .map_err(|_| BrainError::Other("invalid Mcp-Session-Id header value".into()))?
            .to_string();

        let mut guard = self.session_id.lock().await;
        *guard = Some(sid.clone());
        Ok(sid)
    }

    pub async fn delete_session(&self) -> Result<(), BrainError> {
        let sid = {
            let mut guard = self.session_id.lock().await;
            guard.take()
        };

        let Some(sid) = sid else {
            return Ok(());
        };

        let _ = self
            .http
            .delete(&self.base_url)
            .bearer_auth(&self.token)
            .header("Mcp-Session-Id", sid)
            .header("Content-Length", "0")
            .send()
            .await;

        Ok(())
    }

    pub async fn next_turn(&self, wait_seconds: u32) -> Result<Option<Value>, BrainError> {
        let sid = {
            let guard = self.session_id.lock().await;
            guard.clone().ok_or(BrainError::SessionNotFound)?
        };

        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "next_turn",
                "arguments": {
                    "wait_seconds": wait_seconds
                }
            }
        });

        let resp = self
            .http
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .header("Mcp-Session-Id", &sid)
            .json(&body)
            .send()
            .await
            .map_err(|e| BrainError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 404 {
            self.clear_session().await;
            return Err(BrainError::SessionNotFound);
        }
        if status == 409 {
            return Err(BrainError::Conflict);
        }
        if status != 200 {
            return Err(BrainError::Other(format!("next_turn returned HTTP {status}")));
        }

        let val: Value = resp
            .json()
            .await
            .map_err(|e| BrainError::Other(format!("failed to parse json: {e}")))?;

        if val.get("isError").and_then(Value::as_bool) == Some(true)
            || val.get("result").and_then(|r| r.get("isError")).and_then(Value::as_bool) == Some(true)
        {
            let res = val.get("result").unwrap_or(&val);
            let code = res["structuredContent"]["error"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let msg = res["structuredContent"]["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            return Err(BrainError::Business(code, msg));
        }

        let result = &val["result"];
        let structured = &result["structuredContent"];
        if structured.get("turn").is_some_and(|t| t.is_null()) {
            return Ok(None);
        }

        // 轮次数据 v1 结构
        if structured.get("turn_id").is_some() {
            return Ok(Some(structured.clone()));
        }

        if let Some(turn) = structured.get("turn").filter(|t| !t.is_null()) {
            return Ok(Some(turn.clone()));
        }

        Ok(None)
    }

    pub async fn reply(
        &self,
        turn_id: &str,
        claim_token: &str,
        action: &str,
        text: Option<&str>,
    ) -> Result<ReplyOutcome, BrainError> {
        let sid = {
            let guard = self.session_id.lock().await;
            guard.clone().ok_or(BrainError::SessionNotFound)?
        };

        let mut args = json!({
            "turn_id": turn_id,
            "claim_token": claim_token,
            "action": action,
        });
        if let Some(t) = text {
            args["text"] = json!(t);
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "reply",
                "arguments": args
            }
        });

        let resp = self
            .http
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .header("Mcp-Session-Id", &sid)
            .json(&body)
            .send()
            .await
            .map_err(|e| BrainError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 404 {
            self.clear_session().await;
            return Err(BrainError::SessionNotFound);
        }
        if status == 409 {
            return Err(BrainError::Conflict);
        }
        if status != 200 {
            return Err(BrainError::Other(format!("reply returned HTTP {status}")));
        }

        let val: Value = resp
            .json()
            .await
            .map_err(|e| BrainError::Other(format!("failed to parse json: {e}")))?;

        if val.get("isError").and_then(Value::as_bool) == Some(true)
            || val.get("result").and_then(|r| r.get("isError")).and_then(Value::as_bool) == Some(true)
        {
            let res = val.get("result").unwrap_or(&val);
            let code = res["structuredContent"]["error"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let msg = res["structuredContent"]["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            return Err(BrainError::Business(code, msg));
        }

        let structured = &val["result"]["structuredContent"];
        let accepted = structured["accepted"].as_bool().unwrap_or(false);
        let duplicate = structured["duplicate"].as_bool().unwrap_or(false);

        Ok(ReplyOutcome { accepted, duplicate })
    }
}

/// 按 chars() 截取前 20 个字，并格式化为 "收到：<前 20 个字>"
pub fn make_reply_text(turn_text: &str) -> String {
    let truncated: String = turn_text.chars().take(20).collect();
    format!("收到：{truncated}")
}

/// 从对外轮次 v1 结构提取文本内容
pub fn extract_text_from_turn(turn: &Value) -> String {
    if let Some(messages) = turn.get("messages").and_then(Value::as_array) {
        for msg in messages {
            if msg.get("kind").and_then(Value::as_str) == Some("text")
                && let Some(t) = msg.get("text").and_then(Value::as_str)
            {
                return t.to_string();
            }
        }
    }
    String::new()
}

/// 核心事件循环
pub async fn run(
    client: &BrainClient,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut current_net_backoff = client.initial_backoff;

    // 确保退出时先 DELETE 会话
    let result = async {
        loop {
            if *stop.borrow() {
                break;
            }

            // 1. 确保会话有效
            if client.current_session().await.is_none() {
                match client.initialize().await {
                    Ok(sid) => {
                        println!("会话初始化成功，Session ID: {sid}");
                        current_net_backoff = client.initial_backoff;
                    }
                    Err(BrainError::Conflict) => {
                        eprintln!("已有活动会话 (409)，退避重试...");
                        tokio::select! {
                            _ = stop.changed() => break,
                            _ = tokio::time::sleep(client.initial_backoff) => {}
                        }
                        continue;
                    }
                    Err(BrainError::Network(e)) => {
                        eprintln!("网络连接错误: {e}，退避 {current_net_backoff:?} 重试...");
                        tokio::select! {
                            _ = stop.changed() => break,
                            _ = tokio::time::sleep(current_net_backoff) => {}
                        }
                        current_net_backoff = (current_net_backoff * 2).min(client.max_backoff);
                        continue;
                    }
                    Err(BrainError::Unauthorized) => {
                        return Err(anyhow::anyhow!("认证失败 (401)：令牌错误或未授权"));
                    }
                    Err(e) => {
                        eprintln!("初始化失败: {e}，退避重试...");
                        tokio::select! {
                            _ = stop.changed() => break,
                            _ = tokio::time::sleep(client.initial_backoff) => {}
                        }
                        continue;
                    }
                }
            }

            // 2. 长轮询等待新轮次
            let turn_res = tokio::select! {
                _ = stop.changed() => break,
                res = client.next_turn(30) => res,
            };

            match turn_res {
                Ok(None) => {
                    // 没有轮次，重置网络退避并继续
                    current_net_backoff = client.initial_backoff;
                }
                Ok(Some(turn)) => {
                    current_net_backoff = client.initial_backoff;
                    let turn_id = turn["turn_id"].as_str().unwrap_or_default();
                    let claim_token = turn["claim"]["claim_token"].as_str().unwrap_or_default();
                    let input_text = extract_text_from_turn(&turn);
                    let reply_text = make_reply_text(&input_text);

                    println!("收到轮次 {turn_id}，输入内容：{input_text:?}，准备回复：{reply_text:?}");

                    match client.reply(turn_id, claim_token, "send", Some(&reply_text)).await {
                        Ok(outcome) => {
                            println!(
                                "轮次 {turn_id} 回复成功：accepted={}, duplicate={}",
                                outcome.accepted, outcome.duplicate
                            );
                        }
                        Err(BrainError::Business(code, msg)) => match code.as_str() {
                            "turn_expired" | "invalid_claim_token" | "turn_revoked" => {
                                eprintln!("轮次 {turn_id} 已失效 ({code}: {msg})，丢弃该轮，继续领取新轮次");
                            }
                            _ => {
                                eprintln!("回复出现业务错误 ({code}): {msg}");
                            }
                        },
                        Err(BrainError::SessionNotFound) => {
                            eprintln!("会话已失效 (404)，准备重新初始化会话");
                        }
                        Err(BrainError::Conflict) => {
                            eprintln!("会话冲突 (409)，退避重试...");
                            tokio::select! {
                                _ = stop.changed() => break,
                                _ = tokio::time::sleep(client.initial_backoff) => {}
                            }
                        }
                        Err(BrainError::Network(e)) => {
                            eprintln!("回复时网络错误: {e}，退避...");
                            tokio::select! {
                                _ = stop.changed() => break,
                                _ = tokio::time::sleep(current_net_backoff) => {}
                            }
                            current_net_backoff = (current_net_backoff * 2).min(client.max_backoff);
                        }
                        Err(e) => {
                            eprintln!("回复未知错误: {e:?}");
                        }
                    }
                }
                Err(BrainError::SessionNotFound) => {
                    eprintln!("会话已失效 (404)，准备重新初始化会话");
                }
                Err(BrainError::Conflict) => {
                    eprintln!("已有活动会话冲突 (409)，退避重试...");
                    tokio::select! {
                        _ = stop.changed() => break,
                        _ = tokio::time::sleep(client.initial_backoff) => {}
                    }
                }
                Err(BrainError::Network(e)) => {
                    eprintln!("长轮询网络错误: {e}，退避 {current_net_backoff:?} 重试...");
                    tokio::select! {
                        _ = stop.changed() => break,
                        _ = tokio::time::sleep(current_net_backoff) => {}
                    }
                    current_net_backoff = (current_net_backoff * 2).min(client.max_backoff);
                }
                Err(BrainError::Business(code, msg)) => {
                    eprintln!("长轮询业务错误 ({code}): {msg}");
                }
                Err(e) => {
                    eprintln!("长轮询错误: {e:?}");
                }
            }
        }
        Ok(())
    }
    .await;

    // 退出时先 DELETE 会话
    println!("正在关闭会话并退出...");
    let _ = client.delete_session().await;
    result
}

static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn handle_signal(_: i32) {
    STOPPING.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn install_signals() {
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    unsafe {
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("OPEN_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8770/mcp".into());
    let token = std::env::var("OPEN_API_TOKEN").map_err(|_| {
        anyhow::anyhow!("缺少环境变量 OPEN_API_TOKEN（请在助手面板 MCP 页面中开启对外接口并复制令牌）")
    })?;

    println!("启动外部大脑，连接目标: {url}");
    let client = BrainClient::new(url, token);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

    install_signals();
    tokio::spawn(async move {
        while !STOPPING.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!("收到中断信号 (Ctrl-C/SIGINT/SIGTERM)，准备退出...");
        let _ = stop_tx.send(true);
    });

    run(&client, stop_rx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct FakeHttpServer {
        addr: std::net::SocketAddr,
        stop_tx: tokio::sync::watch::Sender<bool>,
    }

    impl FakeHttpServer {
        fn url(&self) -> String {
            format!("http://{}/mcp", self.addr)
        }

        fn stop(self) {
            let _ = self.stop_tx.send(true);
        }
    }

    async fn spawn_fake_server<F, Fut>(handler: F) -> FakeHttpServer
    where
        F: Fn(String, String, Vec<(String, String)>, Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = (u16, Option<String>, Value)> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let handler = Arc::new(handler);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_rx.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let handler = handler.clone();
                        tokio::spawn(async move {
                            // 读取请求
                            let mut buf = Vec::new();
                            let mut temp = [0u8; 1024];
                            let head_end = loop {
                                let n = stream.read(&mut temp).await.unwrap_or(0);
                                if n == 0 { return; }
                                buf.extend_from_slice(&temp[..n]);
                                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break pos + 4;
                                }
                            };

                            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                            let mut lines = head.split("\r\n");
                            let first_line = lines.next().unwrap_or_default();
                            let parts: Vec<&str> = first_line.split_whitespace().collect();
                            if parts.len() < 2 { return; }
                            let method = parts[0].to_string();
                            let path = parts[1].to_string();

                            let mut headers = Vec::new();
                            let mut content_length = 0;
                            for line in lines {
                                if let Some((k, v)) = line.split_once(':') {
                                    let key = k.trim().to_ascii_lowercase();
                                    let val = v.trim().to_string();
                                    if key == "content-length" {
                                        content_length = val.parse().unwrap_or(0);
                                    }
                                    headers.push((key, val));
                                }
                            }

                            let mut body_bytes = buf[head_end..].to_vec();
                            while body_bytes.len() < content_length {
                                let n = stream.read(&mut temp).await.unwrap_or(0);
                                if n == 0 { break; }
                                body_bytes.extend_from_slice(&temp[..n]);
                            }

                            let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);

                            let (status, session_header, resp_json) = handler(method, path, headers, body_json).await;

                            let resp_body = if resp_json.is_null() {
                                String::new()
                            } else {
                                resp_json.to_string()
                            };

                            let reason = match status {
                                200 => "OK",
                                202 => "Accepted",
                                204 => "No Content",
                                401 => "Unauthorized",
                                404 => "Not Found",
                                409 => "Conflict",
                                _ => "Error",
                            };

                            let session_str = session_header
                                .map(|s| format!("Mcp-Session-Id: {s}\r\n"))
                                .unwrap_or_default();

                            let head = format!(
                                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{session_str}\r\n",
                                resp_body.len()
                            );

                            let _ = stream.write_all(head.as_bytes()).await;
                            let _ = stream.write_all(resp_body.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        });
                    }
                }
            }
        });

        FakeHttpServer { addr, stop_tx }
    }

    #[test]
    fn example_truncates_by_chars() {
        // 中文和 emoji 不截成半个字符，按 chars() 截取前 20 个字
        let chinese = "这是一个用来测试中文字符截断的长句子超过了二十个字后面的内容会被丢弃";
        let res_cn = make_reply_text(chinese);
        assert_eq!(res_cn, "收到：这是一个用来测试中文字符截断的长句子超过");
        assert_eq!(res_cn.chars().count(), 23); // "收到：" 3 字 + 20 字

        let emoji = "😀🎉👍❤️🔥这是带有表情符号的长字符串超过了二十个字测试截断";
        let res_emoji = make_reply_text(emoji);
        assert_eq!(res_emoji, "收到：😀🎉👍❤️🔥这是带有表情符号的长字符串超");
        assert_eq!(res_emoji.chars().count(), 23);

        // 空串和短字符串
        assert_eq!(make_reply_text(""), "收到：");
        assert_eq!(make_reply_text("你好"), "收到：你好");
    }

    #[tokio::test]
    async fn example_handles_one_turn() {
        let turn_replied = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delete_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let turn_replied_clone = turn_replied.clone();
        let delete_called_clone = delete_called.clone();
        let stop_tx_clone = stop_tx.clone();

        let server = spawn_fake_server(move |method, _path, _headers, body| {
            let turn_replied = turn_replied_clone.clone();
            let delete_called = delete_called_clone.clone();
            let stop_tx = stop_tx_clone.clone();
            async move {
                if method == "DELETE" {
                    delete_called.store(true, Ordering::SeqCst);
                    return (204, None, Value::Null);
                }
                if method == "POST" {
                    let rpc_method = body["method"].as_str().unwrap_or_default();
                    if rpc_method == "initialize" {
                        return (
                            200,
                            Some("test-session-1".into()),
                            json!({
                                "jsonrpc": "2.0",
                                "id": body["id"],
                                "result": {
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "wechat-brain", "version": "1"}
                                }
                            }),
                        );
                    }
                    if rpc_method == "tools/call" {
                        let tool_name = body["params"]["name"].as_str().unwrap_or_default();
                        if tool_name == "next_turn" {
                            if !turn_replied.load(Ordering::SeqCst) {
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "content": [{"type": "text", "text": "..."}],
                                            "isError": false,
                                            "structuredContent": {
                                                "api_version": "1",
                                                "turn_id": "turn-test-1",
                                                "account": {"account_id": "A"},
                                                "contact": {"contact_id": "c1", "display_name": "测试客户", "kind": "private"},
                                                "claim": {"claim_token": "tok-1", "lease_expires_at": "2026-11-09T08:01:00Z"},
                                                "deadline_at": "2026-11-09T08:02:00Z",
                                                "messages": [
                                                    {"kind": "text", "text": "你好客服", "message_id": null, "source": "desktop_ocr"}
                                                ],
                                                "context": [],
                                                "unavailable": []
                                            }
                                        }
                                    }),
                                );
                            } else {
                                // 已经回复过，下次无轮次，触发停止
                                let _ = stop_tx.send(true);
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "content": [{"type": "text", "text": "{\"turn\":null}"}],
                                            "isError": false,
                                            "structuredContent": {"turn": null}
                                        }
                                    }),
                                );
                            }
                        }
                        if tool_name == "reply" {
                            let text = body["params"]["arguments"]["text"].as_str().unwrap_or_default();
                            assert_eq!(text, "收到：你好客服");
                            turn_replied.store(true, Ordering::SeqCst);
                            return (
                                200,
                                None,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": body["id"],
                                    "result": {
                                        "content": [{"type": "text", "text": "..."}],
                                        "isError": false,
                                        "structuredContent": {"accepted": true, "duplicate": false}
                                    }
                                }),
                            );
                        }
                    }
                }
                (404, None, Value::Null)
            }
        })
        .await;

        let client = BrainClient::new(server.url(), "dummy-token".into())
            .with_backoff(Duration::from_millis(10), Duration::from_millis(50));

        let run_res = run(&client, stop_rx).await;
        assert!(run_res.is_ok());
        assert!(turn_replied.load(Ordering::SeqCst));
        assert!(delete_called.load(Ordering::SeqCst));

        server.stop();
    }

    #[tokio::test]
    async fn example_backs_off_on_409() {
        let init_attempts = Arc::new(AtomicUsize::new(0));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let init_attempts_clone = init_attempts.clone();
        let stop_tx_clone = stop_tx.clone();

        let server = spawn_fake_server(move |method, _path, _headers, body| {
            let attempts = init_attempts_clone.clone();
            let stop_tx = stop_tx_clone.clone();
            async move {
                if method == "DELETE" {
                    return (204, None, Value::Null);
                }
                if method == "POST" && body["method"] == "initialize" {
                    let count = attempts.fetch_add(1, Ordering::SeqCst);
                    if count == 0 {
                        // 第一次 409
                        return (409, None, Value::Null);
                    } else {
                        // 第二次成功
                        let _ = stop_tx.send(true);
                        return (
                            200,
                            Some("session-after-409".into()),
                            json!({
                                "jsonrpc": "2.0",
                                "id": body["id"],
                                "result": {
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "wechat-brain", "version": "1"}
                                }
                            }),
                        );
                    }
                }
                (200, None, json!({"result":{"structuredContent":{"turn":null}}}))
            }
        })
        .await;

        let client = BrainClient::new(server.url(), "dummy-token".into())
            .with_backoff(Duration::from_millis(10), Duration::from_millis(50));

        let res = run(&client, stop_rx).await;
        assert!(res.is_ok());
        assert!(init_attempts.load(Ordering::SeqCst) >= 2);

        server.stop();
    }

    #[tokio::test]
    async fn example_reinitializes_on_404() {
        let init_count = Arc::new(AtomicUsize::new(0));
        let next_count = Arc::new(AtomicUsize::new(0));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let init_clone = init_count.clone();
        let next_clone = next_count.clone();
        let stop_tx_clone = stop_tx.clone();

        let server = spawn_fake_server(move |method, _path, _headers, body| {
            let inits = init_clone.clone();
            let nexts = next_clone.clone();
            let stop_tx = stop_tx_clone.clone();
            async move {
                if method == "DELETE" {
                    return (204, None, Value::Null);
                }
                if method == "POST" {
                    if body["method"] == "initialize" {
                        let c = inits.fetch_add(1, Ordering::SeqCst);
                        return (
                            200,
                            Some(format!("sess-{c}")),
                            json!({
                                "jsonrpc": "2.0",
                                "id": body["id"],
                                "result": {
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "wechat-brain", "version": "1"}
                                }
                            }),
                        );
                    }
                    if body["method"] == "tools/call" && body["params"]["name"] == "next_turn" {
                        let nc = nexts.fetch_add(1, Ordering::SeqCst);
                        if nc == 0 {
                            // 第一次调用 next_turn 时模拟 404 (会话已过期)
                            return (404, None, Value::Null);
                        } else {
                            // 重新初始化后的第二次 next_turn
                            let _ = stop_tx.send(true);
                            return (
                                200,
                                None,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": body["id"],
                                    "result": {
                                        "content": [],
                                        "isError": false,
                                        "structuredContent": {"turn": null}
                                    }
                                }),
                            );
                        }
                    }
                }
                (200, None, Value::Null)
            }
        })
        .await;

        let client = BrainClient::new(server.url(), "dummy-token".into())
            .with_backoff(Duration::from_millis(10), Duration::from_millis(50));

        let res = run(&client, stop_rx).await;
        assert!(res.is_ok());
        assert!(init_count.load(Ordering::SeqCst) >= 2);
        assert!(next_count.load(Ordering::SeqCst) >= 2);

        server.stop();
    }

    #[tokio::test]
    async fn example_accepts_duplicate_reply() {
        let duplicate_handled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let dup_clone = duplicate_handled.clone();
        let stop_tx_clone = stop_tx.clone();

        let server = spawn_fake_server(move |method, _path, _headers, body| {
            let dup = dup_clone.clone();
            let stop_tx = stop_tx_clone.clone();
            async move {
                if method == "DELETE" {
                    return (204, None, Value::Null);
                }
                if method == "POST" {
                    let rpc_method = body["method"].as_str().unwrap_or_default();
                    if rpc_method == "initialize" {
                        return (
                            200,
                            Some("test-session-dup".into()),
                            json!({
                                "jsonrpc": "2.0",
                                "id": body["id"],
                                "result": {
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "wechat-brain", "version": "1"}
                                }
                            }),
                        );
                    }
                    if rpc_method == "tools/call" {
                        let tool_name = body["params"]["name"].as_str().unwrap_or_default();
                        if tool_name == "next_turn" {
                            if !dup.load(Ordering::SeqCst) {
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "content": [],
                                            "isError": false,
                                            "structuredContent": {
                                                "api_version": "1",
                                                "turn_id": "turn-dup",
                                                "claim": {"claim_token": "tok-dup"},
                                                "messages": [{"kind": "text", "text": "重试测试"}]
                                            }
                                        }
                                    }),
                                );
                            } else {
                                let _ = stop_tx.send(true);
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "structuredContent": {"turn": null}
                                        }
                                    }),
                                );
                            }
                        }
                        if tool_name == "reply" {
                            dup.store(true, Ordering::SeqCst);
                            return (
                                200,
                                None,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": body["id"],
                                    "result": {
                                        "content": [],
                                        "isError": false,
                                        "structuredContent": {
                                            "accepted": true,
                                            "duplicate": true
                                        }
                                    }
                                }),
                            );
                        }
                    }
                }
                (404, None, Value::Null)
            }
        })
        .await;

        let client = BrainClient::new(server.url(), "dummy-token".into())
            .with_backoff(Duration::from_millis(10), Duration::from_millis(50));

        let res = run(&client, stop_rx).await;
        assert!(res.is_ok());
        assert!(duplicate_handled.load(Ordering::SeqCst));

        server.stop();
    }

    #[tokio::test]
    async fn example_drops_expired_turn() {
        let expired_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let dropped_clone = expired_dropped.clone();
        let stop_tx_clone = stop_tx.clone();

        let server = spawn_fake_server(move |method, _path, _headers, body| {
            let dropped = dropped_clone.clone();
            let stop_tx = stop_tx_clone.clone();
            async move {
                if method == "DELETE" {
                    return (204, None, Value::Null);
                }
                if method == "POST" {
                    let rpc_method = body["method"].as_str().unwrap_or_default();
                    if rpc_method == "initialize" {
                        return (
                            200,
                            Some("test-session-exp".into()),
                            json!({
                                "jsonrpc": "2.0",
                                "id": body["id"],
                                "result": {
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "wechat-brain", "version": "1"}
                                }
                            }),
                        );
                    }
                    if rpc_method == "tools/call" {
                        let tool_name = body["params"]["name"].as_str().unwrap_or_default();
                        if tool_name == "next_turn" {
                            if !dropped.load(Ordering::SeqCst) {
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "content": [],
                                            "isError": false,
                                            "structuredContent": {
                                                "api_version": "1",
                                                "turn_id": "turn-exp",
                                                "claim": {"claim_token": "tok-exp"},
                                                "messages": [{"kind": "text", "text": "超时测试"}]
                                            }
                                        }
                                    }),
                                );
                            } else {
                                let _ = stop_tx.send(true);
                                return (
                                    200,
                                    None,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": body["id"],
                                        "result": {
                                            "structuredContent": {"turn": null}
                                        }
                                    }),
                                );
                            }
                        }
                        if tool_name == "reply" {
                            dropped.store(true, Ordering::SeqCst);
                            return (
                                200,
                                None,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": body["id"],
                                    "result": {
                                        "content": [{"type": "text", "text": "{\"error\":{\"code\":\"turn_expired\",\"message\":\"turn expired\"}}"}],
                                        "isError": true,
                                        "structuredContent": {
                                            "error": {
                                                "code": "turn_expired",
                                                "message": "turn expired"
                                            }
                                        }
                                    }
                                }),
                            );
                        }
                    }
                }
                (404, None, Value::Null)
            }
        })
        .await;

        let client = BrainClient::new(server.url(), "dummy-token".into())
            .with_backoff(Duration::from_millis(10), Duration::from_millis(50));

        let res = run(&client, stop_rx).await;
        assert!(res.is_ok());
        assert!(expired_dropped.load(Ordering::SeqCst));

        server.stop();
    }
}
