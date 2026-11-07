//! Read-only HTTP and MCP servers over conversation history.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::query::{HistoryQuery, QueryError};

/// Base64 character set for standard encoding.
const B64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encode without line breaks.
fn encode_base64(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(B64_CHARS[((n >> 18) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Constant-time comparison between two byte slices.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Ensure an authentication token exists in `<history_dir>/http-token`.
pub fn ensure_token(history_dir: &Path) -> io::Result<String> {
    let path = history_dir.join("http-token");
    if !path.exists() {
        // Generate 32 bytes of secure random data
        let mut random_bytes = [0u8; 32];
        let mut read_ok = false;
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            if f.read_exact(&mut random_bytes).is_ok() {
                read_ok = true;
            }
        }
        if !read_ok {
            let u1 = uuid::Uuid::new_v4();
            let u2 = uuid::Uuid::new_v4();
            random_bytes[..16].copy_from_slice(u1.as_bytes());
            random_bytes[16..].copy_from_slice(u2.as_bytes());
        }

        // URL-safe base64 encoding without padding (matches Python secrets.token_urlsafe(32))
        const URL_SAFE_CHARS: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut token = String::with_capacity(43);
        for chunk in random_bytes.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

            let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
            token.push(URL_SAFE_CHARS[((n >> 18) & 63) as usize] as char);
            token.push(URL_SAFE_CHARS[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                token.push(URL_SAFE_CHARS[((n >> 6) & 63) as usize] as char);
            }
            if chunk.len() > 2 {
                token.push(URL_SAFE_CHARS[(n & 63) as usize] as char);
            }
        }

        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true).mode(0o600);
        match opts.open(&path) {
            Ok(mut file) => {
                file.write_all(token.as_bytes())?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Another process created it concurrently
            }
            Err(e) => return Err(e),
        }
    }

    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    let content = fs::read_to_string(&path)?;
    Ok(content.trim().to_string())
}

// ---------------------------------------------------------------------------
// MCP Server Implementation
// ---------------------------------------------------------------------------

struct ToolMeta {
    method: &'static str,
    desc: &'static str,
    props: &'static [(&'static str, &'static str)],
    required: &'static [&'static str],
    order_enum: bool,
}

const MCP_TOOLS: &[(&str, ToolMeta)] = &[
    (
        "history_list_contacts",
        ToolMeta {
            method: "list_contacts",
            desc: "列出联系人（当前名称、全部别名、消息数、最后消息时间）",
            props: &[
                ("query", "string"),
                ("cursor", "string"),
                ("limit", "integer"),
            ],
            required: &[],
            order_enum: false,
        },
    ),
    (
        "history_get_contact",
        ToolMeta {
            method: "get_contact",
            desc: "按 contact_id 或名称（表情/符号不敏感）获取联系人",
            props: &[("contact_id", "string"), ("name", "string")],
            required: &[],
            order_enum: false,
        },
    ),
    (
        "history_get_recent_messages",
        ToolMeta {
            method: "get_recent_messages",
            desc: "某联系人最近 N 条消息（旧→新）",
            props: &[("contact_id", "string"), ("limit", "integer")],
            required: &["contact_id"],
            order_enum: false,
        },
    ),
    (
        "history_get_history",
        ToolMeta {
            method: "get_history",
            desc: "某联系人完整历史，按时间分页",
            props: &[
                ("contact_id", "string"),
                ("cursor", "string"),
                ("since", "integer"),
                ("until", "integer"),
                ("limit", "integer"),
                ("order", "string"),
            ],
            required: &["contact_id"],
            order_enum: true,
        },
    ),
    (
        "history_search_messages",
        ToolMeta {
            method: "search_messages",
            desc: "全文搜索消息（含图片/文件的模型理解要点），返回命中消息及上下文消息 ID",
            props: &[
                ("q", "string"),
                ("contact_id", "string"),
                ("cursor", "string"),
                ("since", "integer"),
                ("until", "integer"),
                ("limit", "integer"),
            ],
            required: &["q"],
            order_enum: false,
        },
    ),
    (
        "history_get_message",
        ToolMeta {
            method: "get_message",
            desc: "单条消息详情（含 meta；附件含来源 original/chat_thumbnail 与理解要点 understanding）",
            props: &[("message_id", "string")],
            required: &["message_id"],
            order_enum: false,
        },
    ),
    (
        "history_get_media",
        ToolMeta {
            method: "get_media",
            desc: "附件元数据（来源、理解要点 understanding）与 base64 内容（≤10MB）",
            props: &[("media_id", "string")],
            required: &["media_id"],
            order_enum: false,
        },
    ),
];

/// Return list of tools formatted exactly as in Python mcp_server.tool_list().
pub fn mcp_tools_list() -> Value {
    let list: Vec<Value> = MCP_TOOLS
        .iter()
        .map(|(name, meta)| {
            let mut props_map = serde_json::Map::new();
            for &(pname, ptype) in meta.props {
                let mut prop_obj = serde_json::Map::new();
                prop_obj.insert("type".to_string(), json!(ptype));
                if pname == "order" && meta.order_enum {
                    prop_obj.insert("enum".to_string(), json!(["asc", "desc"]));
                }
                props_map.insert(pname.to_string(), Value::Object(prop_obj));
            }
            json!({
                "name": name,
                "description": meta.desc,
                "inputSchema": {
                    "type": "object",
                    "properties": Value::Object(props_map),
                    "required": meta.required,
                    "additionalProperties": false
                }
            })
        })
        .collect();
    Value::Array(list)
}

fn call_tool(query: &HistoryQuery, name: &str, arguments: &Value) -> Result<Value, QueryError> {
    let tool_entry = MCP_TOOLS.iter().find(|(tname, _)| *tname == name);
    let (_, meta) = match tool_entry {
        Some(t) => t,
        None => return Err(QueryError::invalid_argument("unknown tool")),
    };

    let args_obj = match arguments.as_object() {
        Some(obj) => obj,
        None => return Err(QueryError::invalid_argument("invalid arguments")),
    };

    let allowed_props: HashSet<&str> = meta.props.iter().map(|(p, _)| *p).collect();
    for k in args_obj.keys() {
        if !allowed_props.contains(k.as_str()) {
            return Err(QueryError::invalid_argument("invalid arguments"));
        }
    }
    for &req in meta.required {
        if !args_obj.contains_key(req) {
            return Err(QueryError::invalid_argument("invalid arguments"));
        }
    }

    for &(pname, ptype) in meta.props {
        if let Some(val) = args_obj.get(pname) {
            if ptype == "integer" {
                if !val.is_i64() && !val.is_u64() {
                    return Err(QueryError::invalid_argument(format!(
                        "{} must be integer",
                        pname
                    )));
                }
            } else if ptype == "string" && !val.is_string() {
                return Err(QueryError::invalid_argument(format!(
                    "{} must be string",
                    pname
                )));
            }
        }
    }

    match meta.method {
        "list_contacts" => {
            let q = args_obj.get("query").and_then(|v| v.as_str());
            let cursor = args_obj.get("cursor").and_then(|v| v.as_str());
            let limit = args_obj.get("limit").and_then(|v| v.as_i64());
            query.list_contacts(q, cursor, limit)
        }
        "get_contact" => {
            let contact_id = args_obj.get("contact_id").and_then(|v| v.as_str());
            let name = args_obj.get("name").and_then(|v| v.as_str());
            query.get_contact(contact_id, name)
        }
        "get_recent_messages" => {
            let contact_id = args_obj.get("contact_id").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args_obj.get("limit").and_then(|v| v.as_i64());
            query.get_recent_messages(contact_id, limit)
        }
        "get_history" => {
            let contact_id = args_obj.get("contact_id").and_then(|v| v.as_str()).unwrap_or("");
            let since = args_obj.get("since").and_then(|v| v.as_i64());
            let until = args_obj.get("until").and_then(|v| v.as_i64());
            let cursor = args_obj.get("cursor").and_then(|v| v.as_str());
            let limit = args_obj.get("limit").and_then(|v| v.as_i64());
            let order = args_obj.get("order").and_then(|v| v.as_str());
            query.get_history(contact_id, since, until, cursor, limit, order)
        }
        "search_messages" => {
            let q = args_obj.get("q").and_then(|v| v.as_str()).unwrap_or("");
            let contact_id = args_obj.get("contact_id").and_then(|v| v.as_str());
            let since = args_obj.get("since").and_then(|v| v.as_i64());
            let until = args_obj.get("until").and_then(|v| v.as_i64());
            let cursor = args_obj.get("cursor").and_then(|v| v.as_str());
            let limit = args_obj.get("limit").and_then(|v| v.as_i64());
            query.search_messages(q, contact_id, since, until, cursor, limit)
        }
        "get_message" => {
            let message_id = args_obj.get("message_id").and_then(|v| v.as_str()).unwrap_or("");
            query.get_message(message_id)
        }
        "get_media" => {
            let media_id = args_obj.get("media_id").and_then(|v| v.as_str()).unwrap_or("");
            let (info, path) = query.get_media(media_id)?;
            let bytes = fs::read(&path).map_err(|e| QueryError::new("read_failed", e.to_string()))?;
            let b64 = encode_base64(&bytes);
            Ok(json!({
                "media": info,
                "base64": b64
            }))
        }
        _ => Err(QueryError::invalid_argument("unknown tool")),
    }
}

/// Handle a single MCP JSON-RPC request against a HistoryQuery instance.
pub fn handle_mcp_request(query: &HistoryQuery, request: &Value) -> Option<Value> {
    let rid = request.get("id")?;
    if rid.is_null() {
        return None;
    }
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

    let response = if method == "initialize" {
        json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wechat-history", "version": "1"}
            }
        })
    } else if method == "tools/list" {
        json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "tools": mcp_tools_list()
            }
        })
    } else if method == "tools/call" {
        let params = request.get("params");
        let name = params.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
        let default_args = Value::Object(serde_json::Map::new());
        let arguments = params.and_then(|p| p.get("arguments")).unwrap_or(&default_args);

        match call_tool(query, name, arguments) {
            Ok(value) => {
                let text = serde_json::to_string(&value).unwrap_or_default();
                json!({
                    "jsonrpc": "2.0",
                    "id": rid,
                    "result": {
                        "content": [{"type": "text", "text": text}],
                        "isError": false
                    }
                })
            }
            Err(error) => {
                let err_obj = json!({"error": {"code": error.code, "message": error.message}});
                let text = serde_json::to_string(&err_obj).unwrap_or_default();
                json!({
                    "jsonrpc": "2.0",
                    "id": rid,
                    "result": {
                        "content": [{"type": "text", "text": text}],
                        "isError": true
                    }
                })
            }
        }
    } else if method == "ping" {
        json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {}
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": rid,
            "error": {"code": -32601, "message": "method not found"}
        })
    };

    Some(response)
}

/// Run read-only stdio MCP loop over conversation history.
pub fn run_mcp_stdio(query: HistoryQuery, reader: impl BufRead, mut writer: impl Write) -> io::Result<()> {
    for line in reader.lines() {
        let l = line?;
        if l.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&l) {
            Ok(req) => {
                if let Some(resp) = handle_mcp_request(&query, &req) {
                    let out = serde_json::to_string(&resp).unwrap_or_default();
                    writer.write_all(out.as_bytes())?;
                    writer.write_all(b"\n")?;
                    writer.flush()?;
                }
            }
            Err(_) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {"code": -32700, "message": "parse error"}
                });
                let out = serde_json::to_string(&err_resp).unwrap_or_default();
                writer.write_all(out.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP Server Implementation
// ---------------------------------------------------------------------------

fn is_hex_str(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn is_contact_id(s: &str) -> bool {
    s.starts_with("c_") && is_hex_str(&s[2..], 16)
}

fn is_message_id(s: &str) -> bool {
    s.starts_with("m_") && is_hex_str(&s[2..], 16)
}

fn is_media_id(s: &str) -> bool {
    is_hex_str(s, 64)
}

#[derive(Debug, PartialEq)]
enum ActionRoute<'a> {
    Contacts,
    GetContact(&'a str),
    GetRecentMessages(&'a str),
    GetHistory(&'a str),
    SearchMessages,
    GetMessage(&'a str),
    GetMedia(&'a str),
}

fn match_route(path: &str) -> Option<ActionRoute<'_>> {
    if path == "/v1/contacts" {
        return Some(ActionRoute::Contacts);
    }
    if path == "/v1/search" {
        return Some(ActionRoute::SearchMessages);
    }
    if let Some(rest) = path.strip_prefix("/v1/contacts/") {
        if let Some(contact_id) = rest.strip_suffix("/recent") {
            if is_contact_id(contact_id) {
                return Some(ActionRoute::GetRecentMessages(contact_id));
            }
        } else if let Some(contact_id) = rest.strip_suffix("/messages") {
            if is_contact_id(contact_id) {
                return Some(ActionRoute::GetHistory(contact_id));
            }
        } else if is_contact_id(rest) {
            return Some(ActionRoute::GetContact(rest));
        }
        return None;
    }
    if let Some(rest) = path.strip_prefix("/v1/messages/") {
        if is_message_id(rest) {
            return Some(ActionRoute::GetMessage(rest));
        }
        return None;
    }
    if let Some(rest) = path.strip_prefix("/v1/media/") {
        if is_media_id(rest) {
            return Some(ActionRoute::GetMedia(rest));
        }
        return None;
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut bytes = Vec::new();
    let in_bytes = s.as_bytes();
    let mut i = 0;
    while i < in_bytes.len() {
        if in_bytes[i] == b'+' {
            bytes.push(b' ');
            i += 1;
        } else if in_bytes[i] == b'%' && i + 2 < in_bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                std::str::from_utf8(&in_bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                bytes.push(val);
                i += 3;
            } else {
                bytes.push(in_bytes[i]);
                i += 1;
            }
        } else {
            bytes.push(in_bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn parse_query_pairs(query: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    if query.is_empty() {
        return map;
    }
    for item in query.split('&') {
        if item.is_empty() {
            continue;
        }
        let (k, v) = match item.split_once('=') {
            Some((k, v)) => (k, v),
            None => (item, ""),
        };
        let decoded_k = url_decode(k);
        let decoded_v = url_decode(v);
        map.entry(decoded_k).or_default().push(decoded_v);
    }
    map
}

fn is_int_param_valid(s: &str) -> bool {
    let trimmed = s.strip_prefix('-').unwrap_or(s);
    !trimmed.is_empty() && trimmed.len() <= 16 && trimmed.chars().all(|c| c.is_ascii_digit())
}

fn guess_mime_type(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn send_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nServer: wechat-history/1\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status, status_text, content_type, body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn send_json(stream: &mut TcpStream, status: u16, status_text: &str, val: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(val).unwrap_or_default();
    send_response(
        stream,
        status,
        status_text,
        "application/json; charset=utf-8",
        &body,
    )
}

fn send_error(stream: &mut TcpStream, status: u16, status_text: &str, code: &str, message: &str) -> io::Result<()> {
    let val = json!({"error": {"code": code, "message": message}});
    send_json(stream, status, status_text, &val)
}

fn dispatch_http(
    query: &HistoryQuery,
    action: ActionRoute,
    query_str: &str,
) -> Result<(u16, &'static str, String, Vec<u8>), QueryError> {
    let pairs = parse_query_pairs(query_str);

    let allowed: &[&str] = match action {
        ActionRoute::Contacts => &["query", "cursor", "limit", "name"],
        ActionRoute::GetContact(_) => &[],
        ActionRoute::GetRecentMessages(_) => &["limit"],
        ActionRoute::GetHistory(_) => &["since", "until", "cursor", "limit", "order"],
        ActionRoute::SearchMessages => &["q", "contact_id", "since", "until", "cursor", "limit"],
        ActionRoute::GetMessage(_) => &[],
        ActionRoute::GetMedia(_) => &[],
    };

    let allowed_set: HashSet<&str> = allowed.iter().copied().collect();
    for (k, v) in &pairs {
        if !allowed_set.contains(k.as_str()) || v.len() != 1 {
            return Err(QueryError::invalid_argument(
                "unsupported or repeated query parameter",
            ));
        }
    }

    let mut single_args: HashMap<String, String> = HashMap::new();
    let mut int_args: HashMap<String, i64> = HashMap::new();

    for (k, v) in pairs {
        let val_str = v.into_iter().next().unwrap();
        if k == "limit" || k == "since" || k == "until" {
            if !is_int_param_valid(&val_str) {
                return Err(QueryError::invalid_argument(format!(
                    "{} must be an integer",
                    k
                )));
            }
            let parsed: i64 = val_str
                .parse()
                .map_err(|_| QueryError::invalid_argument(format!("{} must be an integer", k)))?;
            int_args.insert(k.clone(), parsed);
        }
        single_args.insert(k, val_str);
    }

    match action {
        ActionRoute::Contacts => {
            let res = if let Some(name) = single_args.get("name") {
                query.get_contact(None, Some(name))?
            } else {
                let q = single_args.get("query").map(|s| s.as_str());
                let cursor = single_args.get("cursor").map(|s| s.as_str());
                let limit = int_args.get("limit").copied();
                query.list_contacts(q, cursor, limit)?
            };
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::GetContact(contact_id) => {
            let res = query.get_contact(Some(contact_id), None)?;
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::GetRecentMessages(contact_id) => {
            let limit = int_args.get("limit").copied();
            let res = query.get_recent_messages(contact_id, limit)?;
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::GetHistory(contact_id) => {
            let since = int_args.get("since").copied();
            let until = int_args.get("until").copied();
            let cursor = single_args.get("cursor").map(|s| s.as_str());
            let limit = int_args.get("limit").copied();
            let order = single_args.get("order").map(|s| s.as_str());
            let res = query.get_history(contact_id, since, until, cursor, limit, order)?;
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::SearchMessages => {
            let q = match single_args.get("q") {
                Some(q) => q,
                None => return Err(QueryError::invalid_argument("q is required")),
            };
            let contact_id = single_args.get("contact_id").map(|s| s.as_str());
            let since = int_args.get("since").copied();
            let until = int_args.get("until").copied();
            let cursor = single_args.get("cursor").map(|s| s.as_str());
            let limit = int_args.get("limit").copied();
            let res = query.search_messages(q, contact_id, since, until, cursor, limit)?;
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::GetMessage(message_id) => {
            let res = query.get_message(message_id)?;
            let body = serde_json::to_vec(&res).unwrap_or_default();
            Ok((200, "OK", "application/json; charset=utf-8".to_string(), body))
        }
        ActionRoute::GetMedia(media_id) => {
            let (_info, path) = query.get_media(media_id)?;
            let bytes = fs::read(&path).map_err(|e| QueryError::new("read_failed", e.to_string()))?;
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let mime = guess_mime_type(filename).to_string();
            Ok((200, "OK", mime, bytes))
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    query_lock: &Arc<Mutex<HistoryQuery>>,
    token: &str,
) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut temp = [0u8; 1024];

    loop {
        let n = stream.read(&mut temp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&temp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.windows(2).any(|w| w == b"\n\n") {
            break;
        }
        if buf.len() > 65536 {
            break;
        }
    }

    let req_str = String::from_utf8_lossy(&buf);
    let mut lines = req_str.lines();
    let req_line = match lines.next() {
        Some(l) if !l.trim().is_empty() => l,
        _ => return Ok(()),
    };

    let parts: Vec<&str> = req_line.split_whitespace().collect();
    if parts.len() < 2 {
        return send_error(&mut stream, 400, "Bad Request", "invalid_argument", "malformed request line");
    }

    let method = parts[0];
    let uri = parts[1];

    if method != "GET" {
        return send_error(
            &mut stream,
            405,
            "Method Not Allowed",
            "method_not_allowed",
            "read-only API: GET only",
        );
    }

    let mut auth_supplied = "";
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("authorization") {
                auth_supplied = v.trim();
            }
        }
    }

    let expected_auth = format!("Bearer {}", token);
    if !constant_time_eq(auth_supplied.as_bytes(), expected_auth.as_bytes()) {
        return send_error(
            &mut stream,
            401,
            "Unauthorized",
            "unauthorized",
            "bearer token required",
        );
    }

    let (path, query_str) = match uri.split_once('?') {
        Some((p, q)) => (p, q),
        None => (uri, ""),
    };

    let action = match match_route(path) {
        Some(a) => a,
        None => {
            return send_error(&mut stream, 404, "Not Found", "not_found", "unknown route");
        }
    };

    let dispatch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let guard = query_lock.lock().unwrap();
        dispatch_http(&guard, action, query_str)
    }));

    match dispatch_result {
        Ok(Ok((status, status_text, content_type, body))) => {
            send_response(&mut stream, status, status_text, &content_type, &body)?;
        }
        Ok(Err(err)) => {
            if err.code == "not_found" {
                send_error(&mut stream, 404, "Not Found", &err.code, &err.message)?;
            } else if err.code == "invalid_argument" {
                send_error(&mut stream, 400, "Bad Request", &err.code, &err.message)?;
            } else {
                let log_entry = json!({
                    "event": "history_http_error",
                    "error_type": err.code
                });
                eprintln!("{}", serde_json::to_string(&log_entry).unwrap_or_default());
                send_error(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "internal",
                    "internal error",
                )?;
            }
        }
        Err(panic_err) => {
            let error_type = if panic_err.is::<&str>() || panic_err.is::<String>() {
                "Panic"
            } else {
                "RuntimeError"
            };
            let log_entry = json!({
                "event": "history_http_error",
                "error_type": error_type
            });
            eprintln!("{}", serde_json::to_string(&log_entry).unwrap_or_default());
            send_error(
                &mut stream,
                500,
                "Internal Server Error",
                "internal",
                "internal error",
            )?;
        }
    }

    Ok(())
}

/// HTTP server hosting read-only history endpoints.
pub struct HttpServer {
    addr: SocketAddr,
    shutdown_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl HttpServer {
    pub fn start(
        query: Arc<Mutex<HistoryQuery>>,
        token: String,
        bind: &str,
        port: u16,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((bind, port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let flag_clone = shutdown_flag.clone();

        let handle = thread::spawn(move || {
            while !flag_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let query_ref = query.clone();
                        let token_str = token.clone();
                        thread::spawn(move || {
                            let _ = handle_connection(stream, &query_ref, &token_str);
                        });
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        });

        Ok(Self {
            addr,
            shutdown_flag,
            handle: Some(handle),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn shutdown(mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Unit and Integration Tests (TDD Translated from test_history_servers.py)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::core::config::BotConfig;
    use crate::core::history::store::HistoryStore;
    use serde_json::json;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            let p = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            let _ = fs::create_dir_all(&p);
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct TestFixture {
        _temp: TempDir,
        state: PathBuf,
        path: PathBuf,
        contact_id: String,
    }

    impl TestFixture {
        fn setup() -> Self {
            let temp = TempDir::new("history-serve-test");
            let base = temp.path();
            let state = base.join("state");
            let path = base.join("config.json");
            fs::write(
                &path,
                json!({
                    "schema_version": 1,
                    "host": "local",
                    "account_id": "acct",
                    "allowlist": [{"recipient": "🍓小美", "kind": "private"}],
                    "state_dir": state.to_string_lossy(),
                })
                .to_string(),
            )
            .unwrap();

            let store = HistoryStore::open(&state).unwrap();
            let contact_id = store
                .ensure_contact("acct", &json!({"recipient": "🍓小美", "kind": "private"}), true)
                .unwrap();
            store
                .record_frame(
                    &contact_id,
                    &json!({
                        "bubbles": [
                            {"text": "还是上次那个款", "digest": "还是上次那个款", "direction": "inbound_visual"},
                            {"text": "已发顺丰", "digest": "已发顺丰", "direction": "outbound_visual"},
                            {"text": "最后一条", "digest": "最后一条", "direction": "inbound_visual"}
                        ]
                    }),
                    None,
                    None,
                    false,
                )
                .unwrap();

            Self {
                _temp: temp,
                state,
                path,
                contact_id,
            }
        }
    }

    // Helper for making raw HTTP requests
    fn http_request(
        port: u16,
        method: &str,
        path: &str,
        auth_header: Option<&str>,
    ) -> (u16, HashMap<String, String>, Vec<u8>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\n", method, path);
        if let Some(auth) = auth_header {
            req.push_str(&format!("Authorization: {}\r\n", auth));
        }
        req.push_str("Connection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();

        let mut resp_bytes = Vec::new();
        stream.read_to_end(&mut resp_bytes).unwrap();

        let split_pos = resp_bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(resp_bytes.len());
        let header_str = String::from_utf8_lossy(&resp_bytes[..split_pos]);
        let body = if split_pos + 4 <= resp_bytes.len() {
            resp_bytes[split_pos + 4..].to_vec()
        } else {
            Vec::new()
        };

        let mut lines = header_str.lines();
        let status_line = lines.next().unwrap_or("");
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }

        (status, headers, body)
    }

    // MCP Tests -------------------------------------------------------------

    #[test]
    fn test_history_serve_mcp_lists_tools_and_returns_history() {
        let f = TestFixture::setup();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = HistoryQuery::open(&db_path, None).unwrap();

        let reqs = vec![
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "history_get_recent_messages", "arguments": {"contact_id": f.contact_id}}
            }),
        ];

        let mut out = Vec::new();
        for r in reqs {
            if let Some(resp) = handle_mcp_request(&query, &r) {
                out.push(resp);
            }
        }

        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "wechat-history");
        assert_eq!(out[0]["result"]["protocolVersion"], "2024-11-05");

        let tools = out[1]["result"]["tools"].as_array().unwrap();
        let names: HashSet<String> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        let expected: HashSet<String> = [
            "history_list_contacts",
            "history_get_contact",
            "history_get_recent_messages",
            "history_get_history",
            "history_search_messages",
            "history_get_message",
            "history_get_media",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(names, expected);

        let content_text = out[2]["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(content_text).unwrap();
        let items = parsed["items"].as_array().unwrap();
        assert_eq!(items.last().unwrap()["text"], "最后一条");
    }

    #[test]
    fn test_history_serve_mcp_errors() {
        let f = TestFixture::setup();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = HistoryQuery::open(&db_path, None).unwrap();

        let reqs = vec![
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "history_get_message", "arguments": {"message_id": "m_missing"}}
            }),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "history_get_history", "arguments": {"contact_id": f.contact_id, "bogus": 1}}
            }),
            json!({"jsonrpc": "2.0", "id": 3, "method": "nope"}),
        ];

        let mut out = Vec::new();
        for r in reqs {
            if let Some(resp) = handle_mcp_request(&query, &r) {
                out.push(resp);
            }
        }

        assert!(out[0]["result"]["isError"].as_bool().unwrap());
        assert!(out[1]["result"]["isError"].as_bool().unwrap());
        assert_eq!(out[2]["error"]["code"], -32601);
    }

    // HTTP Tests ------------------------------------------------------------

    #[test]
    fn test_history_serve_http_requires_token_and_only_get() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();

        // No token -> 401
        let (status, _, _) = http_request(port, "GET", "/v1/contacts", None);
        assert_eq!(status, 401);

        // Wrong token -> 401
        let (status, _, _) = http_request(port, "GET", "/v1/contacts", Some("Bearer wrong"));
        assert_eq!(status, 401);

        // Non-GET -> 405
        let (status, _, _) = http_request(port, "POST", "/v1/contacts", Some(&format!("Bearer {}", token)));
        assert_eq!(status, 405);
        let (status, _, _) = http_request(port, "DELETE", "/v1/contacts", Some(&format!("Bearer {}", token)));
        assert_eq!(status, 405);

        server.shutdown();
    }

    #[test]
    fn test_history_serve_http_history_and_search() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();
        let auth = Some(format!("Bearer {}", token));

        // /v1/contacts/<id>/messages?limit=2
        let (status, _, body) = http_request(
            port,
            "GET",
            &format!("/v1/contacts/{}/messages?limit=2", f.contact_id),
            auth.as_deref(),
        );
        assert_eq!(status, 200);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["items"].as_array().unwrap().len(), 2);
        assert!(val["next_cursor"].is_string());

        // /v1/search?q=上次那个
        let (status, _, body) = http_request(
            port,
            "GET",
            "/v1/search?q=%E4%B8%8A%E6%AC%A1%E9%82%A3%E4%B8%AA",
            auth.as_deref(),
        );
        assert_eq!(status, 200);
        let val: Value = serde_json::from_slice(&body).unwrap();
        let texts: Vec<&str> = val["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, vec!["还是上次那个款"]);

        // /v1/contacts/<id>/recent?limit=1
        let (status, _, body) = http_request(
            port,
            "GET",
            &format!("/v1/contacts/{}/recent?limit=1", f.contact_id),
            auth.as_deref(),
        );
        assert_eq!(status, 200);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["items"][0]["text"], "最后一条");

        server.shutdown();
    }

    #[test]
    fn test_history_serve_http_errors() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();
        let auth = Some(format!("Bearer {}", token));

        let (status, _, _) = http_request(port, "GET", "/v1/messages/m_missing", auth.as_deref());
        assert_eq!(status, 404);

        let (status, _, _) = http_request(
            port,
            "GET",
            &format!("/v1/contacts/{}/messages?limit=999", f.contact_id),
            auth.as_deref(),
        );
        assert_eq!(status, 400);

        let (status, _, _) = http_request(
            port,
            "GET",
            &format!("/v1/contacts/{}/messages?limit=abc", f.contact_id),
            auth.as_deref(),
        );
        assert_eq!(status, 400);

        let (status, _, _) = http_request(port, "GET", "/v1/unknown", auth.as_deref());
        assert_eq!(status, 404);

        server.shutdown();
    }

    #[test]
    fn test_history_serve_http_emoji_query_and_traversal_are_safe() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();
        let auth = Some(format!("Bearer {}", token));

        // Emoji query with traversal characters
        let (status, _, _) = http_request(
            port,
            "GET",
            "/v1/contacts?query=%F0%9F%8D%93%E5%B0%8F%E7%BE%8E%2F..%2F",
            auth.as_deref(),
        );
        assert_eq!(status, 200);

        // Path traversal in media -> 404
        let (status, _, _) = http_request(
            port,
            "GET",
            "/v1/media/..%2F..%2Fhistory.sqlite3",
            auth.as_deref(),
        );
        assert_eq!(status, 404);

        // Contacts query by name
        let (status, _, body) = http_request(
            port,
            "GET",
            "/v1/contacts?name=%E5%B0%8F%E7%BE%8E",
            auth.as_deref(),
        );
        assert_eq!(status, 200);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["contact_id"], f.contact_id);

        server.shutdown();
    }

    #[test]
    fn test_history_serve_http_unexpected_error_is_json_500() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();
        let auth = Some(format!("Bearer {}", token));

        // Trigger a database error by dropping the table via PRAGMA foreign_keys = OFF
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE contacts;").unwrap();
        }

        let (status, _, body) = http_request(port, "GET", "/v1/contacts", auth.as_deref());
        assert_eq!(status, 500);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["error"]["code"], "internal");
        let body_str = String::from_utf8_lossy(&body);
        assert!(!body_str.contains("secret"));
        assert!(!body_str.contains("contacts"));

        server.shutdown();
    }

    #[test]
    fn test_history_serve_http_default_bind_is_loopback() {
        let f = TestFixture::setup();
        let cfg = BotConfig::load(&f.path).unwrap();
        assert_eq!(cfg.history_http.bind, "127.0.0.1");
    }

    #[test]
    fn test_history_serve_token_file_private_and_stable() {
        let f = TestFixture::setup();
        let history_dir = f.state.join("history");
        let token = ensure_token(&history_dir).unwrap();
        let path = history_dir.join("http-token");

        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let token2 = ensure_token(&history_dir).unwrap();
        assert_eq!(token, token2);
    }

    #[test]
    fn test_history_serve_parallel_requests_succeed() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();

        let mut threads = Vec::new();
        for _ in 0..16 {
            let cid = f.contact_id.clone();
            let tok = token.clone();
            threads.push(thread::spawn(move || {
                let (status, _, _) = http_request(
                    port,
                    "GET",
                    &format!("/v1/contacts/{}/messages?limit=3", cid),
                    Some(&format!("Bearer {}", tok)),
                );
                status
            }));
        }

        let mut results = Vec::new();
        for t in threads {
            results.push(t.join().unwrap());
        }

        assert_eq!(results, vec![200; 16]);
        server.shutdown();
    }

    #[test]
    fn test_history_serve_mcp_stdio_stream() {
        let f = TestFixture::setup();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = HistoryQuery::open(&db_path, None).unwrap();

        let input_lines = format!(
            "\n   \nnot-a-json\n{}\n{}\n",
            json!({"jsonrpc": "2.0", "method": "notifications/foo"}),
            json!({"jsonrpc": "2.0", "id": 42, "method": "ping"})
        );
        let cursor_in = io::Cursor::new(input_lines.as_bytes());
        let mut cursor_out = Vec::new();

        run_mcp_stdio(query, cursor_in, &mut cursor_out).unwrap();

        let out_str = String::from_utf8(cursor_out).unwrap();
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), 2);

        // 1. Parse error for invalid JSON
        let err_obj: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(err_obj["error"]["code"], -32700);

        // 2. Ping response for id 42
        let ping_obj: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(ping_obj["id"], 42);
        assert_eq!(ping_obj["result"], json!({}));
    }

    #[test]
    fn test_history_serve_get_media_both_http_and_mcp() {
        let f = TestFixture::setup();
        let history_dir = f.state.join("history");
        let token = ensure_token(&history_dir).unwrap();
        let db_path = history_dir.join("history.sqlite3");
        let store = HistoryStore::open(&f.state).unwrap();

        // Find a message to attach media to
        let query = HistoryQuery::open(&db_path, None).unwrap();
        let hist = query.get_recent_messages(&f.contact_id, Some(1)).unwrap();
        let msg_id = hist["items"][0]["message_id"].as_str().unwrap();

        // Create a test image file
        let img_path = f._temp.path().join("test.png");
        let sample_png_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRfixture";
        fs::write(&img_path, sample_png_bytes).unwrap();

        let media_id = store
            .attach_media(
                msg_id,
                &img_path,
                "image",
                Some("test.png"),
                "chat_thumbnail",
                None,
            )
            .unwrap();

        // Test 1: MCP history_get_media
        let mcp_resp = handle_mcp_request(
            &query,
            &json!({
                "jsonrpc": "2.0",
                "id": 100,
                "method": "tools/call",
                "params": {
                    "name": "history_get_media",
                    "arguments": {"media_id": media_id}
                }
            }),
        )
        .unwrap();
        assert_eq!(mcp_resp["result"]["isError"], false);
        let content_text = mcp_resp["result"]["content"][0]["text"].as_str().unwrap();
        let mcp_val: Value = serde_json::from_str(content_text).unwrap();
        assert_eq!(mcp_val["media"]["media_id"], media_id);
        assert_eq!(mcp_val["media"]["filename"], "test.png");
        let expected_b64 = encode_base64(sample_png_bytes);
        assert_eq!(mcp_val["base64"], expected_b64);

        // Test 2: HTTP GET /v1/media/<media_id>
        let server = HttpServer::start(
            Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap())),
            token.clone(),
            "127.0.0.1",
            0,
        )
        .unwrap();
        let port = server.port();

        let (status, headers, body) = http_request(
            port,
            "GET",
            &format!("/v1/media/{}", media_id),
            Some(&format!("Bearer {}", token)),
        );
        assert_eq!(status, 200);
        assert_eq!(headers.get("content-type").unwrap(), "image/png");
        assert_eq!(body, sample_png_bytes);

        server.shutdown();
    }

    // Exact Match Tests with Python Outputs ---------------------------------

    #[test]
    fn test_history_serve_tools_list_exact_match() {
        // Output obtained directly from python3 scripts/wechat_bot/history/mcp_server.py
        let python_tools_json = r#"[
  {
    "name": "history_list_contacts",
    "description": "列出联系人（当前名称、全部别名、消息数、最后消息时间）",
    "inputSchema": {
      "type": "object",
      "properties": {
        "query": {
          "type": "string"
        },
        "cursor": {
          "type": "string"
        },
        "limit": {
          "type": "integer"
        }
      },
      "required": [],
      "additionalProperties": false
    }
  },
  {
    "name": "history_get_contact",
    "description": "按 contact_id 或名称（表情/符号不敏感）获取联系人",
    "inputSchema": {
      "type": "object",
      "properties": {
        "contact_id": {
          "type": "string"
        },
        "name": {
          "type": "string"
        }
      },
      "required": [],
      "additionalProperties": false
    }
  },
  {
    "name": "history_get_recent_messages",
    "description": "某联系人最近 N 条消息（旧→新）",
    "inputSchema": {
      "type": "object",
      "properties": {
        "contact_id": {
          "type": "string"
        },
        "limit": {
          "type": "integer"
        }
      },
      "required": [
        "contact_id"
      ],
      "additionalProperties": false
    }
  },
  {
    "name": "history_get_history",
    "description": "某联系人完整历史，按时间分页",
    "inputSchema": {
      "type": "object",
      "properties": {
        "contact_id": {
          "type": "string"
        },
        "cursor": {
          "type": "string"
        },
        "since": {
          "type": "integer"
        },
        "until": {
          "type": "integer"
        },
        "limit": {
          "type": "integer"
        },
        "order": {
          "type": "string",
          "enum": [
            "asc",
            "desc"
          ]
        }
      },
      "required": [
        "contact_id"
      ],
      "additionalProperties": false
    }
  },
  {
    "name": "history_search_messages",
    "description": "全文搜索消息（含图片/文件的模型理解要点），返回命中消息及上下文消息 ID",
    "inputSchema": {
      "type": "object",
      "properties": {
        "q": {
          "type": "string"
        },
        "contact_id": {
          "type": "string"
        },
        "cursor": {
          "type": "string"
        },
        "since": {
          "type": "integer"
        },
        "until": {
          "type": "integer"
        },
        "limit": {
          "type": "integer"
        }
      },
      "required": [
        "q"
      ],
      "additionalProperties": false
    }
  },
  {
    "name": "history_get_message",
    "description": "单条消息详情（含 meta；附件含来源 original/chat_thumbnail 与理解要点 understanding）",
    "inputSchema": {
      "type": "object",
      "properties": {
        "message_id": {
          "type": "string"
        }
      },
      "required": [
        "message_id"
      ],
      "additionalProperties": false
    }
  },
  {
    "name": "history_get_media",
    "description": "附件元数据（来源、理解要点 understanding）与 base64 内容（≤10MB）",
    "inputSchema": {
      "type": "object",
      "properties": {
        "media_id": {
          "type": "string"
        }
      },
      "required": [
        "media_id"
      ],
      "additionalProperties": false
    }
  }
]"#;
        let expected: Value = serde_json::from_str(python_tools_json).unwrap();
        let actual = mcp_tools_list();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_history_serve_mcp_error_formats_exact_match() {
        let f = TestFixture::setup();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = HistoryQuery::open(&db_path, None).unwrap();

        // 1. Unknown tool
        let resp = handle_mcp_request(
            &query,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "foo"}}),
        )
        .unwrap();
        assert_eq!(
            resp,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": "{\"error\":{\"code\":\"invalid_argument\",\"message\":\"unknown tool\"}}"}],
                    "isError": true
                }
            })
        );

        // 2. Missing required argument
        let resp = handle_mcp_request(
            &query,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "history_get_message", "arguments": {}}}),
        )
        .unwrap();
        assert_eq!(
            resp,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "content": [{"type": "text", "text": "{\"error\":{\"code\":\"invalid_argument\",\"message\":\"invalid arguments\"}}"}],
                    "isError": true
                }
            })
        );

        // 3. Wrong type argument
        let resp = handle_mcp_request(
            &query,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "history_get_message", "arguments": {"message_id": 123}}}),
        )
        .unwrap();
        assert_eq!(
            resp,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": {
                    "content": [{"type": "text", "text": "{\"error\":{\"code\":\"invalid_argument\",\"message\":\"message_id must be string\"}}"}],
                    "isError": true
                }
            })
        );
    }

    #[test]
    fn test_history_serve_http_error_formats_exact_match() {
        let f = TestFixture::setup();
        let token = ensure_token(&f.state.join("history")).unwrap();
        let db_path = f.state.join("history").join("history.sqlite3");
        let query = Arc::new(Mutex::new(HistoryQuery::open(&db_path, None).unwrap()));
        let server = HttpServer::start(query, token.clone(), "127.0.0.1", 0).unwrap();
        let port = server.port();

        // 401 unauthorized
        let (status, _, body) = http_request(port, "GET", "/v1/contacts", None);
        assert_eq!(status, 401);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            val,
            json!({"error": {"code": "unauthorized", "message": "bearer token required"}})
        );

        // 405 method not allowed
        let (status, _, body) = http_request(port, "POST", "/v1/contacts", Some(&format!("Bearer {}", token)));
        assert_eq!(status, 405);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            val,
            json!({"error": {"code": "method_not_allowed", "message": "read-only API: GET only"}})
        );

        // 404 unknown route
        let (status, _, body) = http_request(port, "GET", "/v1/nonexistent", Some(&format!("Bearer {}", token)));
        assert_eq!(status, 404);
        let val: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            val,
            json!({"error": {"code": "not_found", "message": "unknown route"}})
        );

        server.shutdown();
    }
}
