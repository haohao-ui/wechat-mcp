//! 外部大脑服务实现与 core.sock 服务端
//! 支持 next_turn, reply, resume_session, status

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Notify, oneshot};

use crate::brain::{Brain, BrainReply, Turn as BrainTurn};
use crate::core::config::{BrainConfig, BrainMode, ModelConfig, TimeoutAction};
use crate::core::policy::validate_reply;
use crate::core::turns::BrainFn;

static CLIENT_COUNTER: AtomicU64 = AtomicU64::new(1);

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub struct PendingTurn {
    pub turn: BrainTurn,
    pub deadline_at: u64,
    pub claimed_by: Option<u64>,
    pub claim_token: Option<String>,
    pub reply_sender: Option<oneshot::Sender<BrainReply>>,
    pub answered: bool,
}

pub struct BrainState {
    pub mode: BrainMode,
    pub timeout_seconds: u64,
    pub timeout_action: TimeoutAction,
    pub queue_minutes: u64,
    pub model_config: ModelConfig,
    pub pending: HashMap<String, PendingTurn>,
    pub order: VecDeque<String>,
    pub last_poll_at: Option<f64>,
    pub state_dir: PathBuf,
}

pub struct ExternalBrain {
    pub state: Mutex<BrainState>,
    pub notify_new_turn: Notify,
    pub socket_path: PathBuf,
}

impl ExternalBrain {
    pub fn new(config: &BrainConfig, model_config: &ModelConfig, state_dir: PathBuf) -> Arc<Self> {
        let sock_path = default_socket_path();
        Self::new_with_socket(config, model_config, state_dir, sock_path)
    }

    pub fn new_with_socket(
        config: &BrainConfig,
        model_config: &ModelConfig,
        state_dir: PathBuf,
        socket_path: PathBuf,
    ) -> Arc<Self> {
        let state = BrainState {
            mode: config.mode,
            timeout_seconds: config.external.timeout_seconds,
            timeout_action: config.external.timeout_action,
            queue_minutes: config.external.queue_minutes,
            model_config: model_config.clone(),
            pending: HashMap::new(),
            order: VecDeque::new(),
            last_poll_at: None,
            state_dir,
        };

        Arc::new(Self {
            state: Mutex::new(state),
            notify_new_turn: Notify::new(),
            socket_path,
        })
    }

    /// 获取状态快照，用于写入 health.json 或响应 status 查询
    pub async fn status_summary(&self) -> Value {
        let state = self.state.lock().await;
        let mode_str = match state.mode {
            BrainMode::Builtin => "builtin",
            BrainMode::External => "external",
        };
        json!({
            "mode": mode_str,
            "pending": state.pending.len(),
            "last_poll_at": state.last_poll_at,
        })
    }

    /// 生成 turns.rs 所需的 BrainFn
    pub fn brain_fn(self: &Arc<Self>) -> BrainFn {
        let this = self.clone();
        Arc::new(move |turn: &BrainTurn| {
            let this = this.clone();
            let turn_clone = turn.clone();
            Box::pin(async move { this.handle_turn(turn_clone).await })
        })
    }

    async fn handle_turn(&self, turn: BrainTurn) -> Result<BrainReply> {
        let (mode, timeout_sec, timeout_action, model_cfg) = {
            let state = self.state.lock().await;
            (
                state.mode,
                state.timeout_seconds,
                state.timeout_action,
                state.model_config.clone(),
            )
        };

        if mode == BrainMode::Builtin {
            let builtin = crate::brain::builtin::BuiltinBrain::new(model_cfg);
            return builtin.reply(&turn).await;
        }

        let turn_id = turn.turn_id.clone();
        let (tx, rx) = oneshot::channel();
        let deadline_at = (now_secs() as u64) + timeout_sec;

        {
            let mut state = self.state.lock().await;
            state.pending.insert(
                turn_id.clone(),
                PendingTurn {
                    turn: turn.clone(),
                    deadline_at,
                    claimed_by: None,
                    claim_token: None,
                    reply_sender: Some(tx),
                    answered: false,
                },
            );
            state.order.push_back(turn_id.clone());
        }
        self.notify_new_turn.notify_waiters();

        let timeout_dur = Duration::from_secs(timeout_sec);
        tokio::select! {
            reply_res = rx => {
                match reply_res {
                    Ok(reply) => Ok(reply),
                    Err(_) => bail!("reply channel closed"),
                }
            }
            _ = tokio::time::sleep(timeout_dur) => {
                // 超时处理
                let mut state = self.state.lock().await;
                if let Some(mut pending) = state.pending.remove(&turn_id) {
                    pending.answered = true;
                    state.order.retain(|id| id != &turn_id);
                }
                drop(state);

                match timeout_action {
                    TimeoutAction::Skip => {
                        let mut meta = HashMap::new();
                        meta.insert("action".to_string(), json!("skip"));
                        Ok(BrainReply {
                            reply: String::new(),
                            metadata: meta,
                        })
                    }
                    TimeoutAction::Handoff => {
                        let mut meta = HashMap::new();
                        meta.insert("action".to_string(), json!("handoff"));
                        Ok(BrainReply {
                            reply: String::new(),
                            metadata: meta,
                        })
                    }
                    TimeoutAction::Builtin => {
                        let builtin = crate::brain::builtin::BuiltinBrain::new(model_cfg);
                        builtin.reply(&turn).await
                    }
                }
            }
        }
    }

    /// 处理客户端发来的单行请求
    pub async fn dispatch_request(&self, client_id: u64, line: &str) -> Result<String> {
        let req: Value = serde_json::from_str(line).context("invalid json request")?;
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let op = req
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing op"))?;

        let res = match op {
            "next_turn" => self.handle_next_turn(client_id, &req).await,
            "reply" => self.handle_reply(&req).await,
            "resume_session" => self.handle_resume_session(&req).await,
            "status" => self.handle_status().await,
            _ => Ok(json!({ "id": id, "ok": false, "error": format!("unknown op: {op}") })),
        };

        match res {
            Ok(val) => {
                let mut resp = val;
                if let Some(obj) = resp.as_object_mut() {
                    if !obj.contains_key("id") {
                        obj.insert("id".to_string(), id);
                    }
                }
                Ok(serde_json::to_string(&resp)?)
            }
            Err(e) => {
                let resp = json!({
                    "id": id,
                    "ok": false,
                    "error": e.to_string(),
                });
                Ok(serde_json::to_string(&resp)?)
            }
        }
    }

    async fn handle_status(&self) -> Result<Value> {
        let summary = self.status_summary().await;
        Ok(json!({
            "ok": true,
            "result": summary
        }))
    }

    async fn handle_next_turn(&self, client_id: u64, req: &Value) -> Result<Value> {
        let wait_seconds = req
            .get("wait_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .min(60);

        let now = now_secs();
        {
            let mut state = self.state.lock().await;
            state.last_poll_at = Some(now);
            if state.mode == BrainMode::Builtin {
                return Ok(json!({
                    "ok": true,
                    "result": { "turn": Value::Null }
                }));
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_seconds);

        loop {
            // 尝试从队列中领取一个 turn
            {
                let mut state = self.state.lock().await;
                let cur_now = now_secs() as u64;

                let mut claimed_turn = None;
                let found_id = state
                    .order
                    .iter()
                    .find(|id| {
                        state
                            .pending
                            .get(*id)
                            .map_or(false, |p| p.claimed_by.is_none() && !p.answered && p.deadline_at > cur_now)
                    })
                    .cloned();

                if let Some(turn_id) = found_id {
                    if let Some(p) = state.pending.get_mut(&turn_id) {
                        let token = format!("claim_{}", uuid::Uuid::new_v4());
                        p.claimed_by = Some(client_id);
                        p.claim_token = Some(token.clone());
                        let mut turn_val = serde_json::to_value(&p.turn).unwrap();
                        if let Some(obj) = turn_val.as_object_mut() {
                            obj.insert("deadline_at".to_string(), json!(p.deadline_at));
                            obj.insert("claim_token".to_string(), json!(token));
                        }
                        claimed_turn = Some(turn_val);
                    }
                }

                if let Some(t) = claimed_turn {
                    return Ok(json!({
                        "ok": true,
                        "result": { "turn": t }
                    }));
                }
            }

            if wait_seconds == 0 || tokio::time::Instant::now() >= deadline {
                return Ok(json!({
                    "ok": true,
                    "result": { "turn": Value::Null }
                }));
            }

            // 等待新 turn 唤醒或超时
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::select! {
                _ = self.notify_new_turn.notified() => {},
                _ = tokio::time::sleep(remaining) => {
                    return Ok(json!({
                        "ok": true,
                        "result": { "turn": Value::Null }
                    }));
                }
            }
        }
    }

    async fn handle_reply(&self, req: &Value) -> Result<Value> {
        let turn_id = req
            .get("turn_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("unknown_turn"))?;

        let action = req
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("invalid_action"))?;

        if !["send", "skip", "handoff"].contains(&action) {
            bail!("invalid_action");
        }

        let mut state = self.state.lock().await;
        let Some(pending_ref) = state.pending.get(turn_id) else {
            bail!("unknown_turn");
        };

        if pending_ref.answered {
            bail!("already_answered");
        }

        if let Some(expected) = &pending_ref.claim_token {
            if let Some(provided) = req.get("claim_token").and_then(Value::as_str) {
                if provided != expected {
                    bail!("invalid_claim_token");
                }
            }
        }

        let mut pending = state.pending.remove(turn_id).unwrap();

        let mut text = String::new();
        if action == "send" {
            let t = req
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("");
            if t.trim().is_empty() || validate_reply(t).is_err() {
                // 放回状态，并返回 invalid_text
                state.pending.insert(turn_id.to_string(), pending);
                bail!("invalid_text");
            }
            text = t.to_string();
        }

        pending.answered = true;
        state.order.retain(|id| id != turn_id);

        let mut meta = HashMap::new();
        meta.insert("action".to_string(), json!(action));
        let reply = BrainReply {
            reply: text,
            metadata: meta,
        };

        if let Some(sender) = pending.reply_sender.take() {
            let _ = sender.send(reply);
        }

        Ok(json!({
            "ok": true,
            "result": { "accepted": true }
        }))
    }

    async fn handle_resume_session(&self, req: &Value) -> Result<Value> {
        let recipient = req
            .get("recipient")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing recipient"))?;

        let state_dir = {
            let state = self.state.lock().await;
            state.state_dir.clone()
        };

        // 遍历并在 sessions 目录和 state_dir 根目录下查找匹配 recipient 的 handoff.json 并删除
        let sessions_dir = state_dir.join("sessions");
        if sessions_dir.exists() {
            if let Ok(entries) = fs::read_dir(&sessions_dir) {
                for entry in entries.flatten() {
                    let handoff_path = entry.path().join("handoff.json");
                    if handoff_path.exists() {
                        if let Ok(data) = fs::read_to_string(&handoff_path) {
                            if let Ok(json_val) = serde_json::from_str::<Value>(&data) {
                                if json_val.get("recipient").and_then(Value::as_str) == Some(recipient) {
                                    let _ = fs::remove_file(&handoff_path);
                                }
                            }
                        }
                    }
                }
            }
        }

        let root_handoff = state_dir.join("handoff.json");
        if root_handoff.exists() {
            if let Ok(data) = fs::read_to_string(&root_handoff) {
                if let Ok(json_val) = serde_json::from_str::<Value>(&data) {
                    if json_val.get("recipient").and_then(Value::as_str) == Some(recipient) {
                        let _ = fs::remove_file(&root_handoff);
                    }
                }
            }
        }

        Ok(json!({
            "ok": true,
            "result": { "resumed": recipient }
        }))
    }

    /// 认领不随客户端单次调用断开连接而释放（保留接口兼容）
    pub async fn on_client_disconnect(&self, _client_id: u64) {
        // 外部大脑基于 lease/token 机制：认领持续有效直至被回复或整体超时（deadline_at）。
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("WECHAT_CORE_SOCK") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    home.join(".wechat-rpa/core.sock")
}

/// 检查并创建父目录权限（必须 0700），绑定 core.sock（权限 0600）
pub fn bind_core_socket(socket_path: &Path) -> Result<UnixListener> {
    let parent = socket_path.parent().context("socket path missing parent")?;
    if !parent.exists() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    } else {
        let meta = fs::metadata(parent)?;
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o700 {
            bail!(
                "core.sock parent directory must have permission 0700, found {:o}",
                mode
            );
        }
    }

    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }

    let listener = UnixListener::bind(socket_path)?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// 启动 core.sock 监听服务并在后台处理连接
pub fn start_socket_server(
    brain: Arc<ExternalBrain>,
    listener: UnixListener,
    mut stop_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    let Ok((stream, _)) = accept_res else {
                        break;
                    };
                    let b = brain.clone();
                    let client_id = CLIENT_COUNTER.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        handle_client(b, client_id, stream).await;
                    });
                }
                _ = async {
                    if let Some(rx) = stop_rx.as_mut() {
                        let _ = rx.changed().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    break;
                }
            }
        }
    })
}

async fn handle_client(brain: Arc<ExternalBrain>, client_id: u64, stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match brain.dispatch_request(client_id, &line).await {
            Ok(resp_str) => {
                if writer.write_all(resp_str.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    // 外部大脑工具调用属于短连接/单次调用，领取不随单次调用断开而释放
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::ContactInfo;
    use crate::core::config::{BrainConfig, BrainMode, ExternalBrainConfig, TimeoutAction};

    static SOCK_COUNT: AtomicU64 = AtomicU64::new(1);

    fn short_temp_socket() -> PathBuf {
        let count = SOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        PathBuf::from(format!("/tmp/c_{pid}_{count}.sock"))
    }

    fn make_test_brain(mode: BrainMode, action: TimeoutAction, timeout: u64) -> (Arc<ExternalBrain>, PathBuf) {
        let td = std::env::temp_dir().join(format!("test-ext-brain-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&td).unwrap();
        let sock_path = short_temp_socket();

        let cfg = BrainConfig {
            mode,
            external: ExternalBrainConfig {
                timeout_seconds: timeout,
                timeout_action: action,
                queue_minutes: 30,
            },
        };
        let model_cfg = ModelConfig {
            backend: "codex-exec".to_string(),
            base_url: None,
            model: None,
            api_key_file: None,
            api_key_env: None,
            supports_images: None,
            timeout_seconds: None,
            json_output: None,
        };
        let b = ExternalBrain::new_with_socket(&cfg, &model_cfg, td.clone(), sock_path);
        (b, td)
    }

    fn sample_turn(turn_id: &str) -> BrainTurn {
        BrainTurn {
            turn_id: turn_id.to_string(),
            account_id: "test-acc".to_string(),
            contact: ContactInfo {
                name: "示例联系人甲".to_string(),
                kind: "private".to_string(),
            },
            text: "测试消息".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            notes: Vec::new(),
            failed: Vec::new(),
            image_count: Some(0),
            earlier_images: 0,
            max_attachment_mb: 50,
            media_refs: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_external_brain_send_skip_handoff_replies() {
        let (brain, td) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);

        // 1. send 回复
        let b_fn = brain.brain_fn();
        let b_clone = brain.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let req = json!({
                "id": "1",
                "op": "reply",
                "turn_id": "turn-1",
                "action": "send",
                "text": "回复测试"
            });
            let resp = b_clone.dispatch_request(100, &req.to_string()).await.unwrap();
            let v: Value = serde_json::from_str(&resp).unwrap();
            assert_eq!(v["ok"], true);
            assert_eq!(v["result"]["accepted"], true);
        });

        let reply = b_fn(&sample_turn("turn-1")).await.unwrap();
        assert_eq!(reply.reply, "回复测试");
        assert_eq!(reply.metadata.get("action").and_then(Value::as_str), Some("send"));

        // 2. skip 回复
        let b_clone = brain.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let req = json!({
                "id": "2",
                "op": "reply",
                "turn_id": "turn-2",
                "action": "skip"
            });
            let _ = b_clone.dispatch_request(100, &req.to_string()).await.unwrap();
        });

        let reply = b_fn(&sample_turn("turn-2")).await.unwrap();
        assert_eq!(reply.reply, "");
        assert_eq!(reply.metadata.get("action").and_then(Value::as_str), Some("skip"));

        // 3. handoff 回复
        let b_clone = brain.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let req = json!({
                "id": "3",
                "op": "reply",
                "turn_id": "turn-3",
                "action": "handoff"
            });
            let _ = b_clone.dispatch_request(100, &req.to_string()).await.unwrap();
        });

        let reply = b_fn(&sample_turn("turn-3")).await.unwrap();
        assert_eq!(reply.reply, "");
        assert_eq!(reply.metadata.get("action").and_then(Value::as_str), Some("handoff"));

        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_reply_first_wins_second_already_answered() {
        let (brain, td) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
        let b_fn = brain.brain_fn();

        let b_clone = brain.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            // 第一次 reply 成功
            let req1 = json!({
                "id": "r1",
                "op": "reply",
                "turn_id": "turn-dup",
                "action": "send",
                "text": "第一次回复"
            });
            let resp1: Value = serde_json::from_str(&b_clone.dispatch_request(1, &req1.to_string()).await.unwrap()).unwrap();
            assert_eq!(resp1["ok"], true);

            // 第二次 reply 应该返回 already_answered 或 unknown_turn (已被移出)
            let req2 = json!({
                "id": "r2",
                "op": "reply",
                "turn_id": "turn-dup",
                "action": "skip"
            });
            let resp2: Value = serde_json::from_str(&b_clone.dispatch_request(2, &req2.to_string()).await.unwrap()).unwrap();
            assert_eq!(resp2["ok"], false);
            assert!(["already_answered", "unknown_turn"].contains(&resp2["error"].as_str().unwrap()));
        });

        let reply = b_fn(&sample_turn("turn-dup")).await.unwrap();
        assert_eq!(reply.reply, "第一次回复");

        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_timeout_actions() {
        // 1. timeout_action: skip
        let (brain_skip, td1) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 1);
        let b_fn = brain_skip.brain_fn();
        let reply = b_fn(&sample_turn("turn-timeout-skip")).await.unwrap();
        assert_eq!(reply.metadata.get("action").and_then(Value::as_str), Some("skip"));
        let _ = fs::remove_dir_all(td1);

        // 2. timeout_action: handoff
        let (brain_handoff, td2) = make_test_brain(BrainMode::External, TimeoutAction::Handoff, 1);
        let b_fn = brain_handoff.brain_fn();
        let reply = b_fn(&sample_turn("turn-timeout-handoff")).await.unwrap();
        assert_eq!(reply.metadata.get("action").and_then(Value::as_str), Some("handoff"));
        let _ = fs::remove_dir_all(td2);
    }

    #[tokio::test]
    async fn test_claim_persists_on_disconnect_and_mutually_excludes() {
        let (brain, td) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
        let b_fn = brain.brain_fn();

        // 放入一个 turn 到待处理队列
        tokio::spawn(async move {
            let _ = b_fn(&sample_turn("turn-reclaim")).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        // client 1 通过 next_turn 领取，返回包含 claim_token
        let req1 = json!({ "id": "1", "op": "next_turn", "wait_seconds": 1 });
        let resp1: Value = serde_json::from_str(&brain.dispatch_request(1, &req1.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp1["result"]["turn"]["turn_id"], "turn-reclaim");
        let claim_token = resp1["result"]["turn"]["claim_token"].as_str().unwrap().to_string();
        assert!(!claim_token.is_empty(), "must issue claim_token");

        // client 2 此时调用 next_turn(0) 互斥，拿不到该轮次
        let req2 = json!({ "id": "2", "op": "next_turn", "wait_seconds": 0 });
        let resp2: Value = serde_json::from_str(&brain.dispatch_request(2, &req2.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp2["result"]["turn"], Value::Null);

        // client 1 单次调用断开（触发 on_client_disconnect）
        brain.on_client_disconnect(1).await;

        // 领取不随单次调用断开而释放：client 2 再次调用 next_turn(0) 依然互斥拿不到！
        let resp3: Value = serde_json::from_str(&brain.dispatch_request(2, &req2.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp3["result"]["turn"], Value::Null, "turn must remain claimed across disconnects");

        // client 2 伪造 claim_token 尝试 reply 应当被拒绝
        let fake_reply = json!({
            "id": "bad_rep",
            "op": "reply",
            "turn_id": "turn-reclaim",
            "action": "skip",
            "claim_token": "wrong_token_123"
        });
        let resp_bad: Value = serde_json::from_str(&brain.dispatch_request(2, &fake_reply.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp_bad["ok"], false);
        assert_eq!(resp_bad["error"], "invalid_claim_token");

        // client 1 携带正确的 claim_token 回复，成功被接收
        let good_reply = json!({
            "id": "good_rep",
            "op": "reply",
            "turn_id": "turn-reclaim",
            "action": "send",
            "text": "认领持久化测试正常",
            "claim_token": claim_token
        });
        let resp_good: Value = serde_json::from_str(&brain.dispatch_request(1, &good_reply.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp_good["ok"], true);
        assert_eq!(resp_good["result"]["accepted"], true);

        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_socket_permissions_and_server() {
        let count = SOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let td = PathBuf::from(format!("/tmp/tp_{pid}_{count}"));
        fs::create_dir_all(&td).unwrap();
        fs::set_permissions(&td, fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = td.join("core.sock");

        let listener = bind_core_socket(&sock_path).unwrap();
        let perm = fs::metadata(&sock_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perm, 0o600);

        let cfg = BrainConfig::default();
        let model_cfg = ModelConfig {
            backend: "codex-exec".to_string(),
            base_url: None,
            model: None,
            api_key_file: None,
            api_key_env: None,
            supports_images: None,
            timeout_seconds: None,
            json_output: None,
        };
        let brain = ExternalBrain::new_with_socket(&cfg, &model_cfg, td.clone(), sock_path.clone());
        let (tx, rx) = tokio::sync::watch::channel(false);
        let server_handle = start_socket_server(brain, listener, Some(rx));

        // 用 UnixStream 连接并调用 status
        let stream = UnixStream::connect(&sock_path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let req = json!({"id": "test_stat", "op": "status"}).to_string() + "\n";
        writer.write_all(req.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();

        let mut lines = BufReader::new(reader).lines();
        let resp_line = lines.next_line().await.unwrap().unwrap();
        let resp: Value = serde_json::from_str(&resp_line).unwrap();
        assert_eq!(resp["id"], "test_stat");
        assert_eq!(resp["result"]["mode"], "builtin");

        let _ = tx.send(true);
        server_handle.abort();
        let _ = fs::remove_file(&sock_path);
        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_reply_ignores_fake_recipient() {
        let (brain, td) = make_test_brain(BrainMode::External, TimeoutAction::Skip, 60);
        let b_fn = brain.brain_fn();

        let b_clone = brain.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let req = json!({
                "id": "r_fake",
                "op": "reply",
                "turn_id": "turn-target",
                "recipient": "evil_user",
                "action": "send",
                "text": "测试正常回复"
            });
            let resp: Value = serde_json::from_str(&b_clone.dispatch_request(1, &req.to_string()).await.unwrap()).unwrap();
            assert_eq!(resp["ok"], true);
            assert_eq!(resp["result"]["accepted"], true);
        });

        let orig_turn = sample_turn("turn-target");
        assert_eq!(orig_turn.contact.name, "示例联系人甲");
        let reply = b_fn(&orig_turn).await.unwrap();
        assert_eq!(reply.reply, "测试正常回复");

        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_socket_parent_dir_must_be_0700() {
        let count = SOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let td = PathBuf::from(format!("/tmp/tp_bad_{pid}_{count}"));
        fs::create_dir_all(&td).unwrap();
        fs::set_permissions(&td, fs::Permissions::from_mode(0o755)).unwrap();
        let sock_path = td.join("core.sock");

        let res = bind_core_socket(&sock_path);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("must have permission 0700"));

        let _ = fs::remove_dir_all(td);
    }

    #[tokio::test]
    async fn test_builtin_mode_next_turn_returns_null() {
        let (brain, td) = make_test_brain(BrainMode::Builtin, TimeoutAction::Skip, 60);
        let req = json!({ "id": "b1", "op": "next_turn", "wait_seconds": 1 });
        let resp: Value = serde_json::from_str(&brain.dispatch_request(1, &req.to_string()).await.unwrap()).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["result"]["turn"], Value::Null);

        let _ = fs::remove_dir_all(td);
    }
}
