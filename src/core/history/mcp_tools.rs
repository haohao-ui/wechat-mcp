//! 历史查询 MCP 工具（对应 scripts/wechat_bot/history/mcp_server.py）
//! 提供只读 stdio / 嵌入式 MCP 历史查询工具。

use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::query::{HistoryQuery, QueryError};

pub const TOOL_HISTORY_LIST_CONTACTS: &str = "history_list_contacts";
pub const TOOL_HISTORY_GET_CONTACT: &str = "history_get_contact";
pub const TOOL_HISTORY_GET_RECENT_MESSAGES: &str = "history_get_recent_messages";
pub const TOOL_HISTORY_GET_HISTORY: &str = "history_get_history";
pub const TOOL_HISTORY_SEARCH_MESSAGES: &str = "history_search_messages";
pub const TOOL_HISTORY_GET_MESSAGE: &str = "history_get_message";
pub const TOOL_HISTORY_GET_MEDIA: &str = "history_get_media";

pub const ALL_HISTORY_TOOLS: &[&str] = &[
    TOOL_HISTORY_LIST_CONTACTS,
    TOOL_HISTORY_GET_CONTACT,
    TOOL_HISTORY_GET_RECENT_MESSAGES,
    TOOL_HISTORY_GET_HISTORY,
    TOOL_HISTORY_SEARCH_MESSAGES,
    TOOL_HISTORY_GET_MESSAGE,
    TOOL_HISTORY_GET_MEDIA,
];

/// 判断是否是历史查询 MCP 工具
pub fn is_history_tool(name: &str) -> bool {
    ALL_HISTORY_TOOLS.contains(&name)
}

struct ToolSpec {
    method: &'static str,
    desc: &'static str,
    props: &'static [(&'static str, &'static str)],
    required: &'static [&'static str],
    order_enum: bool,
}

const TOOL_SPECS: &[(&str, ToolSpec)] = &[
    (
        TOOL_HISTORY_LIST_CONTACTS,
        ToolSpec {
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
        TOOL_HISTORY_GET_CONTACT,
        ToolSpec {
            method: "get_contact",
            desc: "按 contact_id 或名称（表情/符号不敏感）获取联系人",
            props: &[("contact_id", "string"), ("name", "string")],
            required: &[],
            order_enum: false,
        },
    ),
    (
        TOOL_HISTORY_GET_RECENT_MESSAGES,
        ToolSpec {
            method: "get_recent_messages",
            desc: "某联系人最近 N 条消息（旧→新）",
            props: &[("contact_id", "string"), ("limit", "integer")],
            required: &["contact_id"],
            order_enum: false,
        },
    ),
    (
        TOOL_HISTORY_GET_HISTORY,
        ToolSpec {
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
        TOOL_HISTORY_SEARCH_MESSAGES,
        ToolSpec {
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
        TOOL_HISTORY_GET_MESSAGE,
        ToolSpec {
            method: "get_message",
            desc: "单条消息详情（含 meta；附件含来源 original/chat_thumbnail 与理解要点 understanding）",
            props: &[("message_id", "string")],
            required: &["message_id"],
            order_enum: false,
        },
    ),
    (
        TOOL_HISTORY_GET_MEDIA,
        ToolSpec {
            method: "get_media",
            desc: "附件元数据（来源、理解要点 understanding）与 base64 内容（≤10MB）",
            props: &[("media_id", "string")],
            required: &["media_id"],
            order_enum: false,
        },
    ),
];

/// 返回历史查询工具的 MCP 工具定义列表（与 Python tool_list() 保持一致）
pub fn tools() -> Vec<Value> {
    TOOL_SPECS
        .iter()
        .map(|(name, spec)| {
            let mut props_map = serde_json::Map::new();
            for &(pname, ptype) in spec.props {
                let mut prop_obj = serde_json::Map::new();
                prop_obj.insert("type".to_string(), json!(ptype));
                if pname == "order" && spec.order_enum {
                    prop_obj.insert("enum".to_string(), json!(["asc", "desc"]));
                }
                props_map.insert(pname.to_string(), Value::Object(prop_obj));
            }
            json!({
                "name": name,
                "description": spec.desc,
                "inputSchema": {
                    "type": "object",
                    "properties": Value::Object(props_map),
                    "required": spec.required,
                    "additionalProperties": false
                }
            })
        })
        .collect()
}

/// 标准 Base64 编码
fn encode_base64(data: &[u8]) -> String {
    const B64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

/// 解析默认或自定义配置文件路径
pub fn resolve_config_path(override_path: Option<&Path>) -> PathBuf {
    if let Some(p) = override_path {
        p.to_path_buf()
    } else if let Ok(env_path) = std::env::var("WECHAT_BOT_CONFIG") {
        PathBuf::from(env_path)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        PathBuf::from(home).join(".WeChatRPABot").join("config.json")
    }
}

/// 根据配置文件推导 state_dir 并只读打开历史数据库
pub fn open_query_from_config(config_path: Option<&Path>) -> Result<HistoryQuery, QueryError> {
    let path = resolve_config_path(config_path);
    if !path.exists() {
        return Err(QueryError::new(
            "config_error",
            format!("config file not found at {}", path.display()),
        ));
    }

    let content = fs::read_to_string(&path).map_err(|e| {
        QueryError::new(
            "config_error",
            format!("failed to read config file {}: {}", path.display(), e),
        )
    })?;

    let val: Value = serde_json::from_str(&content).map_err(|e| {
        QueryError::new(
            "config_error",
            format!("invalid json in config file {}: {}", path.display(), e),
        )
    })?;

    let state_dir_str = val
        .get("state_dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            QueryError::new("config_error", "state_dir missing or invalid in config")
        })?;

    let state_dir = PathBuf::from(state_dir_str);
    let context_messages = val
        .get("context_messages")
        .and_then(|v| v.as_i64())
        .unwrap_or(5);

    let db_path = state_dir.join("history").join("history.sqlite3");
    HistoryQuery::open(&db_path, Some(context_messages))
}

/// 调用历史查询工具核心逻辑，参数校验与异常返回格式保持与 Python mcp_server.py 严格一致
pub fn call_tool(query: &HistoryQuery, name: &str, arguments: &Value) -> Result<Value, QueryError> {
    let spec_entry = TOOL_SPECS.iter().find(|(tname, _)| *tname == name);
    let (_, spec) = match spec_entry {
        Some(s) => s,
        None => return Err(QueryError::invalid_argument("unknown tool")),
    };

    let args_obj = match arguments.as_object() {
        Some(obj) => obj,
        None => return Err(QueryError::invalid_argument("invalid arguments")),
    };

    let allowed_props: HashSet<&str> = spec.props.iter().map(|(p, _)| *p).collect();
    for k in args_obj.keys() {
        if !allowed_props.contains(k.as_str()) {
            return Err(QueryError::invalid_argument("invalid arguments"));
        }
    }
    for &req in spec.required {
        if !args_obj.contains_key(req) {
            return Err(QueryError::invalid_argument("invalid arguments"));
        }
    }

    for &(pname, ptype) in spec.props {
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

    match spec.method {
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

/// 执行历史查询工具（自动从配置推导 state_dir 并只读打开历史库）
pub fn execute(name: &str, arguments: &Value) -> Result<Value, QueryError> {
    execute_with_config(name, arguments, None)
}

/// 执行历史查询工具（显式指定配置文件路径）
pub fn execute_with_config(
    name: &str,
    arguments: &Value,
    config_path: Option<&Path>,
) -> Result<Value, QueryError> {
    let query = open_query_from_config(config_path)?;
    call_tool(&query, name, arguments)
}

/// 处理单条 MCP JSON-RPC 请求（接收 Query 引用）
pub fn handle_request_with_query(query: &HistoryQuery, request: &Value) -> Option<Value> {
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
                "tools": tools()
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

/// 处理单条 MCP JSON-RPC 请求（自动通过 config 打开 Query）
pub fn handle_request(request: &Value, config_path: Option<&Path>) -> Option<Value> {
    let rid = request.get("id")?;
    if rid.is_null() {
        return None;
    }
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

    // initialize, ping, tools/list, method not found 不需要连接数据库即可响应
    if method == "initialize" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wechat-history", "version": "1"}
            }
        }));
    } else if method == "tools/list" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "tools": tools()
            }
        }));
    } else if method == "ping" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {}
        }));
    } else if method != "tools/call" {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": rid,
            "error": {"code": -32601, "message": "method not found"}
        }));
    }

    // tools/call 时打开 query
    let query = match open_query_from_config(config_path) {
        Ok(q) => q,
        Err(err) => {
            let err_obj = json!({"error": {"code": err.code, "message": err.message}});
            let text = serde_json::to_string(&err_obj).unwrap_or_default();
            return Some(json!({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": true
                }
            }));
        }
    };

    handle_request_with_query(&query, request)
}

/// 运行只读 stdio MCP 主循环（替代 scripts/wechat_bot/history/mcp_server.py）
pub fn run_stdio(
    reader: impl BufRead,
    mut writer: impl Write,
    config_path: Option<&Path>,
) -> io::Result<()> {
    for line in reader.lines() {
        let l = line?;
        if l.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&l) {
            Ok(req) => {
                if let Some(resp) = handle_request(&req, config_path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::history::store::HistoryStore;

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
        _state: PathBuf,
        config_path: PathBuf,
        contact_id: String,
    }

    fn bub(text: &str, d: &str) -> Value {
        json!({"text": text, "digest": text, "direction": d})
    }

    impl TestFixture {
        fn setup() -> Self {
            let temp = TempDir::new("mcp-tools-test");
            let base = temp.path();
            let state = base.join("state");
            let config_path = base.join("config.json");
            fs::write(
                &config_path,
                json!({
                    "schema_version": 1,
                    "host": "fixture",
                    "account_id": "acct",
                    "allowlist": [{"recipient": "🍓小美", "kind": "private"}],
                    "state_dir": state.to_string_lossy(),
                    "context_messages": 5
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
                            bub("还是上次那个款", "inbound_visual"),
                            bub("已发顺丰", "outbound_visual"),
                            bub("最后一条", "inbound_visual")
                        ]
                    }),
                    None,
                    None,
                    false,
                )
                .unwrap();

            Self {
                _temp: temp,
                _state: state,
                config_path,
                contact_id,
            }
        }
    }

    // 翻译自 tests/test_history_servers.py::McpTests::test_mcp_lists_tools_and_returns_history
    #[test]
    fn test_history_mcp_lists_tools_and_returns_history() {
        let f = TestFixture::setup();

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
            if let Some(resp) = handle_request(&r, Some(&f.config_path)) {
                out.push(resp);
            }
        }

        assert_eq!(out.len(), 3, "notification 应该被忽略，返回 3 个响应");
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "wechat-history");
        assert_eq!(out[0]["result"]["protocolVersion"], "2024-11-05");

        let tool_list = out[1]["result"]["tools"].as_array().unwrap();
        let names: HashSet<String> = tool_list
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        let expected_names: HashSet<String> = [
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
        assert_eq!(names, expected_names);

        let content_text = out[2]["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(content_text).unwrap();
        let items = parsed["items"].as_array().unwrap();
        assert_eq!(items.last().unwrap()["text"], "最后一条");
    }

    // 翻译自 tests/test_history_servers.py::McpTests::test_mcp_errors
    #[test]
    fn test_history_mcp_errors() {
        let f = TestFixture::setup();

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
            if let Some(resp) = handle_request(&r, Some(&f.config_path)) {
                out.push(resp);
            }
        }

        assert!(out[0]["result"]["isError"].as_bool().unwrap());
        assert!(out[1]["result"]["isError"].as_bool().unwrap());
        assert_eq!(out[2]["error"]["code"], -32601);
    }

    // 测试未读到配置时返回明确的错误
    #[test]
    fn test_history_mcp_config_error_when_config_missing() {
        let missing_path = PathBuf::from("/nonexistent/path/to/config.json");
        let req = json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": {"name": "history_list_contacts", "arguments": {}}
        });
        let resp = handle_request(&req, Some(&missing_path)).unwrap();
        assert!(resp["result"]["isError"].as_bool().unwrap());
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("config_error"));
    }

    // 测试 stdio 循环模式
    #[test]
    fn test_history_mcp_run_stdio() {
        let f = TestFixture::setup();
        let input_lines = format!(
            "{}\n{}\n",
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "history_get_recent_messages", "arguments": {"contact_id": f.contact_id}}})
        );

        let mut output = Vec::new();
        run_stdio(input_lines.as_bytes(), &mut output, Some(&f.config_path)).unwrap();

        let out_str = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), 2);
        let resp1: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(resp1["result"]["serverInfo"]["name"], "wechat-history");
        let resp2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(resp2["result"]["isError"], false);
    }

    // 测试工具判别与参数类型校验
    #[test]
    fn test_history_mcp_is_tool_and_type_check() {
        for name in ALL_HISTORY_TOOLS {
            assert!(is_history_tool(name));
        }
        assert!(!is_history_tool("send_message"));
        assert!(!is_history_tool("next_turn"));

        let f = TestFixture::setup();
        let query = open_query_from_config(Some(&f.config_path)).unwrap();

        // integer 参数传了 string 应报 invalid_argument
        let err = call_tool(
            &query,
            "history_get_recent_messages",
            &json!({"contact_id": f.contact_id, "limit": "not_an_int"}),
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_argument");
        assert!(err.message.contains("must be integer"));

        // string 参数传了 int 应报 invalid_argument
        let err = call_tool(
            &query,
            "history_get_recent_messages",
            &json!({"contact_id": 12345}),
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_argument");
        assert!(err.message.contains("must be string"));
    }
}
