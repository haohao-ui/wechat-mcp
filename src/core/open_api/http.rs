//! JSON-only MCP Streamable HTTP. No operations gateway is reachable here.
use super::{
    access::BrainAccess,
    contract::{HistoryMediaLookup, turn_v1},
    history::RestrictedHistory,
    session::Sessions,
};
use crate::core::{
    config::BotConfig,
    external_brain::{ExternalBrain, StrictReply},
    history::{query::HistoryQuery, serve::constant_time_eq},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    future::Future,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore, watch},
    task::JoinHandle,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpenApiConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
}
impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1".into(),
            port: 8770,
        }
    }
}
pub fn token_path(state_dir: &Path) -> PathBuf {
    state_dir.join("open-api/token")
}
pub fn ensure_token(state_dir: &Path) -> io::Result<String> {
    let path = token_path(state_dir);
    if !path.exists() {
        return rotate_token(state_dir);
    }
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    read_token(state_dir)
}
fn read_token(state_dir: &Path) -> io::Result<String> {
    let token = fs::read_to_string(token_path(state_dir))?;
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid open-api token",
        ));
    }
    Ok(token.into())
}
pub fn rotate_token(state_dir: &Path) -> io::Result<String> {
    let path = token_path(state_dir);
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let mut random = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temp = dir.join(format!(".token-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result?;
    Ok(token)
}

struct State {
    brain: Arc<ExternalBrain>,
    config: Arc<BotConfig>,
    sessions: Mutex<Sessions>,
    addr: SocketAddr,
}
pub struct OpenApiServer {
    addr: SocketAddr,
    stop: watch::Sender<bool>,
    _task: JoinHandle<()>,
}
impl OpenApiServer {
    pub async fn start(
        cfg: &OpenApiConfig,
        brain: Arc<ExternalBrain>,
        config: Arc<BotConfig>,
    ) -> anyhow::Result<Self> {
        Self::start_with_lease(cfg, brain, config, Duration::from_secs(90)).await
    }
    async fn start_with_lease(
        cfg: &OpenApiConfig,
        brain: Arc<ExternalBrain>,
        config: Arc<BotConfig>,
        lease: Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(cfg.enabled, "open_api is disabled");
        let ip: IpAddr = cfg.bind.parse()?;
        anyhow::ensure!(
            !ip.is_unspecified(),
            "open_api.bind must be a specific address"
        );
        ensure_token(&config.state_dir)?;
        let listener = TcpListener::bind((ip, cfg.port)).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            brain,
            config,
            sessions: Mutex::new(Sessions {
                lease,
                ..Default::default()
            }),
            addr,
        });
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            accept_connections(state, stop_rx, || listener.accept()).await;
        });
        tracing::info!(%addr, "open-api listening");
        Ok(Self {
            addr,
            stop,
            _task: task,
        })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn shutdown(self) {
        let _ = self.stop.send(true);
    }
}
impl Drop for OpenApiServer {
    fn drop(&mut self) {
        let _ = self.stop.send(true); /* task performs claim cleanup */
    }
}
impl State {
    async fn cleanup(&self) {
        let token = read_token(&self.config.state_dir).unwrap_or_default();
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.take_expired(&token) {
            self.brain.release_session(&session.id).await;
        }
    }
}

/// The accept source is injectable so recovery can be tested without exhausting
/// the process's file descriptors or depending on an OS-specific socket failure.
async fn accept_connections<F, Fut>(
    state: Arc<State>,
    mut stop_rx: watch::Receiver<bool>,
    mut accept: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = io::Result<(TcpStream, SocketAddr)>>,
{
    let permits = Arc::new(Semaphore::new(64));
    let mut connections = tokio::task::JoinSet::new();
    let mut cleanup = tokio::time::interval(Duration::from_secs(5));
    let mut accept_warnings = HashMap::new();
    let mut limit_warning = None;
    loop {
        tokio::select! {
            _ = stop_rx.changed() => break,
            _ = cleanup.tick() => { state.cleanup().await; }
            accepted = accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let Ok(permit) = permits.clone().try_acquire_owned() else {
                            // Close before reading anything from an excess connection.
                            drop(stream);
                            if warning_due(&mut limit_warning) {
                                tracing::warn!(limit = 64, "open-api connection limit reached");
                            }
                            continue;
                        };
                        let state = state.clone();
                        connections.spawn(async move {
                            let _permit = permit;
                            serve_connection(stream, state).await;
                        });
                    }
                    Err(error) => {
                        if warning_due(accept_warnings.entry(error.kind()).or_default()) {
                            tracing::warn!(%error, "open-api accept failed; retrying");
                        }
                        // Back off persistent failures such as EMFILE, while allowing
                        // shutdown to interrupt the backoff immediately.
                        tokio::select! {
                            _ = stop_rx.changed() => break,
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                        }
                    }
                }
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.end() {
        state.brain.release_session(&session.id).await;
    }
}

fn warning_due(last: &mut Option<Instant>) -> bool {
    if last.is_some_and(|time| time.elapsed() < Duration::from_secs(60)) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}
struct Response {
    status: u16,
    body: Option<Value>,
    session: Option<String>,
}
impl Response {
    fn status(status: u16) -> Self {
        Self {
            status,
            body: None,
            session: None,
        }
    }
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            body: Some(body),
            session: None,
        }
    }
}
async fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    const HEAD: usize = 16 * 1024;
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            if i + 4 > HEAD {
                return Err(431);
            }
            break i + 4;
        }
        if bytes.len() >= HEAD {
            return Err(431);
        }
        let mut chunk = [0u8; 2048];
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| 400u16)?;
    let mut lines = head.split("\r\n");
    let parts: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err(400);
    }
    let method = parts[0].into();
    let path = parts[1].into();
    let mut headers = HashMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (key, value) = line.split_once(':').ok_or(400u16)?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b < 32 && b != b'\t')
        {
            return Err(400);
        }
        if headers
            .insert(key.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return Err(400);
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(400);
    }
    // A bodiless DELETE usually carries no Content-Length (curl, urllib, reqwest); treat it as empty.
    let length = match headers.get("content-length") {
        Some(length) => length.as_str(),
        None if parts[0] == "DELETE" => "0",
        None => return Err(411),
    };
    if length.is_empty() || !length.bytes().all(|b| b.is_ascii_digit()) {
        return Err(400);
    }
    let length: usize = length.parse().map_err(|_| 413u16)?;
    if length > 1024 * 1024 {
        return Err(413);
    }
    let mut body = bytes[end..].to_vec();
    if body.len() > length {
        return Err(400);
    }
    while body.len() < length {
        let mut chunk = [0u8; 8192];
        let take = chunk.len().min(length - body.len());
        let n = stream.read(&mut chunk[..take]).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}
async fn serve_connection(mut stream: TcpStream, state: Arc<State>) {
    let response =
        match tokio::time::timeout(Duration::from_secs(10), read_request(&mut stream)).await {
            Ok(Ok(request)) => dispatch(request, &state).await,
            Ok(Err(status)) => Response::status(status),
            Err(_) => return,
        };
    let body = response.body.map(|v| v.to_string()).unwrap_or_default();
    let session = response
        .session
        .map(|s| format!("Mcp-Session-Id: {s}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{session}\r\n",
        response.status,
        reason(response.status),
        body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await
    })
    .await;
    // Send FIN before closing a rejected request; briefly drain already queued bytes
    // so an unread oversized header does not turn the HTTP error into a TCP reset.
    let _ = stream.shutdown().await;
    if response.status >= 400 {
        let _ = tokio::time::timeout(Duration::from_millis(100), async {
            let mut remaining = 64 * 1024;
            let mut chunk = [0u8; 4096];
            while remaining > 0 {
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                remaining -= n;
            }
            Ok::<_, io::Error>(())
        })
        .await;
    }
}
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    }
}
fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
    Response::json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
fn tool_result(value: Value, error: bool) -> Value {
    let mut result = crate::mcp::make_tool_result(&value.to_string(), error);
    result["structuredContent"] = value;
    result
}
fn business(code: &str, message: impl ToString) -> Value {
    tool_result(
        json!({"error":{"code":code,"message":message.to_string()}}),
        true,
    )
}

async fn dispatch(req: Request, state: &State) -> Response {
    // Invalidate claims even when this particular request still uses the old bearer.
    state.cleanup().await;
    let Ok(token) = read_token(&state.config.state_dir) else {
        return Response::status(401);
    };
    let supplied = req
        .headers
        .get("authorization")
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(supplied.as_bytes(), token.as_bytes()) {
        return Response::status(401);
    }
    if let Some(origin) = req.headers.get("origin") {
        let origins = [
            format!("http://127.0.0.1:{}", state.addr.port()),
            format!("http://localhost:{}", state.addr.port()),
            format!("http://{}", state.addr),
        ];
        if !origins.contains(origin) {
            return Response::status(403);
        }
    }
    if req.path != "/mcp" {
        return Response::status(404);
    }
    if req.method != "POST" && req.method != "DELETE" {
        return Response::status(405);
    }
    if let Some(version) = req.headers.get("mcp-protocol-version")
        && version != "2025-06-18"
        && version != "2024-11-05"
        && version != "2025-03-26"
    {
        return Response::status(400);
    }
    let rpc: Value = if req.method == "POST" {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(_) => return rpc_error(&Value::Null, -32700, "Parse error"),
        }
    } else {
        Value::Null
    };
    let id = rpc.get("id").cloned().unwrap_or(Value::Null);
    if req.method == "POST"
        && (!rpc.is_object()
            || rpc["jsonrpc"] != "2.0"
            || (!rpc["method"].is_string()
                && rpc.get("result").is_none()
                && rpc.get("error").is_none())
            || rpc
                .get("id")
                .is_some_and(|v| !v.is_null() && !v.is_string() && !v.is_number()))
    {
        return rpc_error(&id, -32600, "Invalid Request");
    }
    let initializing =
        req.method == "POST" && rpc["method"] == "initialize" && rpc.get("id").is_some();
    let (session_id, mut ended) = {
        let mut sessions = state.sessions.lock().await;
        if let Some(old) = sessions.take_expired(&token) {
            state.brain.release_session(&old.id).await;
        }
        if initializing {
            if !rpc["params"].is_object() || !rpc["params"]["protocolVersion"].is_string() {
                return rpc_error(&id, -32602, "Invalid params");
            }
            if sessions.active.is_some() {
                return Response::status(409);
            }
            let sid = sessions.initialize(token);
            let (version, _) =
                crate::negotiate_protocol_version(rpc["params"]["protocolVersion"].as_str());
            let mut response = Response::json(
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"wechat-brain","version":"1"}}}),
            );
            response.session = Some(sid);
            return response;
        }
        let Some(sid) = req.headers.get("mcp-session-id") else {
            return Response::status(400);
        };
        let Some(session) = sessions.active.as_mut().filter(|s| s.id == *sid) else {
            return Response::status(404);
        };
        session.last_seen = std::time::Instant::now();
        let session_id = session.id.clone();
        let ended = session.ended.subscribe();
        state.brain.renew_session(&session_id).await;
        if req.method == "DELETE" {
            sessions.end();
            state.brain.release_session(&session_id).await;
            return Response::status(204);
        }
        (session_id, ended)
    };
    if rpc.get("id").is_none() || rpc.get("method").is_none() {
        return Response::status(202);
    }
    let result = match rpc["method"].as_str().unwrap() {
        "ping" => json!({}),
        "tools/list" => json!({"tools":open_tools()}),
        "tools/call" => {
            let Some(name) = rpc["params"]["name"].as_str() else {
                return rpc_error(&id, -32602, "Invalid params");
            };
            let Some(schema) = open_tools().into_iter().find(|t| t["name"] == name) else {
                return rpc_error(&id, -32601, "Method not found");
            };
            let args = rpc["params"].get("arguments").cloned().unwrap_or(json!({}));
            if !valid_args(&schema, &args) {
                return rpc_error(&id, -32602, "Invalid params");
            }
            // No SQLite connection (not Sync) or session guard survives across a long poll.
            let access = match load_access(&state.config) {
                Ok(v) => v,
                Err(e) => {
                    return Response::json(
                        json!({"jsonrpc":"2.0","id":id,"result":business("internal",e)}),
                    );
                }
            };
            let call = call_tool(state, name, &args, &session_id, access);
            tokio::select! {
                _=ended.changed()=>return Response::status(404),
                result=call=>result,
            }
        }
        _ => return rpc_error(&id, -32601, "Method not found"),
    };
    // Serialize claim completion against session deletion, expiry and rotation.
    let mut sessions = state.sessions.lock().await;
    let token = read_token(&state.config.state_dir).unwrap_or_default();
    if let Some(old) = sessions.take_expired(&token) {
        state.brain.release_session(&old.id).await;
    }
    if !sessions.active.as_ref().is_some_and(|s| s.id == session_id) {
        state.brain.release_session(&session_id).await;
        return Response::status(404);
    }
    Response::json(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
fn load_access(config: &BotConfig) -> anyhow::Result<BrainAccess> {
    let conn = rusqlite::Connection::open_with_flags(
        config.state_dir.join("history/history.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    BrainAccess::load(config, &conn)
}
fn valid_args(schema: &Value, args: &Value) -> bool {
    let Some(args) = args.as_object() else {
        return false;
    };
    if schema["inputSchema"]["required"]
        .as_array()
        .is_some_and(|r| r.iter().any(|k| !args.contains_key(k.as_str().unwrap())))
    {
        return false;
    }
    for (key, value) in args {
        let prop = &schema["inputSchema"]["properties"][key];
        match prop["type"].as_str() {
            Some("string") if value.is_string() => {}
            Some("integer") if value.is_i64() || value.is_u64() => {}
            _ => return false,
        }
        if prop["enum"]
            .as_array()
            .is_some_and(|values| !values.contains(value))
        {
            return false;
        }
        if prop["minimum"]
            .as_i64()
            .is_some_and(|min| value.as_i64().is_some_and(|n| n < min))
        {
            return false;
        }
        if let Some(n) = value.as_u64()
            && prop["maximum"].as_u64().is_some_and(|max| n > max)
        {
            return false;
        }
    }
    !(schema["name"] == "reply"
        && args.get("action").is_some_and(|v| v == "send")
        && !args.contains_key("text"))
}
async fn call_tool(
    state: &State,
    name: &str,
    args: &Value,
    session: &str,
    access: BrainAccess,
) -> Value {
    let outcome = match name {
        "next_turn" => match state
            .brain
            .claim_next(
                session,
                Duration::from_secs(args["wait_seconds"].as_u64().unwrap_or(30)),
                &access,
            )
            .await
        {
            Ok(None) => Ok(json!({"turn":null})),
            Ok(Some(claimed)) => {
                let converted = (|| -> anyhow::Result<Value> {
                    let conn = rusqlite::Connection::open_with_flags(
                        state.config.state_dir.join("history/history.sqlite3"),
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )?;
                    let root = state.config.state_dir.join("history/media");
                    Ok(turn_v1(
                        &claimed.turn,
                        &claimed.claim,
                        &claimed.contact_id,
                        &HistoryMediaLookup {
                            conn: &conn,
                            media_root: root,
                        },
                    ))
                })();
                // Same envelope as the idle case, so clients read one field either way.
                converted
                    .map(|turn| json!({ "turn": turn }))
                    .map_err(|e| super::OpenApiError::Internal(e.to_string()))
            }
            Err(e) => Err(e),
        },
        "reply" => state
            .brain
            .reply_strict(
                StrictReply {
                    turn_id: args["turn_id"].as_str().unwrap().into(),
                    claim_token: args["claim_token"].as_str().unwrap().into(),
                    action: args["action"].as_str().unwrap().into(),
                    text: args["text"].as_str().map(str::to_string),
                },
                &access,
            )
            .await
            .map(|v| json!({"accepted":v.accepted,"duplicate":v.duplicate})),
        "get_turn_status" => {
            state
                .brain
                .turn_status(args["turn_id"].as_str().unwrap())
                .await
        }
        _ => {
            let prepared = (|| -> anyhow::Result<_> {
                let query = HistoryQuery::open(
                    state.config.state_dir.join("history/history.sqlite3"),
                    Some(state.config.context_messages as i64),
                )?;
                let restricted = RestrictedHistory::new(query, access);
                if name == "resume_session" {
                    let id = args["contact_id"].as_str().unwrap();
                    restricted.check_contact(id)?;
                    let contact =
                        restricted.call_tool("history_get_contact", &json!({"contact_id":id}))?;
                    Ok(Ok(contact["display_name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()))
                } else {
                    Ok(Err(restricted.call_tool(name, args)))
                }
            })();
            match prepared {
                Ok(Ok(display_name)) => {
                    return match state.brain.resume_contact(&display_name).await {
                        Ok(()) => tool_result(json!({"resumed":true}), false),
                        Err(e) => business("internal", e),
                    };
                }
                Ok(Err(Ok(value))) => return tool_result(value, false),
                Ok(Err(Err(e))) => return business(&e.code, &e.message),
                Err(e) => {
                    if let Some(q) = e.downcast_ref::<crate::core::history::query::QueryError>() {
                        return business(&q.code, &q.message);
                    }
                    return business("internal", e);
                }
            }
        }
    };
    match outcome {
        Ok(value) => tool_result(value, false),
        Err(e) => business(e.code(), format!("{e:?}")),
    }
}

pub fn open_tool_names() -> Vec<&'static str> {
    vec![
        "next_turn",
        "reply",
        "get_turn_status",
        "resume_session",
        "history_list_contacts",
        "history_get_contact",
        "history_get_recent_messages",
        "history_get_history",
        "history_search_messages",
        "history_get_message",
        "history_get_media",
    ]
}
fn open_tools() -> Vec<Value> {
    let definitions = [
        (
            "next_turn",
            json!({"wait_seconds":{"type":"integer","minimum":0,"maximum":60,"default":30}}),
            json!([]),
        ),
        (
            "reply",
            json!({"turn_id":{"type":"string"},"claim_token":{"type":"string"},"action":{"type":"string","enum":["send","skip","handoff"]},"text":{"type":"string"}}),
            json!(["turn_id", "claim_token", "action"]),
        ),
        (
            "get_turn_status",
            json!({"turn_id":{"type":"string"}}),
            json!(["turn_id"]),
        ),
        (
            "resume_session",
            json!({"contact_id":{"type":"string"}}),
            json!(["contact_id"]),
        ),
    ];
    let mut tools:Vec<_>=definitions.into_iter().map(|(name,props,required)|json!({"name":name,"description":name,"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false}})).collect();
    tools.extend(RestrictedHistory::tools());
    tools
}
pub async fn self_check(cfg: &OpenApiConfig, state_dir: &Path) -> Value {
    let result=async {
        anyhow::ensure!(cfg.enabled,"open_api is disabled");
        let token=read_token(state_dir)?;
        let url=format!("http://{}/mcp",SocketAddr::new(cfg.bind.parse()?,cfg.port));
        let client=reqwest::Client::builder().timeout(Duration::from_secs(5)).build()?;
        let response=client.post(&url).bearer_auth(&token).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"self-check","version":"1"}}})).send().await?;
        if response.status()==409 {return Ok::<_,anyhow::Error>(json!({"ok":false,"busy":true,"tools":[],"error":"brain_already_connected"}));}
        let response=response.error_for_status()?;
        let sid=response.headers().get("mcp-session-id").ok_or_else(||anyhow::anyhow!("missing session"))?.to_str()?.to_string();
        let listed=client.post(&url).bearer_auth(&token).header("Mcp-Session-Id",&sid).json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).send().await;
        let listed=match listed {Ok(r)=>match r.error_for_status() {Ok(r)=>r.json::<Value>().await.map_err(anyhow::Error::from),Err(e)=>Err(e.into())},Err(e)=>Err(e.into())};
        let deleted=client.delete(&url).bearer_auth(&token).header("Mcp-Session-Id",&sid).header("Content-Length","0").send().await?;
        anyhow::ensure!(deleted.status()==204,"failed to end self-check session");
        let listed=listed?;
        let tools=listed["result"]["tools"].as_array().ok_or_else(||anyhow::anyhow!("invalid tools response"))?.iter().map(|t|t["name"].clone()).collect::<Vec<_>>();
        anyhow::ensure!(tools==open_tool_names().into_iter().map(Value::from).collect::<Vec<_>>(),"unexpected tools");
        Ok(json!({"ok":true,"busy":false,"tools":tools,"error":null}))
    }.await;
    result.unwrap_or_else(|e| json!({"ok":false,"busy":false,"tools":[],"error":e.to_string()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{config::BrainMode, open_api::compat_tests::Fixture};
    struct TestServer {
        server: OpenApiServer,
        fixture: Fixture,
        brain: Arc<ExternalBrain>,
        token: String,
        client: reqwest::Client,
    }
    impl TestServer {
        async fn new(lease: Duration) -> Self {
            let fixture = Fixture::new();
            fixture.contact("allowed", "A", "允许", "private", None);
            fixture.contact("forbidden", "A", "禁止", "private", None);
            let mut config =
                fixture.config(json!([{"recipient":"允许","kind":"private"}]), json!([]));
            config.brain.mode = BrainMode::External;
            config.brain.external.timeout_seconds = 120;
            let brain =
                ExternalBrain::new(&config.brain, &config.model_config, fixture.state.clone());
            let server = OpenApiServer::start_with_lease(
                &OpenApiConfig {
                    enabled: true,
                    port: 0,
                    ..Default::default()
                },
                brain.clone(),
                Arc::new(config),
                lease,
            )
            .await
            .unwrap();
            let token = read_token(&fixture.state).unwrap();
            Self {
                server,
                fixture,
                brain,
                token,
                client: reqwest::Client::new(),
            }
        }
        fn url(&self) -> String {
            format!("http://{}/mcp", self.server.local_addr())
        }
        async fn rpc(&self, sid: Option<&str>, method: &str, params: Value) -> reqwest::Response {
            let mut req = self
                .client
                .post(self.url())
                .bearer_auth(&self.token)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}));
            if let Some(sid) = sid {
                req = req.header("Mcp-Session-Id", sid);
            }
            req.send().await.unwrap()
        }
        async fn init(&self) -> String {
            let r=self.rpc(None,"initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}})).await;
            assert_eq!(r.status(), 200);
            r.headers()["mcp-session-id"].to_str().unwrap().into()
        }
        async fn call(&self, sid: &str, name: &str, args: Value) -> Value {
            self.rpc(
                Some(sid),
                "tools/call",
                json!({"name":name,"arguments":args}),
            )
            .await
            .json()
            .await
            .unwrap()
        }
        async fn enqueue(&self, id: &str) -> JoinHandle<anyhow::Result<crate::brain::BrainReply>> {
            let turn:crate::brain::Turn=serde_json::from_value(json!({"turn_id":id,"account_id":"A","contact":{"name":"允许","kind":"private"},"text":"hello","images":["/private/local/image.png"]})).unwrap();
            let brain = self.brain.brain_fn();
            let handle = tokio::spawn(async move { brain(&turn).await });
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if self.brain.state.lock().await.pending.contains_key(id) {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            handle
        }
        async fn raw(&self, request: &str) -> String {
            let mut stream = TcpStream::connect(self.server.local_addr()).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
            let mut out = String::new();
            if let Err(error) = stream.read_to_string(&mut out).await {
                assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
                assert!(!out.is_empty(), "reset before HTTP rejection");
            }
            out
        }
    }
    #[tokio::test]
    async fn accept_error_does_not_stop_server() {
        let mut t = TestServer::new(Duration::from_secs(90)).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = t
            .fixture
            .config(json!([{"recipient":"允许","kind":"private"}]), json!([]));
        let state = Arc::new(State {
            brain: t.brain.clone(),
            config: Arc::new(config),
            sessions: Mutex::new(Sessions::default()),
            addr,
        });
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut fail_next = true;
            accept_connections(state, stop_rx, || {
                let fail = std::mem::take(&mut fail_next);
                let listener = &listener;
                async move {
                    if fail {
                        Err(io::Error::from(io::ErrorKind::ConnectionAborted))
                    } else {
                        listener.accept().await
                    }
                }
            })
            .await;
        });
        t.server = OpenApiServer {
            addr,
            stop,
            _task: task,
        };
        let sid = tokio::time::timeout(Duration::from_secs(1), t.init())
            .await
            .unwrap();
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            200
        );
    }

    #[tokio::test]
    async fn connection_limit_rejects_excess() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let mut hanging = Vec::new();
        for _ in 0..64 {
            hanging.push(TcpStream::connect(t.server.local_addr()).await.unwrap());
        }
        let mut excess = TcpStream::connect(t.server.local_addr()).await.unwrap();
        let mut byte = [0u8; 1];
        match tokio::time::timeout(Duration::from_secs(1), excess.read(&mut byte))
            .await
            .unwrap()
        {
            Ok(0) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            other => panic!("excess connection was not closed: {other:?}"),
        }
        let mut first = hanging.remove(0);
        first
            .write_all(
                format!(
                    "GET /mcp HTTP/1.1\r\nContent-Length: 0\r\nAuthorization: Bearer {}\r\n\r\n",
                    t.token
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        first.shutdown().await.unwrap();
        let mut response = String::new();
        first.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 405"));
        let sid = tokio::time::timeout(Duration::from_secs(1), t.init())
            .await
            .unwrap();
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            200
        );
    }

    #[tokio::test]
    async fn rejects_missing_or_wrong_token() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let pending = t.enqueue("unauthorized").await;
        for token in ["", "wrong"] {
            assert_eq!(
                t.client
                    .post(t.url())
                    .bearer_auth(token)
                    .body("{}")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401
            );
        }
        assert!(
            t.brain.state.lock().await.pending["unauthorized"]
                .claim_token
                .is_none()
        );
        pending.abort();
    }
    #[tokio::test]
    async fn rejects_cross_origin() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        assert_eq!(
            t.client
                .post(t.url())
                .bearer_auth(&t.token)
                .header("Origin", "http://evil.test")
                .body("{}")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    #[tokio::test]
    async fn second_initialize_conflicts() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        t.init().await;
        assert_eq!(
            t.rpc(None, "initialize", json!({"protocolVersion":"2025-06-18"}))
                .await
                .status(),
            409
        );
    }
    #[tokio::test]
    async fn session_expires_and_frees_slot() {
        let t = TestServer::new(Duration::from_secs(1)).await;
        let sid = t.init().await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            404
        );
        assert_ne!(t.init().await, sid);
    }
    #[tokio::test]
    async fn delete_without_content_length_ends_session() {
        // curl -X DELETE and most HTTP clients send no Content-Length for an empty body.
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let request = format!(
            "DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nMcp-Session-Id: {}\r\n\r\n",
            t.token, sid
        );
        let response = t.raw(&request).await;
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            404
        );
        let next = t.init().await;
        assert_ne!(next, sid, "a new session can start right away");
    }
    #[tokio::test]
    async fn delete_ends_session_and_requeues() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let pending = t.enqueue("turn-delete").await;
        let first = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        assert!(first["result"]["structuredContent"]["turn"]["claim"]["claim_token"].is_string());
        assert_eq!(
            t.client
                .delete(t.url())
                .bearer_auth(&t.token)
                .header("Mcp-Session-Id", &sid)
                .header("Content-Length", "0")
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            404
        );
        let sid = t.init().await;
        let second = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        assert_eq!(
            second["result"]["structuredContent"]["turn"]["turn_id"],
            "turn-delete"
        );
        assert_ne!(
            first["result"]["structuredContent"]["turn"]["claim"]["claim_token"],
            second["result"]["structuredContent"]["turn"]["claim"]["claim_token"]
        );
        pending.abort();
    }
    #[tokio::test]
    async fn tools_list_is_whitelist_only() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let value: Value = t
            .rpc(Some(&sid), "tools/list", json!({}))
            .await
            .json()
            .await
            .unwrap();
        let names: Vec<_> = value["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, open_tool_names());
        assert_eq!(names.len(), 11);
        assert!(!names.contains(&"send_message"));
        assert!(!names.contains(&"desktop_observe"));
    }
    #[tokio::test]
    async fn unknown_tool_is_method_not_found() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        assert_eq!(
            t.call(&sid, "send_message", json!({})).await["error"]["code"],
            -32601
        );
    }
    #[tokio::test]
    async fn notification_returns_202() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let r = t
            .client
            .post(t.url())
            .bearer_auth(&t.token)
            .header("Mcp-Session-Id", sid)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 202);
        assert!(r.bytes().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn next_turn_returns_v1_without_paths() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let pending = t.enqueue("v1").await;
        let r = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        assert_eq!(r["result"]["structuredContent"]["turn"]["api_version"], "1");
        assert_eq!(
            r["result"]["structuredContent"]["turn"]["contact"]["contact_id"],
            "allowed"
        );
        assert!(!r.to_string().contains("/private/local"));
        pending.abort();
        let empty = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        assert_eq!(empty["result"]["structuredContent"], json!({"turn":null}));
    }
    #[tokio::test]
    async fn long_poll_and_reply_in_parallel() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let pending = t.enqueue("parallel").await;
        let claim = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        let polling = t.call(&sid, "next_turn", json!({"wait_seconds":30}));
        tokio::pin!(polling);
        tokio::select! {r=&mut polling=>panic!("poll returned early: {r}"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
        let reply=tokio::time::timeout(Duration::from_secs(1),t.call(&sid,"reply",json!({"turn_id":"parallel","claim_token":claim["result"]["structuredContent"]["turn"]["claim"]["claim_token"],"action":"send","text":"好的"}))).await.unwrap();
        assert_eq!(reply["result"]["structuredContent"]["accepted"], true);
        assert!(pending.await.unwrap().is_ok());
        let next = t.enqueue("parallel-next").await;
        assert_eq!(
            polling.await["result"]["structuredContent"]["turn"]["turn_id"],
            "parallel-next"
        );
        next.abort();
    }
    #[tokio::test]
    async fn malformed_requests_do_not_block_long_poll() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let polling = t.call(&sid, "next_turn", json!({"wait_seconds":30}));
        tokio::pin!(polling);
        tokio::select! {r=&mut polling=>panic!("poll returned early: {r}"),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
        assert!(
            t.raw("POST /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .starts_with("HTTP/1.1 411")
        );
        assert!(
            t.raw("POST /mcp HTTP/1.1\r\nContent-Length: 2097152\r\n\r\n")
                .await
                .starts_with("HTTP/1.1 413")
        );
        assert!(
            t.raw(&format!(
                "POST /mcp HTTP/1.1\r\nX-Long: {}\r\n\r\n",
                "a".repeat(20 * 1024)
            ))
            .await
            .starts_with("HTTP/1.1 431")
        );
        let mut slow = TcpStream::connect(t.server.local_addr()).await.unwrap();
        slow.write_all(b"POST /mcp HTTP/1.1\r\nContent-Length: 100\r\n\r\n{")
            .await
            .unwrap();
        assert!(
            t.raw("POST /mcp HTTP/1.1\r\nContent-Length: 100\r\n\r\n{")
                .await
                .starts_with("HTTP/1.1 400")
        );
        let pending = t.enqueue("after-malformed").await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut polling)
                .await
                .unwrap()["result"]["structuredContent"]["turn"]["turn_id"],
            "after-malformed"
        );
        drop(slow);
        pending.abort();
    }
    #[tokio::test]
    async fn token_rotation_terminates_session() {
        let mut t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let pending = t.enqueue("rotate").await;
        let claim = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        let new_token = rotate_token(&t.fixture.state).unwrap();
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            401
        );
        t.token = new_token;
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            404
        );
        let new_sid = t.init().await;
        let again = t
            .call(&new_sid, "next_turn", json!({"wait_seconds":0}))
            .await;
        assert_eq!(
            again["result"]["structuredContent"]["turn"]["turn_id"],
            "rotate"
        );
        assert_ne!(
            claim["result"]["structuredContent"]["turn"]["claim"],
            again["result"]["structuredContent"]["turn"]["claim"]
        );
        pending.abort();
    }
    #[tokio::test]
    async fn refuses_unspecified_bind() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        for bind in ["0.0.0.0", "::"] {
            assert!(
                OpenApiServer::start(
                    &OpenApiConfig {
                        enabled: true,
                        bind: bind.into(),
                        port: 0
                    },
                    t.brain.clone(),
                    Arc::new(t.fixture.config(json!([]), json!([])))
                )
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("must be a specific address")
            );
        }
    }
    #[tokio::test]
    async fn disabled_by_default_starts_nothing() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let fixture = Fixture::new();
        let config = Arc::new(fixture.config(json!([]), json!([])));
        assert!(!config.open_api.enabled);
        assert!(
            OpenApiServer::start(&Default::default(), t.brain.clone(), config)
                .await
                .is_err()
        );
        assert!(!token_path(&fixture.state).exists());
    }
    #[tokio::test]
    async fn self_check_reports_busy_when_session_active() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let cfg = OpenApiConfig {
            enabled: true,
            port: t.server.local_addr().port(),
            ..Default::default()
        };
        let r = self_check(&cfg, &t.fixture.state).await;
        assert_eq!(r["busy"], true);
        assert_eq!(r["ok"], false);
        assert_eq!(
            t.rpc(Some(&sid), "tools/list", json!({})).await.status(),
            200
        );
    }
    #[tokio::test]
    async fn self_check_initializes_lists_and_deletes() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let cfg = OpenApiConfig {
            enabled: true,
            port: t.server.local_addr().port(),
            ..Default::default()
        };
        assert_eq!(self_check(&cfg, &t.fixture.state).await["ok"], true);
        t.init().await;
    }
    #[tokio::test]
    async fn end_to_end_claim_reply_status() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        let pending = t.enqueue("e2e").await;
        let claim = t.call(&sid, "next_turn", json!({"wait_seconds":0})).await;
        let args = json!({"turn_id":"e2e","claim_token":claim["result"]["structuredContent"]["turn"]["claim"]["claim_token"],"action":"send","text":"收到，谢谢"});
        let reply = t.call(&sid, "reply", args.clone()).await;
        assert_eq!(
            reply["result"]["structuredContent"],
            json!({"accepted":true,"duplicate":false})
        );
        let received = pending.await.unwrap().unwrap();
        assert_eq!(received.metadata["action"], "send");
        assert_eq!(received.reply, "收到，谢谢");
        assert_eq!(
            t.call(&sid, "reply", args).await["result"]["structuredContent"]["duplicate"],
            true
        );
        let status = t
            .call(&sid, "get_turn_status", json!({"turn_id":"e2e"}))
            .await;
        assert_eq!(status["result"]["structuredContent"]["state"], "accepted");
    }
    #[tokio::test]
    async fn history_tool_forbidden_contact_not_found() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        for name in ["history_get_contact", "resume_session"] {
            let r = t.call(&sid, name, json!({"contact_id":"forbidden"})).await;
            assert_eq!(r["result"]["isError"], true);
            assert_eq!(
                r["result"]["structuredContent"]["error"]["code"],
                "not_found"
            );
        }
    }
    #[test]
    fn config_rejects_unknown_open_api_field() {
        let f = Fixture::new();
        let path = f.state.join("config.json");
        f.config(json!([]), json!([]));
        let mut value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value["open_api"] = json!({"enabled":true,"unknown":1});
        fs::write(&path, value.to_string()).unwrap();
        assert!(
            BotConfig::load(&path)
                .unwrap_err()
                .to_string()
                .contains("invalid open_api")
        );
        value["open_api"] = json!({"enabled":true,"port":8770});
        fs::write(&path, value.to_string()).unwrap();
        assert!(BotConfig::load(path).unwrap().open_api.enabled);
    }
    #[tokio::test]
    async fn invalid_tool_arguments_are_rpc_errors() {
        let t = TestServer::new(Duration::from_secs(90)).await;
        let sid = t.init().await;
        for args in [
            json!({"wait_seconds":61}),
            json!({"wait_seconds":-1}),
            json!({"wait_seconds":0.5}),
            json!({"wait_seconds":"0"}),
        ] {
            assert_eq!(
                t.call(&sid, "next_turn", args).await["error"]["code"],
                -32602
            );
        }
        assert_eq!(
            t.call(&sid, "reply", json!({"turn_id":"x","action":"send"}))
                .await["error"]["code"],
            -32602
        );
    }
}
