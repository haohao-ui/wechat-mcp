//! 外部大脑 MCP 工具：next_turn / reply / resume_session
//! 通过本机 Unix 域套接字 ~/.wechat-rpa/core.sock 转发给 serve-core。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub const TOOL_NEXT_TURN: &str = "next_turn";
pub const TOOL_REPLY: &str = "reply";
pub const TOOL_RESUME_SESSION: &str = "resume_session";

static REQ_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 判断是否是外部大脑 MCP 工具
pub fn is_brain_tool(name: &str) -> bool {
    matches!(name, TOOL_NEXT_TURN | TOOL_REPLY | TOOL_RESUME_SESSION)
}

/// 返回外部大脑工具的 MCP 工具定义列表
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": TOOL_NEXT_TURN,
            "description": "长轮询拉取待外部大脑回复的一轮客户消息；等待指定秒数（0..60 秒），若超时无新轮次则返回 null",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "wait_seconds": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 60,
                        "default": 30,
                        "description": "长轮询等待秒数（0..60），默认 30"
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": TOOL_REPLY,
            "description": "回复指定的轮次，action 可选 send/skip/handoff，send 时可带回复文本 text；接收人由系统决定，不可篡改",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turn_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "待回复轮次的 ID（服务分配的幂等键）"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["send", "skip", "handoff"],
                        "description": "回复动作：send（发送回复）、skip（跳过本轮不回复）、handoff（转人工）"
                    },
                    "text": {
                        "type": "string",
                        "description": "回复文本内容（当 action 为 send 时）"
                    }
                },
                "required": ["turn_id", "action"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": TOOL_RESUME_SESSION,
            "description": "解除指定会话的转人工状态，恢复自动回复",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "minLength": 1,
                        "description": "要恢复自动回复的联系人昵称或群名称"
                    }
                },
                "required": ["recipient"],
                "additionalProperties": false
            }
        }),
    ]
}

/// 获取默认的 core.sock 路径
pub fn default_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("WECHAT_CORE_SOCK") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    home.join(".wechat-rpa/core.sock")
}

/// 默认路径执行外部大脑工具
pub async fn execute(name: &str, args: Value) -> Result<Value> {
    execute_at(&default_socket_path(), name, args).await
}

/// 在指定的 socket 路径执行外部大脑工具并转发到 core.sock
pub async fn execute_at(socket_path: &Path, name: &str, args: Value) -> Result<Value> {
    let req = prepare_request(name, args)?;
    forward_to_socket(socket_path, req).await
}

fn prepare_request(name: &str, args: Value) -> Result<Value> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow!("arguments 必须是对象"))?;

    let req_id = format!("req_{}", REQ_COUNTER.fetch_add(1, Ordering::Relaxed));

    match name {
        TOOL_NEXT_TURN => {
            for key in object.keys() {
                if key != "wait_seconds" {
                    bail!("不支持的参数: {key}");
                }
            }
            let wait_seconds = match object.get("wait_seconds") {
                Some(v) => {
                    let sec = v
                        .as_i64()
                        .ok_or_else(|| anyhow!("wait_seconds 必须是整数"))?;
                    if !(0..=60).contains(&sec) {
                        bail!("wait_seconds 必须在 0..60 之间: {sec}");
                    }
                    sec as u64
                }
                None => 30,
            };
            Ok(json!({
                "id": req_id,
                "op": "next_turn",
                "wait_seconds": wait_seconds
            }))
        }
        TOOL_REPLY => {
            for key in object.keys() {
                if !["turn_id", "action", "text"].contains(&key.as_str()) {
                    bail!("不支持的参数: {key}");
                }
            }
            let turn_id = object
                .get("turn_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("必须提供 turn_id"))?;
            if turn_id.trim().is_empty() {
                bail!("turn_id 不能为空");
            }
            let action = object
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("必须提供 action"))?;
            if !["send", "skip", "handoff"].contains(&action) {
                bail!("无效的 action: {action}，必须是 send、skip 或 handoff");
            }

            let mut req = json!({
                "id": req_id,
                "op": "reply",
                "turn_id": turn_id,
                "action": action
            });

            if let Some(text_val) = object.get("text") {
                let text = text_val
                    .as_str()
                    .ok_or_else(|| anyhow!("text 必须是字符串"))?;
                req["text"] = json!(text);
            }
            Ok(req)
        }
        TOOL_RESUME_SESSION => {
            for key in object.keys() {
                if key != "recipient" {
                    bail!("不支持的参数: {key}");
                }
            }
            let recipient = object
                .get("recipient")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("必须提供 recipient"))?;
            if recipient.trim().is_empty() {
                bail!("recipient 不能为空");
            }
            Ok(json!({
                "id": req_id,
                "op": "resume_session",
                "recipient": recipient
            }))
        }
        _ => bail!("未知外部大脑工具: {name}"),
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct CoreResponse {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

async fn forward_to_socket(socket_path: &Path, request: Value) -> Result<Value> {
    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(_) => bail!("core_not_running"),
    };

    let (reader, mut writer) = stream.into_split();
    let mut payload = serde_json::to_string(&request)?;
    payload.push('\n');

    writer
        .write_all(payload.as_bytes())
        .await
        .map_err(|_| anyhow!("core_not_running"))?;
    writer
        .flush()
        .await
        .map_err(|_| anyhow!("core_not_running"))?;

    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await
        .map_err(|_| anyhow!("core_not_running"))?
        .ok_or_else(|| anyhow!("core_not_running"))?;

    let resp: CoreResponse = serde_json::from_str(&line)
        .map_err(|e| anyhow!("core.sock 响应解析失败: {e}"))?;

    if resp.ok {
        Ok(resp.result.unwrap_or(json!({})))
    } else {
        let err_msg = resp.error.unwrap_or_else(|| "unknown_error".into());
        bail!("{err_msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    static SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_socket_path() -> PathBuf {
        let count = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("test-core-sock-{pid}-{count}.sock"))
    }

    #[test]
    fn test_tools_list_schema() {
        let list = tools();
        assert_eq!(list.len(), 3, "brain tools should have 3 tools");
        let names: Vec<&str> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&TOOL_NEXT_TURN));
        assert!(names.contains(&TOOL_REPLY));
        assert!(names.contains(&TOOL_RESUME_SESSION));

        for t in &list {
            assert!(is_brain_tool(t["name"].as_str().unwrap()));
        }
        assert!(!is_brain_tool("send_message"));
    }

    #[tokio::test]
    async fn test_core_not_running_when_socket_missing() {
        let missing = PathBuf::from("/tmp/nonexistent-core-sock-path.sock");
        let _ = std::fs::remove_file(&missing);

        // next_turn
        let err = execute_at(&missing, TOOL_NEXT_TURN, json!({"wait_seconds": 1}))
            .await
            .expect_err("should fail when core.sock is missing");
        assert_eq!(err.to_string(), "core_not_running");

        // reply
        let err = execute_at(
            &missing,
            TOOL_REPLY,
            json!({"turn_id": "turn-1", "action": "skip"}),
        )
        .await
        .expect_err("should fail when core.sock is missing");
        assert_eq!(err.to_string(), "core_not_running");

        // resume_session
        let err = execute_at(
            &missing,
            TOOL_RESUME_SESSION,
            json!({"recipient": "alice"}),
        )
        .await
        .expect_err("should fail when core.sock is missing");
        assert_eq!(err.to_string(), "core_not_running");
    }

    #[test]
    fn test_argument_validation() {
        // next_turn: wait_seconds 超过 60
        let err = prepare_request(TOOL_NEXT_TURN, json!({"wait_seconds": 61})).unwrap_err();
        assert!(err.to_string().contains("wait_seconds 必须在 0..60 之间"));

        // next_turn: wait_seconds < 0
        let err = prepare_request(TOOL_NEXT_TURN, json!({"wait_seconds": -1})).unwrap_err();
        assert!(err.to_string().contains("wait_seconds 必须在 0..60 之间"));

        // next_turn: 不支持额外参数
        let err = prepare_request(TOOL_NEXT_TURN, json!({"wait_seconds": 10, "extra": 1})).unwrap_err();
        assert!(err.to_string().contains("不支持的参数: extra"));

        // next_turn: 默认值 30
        let req = prepare_request(TOOL_NEXT_TURN, json!({})).unwrap();
        assert_eq!(req["wait_seconds"], 30);

        // reply: 缺少 turn_id
        let err = prepare_request(TOOL_REPLY, json!({"action": "send"})).unwrap_err();
        assert!(err.to_string().contains("必须提供 turn_id"));

        // reply: 空 turn_id
        let err = prepare_request(TOOL_REPLY, json!({"turn_id": " ", "action": "send"})).unwrap_err();
        assert!(err.to_string().contains("turn_id 不能为空"));

        // reply: 缺少 action
        let err = prepare_request(TOOL_REPLY, json!({"turn_id": "t1"})).unwrap_err();
        assert!(err.to_string().contains("必须提供 action"));

        // reply: 非法 action
        let err = prepare_request(TOOL_REPLY, json!({"turn_id": "t1", "action": "bogus"})).unwrap_err();
        assert!(err.to_string().contains("无效的 action: bogus"));

        // reply: 拒绝客户端伪造接收人 recipient
        let err = prepare_request(
            TOOL_REPLY,
            json!({"turn_id": "t1", "action": "send", "recipient": "hacker"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("不支持的参数: recipient"));

        // resume_session: 缺少 recipient
        let err = prepare_request(TOOL_RESUME_SESSION, json!({})).unwrap_err();
        assert!(err.to_string().contains("必须提供 recipient"));

        // resume_session: 拒绝额外参数
        let err = prepare_request(
            TOOL_RESUME_SESSION,
            json!({"recipient": "alice", "other": "x"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("不支持的参数: other"));
    }

    #[tokio::test]
    async fn test_brain_tools_forward_and_map_errors() {
        let sock_path = temp_socket_path();
        let _ = std::fs::remove_file(&sock_path);

        let listener = UnixListener::bind(&sock_path).expect("bind fake core.sock");
        let server_path = sock_path.clone();

        // 假服务端处理请求
        let server_handle = tokio::spawn(async move {
            for _ in 0..6 {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (reader, mut writer) = stream.into_split();
                let mut lines = BufReader::new(reader).lines();
                if let Ok(Some(line)) = lines.next_line().await {
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let id = req["id"].clone();
                    let op = req["op"].as_str().unwrap();

                    match op {
                        "next_turn" => {
                            let wait = req["wait_seconds"].as_u64().unwrap();
                            let resp = json!({
                                "id": id,
                                "ok": true,
                                "result": {
                                    "turn": {
                                        "turn_id": "turn_123",
                                        "deadline_at": 1731000000,
                                        "wait": wait
                                    }
                                }
                            });
                            writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                            writer.write_all(b"\n").await.unwrap();
                        }
                        "reply" => {
                            let action = req["action"].as_str().unwrap();
                            let turn_id = req["turn_id"].as_str().unwrap();
                            if turn_id == "unknown_id" {
                                let resp = json!({
                                    "id": id,
                                    "ok": false,
                                    "error": "unknown_turn"
                                });
                                writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                                writer.write_all(b"\n").await.unwrap();
                            } else if turn_id == "already_answered_id" {
                                let resp = json!({
                                    "id": id,
                                    "ok": false,
                                    "error": "already_answered"
                                });
                                writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                                writer.write_all(b"\n").await.unwrap();
                            } else if action == "send" && req.get("text").is_none() {
                                let resp = json!({
                                    "id": id,
                                    "ok": false,
                                    "error": "invalid_text"
                                });
                                writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                                writer.write_all(b"\n").await.unwrap();
                            } else {
                                let resp = json!({
                                    "id": id,
                                    "ok": true,
                                    "result": { "accepted": true }
                                });
                                writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                                writer.write_all(b"\n").await.unwrap();
                            }
                        }
                        "resume_session" => {
                            let recipient = req["recipient"].as_str().unwrap();
                            let resp = json!({
                                "id": id,
                                "ok": true,
                                "result": { "resumed": recipient }
                            });
                            writer.write_all(resp.to_string().as_bytes()).await.unwrap();
                            writer.write_all(b"\n").await.unwrap();
                        }
                        _ => panic!("unexpected op: {op}"),
                    }
                }
            }
        });

        // 1. next_turn
        let res = execute_at(&server_path, TOOL_NEXT_TURN, json!({"wait_seconds": 15}))
            .await
            .unwrap();
        assert_eq!(res["turn"]["turn_id"], "turn_123");
        assert_eq!(res["turn"]["wait"], 15);

        // 2. reply send accepted
        let res = execute_at(
            &server_path,
            TOOL_REPLY,
            json!({"turn_id": "turn_123", "action": "send", "text": "你好"}),
        )
        .await
        .unwrap();
        assert_eq!(res["accepted"], true);

        // 3. reply unknown_turn error mapping
        let err = execute_at(
            &server_path,
            TOOL_REPLY,
            json!({"turn_id": "unknown_id", "action": "skip"}),
        )
        .await
        .expect_err("unknown_turn should error");
        assert_eq!(err.to_string(), "unknown_turn");

        // 4. reply already_answered error mapping
        let err = execute_at(
            &server_path,
            TOOL_REPLY,
            json!({"turn_id": "already_answered_id", "action": "skip"}),
        )
        .await
        .expect_err("already_answered should error");
        assert_eq!(err.to_string(), "already_answered");

        // 5. reply invalid_text error mapping
        let err = execute_at(
            &server_path,
            TOOL_REPLY,
            json!({"turn_id": "turn_123", "action": "send"}),
        )
        .await
        .expect_err("invalid_text should error");
        assert_eq!(err.to_string(), "invalid_text");

        // 6. resume_session
        let res = execute_at(
            &server_path,
            TOOL_RESUME_SESSION,
            json!({"recipient": "张三"}),
        )
        .await
        .unwrap();
        assert_eq!(res["resumed"], "张三");

        server_handle.abort();
        let _ = std::fs::remove_file(&server_path);
    }
}
