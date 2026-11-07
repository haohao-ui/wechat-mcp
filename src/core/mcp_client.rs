use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::core::delivery::ToolClient;
use crate::core::worker_client::Navigator;

struct McpTransport {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

struct McpState {
    transport: Option<McpTransport>,
    counter: u64,
}

pub struct McpSocketClient {
    pub socket_path: PathBuf,
    pub account: Value,
    pub req_timeout: Duration,
    state: Arc<Mutex<McpState>>,
}

impl McpSocketClient {
    pub fn new(socket_path: PathBuf, account_id: String) -> Self {
        Self::with_timeout(socket_path, account_id, Duration::from_secs(40))
    }

    pub fn with_timeout(socket_path: PathBuf, account_id: String, req_timeout: Duration) -> Self {
        Self {
            socket_path,
            account: json!({
                "platform": "wechat",
                "account_id": account_id,
            }),
            req_timeout,
            state: Arc::new(Mutex::new(McpState {
                transport: None,
                counter: 0,
            })),
        }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        home.join(".wechat-rpa").join("worker.sock")
    }

    async fn connect_and_handshake(
        socket_path: &Path,
        req_timeout: Duration,
        counter: &mut u64,
    ) -> Result<McpTransport> {
        let stream = UnixStream::connect(socket_path)
            .await
            .with_context(|| format!("failed to connect to MCP socket at {}", socket_path.display()))?;
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        *counter += 1;
        let init_id = *counter;
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "wechat-agent-bridge",
                    "version": "1"
                }
            }
        });

        let mut req_bytes = serde_json::to_vec(&init_req)?;
        req_bytes.push(b'\n');
        write_half.write_all(&req_bytes).await?;
        write_half.flush().await?;

        // Read initialize reply
        loop {
            let mut line = String::new();
            let n = match timeout(req_timeout, reader.read_line(&mut line)).await {
                Ok(res) => res.context("failed to read line from MCP socket during initialize")?,
                Err(_) => bail!("MCP initialize timed out"),
            };
            if n == 0 {
                bail!("MCP closed before response");
            }
            let reply: Value = serde_json::from_str(&line)
                .context("invalid JSON in MCP initialize response")?;
            if reply.get("id").is_none() {
                continue;
            }
            if reply.get("id").and_then(|v| v.as_u64()) != Some(init_id) {
                bail!("MCP response ID mismatch");
            }
            if let Some(err) = reply.get("error") {
                let code = err.get("code").map(|c| c.to_string()).unwrap_or_default();
                let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let truncated: String = msg.chars().take(512).collect();
                bail!("MCP RPC {code}: {truncated}");
            }
            break;
        }

        // Send notifications/initialized
        let notify = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
        write_half.write_all(notify).await?;
        write_half.flush().await?;

        Ok(McpTransport { reader, writer: write_half })
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let mut state = self.state.lock().await;
        let req_timeout = self.req_timeout;

        let result = Self::request_internal(&mut state, &self.socket_path, req_timeout, method, params).await;
        if result.is_err() {
            // Discard broken transport on error, timeout, or EOF so next call auto-reconnects
            state.transport = None;
        }
        result
    }

    async fn request_internal(
        state: &mut McpState,
        socket_path: &Path,
        req_timeout: Duration,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        if state.transport.is_none() {
            let transport = Self::connect_and_handshake(socket_path, req_timeout, &mut state.counter).await?;
            state.transport = Some(transport);
        }

        let transport = state.transport.as_mut().unwrap();
        state.counter += 1;
        let request_id = state.counter;
        let req = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(b'\n');
        transport.writer.write_all(&req_bytes).await?;
        transport.writer.flush().await?;

        loop {
            let mut line = String::new();
            let n = match timeout(req_timeout, transport.reader.read_line(&mut line)).await {
                Ok(res) => res.context("failed to read line from MCP socket")?,
                Err(_) => bail!("MCP request timed out"),
            };
            if n == 0 {
                bail!("MCP closed before response");
            }
            let reply: Value = serde_json::from_str(&line)
                .context("invalid JSON in MCP response")?;
            if reply.get("id").is_none() {
                continue;
            }
            if reply.get("id").and_then(|v| v.as_u64()) != Some(request_id) {
                bail!("MCP response ID mismatch");
            }
            if let Some(err) = reply.get("error") {
                let code = err.get("code").map(|c| c.to_string()).unwrap_or_default();
                let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let truncated: String = msg.chars().take(512).collect();
                bail!("MCP RPC {code}: {truncated}");
            }
            let res = reply.get("result").cloned().context("MCP response missing result")?;
            return Ok(res);
        }
    }

    pub async fn tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let mut merged = self.account.clone();
        if let Some(obj) = arguments.as_object() {
            if let Some(target) = merged.as_object_mut() {
                for (k, v) in obj {
                    target.insert(k.clone(), v.clone());
                }
            }
        }
        let params = json!({
            "name": name,
            "arguments": merged,
        });

        let result = self.request("tools/call", params).await?;
        if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
            let mut details = Vec::new();
            if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
                for c in content {
                    if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                        details.push(text);
                    }
                }
            }
            let detail_str = details.join(" ");
            let msg = if !detail_str.is_empty() {
                let truncated: String = detail_str.chars().take(300).collect();
                format!("MCP tool failed: {name}: {truncated}")
            } else {
                format!("MCP tool failed: {name}")
            };
            bail!("{msg}");
        }

        let output = if let Some(structured) = result.get("structuredContent") {
            structured.clone()
        } else if let Some(first_text) = result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
        {
            serde_json::from_str::<Value>(first_text)
                .context("failed to parse MCP content[0].text JSON")?
        } else {
            bail!("MCP tool response missing structuredContent or content[0].text");
        };

        if output.get("account") != Some(&self.account) {
            bail!("MCP account mismatch");
        }

        output
            .get("data")
            .cloned()
            .context("MCP tool response missing data")
    }

    pub async fn close(&self) {
        let mut state = self.state.lock().await;
        state.transport = None;
    }
}

impl ToolClient for McpSocketClient {
    fn tool(&self, name: &str, args: Value) -> impl std::future::Future<Output = Result<Value>> + Send {
        let fut = self.tool(name, args);
        async move { fut.await }
    }
}

impl ToolClient for Arc<McpSocketClient> {
    fn tool(&self, name: &str, args: Value) -> impl std::future::Future<Output = Result<Value>> + Send {
        let client = self.clone();
        let name = name.to_string();
        async move {
            McpSocketClient::tool(&client, &name, args).await
        }
    }
}

impl Navigator for McpSocketClient {
    fn get_messages<'a>(
        &'a self,
        recipient: Option<&'a str>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let args = match recipient {
                Some(r) => json!({"recipient": r}),
                None => json!({}),
            };
            self.tool("get_messages", args).await
        })
    }
}

impl Navigator for Arc<McpSocketClient> {
    fn get_messages<'a>(
        &'a self,
        recipient: Option<&'a str>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        (**self).get_messages(recipient)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::UnixListener;

    struct FakeMcpServer {
        pub sock_path: PathBuf,
        pub requests: Arc<Mutex<Vec<Value>>>,
        pub init_count: Arc<AtomicUsize>,
    }

    impl FakeMcpServer {
        async fn start<F>(handler: F) -> Self
        where
            F: Fn(Value) -> Option<Value> + Send + Sync + 'static,
        {
            let unique = uuid::Uuid::new_v4();
            let sock_path = std::env::temp_dir().join(format!("fake_mcp_{unique}.sock"));
            if sock_path.exists() {
                let _ = std::fs::remove_file(&sock_path);
            }
            let listener = UnixListener::bind(&sock_path).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let init_count = Arc::new(AtomicUsize::new(0));

            let reqs = requests.clone();
            let inits = init_count.clone();
            let handler = Arc::new(handler);

            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut reader = BufReader::new(read_half);
                    let reqs = reqs.clone();
                    let inits = inits.clone();
                    let handler = handler.clone();

                    tokio::spawn(async move {
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break;
                            }
                            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                                reqs.lock().await.push(val.clone());
                                if val.get("method").and_then(|m| m.as_str()) == Some("initialize") {
                                    inits.fetch_add(1, Ordering::SeqCst);
                                    let id = val.get("id").cloned().unwrap_or(json!(1));
                                    let resp = json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "protocolVersion": "2025-06-18",
                                            "capabilities": {},
                                            "serverInfo": {"name": "test-server", "version": "1.0"}
                                        }
                                    });
                                    let mut out = serde_json::to_vec(&resp).unwrap();
                                    out.push(b'\n');
                                    let _ = write_half.write_all(&out).await;
                                    let _ = write_half.flush().await;
                                } else if val.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
                                    // Notification: no response
                                } else if let Some(reply) = handler(val) {
                                    let mut out = serde_json::to_vec(&reply).unwrap();
                                    out.push(b'\n');
                                    let _ = write_half.write_all(&out).await;
                                    let _ = write_half.flush().await;
                                }
                            }
                            line.clear();
                        }
                    });
                }
            });

            Self {
                sock_path,
                requests,
                init_count,
            }
        }
    }

    impl Drop for FakeMcpServer {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.sock_path);
        }
    }

    #[tokio::test]
    async fn test_mcp_client_handshake_and_tool_call_structured() {
        let server = FakeMcpServer::start(|val| {
            if val.get("method").and_then(|m| m.as_str()) == Some("tools/call") {
                let id = val.get("id").unwrap();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "structuredContent": {
                            "account": {"platform": "wechat", "account_id": "test_account"},
                            "data": {"status": "ok", "delivered": true}
                        }
                    }
                }))
            } else {
                None
            }
        }).await;

        let client = McpSocketClient::new(server.sock_path.clone(), "test_account".to_string());
        let res = client.tool("test_tool", json!({"foo": "bar"})).await.unwrap();
        assert_eq!(res["status"], "ok");
        assert_eq!(res["delivered"], true);
        assert_eq!(server.init_count.load(Ordering::SeqCst), 1);

        let reqs = server.requests.lock().await;
        assert_eq!(reqs.len(), 3); // initialize, notifications/initialized, tools/call
        let call_args = &reqs[2]["params"]["arguments"];
        assert_eq!(call_args["platform"], "wechat");
        assert_eq!(call_args["account_id"], "test_account");
        assert_eq!(call_args["foo"], "bar");
    }

    #[tokio::test]
    async fn test_mcp_client_content_text_json_fallback() {
        let server = FakeMcpServer::start(|val| {
            if val.get("method").and_then(|m| m.as_str()) == Some("tools/call") {
                let id = val.get("id").unwrap();
                let inner_json = serde_json::to_string(&json!({
                    "account": {"platform": "wechat", "account_id": "acc2"},
                    "data": {"found": 42}
                })).unwrap();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            {"type": "text", "text": inner_json}
                        ]
                    }
                }))
            } else {
                None
            }
        }).await;

        let client = McpSocketClient::new(server.sock_path.clone(), "acc2".to_string());
        let res = client.tool("search", json!({})).await.unwrap();
        assert_eq!(res["found"], 42);
    }

    #[tokio::test]
    async fn test_mcp_client_is_error_exact_format() {
        let server = FakeMcpServer::start(|val| {
            let id = val.get("id").unwrap();
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "isError": true,
                    "content": [
                        {"type": "text", "text": "发送尝试 send-1 未触发发送: 微信未响应"}
                    ]
                }
            }))
        }).await;

        let client = McpSocketClient::new(server.sock_path.clone(), "acc".to_string());
        let err = client.tool("send_message", json!({})).await.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.starts_with("MCP tool failed: send_message: 发送尝试 send-1 未触发发送"),
            "Error was: {err_str}"
        );
        // Verify delivery.rs is_not_sent detects it:
        assert!(crate::core::delivery::is_not_sent(Some(&err_str), None));
    }

    #[tokio::test]
    async fn test_mcp_client_account_mismatch() {
        let server = FakeMcpServer::start(|val| {
            let id = val.get("id").unwrap();
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "structuredContent": {
                        "account": {"platform": "wechat", "account_id": "other_account"},
                        "data": {}
                    }
                }
            }))
        }).await;

        let client = McpSocketClient::new(server.sock_path.clone(), "my_account".to_string());
        let err = client.tool("info", json!({})).await.unwrap_err();
        assert!(err.to_string().contains("MCP account mismatch"));
    }

    #[tokio::test]
    async fn test_mcp_client_auto_reconnect_on_disconnect() {
        let server = FakeMcpServer::start(|val| {
            let id = val.get("id").unwrap();
            let name = val["params"]["name"].as_str().unwrap_or("");
            if name == "drop_connection" {
                // Return None to close connection or don't reply so EOF happens
                None
            } else {
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "structuredContent": {
                            "account": {"platform": "wechat", "account_id": "acc"},
                            "data": {"step": name}
                        }
                    }
                }))
            }
        }).await;

        let client = McpSocketClient::with_timeout(server.sock_path.clone(), "acc".to_string(), Duration::from_millis(200));
        let r1 = client.tool("first", json!({})).await.unwrap();
        assert_eq!(r1["step"], "first");
        assert_eq!(server.init_count.load(Ordering::SeqCst), 1);

        // Force a drop/timeout
        let _ = client.tool("drop_connection", json!({})).await;

        // Next call should automatically reconnect and succeed
        let r2 = client.tool("second", json!({})).await.unwrap();
        assert_eq!(r2["step"], "second");
        assert_eq!(server.init_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_mcp_timeout_raises_timeout_error() {
        let server = FakeMcpServer::start(|val| {
            let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("");
            if method == "ping" {
                // Do not reply, let the client time out
                None
            } else {
                let id = val.get("id").unwrap();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                }))
            }
        }).await;

        let client = McpSocketClient::with_timeout(
            server.sock_path.clone(),
            "test".to_string(),
            Duration::from_millis(50),
        );
        let err = client.request("ping", json!({})).await.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("MCP request timed out"),
            "Expected MCP request timed out, got: {err_str}"
        );
    }
}
