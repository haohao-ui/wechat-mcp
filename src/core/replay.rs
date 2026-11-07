use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::brain::{BrainReply, Turn as BrainTurn};
use crate::core::config::{AllowlistEntry, BotConfig};
use crate::core::delivery::ToolClient;
use crate::core::history::store::HistoryStore;
use crate::core::model::{Bubble, Frame};
use crate::core::state::read_json;
use crate::core::turns::{AttachmentFile, AttachmentImage, BotService, Capture, FailedAttachmentItem};

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayArgs {
    pub config: PathBuf,
    pub session: String,
    pub frames: Vec<PathBuf>,
    pub journals: PathBuf,
    pub work: PathBuf,
}

pub fn parse_replay_args(args: &[String]) -> Result<ReplayArgs> {
    let mut config = None;
    let mut session = None;
    let mut frames = Vec::new();
    let mut journals = None;
    let mut work = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    bail!("--config requires a value");
                }
                config = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--session" => {
                if i + 1 >= args.len() {
                    bail!("--session requires a value");
                }
                session = Some(args[i + 1].clone());
                i += 2;
            }
            "--journals" => {
                if i + 1 >= args.len() {
                    bail!("--journals requires a value");
                }
                journals = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--work" => {
                if i + 1 >= args.len() {
                    bail!("--work requires a value");
                }
                work = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--frames" => {
                i += 1;
                let mut found = false;
                while i < args.len() && !args[i].starts_with("--") {
                    frames.push(PathBuf::from(&args[i]));
                    found = true;
                    i += 1;
                }
                if !found {
                    bail!("--frames requires at least one file path");
                }
            }
            other => {
                bail!("unknown argument: {other}");
            }
        }
    }

    let config = config.context("missing --config")?;
    let session = session.context("missing --session")?;
    let journals = journals.context("missing --journals")?;
    let work = work.context("missing --work")?;
    if frames.is_empty() {
        bail!("missing --frames");
    }

    Ok(ReplayArgs {
        config,
        session,
        frames,
        journals,
        work,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusMismatch {
    pub key: String,
    pub rust: String,
    pub python: String,
    pub rust_status: String,
    pub python_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayReport {
    pub frames: usize,
    pub rust_logs: BTreeMap<String, String>,
    pub python_logs: BTreeMap<String, String>,
    pub matched: Vec<String>,
    pub rust_only: Vec<String>,
    pub python_only: Vec<String>,
    pub status_mismatch: Vec<StatusMismatch>,
    pub missing_reply: Vec<String>,
    pub send_calls: usize,
    pub send_message_calls: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rust: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub python: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct FrameLine {
    t: f64,
    #[serde(default)]
    #[allow(dead_code)]
    op: Option<String>,
    recipient: String,
    frame: Frame,
}

struct ReplayCapture {
    current_frame: Arc<Mutex<Frame>>,
}

impl Capture for ReplayCapture {
    fn frame(&mut self) -> impl Future<Output = Result<Frame>> + Send {
        let f = self.current_frame.lock().unwrap().clone();
        async move { Ok(f) }
    }

    fn scroll(&mut self, _direction: &str, _pages: u32) -> impl Future<Output = Result<Frame>> + Send {
        let f = self.current_frame.lock().unwrap().clone();
        async move { Ok(f) }
    }

    fn download(&mut self, _media_id: &str) -> impl Future<Output = Result<PathBuf>> + Send {
        async move { bail!("download not supported in replay") }
    }

    fn attachments(
        &mut self,
        _items: &[Bubble],
        _frame: &Frame,
    ) -> impl Future<Output = Result<(Vec<AttachmentImage>, Vec<AttachmentFile>, Vec<FailedAttachmentItem>)>> + Send {
        async move { Ok((Vec::new(), Vec::new(), Vec::new())) }
    }
}

struct ReplayToolClient {
    send_message_calls: Arc<AtomicUsize>,
}

impl ToolClient for ReplayToolClient {
    fn tool(&self, name: &str, _args: Value) -> impl Future<Output = Result<Value>> + Send {
        let name_owned = name.to_string();
        let calls = self.send_message_calls.clone();
        async move {
            match name_owned.as_str() {
                "send_message" => {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(json!("triggered"))
                }
                "get_send_status" => {
                    Ok(json!({
                        "status": "triggered",
                        "delivery": "unknown"
                    }))
                }
                _ => Ok(json!({})),
            }
        }
    }
}

fn collect_send_journals(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_send_journals(&p, out);
            } else if p.is_file() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("send-") && name.ends_with(".json") {
                        out.push(p);
                    }
                }
            }
        }
    }
}

pub async fn run_replay_cli(args: &[String]) -> Result<i32> {
    let replay_args = match parse_replay_args(args) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(2);
        }
    };

    if replay_args.work.exists() {
        if !replay_args.work.is_dir() {
            eprintln!("error: --work must be a directory");
            return Ok(2);
        }
        if let Ok(mut entries) = fs::read_dir(&replay_args.work) {
            if entries.next().is_some() {
                eprintln!("error: --work directory must be empty");
                return Ok(2);
            }
        }
    }

    match execute_replay(&replay_args).await {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(0)
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            Ok(2)
        }
    }
}

pub async fn execute_replay(args: &ReplayArgs) -> Result<ReplayReport> {
    if args.work.exists() {
        if !args.work.is_dir() {
            bail!("--work must be a directory");
        }
        let mut entries = fs::read_dir(&args.work)?;
        if entries.next().is_some() {
            bail!("--work directory must be empty");
        }
    } else {
        fs::create_dir_all(&args.work)
            .with_context(|| format!("failed to create --work directory {}", args.work.display()))?;
    }

    // 1. 读取帧文件并过滤出 recipient == args.session，按 t 排序
    let mut recorded_frames: Vec<(f64, Frame)> = Vec::new();
    for file_path in &args.frames {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("failed to read frames file {}", file_path.display()))?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(row) = serde_json::from_str::<FrameLine>(line) {
                if row.recipient == args.session {
                    recorded_frames.push((row.t, row.frame));
                }
            }
        }
    }

    recorded_frames.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let frames_count = recorded_frames.len();
    let (first_t, last_t) = if frames_count > 0 {
        (recorded_frames.first().unwrap().0, recorded_frames.last().unwrap().0)
    } else {
        (0.0, 0.0)
    };

    // 2. 状态目录与历史库
    let state_dir = args.work.join("state");
    fs::create_dir_all(&state_dir)?;
    let history = Arc::new(HistoryStore::open(&state_dir)?);

    let mut config = BotConfig::load(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    config.state_dir = state_dir.clone();

    // 保证 allowlist 匹配 session
    let target_entry = if let Some(existing) = config.allowlist.iter().find(|e| e.recipient == args.session) {
        existing.clone()
    } else {
        AllowlistEntry {
            recipient: args.session.clone(),
            kind: "private".to_string(),
            aliases: None,
        }
    };
    config.allowlist = vec![target_entry.clone()];

    let contact_id = history.ensure_contact(&config.account_id, &serde_json::to_value(&target_entry)?, false)?;

    let mut service = BotService::new(
        config,
        None,
        Some(history),
        Some(contact_id),
        None,
        None,
    )?;

    // 3. 注入时钟与 sleep
    let current_time = Arc::new(Mutex::new(first_t));
    let time_reader = current_time.clone();
    service.clock = Arc::new(move || *time_reader.lock().unwrap());
    service.sleep = Arc::new(|_| Box::pin(async {}));

    // 4. 注入 Capture
    let initial_frame = if frames_count > 0 {
        recorded_frames[0].1.clone()
    } else {
        Frame {
            profile: Some("wechat-4-dark".to_string()),
            scale: Some(json!(1)),
            title_lines: vec![args.session.clone()],
            bubbles: Vec::new(),
            platform_message_verified: None,
            sender_verified: None,
            clipped_bubble: None,
            width: None,
            height: None,
            captured_width: None,
            snapshot_id: None,
            window_id: None,
            observed_title: None,
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: serde_json::Map::new(),
        }
    };
    let capture_frame = Arc::new(Mutex::new(initial_frame));
    let mut capture = ReplayCapture {
        current_frame: capture_frame.clone(),
    };

    // 5. 注入 ToolClient
    let send_message_calls = Arc::new(AtomicUsize::new(0));
    let tool_client = ReplayToolClient {
        send_message_calls: send_message_calls.clone(),
    };

    // 6. 从 --journals 只读提取 reply 记录（支持 Python 与 Rust 格式，支持递归子目录与会话过滤）
    let mut journal_files = Vec::new();
    if args.journals.exists() {
        if args.journals.is_dir() {
            collect_send_journals(&args.journals, &mut journal_files);
        } else if args.journals.is_file() {
            journal_files.push(args.journals.clone());
        }
    }

    let mut journal_replies: HashMap<String, String> = HashMap::new();
    for path in &journal_files {
        if let Ok(data) = read_json::<Value>(path) {
            if let Some(rec) = data.get("recipient").and_then(|v| v.as_str()) {
                if rec != args.session {
                    continue;
                }
            }
            let key = data
                .get("idempotency_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("send-"))
                        .and_then(|n| n.strip_suffix(".json"))
                        .unwrap_or("")
                        .to_string()
                });
            if key.is_empty() {
                continue;
            }
            if let Some(reply) = data.get("reply").and_then(|v| v.as_str()) {
                if !reply.trim().is_empty() {
                    journal_replies.insert(key, reply.to_string());
                }
            }
        }
    }

    let missing_reply_list = Arc::new(Mutex::new(Vec::new()));
    let missing_clone = missing_reply_list.clone();
    let journal_replies = Arc::new(journal_replies);
    let replies_map = journal_replies.clone();

    let brain_fn: crate::core::turns::BrainFn = Arc::new(move |turn: &BrainTurn| {
        let key = turn.turn_id.clone();
        let replies = replies_map.clone();
        let missing = missing_clone.clone();
        Box::pin(async move {
            let reply_text = match replies.get(&key) {
                Some(r) => r.clone(),
                None => {
                    let mut list = missing.lock().unwrap();
                    if !list.contains(&key) {
                        list.push(key.clone());
                    }
                    "回放占位回复".to_string()
                }
            };
            Ok(BrainReply {
                reply: reply_text,
                metadata: HashMap::new(),
            })
        })
    });
    service.brain = Some(brain_fn);

    // 7. 回放帧执行：第一帧 advance 作基线，之后每帧执行 cycle
    if frames_count > 0 {
        let (t0, frame0) = &recorded_frames[0];
        *current_time.lock().unwrap() = *t0;
        service.advance(frame0.clone())?;

        for (t, frame) in &recorded_frames[1..] {
            *current_time.lock().unwrap() = *t;
            *capture_frame.lock().unwrap() = frame.clone();
            let _ = service.cycle(&tool_client, &mut capture).await;
        }
    }

    // 8. 收集 Rust 产生的日志（key -> status）
    let mut rust_logs: BTreeMap<String, String> = BTreeMap::new();
    let mut current_rust_files = Vec::new();
    collect_send_journals(&state_dir, &mut current_rust_files);
    for path in current_rust_files {
        if let Ok(data) = read_json::<Value>(&path) {
            let key = data
                .get("idempotency_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("send-"))
                        .and_then(|n| n.strip_suffix(".json"))
                        .unwrap_or("")
                        .to_string()
                });
            if !key.is_empty() {
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                rust_logs.insert(key, status);
            }
        }
    }

    // 9. 收集历史日志（Python 或 Rust，只取 created_at/attempted_at 在首帧与末帧时间范围内的，key -> status）
    let mut python_logs: BTreeMap<String, String> = BTreeMap::new();
    if frames_count > 0 {
        for path in &journal_files {
            if let Ok(data) = read_json::<Value>(path) {
                if let Some(rec) = data.get("recipient").and_then(|v| v.as_str()) {
                    if rec != args.session {
                        continue;
                    }
                }
                let created_at = data
                    .get("created_at")
                    .or_else(|| data.get("attempted_at"))
                    .and_then(|v| {
                        v.as_f64()
                            .or_else(|| v.as_i64().map(|i| i as f64))
                            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                    });
                if let Some(cat) = created_at {
                    if cat >= first_t - 1e-6 && cat <= last_t + 1e-6 {
                        let key = data
                            .get("idempotency_key")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                path.file_name()
                                    .and_then(|n| n.to_str())
                                    .and_then(|n| n.strip_prefix("send-"))
                                    .and_then(|n| n.strip_suffix(".json"))
                                    .unwrap_or("")
                                    .to_string()
                            });
                        if !key.is_empty() {
                            let status = data
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            python_logs.insert(key, status);
                        }
                    }
                }
            }
        }
    }

    // 10. 四类对比
    let mut all_keys: HashSet<String> = HashSet::new();
    for k in rust_logs.keys() {
        all_keys.insert(k.clone());
    }
    for k in python_logs.keys() {
        all_keys.insert(k.clone());
    }

    let mut matched = Vec::new();
    let mut rust_only = Vec::new();
    let mut python_only = Vec::new();
    let mut status_mismatch = Vec::new();

    let mut sorted_keys: Vec<String> = all_keys.into_iter().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        match (rust_logs.get(&key), python_logs.get(&key)) {
            (Some(r_stat), Some(p_stat)) => {
                if r_stat == p_stat {
                    matched.push(key);
                } else {
                    status_mismatch.push(StatusMismatch {
                        key,
                        rust: r_stat.clone(),
                        python: p_stat.clone(),
                        rust_status: r_stat.clone(),
                        python_status: p_stat.clone(),
                    });
                }
            }
            (Some(_), None) => {
                rust_only.push(key);
            }
            (None, Some(_)) => {
                python_only.push(key);
            }
            (None, None) => {}
        }
    }

    let missing_reply = missing_reply_list.lock().unwrap().clone();
    let send_calls_count = send_message_calls.load(Ordering::SeqCst);

    Ok(ReplayReport {
        frames: frames_count,
        rust: rust_logs.clone(),
        python: python_logs.clone(),
        rust_logs,
        python_logs,
        matched,
        rust_only,
        python_only,
        status_mismatch,
        missing_reply,
        send_calls: send_calls_count,
        send_message_calls: send_calls_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use std::time::SystemTime;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("test-replay-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_test_config(path: &Path, state_dir: &Path, recipient: &str) {
        let json_data = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test_account",
            "allowlist": [
                {
                    "recipient": recipient,
                    "kind": "private"
                }
            ],
            "state_dir": state_dir.to_str().unwrap(),
            "poll_seconds": 2,
            "cooldown_seconds": 0,
            "max_replies_per_hour": 100,
            "idle_check_seconds": 60,
            "turn_quiet_seconds": 0,
            "turn_max_seconds": 60,
            "images_per_model_call": 4,
            "max_attachment_mb": 50,
            "context_messages": 5,
            "manage_window": true,
            "capture_backend": "native",
            "history_http": {
                "bind": "127.0.0.1",
                "port": 8765
            },
            "model_config": {
                "backend": "codex-exec"
            }
        });
        fs::write(path, serde_json::to_string_pretty(&json_data).unwrap()).unwrap();
    }

    fn test_bubble(text: &str, direction: &str) -> Bubble {
        Bubble {
            text: text.to_string(),
            digest: text.to_string(),
            direction: direction.to_string(),
            kind: Some("text".to_string()),
            media_id: None,
            phash: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: Some(10),
            y: Some(10),
            width: Some(100),
            height: Some(30),
            extra: serde_json::Map::new(),
        }
    }

    fn test_frame(bubbles: Vec<Bubble>, title: &str) -> Frame {
        Frame {
            profile: Some("wechat-4-dark".to_string()),
            scale: Some(json!(1)),
            title_lines: vec![title.to_string()],
            bubbles,
            platform_message_verified: None,
            sender_verified: None,
            clipped_bubble: None,
            width: Some(800),
            height: Some(600),
            captured_width: Some(800),
            snapshot_id: None,
            window_id: None,
            observed_title: None,
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: serde_json::Map::new(),
        }
    }

    // 目录快照比对工具
    fn snapshot_dir(dir: &Path) -> HashMap<PathBuf, (u64, Option<SystemTime>)> {
        let mut map = HashMap::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Ok(meta) = entry.metadata() {
                    map.insert(p, (meta.len(), meta.modified().ok()));
                }
            }
        }
        map
    }

    #[tokio::test]
    async fn test_replay_args_parsing_and_validation() {
        let args = vec![
            "--config".to_string(),
            "cfg.json".to_string(),
            "--session".to_string(),
            "alice".to_string(),
            "--frames".to_string(),
            "f1.jsonl".to_string(),
            "f2.jsonl".to_string(),
            "--journals".to_string(),
            "py_dir".to_string(),
            "--work".to_string(),
            "work_dir".to_string(),
        ];
        let parsed = parse_replay_args(&args).unwrap();
        assert_eq!(parsed.config, PathBuf::from("cfg.json"));
        assert_eq!(parsed.session, "alice");
        assert_eq!(parsed.frames, vec![PathBuf::from("f1.jsonl"), PathBuf::from("f2.jsonl")]);
        assert_eq!(parsed.journals, PathBuf::from("py_dir"));
        assert_eq!(parsed.work, PathBuf::from("work_dir"));

        // 缺少参数测试
        let bad_args = vec!["--config".to_string(), "cfg.json".to_string()];
        assert!(parse_replay_args(&bad_args).is_err());
        assert_eq!(run_replay_cli(&bad_args).await.unwrap(), 2, "参数不合法退出码为 2");
    }

    #[tokio::test]
    async fn test_work_directory_non_empty_exits_2() {
        let td = TestDir::new();
        let work = td.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("existing.txt"), "hello").unwrap();

        let cfg = td.path().join("config.json");
        write_test_config(&cfg, &work.join("state"), "alice");

        let frames = td.path().join("frames.jsonl");
        fs::write(&frames, "").unwrap();

        let journals = td.path().join("journals");
        fs::create_dir_all(&journals).unwrap();

        let args = vec![
            "--config".to_string(),
            cfg.to_str().unwrap().to_string(),
            "--session".to_string(),
            "alice".to_string(),
            "--frames".to_string(),
            frames.to_str().unwrap().to_string(),
            "--journals".to_string(),
            journals.to_str().unwrap().to_string(),
            "--work".to_string(),
            work.to_str().unwrap().to_string(),
        ];

        let code = run_replay_cli(&args).await.unwrap();
        assert_eq!(code, 2, "--work 非空必须返回退出码 2");
    }

    #[tokio::test]
    async fn test_replay_baseline_first_frame_does_not_reply() {
        let td = TestDir::new();
        let work = td.path().join("work");
        let cfg = td.path().join("config.json");
        write_test_config(&cfg, &work.join("state"), "alice");

        let frames_path = td.path().join("frames.jsonl");
        let f0 = test_frame(vec![test_bubble("历史消息", "inbound_visual")], "alice");
        let line0 = json!({
            "t": 1000.0,
            "op": "frame",
            "recipient": "alice",
            "frame": f0,
        });
        fs::write(&frames_path, format!("{}\n", line0)).unwrap();

        let journals = td.path().join("journals");
        fs::create_dir_all(&journals).unwrap();

        let args = ReplayArgs {
            config: cfg,
            session: "alice".to_string(),
            frames: vec![frames_path],
            journals,
            work,
        };

        let report = execute_replay(&args).await.unwrap();
        assert_eq!(report.frames, 1);
        assert_eq!(report.send_calls, 0, "第一帧建立基线，不发回复");
        assert!(report.rust_logs.is_empty());
    }

    #[tokio::test]
    async fn test_replay_does_not_modify_inputs() {
        let td = TestDir::new();
        let work = td.path().join("work");
        let cfg = td.path().join("config.json");
        write_test_config(&cfg, &work.join("state"), "alice");

        let frames_dir = td.path().join("frames_dir");
        fs::create_dir_all(&frames_dir).unwrap();
        let frames_path = frames_dir.join("frames.jsonl");

        let f0 = test_frame(vec![test_bubble("初始气泡", "inbound_visual")], "alice");
        let line0 = json!({
            "t": 1000.0,
            "op": "frame",
            "recipient": "alice",
            "frame": f0,
        });
        fs::write(&frames_path, format!("{}\n", line0)).unwrap();

        let journals_dir = td.path().join("journals_dir");
        fs::create_dir_all(&journals_dir).unwrap();
        let py_journal = journals_dir.join("send-test.json");
        fs::write(&py_journal, json!({
            "idempotency_key": "test_key",
            "created_at": 1000.0,
            "status": "triggered",
            "reply": "python reply"
        }).to_string()).unwrap();

        let frames_snap_before = snapshot_dir(&frames_dir);
        let journals_snap_before = snapshot_dir(&journals_dir);

        let args = ReplayArgs {
            config: cfg,
            session: "alice".to_string(),
            frames: vec![frames_path],
            journals: journals_dir.clone(),
            work,
        };

        let _ = execute_replay(&args).await.unwrap();

        let frames_snap_after = snapshot_dir(&frames_dir);
        let journals_snap_after = snapshot_dir(&journals_dir);

        assert_eq!(frames_snap_before, frames_snap_after, "frames 目录绝不被修改");
        assert_eq!(journals_snap_before, journals_snap_after, "journals 目录绝不被修改");
    }

    #[tokio::test]
    async fn test_replay_e2e_full_comparisons_and_missing_reply() {
        let td = TestDir::new();
        let work = td.path().join("work");
        let cfg = td.path().join("config.json");
        write_test_config(&cfg, &work.join("state"), "alice");

        let journals = td.path().join("journals");
        fs::create_dir_all(&journals).unwrap();

        // 构造基线帧和后续消息帧
        let f0 = test_frame(vec![test_bubble("旧消息", "inbound_visual")], "alice");
        let b1 = test_bubble("第一条新消息", "inbound_visual");
        let f1 = test_frame(vec![test_bubble("旧消息", "inbound_visual"), b1.clone()], "alice");

        let b2 = test_bubble("第二条新消息", "inbound_visual");
        let f2 = test_frame(vec![test_bubble("旧消息", "inbound_visual"), b1.clone(), b2.clone()], "alice");

        let b3 = test_bubble("第三条新消息", "inbound_visual");
        let f3 = test_frame(vec![test_bubble("旧消息", "inbound_visual"), b1.clone(), b2.clone(), b3.clone()], "alice");

        // 使用临时 BotService 计算 batch_key
        let tmp_dir = td.path().join("tmp_state");
        let tmp_cfg_path = td.path().join("tmp_cfg.json");
        write_test_config(&tmp_cfg_path, &tmp_dir, "alice");
        let tmp_svc = BotService::new(
            BotConfig::load(&tmp_cfg_path).unwrap(),
            None,
            None,
            None,
            None,
            None,
        ).unwrap();

        let k1 = tmp_svc.batch_key(&f0, &[b1.clone()]);
        let k2 = tmp_svc.batch_key(&f1, &[b2.clone()]);
        let k3 = tmp_svc.batch_key(&f2, &[b3.clone()]);

        // 写入 Python journals：
        // 1. k1 -> matched: Python status = triggered, reply 提供
        let k1_file = journals.join(format!("send-{k1}.json"));
        fs::write(&k1_file, json!({
            "idempotency_key": k1,
            "created_at": 1001.0,
            "status": "triggered",
            "reply": "已收到第一条消息"
        }).to_string()).unwrap();

        // 2. k2 -> status_mismatch: Python status = not_sent, reply 提供
        let k2_file = journals.join(format!("send-{k2}.json"));
        fs::write(&k2_file, json!({
            "idempotency_key": k2,
            "created_at": 1002.0,
            "status": "not_sent",
            "reply": "已收到第二条消息"
        }).to_string()).unwrap();

        // 3. k3 -> rust_only + missing_reply: 不写入 journals，由 Rust 产生且缺 reply

        // 4. python_only -> 只有 Python 有（created_at 在范围内）
        let k_py_only = "py-only-key-123456";
        let k_py_file = journals.join(format!("send-{k_py_only}.json"));
        fs::write(&k_py_file, json!({
            "idempotency_key": k_py_only,
            "created_at": 1002.5,
            "status": "triggered",
            "reply": "仅 Python 记录"
        }).to_string()).unwrap();

        // 5. 超出时间范围的 Python 日志 -> 应该被忽略
        let k_out_file = journals.join("send-out-of-range.json");
        fs::write(&k_out_file, json!({
            "idempotency_key": "out_of_range_key",
            "created_at": 500.0,
            "status": "triggered",
            "reply": "早期消息"
        }).to_string()).unwrap();

        // 写入 frames 文件
        let frames_path = td.path().join("frames.jsonl");
        let mut file = File::create(&frames_path).unwrap();
        writeln!(file, "{}", json!({"t": 1000.0, "op": "frame", "recipient": "alice", "frame": f0})).unwrap();
        writeln!(file, "{}", json!({"t": 1001.0, "op": "frame", "recipient": "alice", "frame": f1})).unwrap();
        writeln!(file, "{}", json!({"t": 1002.0, "op": "frame", "recipient": "alice", "frame": f2})).unwrap();
        writeln!(file, "{}", json!({"t": 1003.0, "op": "frame", "recipient": "alice", "frame": f3})).unwrap();
        // 混入一条其他 recipient 的记录，验证过滤
        writeln!(file, "{}", json!({"t": 1004.0, "op": "frame", "recipient": "bob", "frame": f3})).unwrap();

        let args = ReplayArgs {
            config: cfg,
            session: "alice".to_string(),
            frames: vec![frames_path],
            journals,
            work,
        };

        let report = execute_replay(&args).await.unwrap();

        // 验证 frames 数量
        assert_eq!(report.frames, 4);

        // 验证四类对比
        assert_eq!(report.matched, vec![k1.clone()], "k1 应该匹配");
        assert_eq!(report.rust_only, vec![k3.clone()], "k3 只有 Rust 有");
        assert_eq!(report.python_only, vec![k_py_only.to_string()], "k_py_only 只有 Python 有");
        
        assert_eq!(report.status_mismatch.len(), 1);
        let mismatch = &report.status_mismatch[0];
        assert_eq!(mismatch.key, k2);
        assert_eq!(mismatch.rust, "triggered");
        assert_eq!(mismatch.python, "not_sent");
        assert_eq!(mismatch.rust_status, "triggered");
        assert_eq!(mismatch.python_status, "not_sent");

        // 验证 missing_reply
        assert_eq!(report.missing_reply, vec![k3.clone()]);

        // 验证发送调用次数：k1, k2, k3 各调用一次 send_message
        assert_eq!(report.send_calls, 3);
        assert_eq!(report.send_message_calls, 3);

        // 验证超出范围的 Python 日志被正确排除
        assert!(!report.python_logs.contains_key("out_of_range_key"));
    }

    #[tokio::test]
    async fn test_replay_supports_rust_journals_in_subdirectories() {
        let td = TestDir::new();
        let work = td.path().join("work");
        let cfg = td.path().join("config.json");
        write_test_config(&cfg, &work.join("state"), "alice");

        // 构造 Rust 核心运行时生成的 sessions/<contact_id>/send-*.json 结构
        let journals_root = td.path().join("state_journals");
        let alice_dir = journals_root.join("sessions").join("c_alice_123");
        let bob_dir = journals_root.join("sessions").join("c_bob_456");
        fs::create_dir_all(&alice_dir).unwrap();
        fs::create_dir_all(&bob_dir).unwrap();

        let f0 = test_frame(vec![test_bubble("初始", "inbound_visual")], "alice");
        let b1 = test_bubble("新消息", "inbound_visual");
        let f1 = test_frame(vec![test_bubble("初始", "inbound_visual"), b1.clone()], "alice");

        let tmp_dir = td.path().join("tmp_state");
        let tmp_cfg = td.path().join("tmp_cfg.json");
        write_test_config(&tmp_cfg, &tmp_dir, "alice");
        let tmp_svc = BotService::new(BotConfig::load(&tmp_cfg).unwrap(), None, None, None, None, None).unwrap();
        let k1 = tmp_svc.batch_key(&f0, &[b1.clone()]);

        // 写入由 Rust 核心生成的 journal（含 recipient: alice，嵌套在 sessions/c_alice_123/）
        let k1_file = alice_dir.join(format!("send-{k1}.json"));
        fs::write(&k1_file, json!({
            "account_id": "test_account",
            "recipient": "alice",
            "idempotency_key": k1,
            "created_at": 1001.0,
            "status": "triggered",
            "reply": "Rust 核心在线回复"
        }).to_string()).unwrap();

        // 写入另一个会话 bob 的 journal，验证不被混淆
        let bob_file = bob_dir.join("send-bobkey.json");
        fs::write(&bob_file, json!({
            "account_id": "test_account",
            "recipient": "bob",
            "idempotency_key": "bobkey",
            "created_at": 1001.0,
            "status": "triggered",
            "reply": "Bob 回复"
        }).to_string()).unwrap();

        let frames_path = td.path().join("frames.jsonl");
        let mut file = File::create(&frames_path).unwrap();
        writeln!(file, "{}", json!({"t": 1000.0, "op": "capture", "recipient": "alice", "frame": f0})).unwrap();
        writeln!(file, "{}", json!({"t": 1001.0, "op": "scroll_chat", "recipient": "alice", "frame": f1})).unwrap();

        let args = ReplayArgs {
            config: cfg,
            session: "alice".to_string(),
            frames: vec![frames_path],
            journals: journals_root,
            work,
        };

        let report = execute_replay(&args).await.unwrap();
        assert_eq!(report.frames, 2);
        assert_eq!(report.matched, vec![k1.clone()]);
        assert_eq!(report.rust_only, Vec::<String>::new());
        assert_eq!(report.python_only, Vec::<String>::new());
        assert!(report.missing_reply.is_empty());
        assert!(!report.python_logs.contains_key("bobkey"), "属于其他会话的 journal 不被混淆");
        assert_eq!(report.python_logs.get(&k1).map(|s| s.as_str()), Some("triggered"));
    }
}
