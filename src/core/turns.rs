fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn format_fp_json(fp: &crate::core::policy::Fingerprint) -> String {
    match &fp.2 {
        Some(p) => format!(
            "[\"{}\", \"{}\", \"{}\"]",
            escape_json_str(&fp.0),
            escape_json_str(&fp.1),
            escape_json_str(p)
        ),
        None => format!(
            "[\"{}\", \"{}\", null]",
            escape_json_str(&fp.0),
            escape_json_str(&fp.1)
        ),
    }
}
use crate::brain::{
    Brain, BrainReply, ContactInfo, ContextItem, FailedAttachment, MediaRef, Turn as BrainTurn,
};
use crate::core::config::{BotConfig, candidate_names, clean_title};
use crate::core::delivery::{EchoOutcome, ToolClient, is_not_sent};
use crate::core::history::store::{HistoryStore, similar};
use crate::core::model::{Bubble, Frame};
use crate::core::policy::{
    ContinuityError, Fingerprint, Policy, echo_matches, fingerprint, merge_pages, new_bubbles,
    reconcile, relocate, separator_tail, strip_member_count, validate_reply,
};
use crate::core::state::{read_json, write_private};
use crate::core::worker_client::action_interrupted;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const MODEL_RETRIES: usize = 2;
pub const MODEL_RETRY_SECONDS: f64 = 30.0;
pub const UPLOAD_EXTRA_SECONDS: f64 = 120.0;
pub const MAX_MODEL_IMAGE_BYTES: u64 = 30 * 1024 * 1024;
/// 找回衔接点时最多向上滚动的格数。一格 = 布局里的 points_per_tick（4.1.15 为 90 点），不是一整屏。
/// 微信一次最多发 9 张图、每张约 155 点高加间距，最坏约 1600 点 ≈ 18 格；取 24 留余量。
/// 原来的 5 格（约 450 点）连 3 张图都翻不过去，多图会被整批当历史跳过（2026-11-08 实机）。
/// 找到衔接点就停，平时不会多滚。
pub const GAP_SCROLL_PAGES: u32 = 24;

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub type BrainFn = Arc<
    dyn Fn(&BrainTurn) -> Pin<Box<dyn Future<Output = Result<BrainReply>> + Send>> + Send + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentImage {
    pub path: PathBuf,
    pub label: String,
    pub source: String,
    pub bubble: Bubble,
    #[serde(default)]
    pub media_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentFile {
    pub filename: String,
    pub label: String,
    pub original_bytes: bool,
    pub path: PathBuf,
    pub bubble: Bubble,
    #[serde(flatten)]
    pub content: Map<String, Value>,
}

pub fn model_view_file(file: &AttachmentFile) -> Value {
    let mut map = Map::new();
    map.insert("filename".to_string(), json!(file.filename));
    map.insert("label".to_string(), json!(file.label));
    map.insert("original_bytes".to_string(), json!(file.original_bytes));
    for (k, v) in &file.content {
        if k != "path" && k != "bubble" {
            map.insert(k.clone(), v.clone());
        }
    }
    Value::Object(map)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailedAttachmentItem {
    pub label: String,
    pub kind: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub trait Capture: Send + Sync {
    fn frame(&mut self) -> impl Future<Output = Result<Frame>> + Send;
    fn scroll(
        &mut self,
        _direction: &str,
        _pages: u32,
    ) -> impl Future<Output = Result<Frame>> + Send {
        async move { bail!("scroll not supported") }
    }
    fn download(&mut self, media_id: &str) -> impl Future<Output = Result<PathBuf>> + Send {
        self.download_kind(media_id, "image")
    }
    fn download_kind(
        &mut self,
        _media_id: &str,
        _kind: &str,
    ) -> impl Future<Output = Result<PathBuf>> + Send {
        async move { bail!("download not supported") }
    }
    fn attachments(
        &mut self,
        _items: &[Bubble],
        _frame: &Frame,
    ) -> impl Future<
        Output = Result<(
            Vec<AttachmentImage>,
            Vec<AttachmentFile>,
            Vec<FailedAttachmentItem>,
        )>,
    > + Send {
        async move { bail!("attachments not supported") }
    }
}

pub fn ensure_unreadable_named(reply: &str, failed: &[FailedAttachmentItem]) -> Result<String> {
    let missing: Vec<&str> = failed
        .iter()
        .map(|f| f.label.as_str())
        .filter(|label| !reply.contains(*label))
        .collect();
    if missing.is_empty() {
        return Ok(reply.to_string());
    }
    let mut suffix = format!("{}打不开，麻烦重发一下。", missing.join("、"));
    if suffix.chars().count() > 200 {
        suffix = format!("有{}个文件或图片打不开，麻烦重发一下。", missing.len());
    }
    let suffix_len = suffix.chars().count();
    let max_head = 200usize.saturating_sub(suffix_len);
    let head_chars: String = reply.chars().take(max_head).collect();
    let combined = format!("{}{}", head_chars.trim_end(), suffix);
    validate_reply(&combined).map_err(|e| anyhow::anyhow!("{e:?}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnData {
    pub opened_at: f64,
    pub baseline: Frame,
    pub bubbles: Vec<Bubble>,
    pub last_inbound_at: f64,
    pub frame: Frame,
    #[serde(default)]
    pub model_failures: usize,
    #[serde(default)]
    pub retry_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_snapshot: Option<Value>,
}

pub type UnderstandFn = Arc<
    dyn Fn(
            Vec<crate::brain::understanding::ImageItem>,
            String,
            crate::core::config::ModelConfig,
            usize,
        ) -> Pin<Box<dyn Future<Output = Vec<Option<String>>> + Send>>
        + Send
        + Sync,
>;

pub struct BotService {
    pub config: BotConfig,
    pub root: PathBuf,
    pub recipient: String,
    pub account: String,
    pub history: Option<Arc<HistoryStore>>,
    pub contact_id: Option<String>,
    pub names: Vec<String>,
    pub group: bool,
    pub policy: Policy,
    pub stop: Arc<AtomicBool>,
    pub cursor_path: PathBuf,
    pub cursor: Option<Frame>,
    pub paused: bool,
    pub pause_reason: Option<String>,
    pub recovery_unconfirmed: bool,
    pub upload_pending: bool,
    pub had_incoming: bool,
    pub turn_path: PathBuf,
    pub turn: Option<TurnData>,
    pub clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub sleep: Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
    pub ownership: String,
    pub brain: Option<BrainFn>,
    pub routing_check: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    pub understand_fn: Option<UnderstandFn>,
    pub max_model_image_bytes: Option<u64>,
    pub model_retries: Option<usize>,
    last_health_log: Arc<std::sync::Mutex<Option<(String, String)>>>,
    last_health_log_at: Arc<std::sync::Mutex<f64>>,
}

impl BotService {
    pub fn new(
        config: BotConfig,
        routing_check: Option<Arc<dyn Fn() -> String + Send + Sync>>,
        history: Option<Arc<HistoryStore>>,
        contact_id: Option<String>,
        ownership: Option<String>,
        names: Option<Vec<String>>,
    ) -> Result<Self> {
        let root = config.state_dir.clone();
        fs::create_dir_all(&root)?;
        let first_entry = config
            .allowlist
            .first()
            .context("allowlist must have at least one entry")?;
        let recipient = first_entry.recipient.clone();
        let account = config.account_id.clone();
        let group = first_entry.kind == "group";
        let names_list = names.unwrap_or_else(|| candidate_names(first_entry));
        let policy = Policy::new(
            account.clone(),
            recipient.clone(),
            "service",
            Some(names_list.clone()),
            group,
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let binding_path = root.join("binding.json");
        let binding = if let Some(cid) = &contact_id {
            json!({
                "host": config.host,
                "account_id": account,
                "contact_id": cid,
                "kind": first_entry.kind,
            })
        } else {
            json!({
                "host": config.host,
                "account_id": account,
                "recipient": recipient,
                "kind": first_entry.kind,
            })
        };

        if binding_path.exists() {
            let existing: Value = read_json(&binding_path)?;
            if existing != binding {
                bail!(
                    "state belongs to a different account or contact; use a separate state directory"
                );
            }
        }
        write_private(&binding_path, &binding)?;

        let cursor_path = root.join("cursor.json");
        let mut cursor: Option<Frame> = if cursor_path.exists() {
            Some(read_json(&cursor_path)?)
        } else {
            None
        };

        if let Some(c) = &mut cursor {
            if c.profile.as_deref() == Some("wechat-4-dark-1x") {
                c.profile = Some("wechat-4-dark".to_string());
                c.scale = Some(json!(1));
            }
            policy
                .check_frame(c)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }

        // 启动时处理遗留的日志文件
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if file_name.starts_with("send-") && file_name.ends_with(".json") {
                        if let Ok(mut data) = read_json::<Value>(&path) {
                            let status = data.get("status").and_then(|v| v.as_str());
                            if matches!(
                                status,
                                Some("preparing_media" | "generating" | "generated")
                            ) {
                                if let Some(map) = data.as_object_mut() {
                                    map.insert("status".to_string(), json!("not_sent"));
                                    map.insert("closed_on_startup".to_string(), json!(true));
                                    let _ = write_private(&path, &data);
                                }
                            }
                        }
                    }
                }
            }
        }

        let turn_path = root.join("turn.json");
        let turn: Option<TurnData> = if turn_path.exists() {
            read_json(&turn_path).ok()
        } else {
            None
        };

        let ownership_str = ownership.unwrap_or_else(|| {
            let mut hasher = Sha256::new();
            hasher.update(root.to_string_lossy().as_bytes());
            to_hex(&hasher.finalize())
        });

        Ok(Self {
            config,
            root,
            recipient,
            account,
            history,
            contact_id,
            names: names_list,
            group,
            policy,
            stop: Arc::new(AtomicBool::new(false)),
            cursor_path,
            cursor,
            paused: false,
            pause_reason: None,
            recovery_unconfirmed: false,
            upload_pending: false,
            had_incoming: false,
            turn_path,
            turn,
            clock: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
            }),
            sleep: Arc::new(|d| Box::pin(tokio::time::sleep(d))),
            ownership: ownership_str,
            brain: None,
            routing_check,
            understand_fn: None,
            max_model_image_bytes: None,
            model_retries: None,
            last_health_log: Arc::new(std::sync::Mutex::new(None)),
            last_health_log_at: Arc::new(std::sync::Mutex::new(0.0)),
        })
    }

    pub fn batch_key(&self, previous: &Frame, incoming: &[Bubble]) -> String {
        let prev_fps: Vec<String> = previous
            .bubbles
            .iter()
            .map(|b| format_fp_json(&fingerprint(b)))
            .collect();
        let inc_fps: Vec<String> = incoming
            .iter()
            .map(|b| format_fp_json(&fingerprint(b)))
            .collect();
        let raw_json = format!(
            "[\"{}\", \"{}\", [{}], [{}]]",
            escape_json_str(&self.account),
            escape_json_str(&self.recipient),
            prev_fps.join(", "),
            inc_fps.join(", ")
        );
        let mut hasher = Sha256::new();
        hasher.update(raw_json.as_bytes());
        format!("bot-service-{}", to_hex(&hasher.finalize()))
    }

    pub fn advance(&mut self, frame: Frame) -> Result<()> {
        write_private(&self.cursor_path, &frame)?;
        self.cursor = Some(frame);
        Ok(())
    }

    pub fn clean_recipient(&self, frame: &Frame) -> String {
        let title = clean_title(frame.title_lines.first().map(|s| s.as_str()).unwrap_or(""));
        if self.group {
            strip_member_count(&title)
        } else {
            title
        }
    }

    pub fn archive_media(
        &self,
        incoming: &[Bubble],
        input_ids: &[String],
        images: &mut [AttachmentImage],
        files: &[AttachmentFile],
    ) {
        let Some(history) = &self.history else {
            return;
        };
        if input_ids.len() != incoming.len() {
            return;
        }
        for item in images.iter_mut() {
            let mid = incoming
                .iter()
                .zip(input_ids.iter())
                .find(|(b, _)| **b == item.bubble)
                .map(|(_, m)| m);
            if let Some(message_id) = mid {
                if let Ok(id) =
                    history.attach_media(message_id, &item.path, "image", None, &item.source, None)
                {
                    item.media_id = Some(id);
                }
            }
        }
        for item in files {
            let mid = incoming
                .iter()
                .zip(input_ids.iter())
                .find(|(b, _)| **b == item.bubble)
                .map(|(_, m)| m);
            if let Some(message_id) = mid {
                let _ = history.attach_media(
                    message_id,
                    &item.path,
                    "file",
                    Some(&item.filename),
                    "original",
                    None,
                );
                let _ = history.finish_media_job(message_id, "done", None);
            }
        }
    }

    pub async fn understand_images(
        &self,
        text: &str,
        images: &[AttachmentImage],
        failed: &mut Vec<FailedAttachmentItem>,
        context: &[ContextItem],
    ) -> Result<(Vec<Value>, Vec<PathBuf>, usize)> {
        let batch = self.config.images_per_model_call;
        let config = self.config.model_config.clone();
        let max_image_bytes = self.max_model_image_bytes.unwrap_or(30 * 1024 * 1024);

        let mut notes = Vec::new();
        let mut model_images = Vec::new();

        if images.len() > batch && config.backend == "openai-compatible" {
            let image_items: Vec<crate::brain::understanding::ImageItem> = images
                .iter()
                .map(|img| crate::brain::understanding::ImageItem {
                    path: img.path.clone(),
                    label: img.label.clone(),
                })
                .collect();

            let found = if let Some(custom_fn) = &self.understand_fn {
                custom_fn(image_items, text.to_string(), config.clone(), batch).await
            } else {
                crate::brain::understanding::understand(&image_items, text, &config, batch).await
            };

            for (item, note_opt) in images.iter().zip(found.into_iter()) {
                match note_opt {
                    None => {
                        failed.push(FailedAttachmentItem {
                            label: item.label.clone(),
                            kind: "image".to_string(),
                            reason: "model_failed".to_string(),
                            detail: None,
                        });
                    }
                    Some(note) => {
                        notes.push(json!({
                            "label": item.label,
                            "notes": note,
                        }));
                        if let Some(history) = &self.history {
                            if let Some(mid) = &item.media_id {
                                let _ = history.set_understanding(mid, &note);
                            }
                        }
                    }
                }
            }
            model_images.clear();
        } else {
            let mut total = 0u64;
            for item in images {
                let size = match fs::metadata(&item.path) {
                    Ok(m) => m.len(),
                    Err(_) => 0,
                };
                if total + size > max_image_bytes {
                    failed.push(FailedAttachmentItem {
                        label: item.label.clone(),
                        kind: "image".to_string(),
                        reason: "too_large".to_string(),
                        detail: None,
                    });
                    continue;
                }
                total += size;
                model_images.push(item.path.clone());
            }
        }

        let mut total: u64 = model_images
            .iter()
            .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        let current = model_images.len();

        for item in context {
            if let Some(media) = &item.media {
                if media.kind.as_deref() == Some("image") && media.understanding.is_none() {
                    if let Some(p) = &media.path {
                        if model_images.len() < batch && p.is_file() {
                            let size = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                            if total + size <= max_image_bytes {
                                model_images.push(p.clone());
                                total += size;
                            }
                        }
                    }
                }
            }
        }

        let earlier = model_images.len() - current;
        Ok((notes, model_images, earlier))
    }

    pub fn limited(&self) -> bool {
        let mut attempts = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("send-") && name.ends_with(".json") {
                        if let Ok(data) = read_json::<Value>(&path) {
                            if let Some(t) = data.get("attempted_at").and_then(|v| v.as_f64()) {
                                if t > 0.0 {
                                    attempts.push(t);
                                }
                            }
                        }
                    }
                }
            }
        }
        let now = (self.clock)();
        let recent: Vec<f64> = attempts.into_iter().filter(|&t| now - t < 3600.0).collect();
        let max_recent = recent.iter().copied().fold(0.0f64, f64::max);
        let count_limited = recent.len() >= self.config.max_replies_per_hour as usize;
        let cooldown_limited =
            !recent.is_empty() && (now - max_recent < self.config.cooldown_seconds as f64);
        count_limited || cooldown_limited
    }

    pub fn target_allowed(&mut self) -> bool {
        if let Some(check) = &self.routing_check {
            let verdict = check();
            if verdict != "allowed" {
                let mut d = Map::new();
                d.insert("rule".to_string(), json!(verdict));
                let _ = self.health("ignored", d);
                return false;
            }
        }
        true
    }

    pub fn health(&mut self, state: &str, details: Map<String, Value>) -> Result<()> {
        let d_map = details;
        let mut map = Map::new();
        map.insert("pid".to_string(), json!(std::process::id()));
        map.insert("state".to_string(), json!(state));
        map.insert("updated_at".to_string(), json!((self.clock)()));
        map.insert("account_id".to_string(), json!(self.account));
        map.insert("allowlist".to_string(), json!([self.recipient]));
        map.insert("groups_enabled".to_string(), json!(self.group));
        let model_backend = self.config.model_config.backend.clone();
        map.insert("model_backend".to_string(), json!(model_backend));
        map.insert("platform_message_verified".to_string(), json!(false));
        for (k, v) in &d_map {
            map.insert(k.clone(), v.clone());
        }

        let now = (self.clock)();
        let details_json = serde_json::to_string(&d_map).unwrap_or_default();
        let sig = (state.to_string(), details_json);

        let mut last_log = self.last_health_log.lock().unwrap();
        let mut last_at = self.last_health_log_at.lock().unwrap();
        if last_log.as_ref() != Some(&sig) || now - *last_at >= 60.0 {
            tracing::info!(
                event = "state",
                state = state,
                account_id = self.account.as_str()
            );
            *last_log = Some(sig);
            *last_at = now;
        }

        let health_path = self.root.join("health.json");
        write_private(&health_path, &Value::Object(map))?;
        Ok(())
    }

    pub fn resume_session(&mut self) -> Result<()> {
        let handoff_path = self.root.join("handoff.json");
        if handoff_path.exists() {
            let _ = fs::remove_file(&handoff_path);
        }
        Ok(())
    }

    pub fn turn_due(&self) -> bool {
        let Some(turn) = &self.turn else {
            return false;
        };
        let now = (self.clock)();
        if now < turn.retry_at {
            return false;
        }
        let limit = self.config.turn_max_seconds as f64;
        if self.upload_pending && now - turn.opened_at < limit + UPLOAD_EXTRA_SECONDS {
            return false;
        }
        (now - turn.last_inbound_at >= self.config.turn_quiet_seconds as f64)
            || (now - turn.opened_at >= limit)
    }

    pub fn keep_turn(&mut self, turn: TurnData) -> Result<()> {
        write_private(&self.turn_path, &turn)?;
        self.turn = Some(turn);
        Ok(())
    }

    pub fn drop_turn(&mut self, source: &str) -> Result<()> {
        self.turn = None;
        let _ = fs::remove_file(&self.turn_path);
        let mut d = Map::new();
        d.insert(
            "reason".to_string(),
            json!(format!("already replied ({source})")),
        );
        self.health("watching", d)?;
        Ok(())
    }

    pub fn own_echo(&self, bubble: &Bubble) -> bool {
        let text = &bubble.text;
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("send-") && name.ends_with(".json") {
                        if let Ok(data) = read_json::<Value>(&path) {
                            if let Some(reply) = data.get("reply").and_then(|v| v.as_str()) {
                                let attempted_at = data
                                    .get("attempted_at")
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.0);
                                if (self.clock)() - attempted_at < 600.0 {
                                    if echo_matches(text, reply) || similar(text, reply, 0.8) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    pub fn replied_meanwhile(
        &mut self,
        confirmed: &Frame,
        latest: &Frame,
        input_ids: &[String],
        send_key: Option<&str>,
    ) -> Result<bool> {
        let mut source = None;
        if let Some(history) = &self.history {
            if let Some(cid) = &self.contact_id {
                let replied = history.replied_after(cid, input_ids, send_key)?;
                if replied {
                    source = Some("history");
                }
            }
        }

        if source.is_none() {
            let reconciled = reconcile(&confirmed.bubbles, &latest.bubbles);
            let later = new_bubbles(&confirmed.bubbles, &reconciled).unwrap_or_default();
            if later
                .iter()
                .any(|b| b.direction == "outbound_visual" && !self.own_echo(b))
            {
                source = Some("screen");
            }
        }

        if let Some(s) = source {
            let mut d = Map::new();
            d.insert(
                "reason".to_string(),
                json!(format!("already replied ({s})")),
            );
            let _ = self.health("watching", d);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn write_journal(&self, journal: &Path, record: &Value) -> Result<()> {
        write_private(journal, record)?;
        if let Some(history) = &self.history {
            if let Some(key) = record.get("idempotency_key").and_then(|v| v.as_str()) {
                if let Some(status) = record.get("status").and_then(|v| v.as_str()) {
                    let echo = record.get("client_echo_observed").and_then(|v| v.as_bool());
                    let _ = history.update_send(key, status, echo);
                }
            }
        }
        Ok(())
    }

    pub async fn recover_startup_journals(&mut self, capture: &mut impl Capture) -> Result<()> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("send-") && name.ends_with(".json") {
                        files.push(path);
                    }
                }
            }
        }
        files.sort();

        for p in files {
            if let Ok(mut data) = read_json::<Value>(&p) {
                let status = data.get("status").and_then(|v| v.as_str());
                if matches!(status, Some("attempting" | "unknown")) {
                    let reply = data.get("reply").and_then(|v| v.as_str()).unwrap_or("");
                    if reply.is_empty() {
                        if let Some(map) = data.as_object_mut() {
                            map.insert("status".to_string(), json!("not_sent"));
                            let _ = write_private(&p, &data);
                        }
                        continue;
                    }
                    if let Ok(frame) = capture.frame().await {
                        if self.policy.check_frame(&frame).is_ok() {
                            let echoed = frame.bubbles.iter().any(|b| {
                                b.direction == "outbound_visual" && echo_matches(&b.text, reply)
                            });
                            if let Some(map) = data.as_object_mut() {
                                if echoed {
                                    map.insert("status".to_string(), json!("triggered"));
                                    map.insert("client_echo_observed".to_string(), json!(true));
                                } else {
                                    map.insert("status".to_string(), json!("not_sent"));
                                }
                                let _ = write_private(&p, &data);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn scroll_to_bottom(
        &mut self,
        capture: &mut impl Capture,
        mut pages: u32,
    ) -> Result<Frame> {
        while pages > 0 {
            let chunk = std::cmp::min(40, pages);
            let _ = capture.scroll("down", chunk).await?;
            pages -= chunk;
        }
        let bottom = capture.frame().await?;
        self.policy
            .check_frame(&bottom)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(bottom)
    }

    pub async fn recover_gap(
        &mut self,
        capture: &mut impl Capture,
        frame: &Frame,
        max_pages: u32,
    ) -> Result<Option<Vec<Bubble>>> {
        self.recovery_unconfirmed = false;
        let Some(cursor) = &self.cursor else {
            return Ok(None);
        };
        if cursor.bubbles.is_empty() {
            return Ok(None);
        }

        let mut pages = vec![frame.bubbles.clone()];
        let mut changes = None;
        let mut merged = None;

        for attempt in 0..max_pages {
            let page = match capture.scroll("up", 1).await {
                Ok(p) => p,
                Err(err) => {
                    let err_str = format!("{err:#}");
                    tracing::warn!(recipient = %self.recipient, attempt, error = %err_str, "gap_scroll_failed");
                    if action_interrupted(&err_str) {
                        return Err(err);
                    }
                    if attempt == 0 {
                        return Ok(None);
                    }
                    break;
                }
            };
            if !pages.is_empty() && page.bubbles == *pages.last().unwrap() {
                break;
            }
            pages.push(page.bubbles);
            match merge_pages(&pages, None) {
                Ok(m) => {
                    let reconciled = reconcile(&cursor.bubbles, &m);
                    match new_bubbles(&cursor.bubbles, &reconciled) {
                        Ok(ch) => {
                            changes = Some(ch);
                            merged = Some(m);
                            break;
                        }
                        Err(_) => {
                            changes = None;
                        }
                    }
                }
                Err(_) => {
                    changes = None;
                }
            }
        }

        let mut bottom = self
            .scroll_to_bottom(capture, pages.len() as u32 + 2)
            .await?;
        let rec_bottom = reconcile(&frame.bubbles, &bottom.bubbles);
        let fp_frame: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
        let fp_rec: Vec<_> = rec_bottom.iter().map(fingerprint).collect();
        // Opening a chat with unread messages, WeChat puts the first unread at the top with blank
        // space below; back at the bottom the view is anchored normally and shows older bubbles
        // above. Confirmed when the bottom still ends with what was seen (2026-11-09 device).
        let confirms = |rec: &[Fingerprint]| !fp_frame.is_empty() && rec.ends_with(&fp_frame);

        if !confirms(&fp_rec) {
            bottom = self.scroll_to_bottom(capture, 40).await?;
            let rec_bottom2 = reconcile(&frame.bubbles, &bottom.bubbles);
            let fp_rec2: Vec<_> = rec_bottom2.iter().map(fingerprint).collect();
            if changes.is_some() && !confirms(&fp_rec2) {
                self.recovery_unconfirmed = true;
                return Ok(None);
            }
        }

        let Some(ch) = changes else {
            return Ok(None);
        };

        if let Some(m) = merged {
            let mut merged_frame = frame.clone();
            merged_frame.bubbles = m;
            if !self.record(&merged_frame, capture).await? {
                self.recovery_unconfirmed = true;
                return Ok(None);
            }
        }

        Ok(Some(ch))
    }

    pub async fn record(&mut self, frame: &Frame, capture: &mut impl Capture) -> Result<bool> {
        let Some(history) = &self.history else {
            return Ok(true);
        };
        let Some(cid) = &self.contact_id else {
            return Ok(true);
        };
        let frame_val = serde_json::to_value(frame)?;
        let rows = match history.record_frame(cid, &frame_val, None, None, false) {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                let mut d = Map::new();
                d.insert(
                    "error".to_string(),
                    json!(err_str.chars().take(200).collect::<String>()),
                );
                let _ = self.health("history_write_failed", d);
                return Ok(false);
            }
        };

        if let Some(title) = frame.title_lines.first() {
            let _ = history.observe_name(cid, title);
        }

        for row in rows {
            let kind = row.get("kind").and_then(|v| v.as_str());
            let mid = row
                .get("bubble")
                .and_then(|b| b.get("media_id"))
                .and_then(|v| v.as_str());
            let message_id = row.get("message_id").and_then(|v| v.as_str());
            if kind == Some("image") {
                if let (Some(mid_str), Some(msg_id)) = (mid, message_id) {
                    if let Ok(path) = capture.download_kind(mid_str, "image").await {
                        let _ = history.attach_media(
                            msg_id,
                            &path,
                            "image",
                            None,
                            "chat_thumbnail",
                            None,
                        );
                    }
                }
            }
        }
        Ok(true)
    }

    /// Identical messages: a time separator never seen before marks the newest bubble as new.
    /// A second capture must show the same bubbles and separators, otherwise nothing is new.
    async fn separator_arrivals(
        &mut self,
        capture: &mut impl Capture,
        cursor: &[Bubble],
        frame: &Frame,
    ) -> Result<Vec<Bubble>> {
        let tail = separator_tail(cursor, &frame.bubbles);
        if tail.is_empty() {
            return Ok(tail);
        }
        let again = capture.frame().await?;
        let key = |b: &Bubble| (fingerprint(b), b.extra.get("time_label").cloned());
        let same = again.bubbles.len() == frame.bubbles.len()
            && again
                .bubbles
                .iter()
                .zip(&frame.bubbles)
                .all(|(a, b)| key(a) == key(b));
        tracing::info!(
            recipient = %self.recipient,
            confirmed = same,
            "separator_disambiguated"
        );
        Ok(if same { tail } else { Vec::new() })
    }

    pub async fn cycle(
        &mut self,
        client: &impl ToolClient,
        capture: &mut impl Capture,
    ) -> Result<()> {
        self.had_incoming = false;
        if !self.target_allowed() {
            return Ok(());
        }

        let mut frame = capture.frame().await?;
        if frame.clipped_bubble == Some(true) {
            frame = capture.scroll("down", 40).await?;
        }
        self.policy
            .check_frame(&frame)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        if !self.record(&frame, capture).await? {
            return Ok(());
        }

        let legacy = self.cursor.as_ref().map_or(false, |c| {
            c.bubbles
                .iter()
                .any(|b| b.kind.as_deref() == Some("image") && b.phash.is_none())
        });

        if self.cursor.is_none() || self.cursor.as_ref().unwrap().bubbles.is_empty() || legacy {
            self.advance(frame)?;
            let mut d = Map::new();
            d.insert("historical_messages_ignored".to_string(), json!(true));
            self.health("watching", d)?;
            return Ok(());
        }

        if frame.bubbles.is_empty() && self.turn.is_none() {
            let mut d = Map::new();
            d.insert("reason".to_string(), json!("no complete bubble visible"));
            self.health("watching", d)?;
            return Ok(());
        }

        let mut ambiguous: Option<String> = None;
        let cursor_bubbles = self.cursor.as_ref().unwrap().bubbles.clone();
        frame.bubbles = reconcile(&cursor_bubbles, &frame.bubbles);

        let changes = match new_bubbles(&cursor_bubbles, &frame.bubbles) {
            Ok(ch) => ch,
            Err(ref err @ ContinuityError::AmbiguousAnchor { ref certain }) => {
                ambiguous = Some(err.to_string());
                if certain.is_empty() {
                    self.separator_arrivals(capture, &cursor_bubbles, &frame)
                        .await?
                } else {
                    certain.clone()
                }
            }
            Err(ContinuityError::ContinuityGap) => {
                // Layout only (kind, direction, position): enough to explain a gap without content.
                let layout = |bubbles: &[Bubble]| {
                    bubbles
                        .iter()
                        .map(|b| {
                            format!(
                                "{}/{}@{}+{}",
                                b.kind.as_deref().unwrap_or("-"),
                                b.direction,
                                b.y.unwrap_or(-1),
                                b.height.unwrap_or(-1)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                };
                tracing::warn!(
                    recipient = %self.recipient,
                    snapshot = frame.snapshot_id.as_deref().unwrap_or("-"),
                    frame = %layout(&frame.bubbles),
                    cursor_tail = %layout(&cursor_bubbles[cursor_bubbles.len().saturating_sub(4)..]),
                    "continuity_gap"
                );
                match self.recover_gap(capture, &frame, GAP_SCROLL_PAGES).await? {
                    Some(recovered) => recovered,
                    None => {
                        if self.recovery_unconfirmed {
                            let mut d = Map::new();
                            d.insert(
                                "reason".to_string(),
                                json!("scrolled back but bottom frame changed"),
                            );
                            self.health("waiting_for_stable_frame", d)?;
                            return Ok(());
                        }
                        ambiguous = Some("visual continuity gap".to_string());
                        Vec::new()
                    }
                }
            }
        };

        if let Some(ambig) = ambiguous.filter(|_| changes.is_empty()) {
            let confirmed = capture.frame().await?;
            self.policy
                .check_frame(&confirmed)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let fp1: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
            let fp2: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
            if fp1 != fp2 {
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(ambig));
                self.health("waiting_for_stable_frame", d)?;
                return Ok(());
            }
            let reason_text = format!("{ambig}: visible history rebaselined without reply");
            // 这里会把画面上的消息当历史、不回复：必须留在 core.log 里，否则事后无法判断消息为什么没回
            tracing::warn!(
                recipient = %self.recipient,
                visible = confirmed.bubbles.len(),
                snapshot = confirmed.snapshot_id.as_deref().unwrap_or("-"),
                reason = %reason_text,
                "rebaselined_without_reply"
            );
            self.advance(confirmed)?;
            let mut d = Map::new();
            d.insert("reason".to_string(), json!(reason_text));
            self.health("watching", d)?;
            return Ok(());
        }

        let mut arrivals = Vec::new();
        for item in changes {
            if item.direction == "inbound_visual" {
                arrivals.push(item);
            } else if !self.own_echo(&item) && (self.turn.is_some() || !arrivals.is_empty()) {
                self.drop_turn("screen")?;
                arrivals.clear();
            }
        }

        if !arrivals.is_empty() {
            let mut confirmed = capture.frame().await?;
            self.policy
                .check_frame(&confirmed)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            confirmed.bubbles = reconcile(&frame.bubbles, &confirmed.bubbles);
            let fp1: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
            let fp2: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
            // Older bubbles appearing above (the unread-jump layout settling at the bottom) are not a
            // change; anything different at the end is.
            if fp1.is_empty() || !fp2.ends_with(&fp1) {
                self.health("waiting_for_stable_frame", Map::new())?;
                return Ok(());
            }

            let now = (self.clock)();
            if self.turn.is_none() {
                self.turn = Some(TurnData {
                    opened_at: now,
                    baseline: self.cursor.clone().unwrap(),
                    bubbles: Vec::new(),
                    last_inbound_at: now,
                    frame: confirmed.clone(),
                    model_failures: 0,
                    retry_at: 0.0,
                    fixed_snapshot: None,
                });
            }
            if let Some(t) = &mut self.turn {
                t.bubbles.extend(arrivals);
                t.last_inbound_at = now;
                t.frame = confirmed.clone();
                write_private(&self.turn_path, t)?;
            }
            self.advance(confirmed)?;
        } else if self.turn.is_none() {
            self.advance(frame.clone())?;
        }

        if self.turn.is_none() {
            self.health("watching", Map::new())?;
            return Ok(());
        }

        self.had_incoming = true;
        self.upload_pending = frame
            .bubbles
            .iter()
            .any(|b| b.direction == "inbound_visual" && b.transfer_pending == Some(true));

        if !self.turn_due() {
            let mut d = Map::new();
            d.insert(
                "pending_visual_messages".to_string(),
                json!(self.turn.as_ref().map_or(0, |t| t.bubbles.len())),
            );
            self.health(
                if self.upload_pending {
                    "waiting_for_upload"
                } else {
                    "waiting_for_turn"
                },
                d,
            )?;
            return Ok(());
        }

        self.process_turn(client, capture).await
    }

    async fn verify_before_send(
        &mut self,
        capture: &mut impl Capture,
        confirmed: &Frame,
        input_ids: &[String],
        key: &str,
    ) -> Result<(Option<&'static str>, Frame)> {
        let f = capture.frame().await?;
        self.policy
            .check_frame(&f)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if self.replied_meanwhile(confirmed, &f, input_ids, Some(key))? {
            return Ok((Some("already_replied"), f));
        }
        if self.stop.load(Ordering::SeqCst) {
            return Ok((Some("stopped_before_send"), f));
        }
        if !self.target_allowed() {
            return Ok((Some("skipped_by_rule"), f));
        }
        Ok((None, f))
    }

    async fn try_send_once(
        &mut self,
        client: &impl ToolClient,
        journal_path: &Path,
        record: &mut Value,
        confirmed: &Frame,
        reply: &str,
        key: &str,
    ) -> Result<(Option<Value>, Value, Option<String>)> {
        if let Some(map) = record.as_object_mut() {
            map.insert("status".to_string(), json!("attempting"));
            map.insert("attempted_at".to_string(), json!((self.clock)()));
        }
        self.write_journal(journal_path, record)?;
        let mut d = Map::new();
        d.insert("idempotency_key".to_string(), json!(key));
        let _ = self.health("sending", d);

        let res_result = client
            .tool(
                "send_message",
                json!({
                    "recipient": self.clean_recipient(confirmed),
                    "message": reply,
                    "idempotency_key": key,
                }),
            )
            .await;

        let rec_result = client
            .tool("get_send_status", json!({"idempotency_key": key}))
            .await;

        let send_err_msg = res_result.as_ref().err().map(|e| format!("{e:#}"));
        let receipt = rec_result.unwrap_or_else(|_| json!({}));
        let res = res_result.ok();
        Ok((res, receipt, send_err_msg))
    }

    pub async fn process_turn(
        &mut self,
        client: &impl ToolClient,
        capture: &mut impl Capture,
    ) -> Result<()> {
        let handoff_path = self.root.join("handoff.json");
        if handoff_path.exists() {
            self.turn = None;
            let _ = fs::remove_file(&self.turn_path);
            let mut d = Map::new();
            d.insert("recipient".to_string(), json!(self.recipient));
            self.health("handoff", d)?;
            return Ok(());
        }

        let turn = self.turn.clone().context("no active turn")?;
        let incoming = turn.bubbles.clone();

        if self.limited() {
            let mut d = Map::new();
            d.insert("pending_visual_messages".to_string(), json!(incoming.len()));
            self.health("rate_limited", d)?;
            return Ok(());
        }

        let confirmed = turn.frame.clone();
        let mut context_items = Vec::new();
        let mut input_ids = Vec::new();

        if let Some(history) = &self.history {
            if let Some(cid) = &self.contact_id {
                match history.match_recent_inbound(cid, &incoming) {
                    Ok(ids) => {
                        input_ids = ids.clone();
                        let limit = self.config.context_messages;
                        let ids_set: HashSet<String> = ids.into_iter().collect();
                        match history.recent_context(cid, limit, &ids_set) {
                            Ok(stored) => {
                                for item in stored {
                                    context_items.push(ContextItem {
                                        role: item
                                            .get("role")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("user")
                                            .to_string(),
                                        text: item
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        media: None,
                                    });
                                }
                                for b in &incoming {
                                    context_items.push(ContextItem {
                                        role: "user".to_string(),
                                        text: b.text.clone(),
                                        media: None,
                                    });
                                }
                            }
                            Err(e) => {
                                let mut d = Map::new();
                                d.insert(
                                    "error".to_string(),
                                    json!(e.to_string().chars().take(200).collect::<String>()),
                                );
                                self.health("history_write_failed", d)?;
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        let mut d = Map::new();
                        d.insert(
                            "error".to_string(),
                            json!(e.to_string().chars().take(200).collect::<String>()),
                        );
                        self.health("history_write_failed", d)?;
                        return Ok(());
                    }
                }
            }
        } else {
            let start = confirmed.bubbles.len().saturating_sub(12);
            for b in &confirmed.bubbles[start..] {
                context_items.push(ContextItem {
                    role: if b.direction == "inbound_visual" {
                        "user".to_string()
                    } else {
                        "assistant".to_string()
                    },
                    text: b.text.clone(),
                    media: None,
                });
            }
        }

        let key = self.batch_key(&turn.baseline, &incoming);
        let journal_path = self.root.join(format!("send-{key}.json"));

        if journal_path.exists() {
            if let Ok(rec) = read_json::<Value>(&journal_path) {
                if rec.get("status").and_then(|v| v.as_str()) == Some("triggered") {
                    self.drop_turn("journal")?;
                    return Ok(());
                }
                if rec.get("status").and_then(|v| v.as_str()) == Some("stopped_before_send") {
                    let _ = fs::remove_file(&journal_path);
                } else {
                    self.paused = true;
                    let mut d = Map::new();
                    d.insert(
                        "reason".to_string(),
                        json!("existing batch requires review"),
                    );
                    d.insert("idempotency_key".to_string(), json!(key));
                    self.health("paused", d)?;
                    return Ok(());
                }
            }
        }

        let mut record = json!({
            "status": "preparing_media",
            "account_id": self.account,
            "recipient": self.recipient,
            "idempotency_key": key,
            "input": incoming,
            "direction_evidence": "visual_candidate",
            "created_at": (self.clock)(),
            "platform_message_verified": false,
            "timings": {},
        });
        self.write_journal(&journal_path, &record)?;

        let pending = turn.clone();
        self.turn = None;
        let _ = fs::remove_file(&self.turn_path);
        let mut d = Map::new();
        d.insert("idempotency_key".to_string(), json!(key));
        self.health("preparing_media", d)?;

        let result = self
            .run_generation_and_send(
                client,
                capture,
                &key,
                &journal_path,
                &pending,
                &incoming,
                &confirmed,
                &context_items,
                &input_ids,
                &mut record,
            )
            .await;

        match result {
            Ok(Some(mut after)) => {
                let reply_text = record
                    .get("reply")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let shows_echo = |frame: &Frame| {
                    frame.bubbles.iter().any(|b| {
                        b.direction == "outbound_visual" && echo_matches(&b.text, &reply_text)
                    })
                };
                // The bubble can appear several seconds after Enter (device 2026-11-08: 6–11 s). Look again
                // once a second for about 15 s; read-only, never re-sends.
                let mut echoed = shows_echo(&after);
                let mut capture_errors = 0;
                for _ in 0..15 {
                    if echoed || capture_errors >= 3 {
                        break;
                    }
                    (self.sleep)(Duration::from_secs(1)).await;
                    match capture.frame().await {
                        Ok(frame) => {
                            if self.policy.check_frame(&frame).is_ok() {
                                if shows_echo(&frame) {
                                    echoed = true;
                                    after = frame;
                                }
                            } else {
                                capture_errors += 1;
                            }
                        }
                        Err(_) => capture_errors += 1,
                    }
                }
                if let Some(map) = record.as_object_mut() {
                    map.insert("client_echo_observed".to_string(), json!(echoed));
                }
                self.write_journal(&journal_path, &record)?;

                let unseen = match new_bubbles(&confirmed.bubbles, &after.bubbles) {
                    Ok(un) => Some(un),
                    Err(ref err @ ContinuityError::ContinuityGap)
                    | Err(ref err @ ContinuityError::AmbiguousAnchor { .. }) => {
                        let reason = err.to_string();
                        if let Some(map) = record.as_object_mut() {
                            map.insert("postflight_error".to_string(), json!(reason));
                        }
                        self.write_journal(&journal_path, &record)?;
                        None
                    }
                };

                if let Some(un) = unseen {
                    if !un.iter().any(|b| b.direction == "inbound_visual") {
                        self.advance(after)?;
                    }
                }
                let mut d = Map::new();
                if let Some(sid) = record.get("durable_status").and_then(|v| v.get("send_id")) {
                    d.insert("last_send_id".to_string(), sid.clone());
                }
                d.insert("last_client_echo_observed".to_string(), json!(echoed));
                d.insert("delivery".to_string(), json!("unknown"));
                self.health("watching", d)?;
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => {
                let err_str = error.to_string();
                let status_str = record
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                if let Some(map) = record.as_object_mut() {
                    map.insert(
                        "error".to_string(),
                        json!(err_str.chars().take(400).collect::<String>()),
                    );
                }

                let failures = pending.model_failures;
                if status_str == "preparing_media" && action_interrupted(&err_str) {
                    let _ = fs::remove_file(&journal_path);
                    self.keep_turn(pending)?;
                    let mut d = Map::new();
                    d.insert(
                        "reason".to_string(),
                        json!(err_str.chars().take(200).collect::<String>()),
                    );
                    self.health("waiting_for_wechat_window", d)?;
                    return Ok(());
                }

                let max_retries = self.model_retries.unwrap_or(MODEL_RETRIES);
                if status_str == "generating" && failures < max_retries {
                    let _ = fs::remove_file(&journal_path);
                    let mut retrying_turn = pending;
                    retrying_turn.model_failures = failures + 1;
                    retrying_turn.retry_at = (self.clock)() + MODEL_RETRY_SECONDS;
                    if let Some(snap) = record.get("fixed_snapshot").cloned() {
                        retrying_turn.fixed_snapshot = Some(snap);
                    }
                    self.keep_turn(retrying_turn)?;
                    let mut d = Map::new();
                    d.insert(
                        "reason".to_string(),
                        json!(err_str.chars().take(200).collect::<String>()),
                    );
                    d.insert("attempt".to_string(), json!(failures + 1));
                    self.health("model_retry_wait", d)?;
                    return Ok(());
                }

                self.write_journal(&journal_path, &record)?;
                self.paused = true;
                self.pause_reason = Some(format!(
                    "model or send attempt requires review: {}",
                    err_str
                ));
                let mut d = Map::new();
                d.insert(
                    "reason".to_string(),
                    json!(self.pause_reason.clone().unwrap()),
                );
                d.insert("idempotency_key".to_string(), json!(key));
                self.health("paused", d)?;
                Err(error)
            }
        }
    }

    async fn run_generation_and_send(
        &mut self,
        client: &impl ToolClient,
        capture: &mut impl Capture,
        key: &str,
        journal_path: &Path,
        pending: &TurnData,
        incoming: &[Bubble],
        confirmed: &Frame,
        context_items: &[ContextItem],
        input_ids: &[String],
        record: &mut Value,
    ) -> Result<Option<Frame>> {
        let (fetch, mut images, files, mut failed) = if incoming
            .iter()
            .any(|b| b.kind.as_deref().unwrap_or("text") != "text")
        {
            let snapshot = capture.frame().await?;
            self.policy
                .check_frame(&snapshot)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let fetch = relocate(incoming, &snapshot.bubbles);
            let (imgs, fls, fld) = capture.attachments(&fetch, &snapshot).await?;
            (fetch, imgs, fls, fld)
        } else {
            (incoming.to_vec(), Vec::new(), Vec::new(), Vec::new())
        };
        self.archive_media(&fetch, input_ids, &mut images, &files);

        let media_meta: Vec<Value> = files
            .iter()
            .map(|f| {
                json!({
                    "kind": f.content.get("kind").unwrap_or(&json!("document")),
                    "filename": f.filename,
                    "original_bytes": f.original_bytes,
                    "truncated": f.content.get("truncated").unwrap_or(&json!(false)),
                })
            })
            .collect();
        let images_meta: Vec<Value> = images
            .iter()
            .map(|i| {
                json!({
                    "label": i.label,
                    "source": i.source,
                })
            })
            .collect();

        if let Some(map) = record.as_object_mut() {
            map.insert("status".to_string(), json!("generating"));
            map.insert("media".to_string(), json!(media_meta));
            map.insert("images".to_string(), json!(images_meta));
            map.insert("failed".to_string(), serde_json::to_value(&failed)?);
        }
        self.write_journal(journal_path, record)?;

        let mut d = Map::new();
        d.insert("idempotency_key".to_string(), json!(key));
        d.insert("image_count".to_string(), json!(images.len()));
        d.insert("file_count".to_string(), json!(files.len()));
        self.health("generating", d)?;

        if !self.target_allowed() {
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!("skipped_by_rule"));
            }
            self.write_journal(journal_path, record)?;
            return Ok(None);
        }

        let mut text_lines = Vec::new();
        for b in incoming {
            let prefix = match (&b.sender, self.group) {
                (Some(sender), true) => format!("{sender}："),
                _ => String::new(),
            };
            text_lines.push(format!("{}{}", prefix, b.text));
        }
        let full_text = text_lines.join("\n");

        let mut fixed_snapshot: Option<Value> = pending.fixed_snapshot.clone();
        if let Some(snap) = &fixed_snapshot {
            if let Some(map) = record.as_object_mut() {
                map.insert("fixed_snapshot".to_string(), snap.clone());
            }
        }
        let mut fixed_role: Option<crate::core::brain_profiles::RoleRevision> = None;
        let bindings_path = self.config.state_dir.join("brain-bindings.json");
        let (mode, store_ctx) = if bindings_path.exists() {
            let store = match crate::core::brain_profiles::Store::open(&self.config.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("integrity_mismatch"));
                    }
                    self.write_journal(journal_path, record)?;
                    return Err(anyhow::anyhow!("brain profile store open failed: {e:?}"));
                }
            };
            let ctx = match crate::core::brain_profiles::AccountContext::new(self.account.clone()) {
                Ok(c) => c,
                Err(e) => {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("integrity_mismatch"));
                    }
                    self.write_journal(journal_path, record)?;
                    return Err(anyhow::anyhow!("invalid account context: {e:?}"));
                }
            };
            match store.execution_mode(&ctx) {
                Ok(m) => (m, Some((store, ctx))),
                Err(e) => {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("integrity_mismatch"));
                    }
                    self.write_journal(journal_path, record)?;
                    return Err(anyhow::anyhow!(
                        "brain profile store integrity mismatch: {e:?}"
                    ));
                }
            }
        } else {
            (crate::core::brain_profiles::ExecutionMode::Legacy, None)
        };

        if fixed_snapshot.is_none() {
            if let Some((store, ctx)) = store_ctx {
                let chat_kind = if self.group {
                    crate::core::brain_profiles::ChatKind::Group
                } else {
                    crate::core::brain_profiles::ChatKind::Private
                };
                if let Some(cid) = &self.contact_id {
                    let db_path = if let Some(h) = &self.history {
                        h.root_dir().join("history.sqlite3")
                    } else {
                        self.config.state_dir.join("history/history.sqlite3")
                    };
                    let conn = rusqlite::Connection::open_with_flags(
                        &db_path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                            | rusqlite::OpenFlags::SQLITE_OPEN_URI,
                    )
                    .ok();
                    let brain_access = conn.as_ref().and_then(|c| {
                        crate::core::open_api::access::BrainAccess::load(&self.config, c).ok()
                    });
                    let access = crate::core::control::ControlContactAccess {
                        account_id: self.account.clone(),
                        brain_access,
                        conn,
                    };
                    match store.resolve(&ctx, cid, &chat_kind, &access) {
                        Ok(role) => {
                            let mut knowledge = Vec::new();
                            for kref in &role.knowledge_refs {
                                if let Ok(k) = store.knowledge(&ctx, kref) {
                                    knowledge.push(k);
                                }
                            }
                            let synthetic_input = crate::brain::snapshot::SyntheticInput {
                                text: full_text.clone(),
                                chat_kind: if self.group {
                                    "group".into()
                                } else {
                                    "private".into()
                                },
                                files: files.iter().map(model_view_file).collect(),
                                images: images
                                    .iter()
                                    .map(|img| json!({"label": img.label, "source": img.source}))
                                    .collect(),
                                failed: failed
                                    .iter()
                                    .map(|f| json!({"label": f.label, "reason": f.reason}))
                                    .collect(),
                                context: context_items
                                    .iter()
                                    .map(|c| json!({"role": c.role, "text": c.text}))
                                    .collect(),
                            };
                            match crate::brain::snapshot::compile(
                                &role,
                                &knowledge,
                                &self.account,
                                synthetic_input,
                                self.config.max_attachment_mb,
                            ) {
                                Ok(snap) => {
                                    let snap_id = snap
                                        .get("snapshot_id")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string();
                                    if let Some(history) = &self.history {
                                        let obs = json!({"bubbles_count": incoming.len(), "images_count": images.len(), "files_count": files.len()});
                                        if let Err(e) = history.record_brain_turn_snapshot(
                                            &self.account,
                                            key,
                                            cid,
                                            &snap_id,
                                            &snap,
                                            input_ids,
                                            &obs,
                                        ) {
                                            if let Some(map) = record.as_object_mut() {
                                                map.insert(
                                                    "status".to_string(),
                                                    json!("snapshot_failed"),
                                                );
                                            }
                                            self.write_journal(journal_path, record)?;
                                            return Err(anyhow::anyhow!(
                                                "snapshot persistence failed: {e}"
                                            ));
                                        }
                                    }
                                    if let Some(map) = record.as_object_mut() {
                                        map.insert("fixed_snapshot".to_string(), snap.clone());
                                    }
                                    fixed_snapshot = Some(snap);
                                    fixed_role = Some(role);
                                }
                                Err(_e) => {
                                    if let Some(map) = record.as_object_mut() {
                                        map.insert("status".to_string(), json!("budget_exceeded"));
                                    }
                                    self.write_journal(journal_path, record)?;
                                    return Ok(None);
                                }
                            }
                        }
                        Err(_e) => {
                            if mode == crate::core::brain_profiles::ExecutionMode::Managed {
                                if let Some(map) = record.as_object_mut() {
                                    map.insert("status".to_string(), json!("role_required"));
                                }
                                self.write_journal(journal_path, record)?;
                                return Ok(None);
                            }
                            // Legacy 模式无配置角色则保留原有链路
                        }
                    }
                }
            }
        }

        let (notes, model_images, earlier) = self
            .understand_images(&full_text, &images, &mut failed, context_items)
            .await?;

        let mut media_refs = Vec::new();
        for img in &images {
            let understanding = notes.iter().find_map(|n| {
                if n.get("label").and_then(|v| v.as_str()) == Some(&img.label) {
                    n.get("notes")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            });
            media_refs.push(MediaRef {
                kind: "image".to_string(),
                media_id: img.media_id.clone().or_else(|| img.bubble.media_id.clone()),
                filename: None,
                understanding,
                label: Some(img.label.clone()),
            });
        }
        for f in &files {
            media_refs.push(MediaRef {
                kind: "file".to_string(),
                media_id: f.bubble.media_id.clone(),
                filename: Some(f.filename.clone()),
                understanding: None,
                label: Some(f.label.clone()),
            });
        }

        let brain_turn = BrainTurn {
            turn_id: key.to_string(),
            account_id: self.account.clone(),
            contact: ContactInfo {
                name: self.recipient.clone(),
                kind: if self.group {
                    "group".to_string()
                } else {
                    "private".to_string()
                },
            },
            text: full_text,
            mode: "service".to_string(),
            chat_kind: if self.group {
                "group".to_string()
            } else {
                "private".to_string()
            },
            context: context_items.to_vec(),
            images: model_images,
            files: files.iter().map(model_view_file).collect(),
            notes,
            failed: failed
                .iter()
                .map(|f| FailedAttachment {
                    label: f.label.clone(),
                    kind: Some(f.kind.clone()),
                    reason: Some(f.reason.clone()),
                    hint: None,
                })
                .collect(),
            image_count: Some(images.len()),
            earlier_images: earlier,
            max_attachment_mb: self.config.max_attachment_mb,
            snapshot: fixed_snapshot.clone(),
            media_refs,
        };

        let brain_reply = if let Some(brain) = &self.brain {
            brain(&brain_turn).await?
        } else {
            let builtin =
                crate::brain::builtin::BuiltinBrain::new(self.config.model_config.clone());
            builtin.reply(&brain_turn).await?
        };

        let mut model_provenance = brain_reply.metadata.clone();
        let action = model_provenance
            .remove("action")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "send".to_string());

        if action == "skip" {
            if let (Some(history), Some(_snap)) = (&self.history, &fixed_snapshot) {
                let dec = json!({"action": "skip"});
                let _ = history.update_brain_turn_decision(
                    &self.account,
                    key,
                    &dec,
                    &json!(model_provenance),
                    None,
                );
                let exec = json!({"phase": "not_sent", "action": "skip", "delivery": "unknown", "updated_at": (self.clock)()});
                let _ = history.update_brain_turn_execution(&self.account, key, &exec);
            }
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!("skipped_by_brain"));
            }
            self.write_journal(journal_path, record)?;
            return Ok(None);
        }

        if action == "handoff" {
            if let (Some(history), Some(_snap)) = (&self.history, &fixed_snapshot) {
                let dec = json!({"action": "handoff"});
                let _ = history.update_brain_turn_decision(
                    &self.account,
                    key,
                    &dec,
                    &json!(model_provenance),
                    None,
                );
                let exec = json!({"phase": "not_sent", "action": "handoff", "delivery": "unknown", "updated_at": (self.clock)()});
                let _ = history.update_brain_turn_execution(&self.account, key, &exec);
            }
            let handoff_path = self.root.join("handoff.json");
            let handoff_data = json!({
                "recipient": self.recipient,
                "turn_id": key,
                "at": (self.clock)(),
            });
            write_private(&handoff_path, &handoff_data)?;
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!("handoff"));
            }
            self.write_journal(journal_path, record)?;
            let mut d = Map::new();
            d.insert("recipient".to_string(), json!(self.recipient));
            d.insert("turn_id".to_string(), json!(key));
            self.health("handoff", d)?;
            return Ok(None);
        }

        let generated_reply = brain_reply.reply;
        let validated = validate_reply(&generated_reply).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let reply = ensure_unreadable_named(&validated, &failed)?;

        if let Some(map) = record.as_object_mut() {
            map.insert("status".to_string(), json!("generated"));
            map.insert("reply".to_string(), json!(reply));
            map.insert("model".to_string(), json!(model_provenance));
            if let Some(secs) = model_provenance.get("seconds") {
                if let Some(timings) = map.get_mut("timings").and_then(|t| t.as_object_mut()) {
                    timings.insert("model_seconds".to_string(), secs.clone());
                } else {
                    let mut timings = Map::new();
                    timings.insert("model_seconds".to_string(), secs.clone());
                    map.insert("timings".to_string(), Value::Object(timings));
                }
            }
        }
        self.write_journal(journal_path, record)?;

        if self.stop.load(Ordering::SeqCst) {
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!("stopped_before_send"));
            }
            self.write_journal(journal_path, record)?;
            self.keep_turn(pending.clone())?;
            return Ok(None);
        }

        let (check_res, _latest) = self
            .verify_before_send(capture, confirmed, input_ids, key)
            .await?;
        if let Some(status) = check_res {
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!(status));
            }
            self.write_journal(journal_path, record)?;
            if status == "stopped_before_send" {
                self.keep_turn(pending.clone())?;
            }
            return Ok(None);
        }

        if let (Some(history), Some(_snap)) = (&self.history, &fixed_snapshot) {
            let dec = json!({"action": "send", "reply": reply});
            let _ = history.update_brain_turn_decision(
                &self.account,
                key,
                &dec,
                &json!(model_provenance),
                Some(key),
            );
        }

        if let Some(history) = &self.history {
            if let Some(cid) = &self.contact_id {
                let mut model_val = record.get("model").cloned().unwrap_or_else(|| json!({}));
                let role_rev_id = fixed_snapshot
                    .as_ref()
                    .and_then(|s| s.get("role"))
                    .and_then(|r| r.get("revision_id"))
                    .and_then(Value::as_str)
                    .or_else(|| fixed_role.as_ref().map(|r| r.revision_id.as_str()));
                if let (Some(snap), Some(rev_id)) = (&fixed_snapshot, role_rev_id) {
                    if let Some(mobj) = model_val.as_object_mut() {
                        mobj.insert(
                            "brain".into(),
                            json!({
                                "snapshot_id": snap["snapshot_id"],
                                "role_revision_id": rev_id,
                            }),
                        );
                    }
                }
                history.record_bot_reply(
                    cid,
                    key,
                    &reply,
                    "generated",
                    &model_val,
                    input_ids,
                    None,
                    None,
                )?;
            }
        }

        let (mut result, mut receipt, mut send_err) = self
            .try_send_once(client, journal_path, record, confirmed, &reply, key)
            .await?;

        if let (Some(history), Some(_snap)) = (&self.history, &fixed_snapshot) {
            let phase = if receipt.get("status").and_then(|v| v.as_str()) == Some("triggered") {
                "triggered"
            } else if send_err.is_some() {
                "failed"
            } else {
                "not_sent"
            };
            let exec = json!({
                "phase": phase,
                "delivery": "unknown",
                "echo_observed": false,
                "error_code": send_err.as_ref().map(|e| e.to_string()),
                "updated_at": (self.clock)(),
            });
            let _ = history.update_brain_turn_execution(&self.account, key, &exec);
        }

        if send_err.is_none() && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered")
        {
            if let Some(map) = record.as_object_mut() {
                map.insert("status".to_string(), json!("triggered"));
                if let Some(r) = result {
                    map.insert("send_result".to_string(), r);
                }
                map.insert("durable_status".to_string(), receipt.clone());
                map.insert("delivery".to_string(), json!("unknown"));
            }
            self.write_journal(journal_path, record)?;
        } else if is_not_sent(send_err.as_deref(), Some(&receipt)) {
            let mut succeeded = false;
            for delay in [5u64, 15u64] {
                if self.stop.load(Ordering::SeqCst) {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("stopped_before_send"));
                    }
                    self.write_journal(journal_path, record)?;
                    self.keep_turn(pending.clone())?;
                    return Ok(None);
                }
                (self.sleep)(Duration::from_secs(delay)).await;
                if self.stop.load(Ordering::SeqCst) {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("stopped_before_send"));
                    }
                    self.write_journal(journal_path, record)?;
                    self.keep_turn(pending.clone())?;
                    return Ok(None);
                }
                let (check_res, _) = self
                    .verify_before_send(capture, confirmed, input_ids, key)
                    .await?;
                if let Some(status) = check_res {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!(status));
                    }
                    self.write_journal(journal_path, record)?;
                    if status == "stopped_before_send" {
                        self.keep_turn(pending.clone())?;
                    }
                    return Ok(None);
                }
                let (r, rec, err) = self
                    .try_send_once(client, journal_path, record, confirmed, &reply, key)
                    .await?;
                result = r;
                receipt = rec;
                send_err = err;
                if send_err.is_none()
                    && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered")
                {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("triggered"));
                        if let Some(r_val) = result {
                            map.insert("send_result".to_string(), r_val);
                        }
                        map.insert("durable_status".to_string(), receipt.clone());
                        map.insert("delivery".to_string(), json!("unknown"));
                    }
                    self.write_journal(journal_path, record)?;
                    succeeded = true;
                    break;
                }
                if !is_not_sent(send_err.as_deref(), Some(&receipt)) {
                    break;
                }
            }

            if !succeeded && is_not_sent(send_err.as_deref(), Some(&receipt)) {
                let err_msg = send_err.clone().unwrap_or_default();
                if let Some(map) = record.as_object_mut() {
                    map.insert("status".to_string(), json!("not_sent"));
                    map.insert(
                        "error".to_string(),
                        json!(err_msg.chars().take(400).collect::<String>()),
                    );
                    map.insert("durable_status".to_string(), receipt);
                    map.insert("delivery".to_string(), json!("not_sent"));
                }
                self.write_journal(journal_path, record)?;
                let short_err = err_msg
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>();
                let mut d = Map::new();
                d.insert(
                    "reason".to_string(),
                    json!(format!("last reply not sent: {short_err}")),
                );
                self.health("watching", d)?;
                return Ok(None);
            }
        }

        let mut after_frame: Option<Frame> = None;

        if record.get("status").and_then(|v| v.as_str()) != Some("triggered") {
            let mut consecutive_errs = 0;
            let mut outcome = EchoOutcome::NotSeen(None);
            for attempt_idx in 0..3 {
                match capture.frame().await {
                    Ok(f) => {
                        if let Err(e) = self.policy.check_frame(&f) {
                            consecutive_errs += 1;
                            if consecutive_errs == 3 {
                                outcome = EchoOutcome::Unverifiable(format!("{e:?}"));
                                break;
                            }
                            if attempt_idx < 2 {
                                (self.sleep)(Duration::from_secs(1)).await;
                            }
                            continue;
                        }
                        let echoed = f.bubbles.iter().any(|b| {
                            b.direction == "outbound_visual" && echo_matches(&b.text, &reply)
                        });
                        if echoed {
                            outcome = EchoOutcome::Seen(f);
                            break;
                        }
                        outcome = EchoOutcome::NotSeen(Some(f));
                        if attempt_idx < 2 {
                            (self.sleep)(Duration::from_secs(1)).await;
                        }
                    }
                    Err(e) => {
                        consecutive_errs += 1;
                        if consecutive_errs == 3 {
                            outcome = EchoOutcome::Unverifiable(e.to_string());
                            break;
                        }
                    }
                }
            }

            if let EchoOutcome::NotSeen(None) = outcome {
                if consecutive_errs > 0 {
                    outcome =
                        EchoOutcome::Unverifiable("echo frames failed verification".to_string());
                }
            }

            match outcome {
                EchoOutcome::Seen(f) => {
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("triggered"));
                        map.insert("client_echo_observed".to_string(), json!(true));
                        map.insert("durable_status".to_string(), receipt);
                    }
                    self.write_journal(journal_path, record)?;
                    after_frame = Some(f);
                }
                EchoOutcome::Unverifiable(err) => {
                    let err_msg = send_err.clone().unwrap_or(err);
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("unknown"));
                        map.insert(
                            "error".to_string(),
                            json!(err_msg.chars().take(400).collect::<String>()),
                        );
                        map.insert("durable_status".to_string(), receipt);
                    }
                    self.write_journal(journal_path, record)?;
                    return Ok(None);
                }
                EchoOutcome::NotSeen(_) => {
                    let err_msg = send_err.clone().unwrap_or_default();
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("not_sent"));
                        map.insert(
                            "error".to_string(),
                            json!(err_msg.chars().take(400).collect::<String>()),
                        );
                        map.insert("durable_status".to_string(), receipt.clone());
                    }
                    self.write_journal(journal_path, record)?;

                    if self.stop.load(Ordering::SeqCst) {
                        if let Some(map) = record.as_object_mut() {
                            map.insert("status".to_string(), json!("stopped_before_send"));
                        }
                        self.write_journal(journal_path, record)?;
                        self.keep_turn(pending.clone())?;
                        return Ok(None);
                    }
                    (self.sleep)(Duration::from_secs(5)).await;
                    if self.stop.load(Ordering::SeqCst) {
                        if let Some(map) = record.as_object_mut() {
                            map.insert("status".to_string(), json!("stopped_before_send"));
                        }
                        self.write_journal(journal_path, record)?;
                        self.keep_turn(pending.clone())?;
                        return Ok(None);
                    }
                    let (check_res, _) = self
                        .verify_before_send(capture, confirmed, input_ids, key)
                        .await?;
                    if let Some(status) = check_res {
                        if let Some(map) = record.as_object_mut() {
                            map.insert("status".to_string(), json!(status));
                        }
                        self.write_journal(journal_path, record)?;
                        if status == "stopped_before_send" {
                            self.keep_turn(pending.clone())?;
                        }
                        return Ok(None);
                    }

                    let (r, rec, err) = self
                        .try_send_once(client, journal_path, record, confirmed, &reply, key)
                        .await?;
                    result = r;
                    receipt = rec;
                    send_err = err;

                    if send_err.is_none()
                        && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered")
                    {
                        if let Some(map) = record.as_object_mut() {
                            map.insert("status".to_string(), json!("triggered"));
                            if let Some(r_val) = result {
                                map.insert("send_result".to_string(), r_val);
                            }
                            map.insert("durable_status".to_string(), receipt);
                            map.insert("delivery".to_string(), json!("unknown"));
                        }
                        self.write_journal(journal_path, record)?;
                    } else {
                        let err_msg2 = send_err.clone().unwrap_or_default();
                        if let Some(map) = record.as_object_mut() {
                            map.insert("status".to_string(), json!("not_sent"));
                            map.insert(
                                "error".to_string(),
                                json!(err_msg2.chars().take(400).collect::<String>()),
                            );
                            map.insert("durable_status".to_string(), receipt);
                            map.insert("delivery".to_string(), json!("not_sent"));
                        }
                        self.write_journal(journal_path, record)?;
                        let short_err = err_msg2
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(80)
                            .collect::<String>();
                        let mut d = Map::new();
                        d.insert(
                            "reason".to_string(),
                            json!(format!("last reply not sent: {short_err}")),
                        );
                        self.health("watching", d)?;
                        return Ok(None);
                    }
                }
            }
        }

        let mut d = Map::new();
        d.insert("idempotency_key".to_string(), json!(key));
        self.health("verifying_send", d)?;

        let final_frame = if let Some(f) = after_frame {
            self.policy
                .check_frame(&f)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            f
        } else {
            let f = capture.frame().await?;
            self.policy
                .check_frame(&f)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            f
        };

        Ok(Some(final_frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::AllowlistEntry;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("test-turns-{}", uuid::Uuid::new_v4()));
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

    fn test_bubble(text: &str, direction: &str) -> Bubble {
        Bubble {
            text: text.to_string(),
            digest: text.to_string(),
            direction: direction.to_string(),
            kind: None,
            media_id: None,
            phash: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: Map::new(),
        }
    }

    fn test_chat(content: &[(&Bubble, i64)], steps: i64, height: i64) -> Frame {
        let mut placed = Vec::new();
        let mut y = height - 8;
        for (item, size) in content.iter().rev() {
            y -= size;
            placed.push((*item, y, *size));
            y -= 20;
        }
        let mut visible = Vec::new();
        for (item, top, size) in placed.into_iter().rev() {
            let cur_y = top + steps * 90;
            if cur_y >= 2 && cur_y + size < height - 2 {
                let mut b = (*item).clone();
                b.y = Some(cur_y);
                b.height = Some(size);
                visible.push(b);
            }
        }
        test_frame(visible, "午夜咖啡")
    }

    fn test_frame(bubbles: Vec<Bubble>, title: &str) -> Frame {
        Frame {
            profile: Some("wechat-4-dark".to_string()),
            scale: Some(json!(1)),
            title_lines: vec![title.to_string()],
            bubbles,
            platform_message_verified: None,
            sender_verified: None,
            clipped_bubble: Some(false),
            width: None,
            height: None,
            captured_width: None,
            snapshot_id: None,
            window_id: None,
            observed_title: None,
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: Map::new(),
        }
    }

    enum MockFrameItem {
        Ok(Frame),
        Err(String),
    }

    struct MockCapture {
        pub frame_sequence: Vec<MockFrameItem>,
        pub frame_idx: usize,
        pub scroll_pages: Vec<Frame>,
        pub scroll_idx: usize,
        pub scroll_calls: Arc<Mutex<Vec<(String, u32)>>>,
        pub download_fn: Option<Arc<dyn Fn(&str, &str) -> Result<PathBuf> + Send + Sync>>,
        pub attachments_fn: Option<
            Arc<
                dyn Fn(
                        &[Bubble],
                        &Frame,
                    ) -> Result<(
                        Vec<AttachmentImage>,
                        Vec<AttachmentFile>,
                        Vec<FailedAttachmentItem>,
                    )> + Send
                    + Sync,
            >,
        >,
    }

    impl MockCapture {
        fn new(frames: Vec<Frame>) -> Self {
            Self {
                frame_sequence: frames.into_iter().map(MockFrameItem::Ok).collect(),
                frame_idx: 0,
                scroll_pages: Vec::new(),
                scroll_idx: 0,
                scroll_calls: Arc::new(Mutex::new(Vec::new())),
                download_fn: None,
                attachments_fn: None,
            }
        }
    }

    impl Capture for MockCapture {
        async fn frame(&mut self) -> Result<Frame> {
            if self.frame_idx < self.frame_sequence.len() {
                let item = &self.frame_sequence[self.frame_idx];
                self.frame_idx += 1;
                match item {
                    MockFrameItem::Ok(f) => Ok(f.clone()),
                    MockFrameItem::Err(msg) => bail!("{msg}"),
                }
            } else if let Some(last) = self.frame_sequence.last() {
                match last {
                    MockFrameItem::Ok(f) => Ok(f.clone()),
                    MockFrameItem::Err(msg) => bail!("{msg}"),
                }
            } else {
                bail!("no more frames in MockCapture");
            }
        }

        async fn scroll(&mut self, direction: &str, pages: u32) -> Result<Frame> {
            self.scroll_calls
                .lock()
                .unwrap()
                .push((direction.to_string(), pages));
            if self.scroll_idx < self.scroll_pages.len() {
                let p = self.scroll_pages[self.scroll_idx].clone();
                self.scroll_idx += 1;
                Ok(p)
            } else if let Some(last) = self.scroll_pages.last() {
                Ok(last.clone())
            } else {
                self.frame().await
            }
        }

        async fn download(&mut self, media_id: &str) -> Result<PathBuf> {
            self.download_kind(media_id, "image").await
        }

        async fn download_kind(&mut self, media_id: &str, kind: &str) -> Result<PathBuf> {
            if let Some(dfn) = &self.download_fn {
                dfn(media_id, kind)
            } else {
                bail!("download not mocked");
            }
        }

        async fn attachments(
            &mut self,
            items: &[Bubble],
            frame: &Frame,
        ) -> Result<(
            Vec<AttachmentImage>,
            Vec<AttachmentFile>,
            Vec<FailedAttachmentItem>,
        )> {
            if let Some(afn) = &self.attachments_fn {
                afn(items, frame)
            } else {
                bail!("attachments not mocked");
            }
        }
    }

    struct MockToolClient {
        pub calls: Arc<Mutex<Vec<(String, Value)>>>,
        pub handler: Arc<dyn Fn(&str, &Value) -> Result<Value> + Send + Sync>,
    }

    impl MockToolClient {
        fn new_default() -> Self {
            let calls = Arc::new(Mutex::new(Vec::new()));

            Self {
                calls,
                handler: Arc::new(move |name, _args| {
                    if name == "send_message" {
                        Ok(
                            json!({"status": "triggered", "send_id": "fixture-send", "delivery": "unknown"}),
                        )
                    } else if name == "get_send_status" {
                        Ok(
                            json!({"status": "triggered", "send_id": "fixture-send", "delivery": "unknown"}),
                        )
                    } else {
                        Ok(json!({}))
                    }
                }),
            }
        }
    }

    impl ToolClient for MockToolClient {
        async fn tool(&self, name: &str, args: Value) -> Result<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), args.clone()));
            (self.handler)(name, &args)
        }
    }

    fn fixture_config(state_dir: &Path) -> BotConfig {
        BotConfig {
            schema_version: 1,
            host: "local".to_string(),
            account_id: "fixture".to_string(),
            allowlist: vec![AllowlistEntry {
                recipient: "午夜咖啡".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
            ignorelist: None,
            state_dir: state_dir.to_path_buf(),
            poll_seconds: 2,
            cooldown_seconds: 0,
            max_replies_per_hour: 20,
            idle_check_seconds: 60,
            turn_quiet_seconds: 0,
            turn_max_seconds: 60,
            images_per_model_call: 4,
            max_attachment_mb: 50,
            context_messages: 5,
            manage_window: true,
            capture_backend: "native".to_string(),
            friend_requests: Default::default(),
            open_api: Default::default(),
            history_http: crate::core::config::HistoryHttpConfig {
                bind: "127.0.0.1".to_string(),
                port: 8765,
            },
            model_config: crate::core::config::ModelConfig {
                backend: "codex-exec".to_string(),
                base_url: None,
                model: None,
                api_key_file: None,
                api_key_env: None,
                supports_images: None,
                timeout_seconds: None,
                json_output: None,
            },
            brain: Default::default(),
        }
    }

    fn default_brain(reply_text: &'static str) -> BrainFn {
        Arc::new(move |_turn| {
            let rep = reply_text.to_string();
            Box::pin(async move {
                Ok(BrainReply {
                    reply: rep,
                    metadata: HashMap::new(),
                })
            })
        })
    }

    // === 固定值对照测试 ===
    #[test]
    fn test_batch_key_matches_python_golden() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.account = "test-account".to_string();
        svc.recipient = "午夜咖啡".to_string();

        let prev = test_frame(vec![test_bubble("hello", "inbound_visual")], "午夜咖啡");
        let inc = vec![test_bubble("world", "inbound_visual")];

        let key = svc.batch_key(&prev, &inc);
        // Python golden: bot-service-d5143f967016e38026a4ea02061965c5b5584395538205585b4962862fb73666
        assert_eq!(
            key,
            "bot-service-d5143f967016e38026a4ea02061965c5b5584395538205585b4962862fb73666"
        );

        // With phash
        svc.account = "acc1".to_string();
        svc.recipient = "测试群".to_string();
        let mut img_bubble = test_bubble("[图片]", "inbound_visual");
        img_bubble.digest = "image:50:40".to_string();
        img_bubble.kind = Some("image".to_string());
        img_bubble.phash = Some("0123456789abcdef".to_string());
        let prev2 = test_frame(vec![img_bubble], "测试群");
        let inc2 = vec![test_bubble("请问在吗", "inbound_visual")];
        let key2 = svc.batch_key(&prev2, &inc2);
        // Python golden: bot-service-a13abb560bb7ac5df1dacb0766724f0d99e5fed4530e4280a4fe3376f0c0361f
        assert_eq!(
            key2,
            "bot-service-a13abb560bb7ac5df1dacb0766724f0d99e5fed4530e4280a4fe3376f0c0361f"
        );
    }

    #[test]
    fn test_ensure_unreadable_named_matches_python_golden() {
        let f1 = vec![FailedAttachmentItem {
            label: "第1张图片".to_string(),
            kind: "image".to_string(),
            reason: "download_failed".to_string(),
            detail: None,
        }];
        let r1 = ensure_unreadable_named("你好", &f1).unwrap();
        assert_eq!(r1, "你好第1张图片打不开，麻烦重发一下。");

        let r2 = ensure_unreadable_named("你好第1张图片打不开", &f1).unwrap();
        assert_eq!(r2, "你好第1张图片打不开");

        let f_long: Vec<FailedAttachmentItem> = (0..15)
            .map(|i| FailedAttachmentItem {
                label: format!("长长长长长长长长文件_{i}.pdf"),
                kind: "file".to_string(),
                reason: "unsupported".to_string(),
                detail: None,
            })
            .collect();
        let r3 = ensure_unreadable_named("你好", &f_long).unwrap();
        assert_eq!(r3, "你好有15个文件或图片打不开，麻烦重发一下。");
    }

    // === AttachmentLimitConfigTests ===
    #[test]
    fn test_attachment_limit_load_config_default_max_attachment_mb() {
        let td = TestDir::new();
        let p = td.path().join("config.json");
        let base = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "testacc",
            "state_dir": td.path().to_str().unwrap(),
            "allowlist": [{"recipient": "testuser", "kind": "private"}],
            "poll_seconds": 2
        });
        fs::write(&p, serde_json::to_string(&base).unwrap()).unwrap();
        let cfg = BotConfig::load(&p).unwrap();
        assert_eq!(cfg.max_attachment_mb, 50);
    }

    #[test]
    fn test_attachment_limit_load_config_accepts_valid_range() {
        let td = TestDir::new();
        let p = td.path().join("config.json");
        let base = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "testacc",
            "state_dir": td.path().to_str().unwrap(),
            "allowlist": [{"recipient": "testuser", "kind": "private"}],
            "poll_seconds": 2,
            "max_attachment_mb": 120
        });
        fs::write(&p, serde_json::to_string(&base).unwrap()).unwrap();
        let cfg = BotConfig::load(&p).unwrap();
        assert_eq!(cfg.max_attachment_mb, 120);
    }

    #[test]
    fn test_attachment_limit_load_config_rejects_out_of_range_or_wrong_type() {
        let td = TestDir::new();
        let p = td.path().join("config.json");
        for bad in [json!(0), json!(201), json!("50"), json!(50.5)] {
            let base = json!({
                "schema_version": 1,
                "host": "local",
                "account_id": "testacc",
                "state_dir": td.path().to_str().unwrap(),
                "allowlist": [{"recipient": "testuser", "kind": "private"}],
                "poll_seconds": 2,
                "max_attachment_mb": bad
            });
            fs::write(&p, serde_json::to_string(&base).unwrap()).unwrap();
            assert!(BotConfig::load(&p).is_err());
        }
    }

    #[test]
    fn test_swift_sources_use_max_bytes_and_no_received_10mb_constants() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = fs::read_to_string(root.join("native/bot/Files.swift")).unwrap();
        let worker = fs::read_to_string(root.join("native/bot/Worker.swift")).unwrap();

        assert!(
            !files.contains("10*1024*1024"),
            "Files.swift should not contain 10*1024*1024"
        );
        assert!(
            files.contains("maxBytes"),
            "Files.swift should contain maxBytes"
        );

        assert!(
            !worker.contains("10*1024*1024"),
            "Worker.swift should not contain 10*1024*1024"
        );
        assert!(
            worker.contains("max_bytes"),
            "Worker.swift should contain max_bytes"
        );
        assert!(
            worker.contains("invalid_max_bytes"),
            "Worker.swift should contain invalid_max_bytes"
        );
    }

    // === SendRetryTests (8 tests) ===
    #[tokio::test]
    async fn test_echo_appearing_seconds_after_send_is_observed() {
        // Device 2026-11-08: our reply first showed up in captures 6–11 s after Enter; a single post-send
        // frame missed it. Keep looking (about 15 s) until the bubble appears.
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        let sleeps = Arc::new(AtomicUsize::new(0));
        let sleeps_clone = sleeps.clone();
        svc.sleep = Arc::new(move |_| {
            sleeps_clone.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        });
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                test_bubble("你好，我是AI客服助手。", "outbound_visual"),
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| match name {
                "send_message" => {
                    Ok(json!({"status": "triggered", "send_id": "send-1", "delivery": "unknown"}))
                }
                "get_send_status" => {
                    Ok(json!({"status": "triggered", "delivery": "unknown", "send_id": "send-1"}))
                }
                _ => Ok(json!({})),
            }),
        };
        let mut frames = vec![new_f.clone(); 6];
        frames.push(after.clone());
        let mut capture = MockCapture::new(frames);
        svc.cycle(&client, &mut capture).await.unwrap();

        let journal = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .unwrap();
        let data: Value = read_json(journal.path()).unwrap();
        assert_eq!(data["status"], "triggered");
        assert_eq!(
            data["client_echo_observed"], true,
            "the late echo must be found"
        );
        assert_eq!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == "send_message")
                .count(),
            1,
            "never re-send"
        );
    }

    #[tokio::test]
    async fn test_send_retry_not_sent_click_failure_retries_and_does_not_pause() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        svc.advance(old).unwrap();

        let attempt = Arc::new(AtomicUsize::new(0));
        let attempt_clone = attempt.clone();
        let client_calls = Arc::new(Mutex::new(Vec::new()));
        let client_calls_clone = client_calls.clone();

        let client = MockToolClient {
            calls: client_calls_clone,
            handler: Arc::new(move |name, _args| {
                if name == "send_message" {
                    let att = attempt_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    if att == 1 {
                        bail!(
                            "MCP tool failed: send_message: 发送尝试 send-1 未触发发送: wechat_click 退出状态异常: Some(1)"
                        );
                    }
                    Ok(json!({"status": "triggered", "send_id": "send-1", "delivery": "unknown"}))
                } else if name == "get_send_status" {
                    let att = attempt_clone.load(Ordering::SeqCst);
                    if att == 1 {
                        Ok(
                            json!({"status": "unknown", "delivery": "not_sent", "send_id": "send-1"}),
                        )
                    } else {
                        Ok(
                            json!({"status": "triggered", "delivery": "unknown", "send_id": "send-1"}),
                        )
                    }
                } else {
                    Ok(json!({}))
                }
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            after.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);
        assert_eq!(
            send_calls[0]["idempotency_key"],
            send_calls[1]["idempotency_key"]
        );

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "triggered");
        assert_eq!(data["client_echo_observed"], true);
    }

    #[tokio::test]
    async fn test_send_retry_not_sent_three_failures_keeps_session_running() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!(
                        "MCP tool failed: send_message: 发送尝试 fail 未触发发送: wechat_click 退出状态异常: Some(1)"
                    );
                }
                if name == "get_send_status" {
                    return Ok(
                        json!({"status": "unknown", "delivery": "not_sent", "send_id": "fail"}),
                    );
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(); 10]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 3);
        assert_eq!(
            send_calls[0]["idempotency_key"],
            send_calls[1]["idempotency_key"]
        );
        assert_eq!(
            send_calls[1]["idempotency_key"],
            send_calls[2]["idempotency_key"]
        );

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "not_sent");

        let health: Value = read_json(svc.root.join("health.json")).unwrap();
        assert_eq!(health["state"], "watching");
        assert!(
            health["reason"]
                .as_str()
                .unwrap()
                .contains("last reply not sent")
        );
    }

    #[tokio::test]
    async fn test_send_retry_unknown_with_echo_marks_triggered_no_resend() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let frame_with_echo = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!("MCP tool failed: send_message: 结果未知: connection closed");
                }
                if name == "get_send_status" {
                    return Ok(
                        json!({"status": "unknown", "delivery": "unknown", "send_id": "unk-1"}),
                    );
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            frame_with_echo.clone(),
            frame_with_echo.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 1);

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "triggered");
        assert_eq!(data["client_echo_observed"], true);
    }

    #[tokio::test]
    async fn test_send_retry_unknown_without_echo_retries_once() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let send_count = Arc::new(AtomicUsize::new(0));
        let send_count_clone = send_count.clone();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(move |name, _args| {
                if name == "send_message" {
                    let cnt = send_count_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    if cnt == 1 {
                        bail!("MCP tool failed: send_message: 结果未知: timeout");
                    }
                    Ok(json!({"status": "triggered", "send_id": "retry-1", "delivery": "unknown"}))
                } else if name == "get_send_status" {
                    let cnt = send_count_clone.load(Ordering::SeqCst);
                    if cnt == 1 {
                        Ok(json!({"status": "unknown", "delivery": "unknown", "send_id": "unk-1"}))
                    } else {
                        Ok(
                            json!({"status": "triggered", "delivery": "unknown", "send_id": "retry-1"}),
                        )
                    }
                } else {
                    Ok(json!({}))
                }
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            new_f.clone(), // 3 screen checks without echo
            new_f.clone(), // pre-send retry
            after.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "triggered");
    }

    #[tokio::test]
    async fn test_send_retry_unknown_unverifiable_does_not_pause_and_never_resends() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!("MCP tool failed: send_message: 结果未知: osascript timeout");
                }
                if name == "get_send_status" {
                    return Ok(
                        json!({"status": "unknown", "delivery": "unknown", "send_id": "unk-err"}),
                    );
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone()]);
        capture
            .frame_sequence
            .push(MockFrameItem::Err("capture device error 1".to_string()));
        capture
            .frame_sequence
            .push(MockFrameItem::Err("capture device error 2".to_string()));
        capture
            .frame_sequence
            .push(MockFrameItem::Err("capture device error 3".to_string()));

        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 1);

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "unknown");
    }

    #[tokio::test]
    async fn test_send_retry_startup_attempting_journal_does_not_pause() {
        let td = TestDir::new();
        let state_dir = td.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let attempting = state_dir.join("send-attempting.json");
        write_private(
            &attempting,
            &json!({
                "status": "attempting",
                "reply": "你好，我是AI客服助手。",
                "idempotency_key": "key-attempting"
            }),
        )
        .unwrap();

        let cfg = fixture_config(&state_dir);
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(!svc.paused);

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let frame_with_echo = test_frame(
            vec![test_bubble("old", "inbound_visual"), reply],
            "午夜咖啡",
        );
        let mut capture = MockCapture::new(vec![frame_with_echo]);

        svc.recover_startup_journals(&mut capture).await.unwrap();
        assert!(!svc.paused);
        let data: Value = read_json(&attempting).unwrap();
        assert_eq!(data["status"], "triggered");
        assert_eq!(data["client_echo_observed"], true);
    }

    #[test]
    fn test_send_retry_startup_generated_journal_closed_not_sent() {
        let td = TestDir::new();
        let state_dir = td.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let gen_path = state_dir.join("send-generated.json");
        write_private(
            &gen_path,
            &json!({
                "status": "generated",
                "reply": "你好",
                "idempotency_key": "key-gen"
            }),
        )
        .unwrap();
        let prep = state_dir.join("send-preparing.json");
        write_private(
            &prep,
            &json!({
                "status": "preparing_media",
                "idempotency_key": "key-prep"
            }),
        )
        .unwrap();

        let cfg = fixture_config(&state_dir);
        let svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(!svc.paused);
        let d_gen: Value = read_json(&gen_path).unwrap();
        assert_eq!(d_gen["status"], "not_sent");
        let d_prep: Value = read_json(&prep).unwrap();
        assert_eq!(d_prep["status"], "not_sent");
    }

    #[tokio::test]
    async fn test_send_retry_gateway_simulation_replays_unless_last_delivery_not_sent() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let dispatches = Arc::new(AtomicUsize::new(0));
        let dispatches_clone = dispatches.clone();
        let outbox = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
        let outbox_clone = outbox.clone();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(move |name, args| {
                if name == "send_message" {
                    let key = args["idempotency_key"].as_str().unwrap().to_string();
                    let mut box_map = outbox_clone.lock().unwrap();
                    if let Some(existing) = box_map.get(&key) {
                        if existing.get("delivery").and_then(|v| v.as_str()) != Some("not_sent") {
                            let mut replayed = existing.clone();
                            replayed["replayed"] = json!(true);
                            return Ok(replayed);
                        }
                    }
                    let count = dispatches_clone.fetch_add(1, Ordering::SeqCst) + 1;
                    if count == 1 {
                        box_map.insert(key, json!({"status": "unknown", "delivery": "not_sent", "send_id": "sim-1"}));
                        bail!(
                            "MCP tool failed: send_message: 发送尝试 sim-1 未触发发送: wechat_click 退出状态异常: Some(1)"
                        );
                    }
                    let success =
                        json!({"status": "triggered", "delivery": "unknown", "send_id": "sim-1"});
                    box_map.insert(key, success.clone());
                    return Ok(success);
                } else if name == "get_send_status" {
                    let key = args["idempotency_key"].as_str().unwrap().to_string();
                    let box_map = outbox_clone.lock().unwrap();
                    return Ok(box_map.get(&key).cloned().unwrap_or(
                        json!({"status": "unknown", "delivery": "unknown", "send_id": "sim-1"}),
                    ));
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            after.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(dispatches.load(Ordering::SeqCst), 2);
        let send_calls: Vec<Value> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);
        assert_eq!(
            send_calls[0]["idempotency_key"],
            send_calls[1]["idempotency_key"]
        );
    }

    // === ServiceTests ===
    #[tokio::test]
    async fn test_service_legacy_cursor_rebaselines_without_reply() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let model_called = Arc::new(AtomicBool::new(false));
        let mc = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "hi".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut legacy_img = test_bubble("[图片]", "inbound_visual");
        legacy_img.digest = "pixels".to_string();
        legacy_img.kind = Some("image".to_string());
        let legacy = test_frame(
            vec![test_bubble("old", "inbound_visual"), legacy_img],
            "午夜咖啡",
        );
        write_private(&svc.cursor_path, &legacy).unwrap();

        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        let mut curr_img = test_bubble("[图片]", "inbound_visual");
        curr_img.digest = "image:50:40".to_string();
        curr_img.kind = Some("image".to_string());
        curr_img.phash = Some("0".repeat(16));
        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                curr_img,
                test_bubble("新消息", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut capture = MockCapture::new(vec![current.clone(), current.clone()]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!model_called.load(Ordering::SeqCst));
        assert_eq!(svc.cursor.unwrap().bubbles, current.bubbles);
    }

    #[tokio::test]
    async fn test_service_text_only_cursor_from_before_v2_still_replies() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let model_called = Arc::new(AtomicBool::new(false));
        let mc = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        write_private(&svc.cursor_path, &old).unwrap();

        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let mc2 = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc2.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut capture = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(model_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_service_reply_waits_for_quiet_period() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct_clone = current_time.clone();
        svc.clock = Arc::new(move || *ct_clone.lock().unwrap());

        let model_calls = Arc::new(AtomicUsize::new(0));
        let mc = model_calls.clone();
        let seen_prompt = Arc::new(Mutex::new(String::new()));
        let sp = seen_prompt.clone();
        svc.brain = Some(Arc::new(move |turn| {
            mc.fetch_add(1, Ordering::SeqCst);
            *sp.lock().unwrap() = turn.text.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let one = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("图片说明", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let two = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("图片说明", "inbound_visual"),
                test_bubble("这是什么", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");

        let client = MockToolClient::new_default();

        // at 0
        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![one.clone(), one.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        // at 5
        *current_time.lock().unwrap() = 1005.0;
        let mut c2 = MockCapture::new(vec![two.clone(), two.clone()]);
        svc.cycle(&client, &mut c2).await.unwrap();

        // at 9
        *current_time.lock().unwrap() = 1009.0;
        let mut c3 = MockCapture::new(vec![two.clone()]);
        svc.cycle(&client, &mut c3).await.unwrap();
        assert_eq!(model_calls.load(Ordering::SeqCst), 0);

        // at 15
        *current_time.lock().unwrap() = 1015.0;
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("图片说明", "inbound_visual"),
                test_bubble("这是什么", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        let mut c4 = MockCapture::new(vec![two.clone(), two.clone(), after]);
        svc.cycle(&client, &mut c4).await.unwrap();

        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*seen_prompt.lock().unwrap(), "图片说明\n这是什么");
        assert!(svc.turn.is_none());
    }

    #[tokio::test]
    async fn test_service_turn_closes_at_max_even_while_customer_keeps_sending() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        cfg.turn_max_seconds = 60;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct_clone = current_time.clone();
        svc.clock = Arc::new(move || *ct_clone.lock().unwrap());

        let model_calls = Arc::new(AtomicUsize::new(0));
        let mc = model_calls.clone();
        let prompt_lines = Arc::new(AtomicUsize::new(0));
        let pl = prompt_lines.clone();
        svc.brain = Some(Arc::new(move |turn| {
            mc.fetch_add(1, Ordering::SeqCst);
            pl.store(turn.text.split('\n').count(), Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut bubbles = vec![test_bubble("old", "inbound_visual")];
        for second in (0..60).step_by(5) {
            bubbles.push(test_bubble(&format!("m{second}"), "inbound_visual"));
            *current_time.lock().unwrap() = 1000.0 + second as f64;
            let f = test_frame(bubbles.clone(), "午夜咖啡");
            let mut cap = MockCapture::new(vec![f.clone(), f.clone()]);
            let client = MockToolClient::new_default();
            svc.cycle(&client, &mut cap).await.unwrap();
        }
        assert_eq!(model_calls.load(Ordering::SeqCst), 0);

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        *current_time.lock().unwrap() = 1060.0;
        let mut after_bubbles = bubbles.clone();
        after_bubbles.push(reply.clone());
        let f_before = test_frame(bubbles.clone(), "午夜咖啡");
        let f_after = test_frame(after_bubbles, "午夜咖啡");
        let mut cap = MockCapture::new(vec![f_before.clone(), f_before.clone(), f_after.clone()]);
        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
        assert_eq!(prompt_lines.load(Ordering::SeqCst), 12);

        let mut later_bubbles = bubbles.clone();
        later_bubbles.push(reply);
        later_bubbles.push(test_bubble("还有一个问题", "inbound_visual"));
        let later = test_frame(later_bubbles, "午夜咖啡");
        *current_time.lock().unwrap() = 1062.0;
        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
        assert_eq!(svc.turn.as_ref().unwrap().bubbles[0].text, "还有一个问题");
    }

    #[tokio::test]
    async fn test_service_open_turn_survives_restart() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct_clone = current_time.clone();
        svc.clock = Arc::new(move || *ct_clone.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        // Restart
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let ct_clone2 = current_time.clone();
        svc.clock = Arc::new(move || *ct_clone2.lock().unwrap());
        let model_calls = Arc::new(AtomicUsize::new(0));
        let mc = model_calls.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        *current_time.lock().unwrap() = 1011.0;
        let mut cap2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_service_image_only_turn_replies() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let image_received = Arc::new(AtomicBool::new(false));
        let ir = image_received.clone();
        svc.brain = Some(Arc::new(move |turn| {
            if !turn.images.is_empty() {
                ir.store(true, Ordering::SeqCst);
            }
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.digest = "image:50:40".to_string();
        img.phash = Some("0".repeat(16));
        img.media_id = Some("a".repeat(64));
        img.kind = Some("image".to_string());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![test_bubble("old", "inbound_visual"), img.clone()],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![test_bubble("old", "inbound_visual"), img.clone(), reply],
            "午夜咖啡",
        );

        let shot_path = td.path().join("x.png");
        fs::write(&shot_path, b"fake png").unwrap();
        let shot_path_clone = shot_path.clone();

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            after,
        ]);
        let img_clone = img.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((
                vec![AttachmentImage {
                    path: shot_path_clone.clone(),
                    label: "第1张图片".to_string(),
                    source: "chat_thumbnail".to_string(),
                    bubble: img_clone.clone(),
                    media_id: None,
                }],
                Vec::new(),
                Vec::new(),
            ))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(image_received.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_service_manual_reply_during_wait_cancels_turn() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        *current_time.lock().unwrap() = 1000.0;
        let mut cap1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap1).await.unwrap();

        let answered = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                test_bubble("人工：我是店长，马上处理", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        *current_time.lock().unwrap() = 1004.0;
        let mut cap2 = MockCapture::new(vec![answered.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert!(svc.turn.is_none());
    }

    #[tokio::test]
    async fn test_service_manual_reply_while_generating_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let answered = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                test_bubble("人工：我是店长，马上处理", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), answered.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let send_calls: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert!(send_calls.is_empty());

        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        let rec: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(rec["status"], "already_replied");
        assert!(!svc.paused);
    }

    #[tokio::test]
    async fn test_service_reprocessed_triggered_turn_is_not_resent() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        *current_time.lock().unwrap() = 1000.0;
        let mut cap1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap1).await.unwrap();

        let key = svc.batch_key(
            &svc.turn.as_ref().unwrap().baseline,
            &svc.turn.as_ref().unwrap().bubbles,
        );
        write_private(
            &svc.root.join(format!("send-{key}.json")),
            &json!({"status": "triggered", "idempotency_key": key}),
        )
        .unwrap();

        *current_time.lock().unwrap() = 1011.0;
        let mut cap2 = MockCapture::new(vec![new_f.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert!(!svc.paused);
        assert!(svc.turn.is_none());
        assert!(client.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_service_own_echo_is_not_human_reply() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let again = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                test_bubble("第二个问题", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();

        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        *current_time.lock().unwrap() = 1011.0;
        let mut c2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), again.clone()]);
        svc.cycle(&client, &mut c2).await.unwrap();

        *current_time.lock().unwrap() = 1012.0;
        let mut c3 = MockCapture::new(vec![again.clone(), again.clone()]);
        svc.cycle(&client, &mut c3).await.unwrap();

        assert_eq!(svc.turn.as_ref().unwrap().bubbles[0].text, "第二个问题");

        *current_time.lock().unwrap() = 1023.0;
        let mut final_bubbles = again.bubbles.clone();
        final_bubbles.push(test_bubble("好的", "outbound_visual"));
        let final_frame = test_frame(final_bubbles, "午夜咖啡");
        let mut c4 = MockCapture::new(vec![again.clone(), again.clone(), final_frame]);
        svc.cycle(&client, &mut c4).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 2);
    }

    #[tokio::test]
    async fn test_service_ocr_noisy_own_echo_is_not_human_reply() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let noisy = test_bubble("你好，我是Al客服助手。", "outbound_visual");
        let again = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                test_bubble("第二个问题", "inbound_visual"),
                noisy,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();

        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        *current_time.lock().unwrap() = 1011.0;
        let mut c2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), again.clone()]);
        svc.cycle(&client, &mut c2).await.unwrap();

        *current_time.lock().unwrap() = 1012.0;
        let mut c3 = MockCapture::new(vec![again.clone(), again.clone()]);
        svc.cycle(&client, &mut c3).await.unwrap();

        assert_eq!(svc.turn.as_ref().unwrap().bubbles[0].text, "第二个问题");
    }

    #[tokio::test]
    async fn test_service_upload_in_progress_delays_turn() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let model_called = Arc::new(AtomicBool::new(false));
        let mc = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut card = test_bubble("报价.pdf\nPDF\n220.3K 对方上传中", "inbound_visual");
        card.transfer_pending = Some(true);
        card.kind = Some("file".to_string());
        card.filename = Some("报价.pdf".to_string());
        card.media_id = Some("1".repeat(64));

        let uploading = test_frame(
            vec![test_bubble("old", "inbound_visual"), card.clone()],
            "午夜咖啡",
        );
        let mut done_card = card.clone();
        done_card.text = "报价.pdf\nPDF\n220.3K".to_string();
        done_card.transfer_pending = Some(false);
        let done = test_frame(
            vec![test_bubble("old", "inbound_visual"), done_card],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();

        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![uploading.clone(), uploading.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        *current_time.lock().unwrap() = 1015.0;
        let mut c2 = MockCapture::new(vec![uploading.clone()]);
        svc.cycle(&client, &mut c2).await.unwrap();

        assert!(!model_called.load(Ordering::SeqCst));
        let health: Value = read_json(svc.root.join("health.json")).unwrap();
        assert_eq!(health["state"], "waiting_for_upload");

        *current_time.lock().unwrap() = 1020.0;
        let mut after_done = done.bubbles.clone();
        after_done.push(test_bubble("好的", "outbound_visual"));
        let after_f = test_frame(after_done, "午夜咖啡");
        let mut c3 = MockCapture::new(vec![done.clone(), done.clone(), done.clone(), after_f]);
        c3.attachments_fn = Some(Arc::new(|_, _| Ok((Vec::new(), Vec::new(), Vec::new()))));
        svc.cycle(&client, &mut c3).await.unwrap();

        assert!(model_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_service_model_connection_failure_retries_before_pausing() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let attempts = Arc::new(AtomicUsize::new(0));
        let att = attempts.clone();
        svc.brain = Some(Arc::new(move |_| {
            att.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { bail!("model connection failed; not automatically retried") })
        }));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();

        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        for second in [1011.0, 1045.0] {
            *current_time.lock().unwrap() = second;
            let mut c2 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
            svc.cycle(&client, &mut c2).await.unwrap();
            assert!(!svc.paused);
            assert!(svc.turn.is_some());
        }

        *current_time.lock().unwrap() = 1080.0;
        let mut c3 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        assert!(svc.cycle(&client, &mut c3).await.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert!(svc.paused);
    }

    #[tokio::test]
    async fn test_service_clipped_bottom_scrolls_down_instead_of_waiting_forever() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut clipped = new_f.clone();
        clipped.clipped_bubble = Some(true);

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let mut cap = MockCapture::new(vec![clipped, new_f.clone(), new_f.clone(), after]);
        cap.scroll_pages = vec![new_f.clone()];

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let calls = cap.scroll_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], ("down".to_string(), 40));
    }

    #[tokio::test]
    async fn test_service_group_turn_names_senders_and_strips_member_count() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.allowlist = vec![AllowlistEntry {
            recipient: "测试群".to_string(),
            kind: "group".to_string(),
            aliases: None,
        }];
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let seen_prompt = Arc::new(Mutex::new(String::new()));
        let sp = seen_prompt.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *sp.lock().unwrap() = turn.text.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "有的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "测试群 (3)");
        svc.advance(old).unwrap();

        let mut asked = test_bubble("@享味来 有货吗", "inbound_visual");
        asked.sender = Some("午夜咖啡".to_string());
        let new_f = test_frame(
            vec![test_bubble("old", "inbound_visual"), asked.clone()],
            "测试群 (3)",
        );
        let reply = test_bubble("有的", "outbound_visual");
        let after = test_frame(
            vec![test_bubble("old", "inbound_visual"), asked, reply],
            "测试群 (3)",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*seen_prompt.lock().unwrap(), "午夜咖啡：@享味来 有货吗");
        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends[0].1["recipient"], "测试群");
    }

    #[tokio::test]
    async fn test_service_malformed_model_output_retries_before_pausing() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let current_time = Arc::new(Mutex::new(1000.0f64));
        let ct = current_time.clone();
        svc.clock = Arc::new(move || *ct.lock().unwrap());

        let attempt = Arc::new(AtomicUsize::new(0));
        let att = attempt.clone();
        svc.brain = Some(Arc::new(move |_| {
            let n = att.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if n == 0 {
                    bail!("model returned malformed JSON twice");
                }
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();

        *current_time.lock().unwrap() = 1000.0;
        let mut c1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut c1).await.unwrap();

        *current_time.lock().unwrap() = 1011.0;
        let mut c2 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut c2).await.unwrap();
        assert!(!svc.paused);

        *current_time.lock().unwrap() = 1045.0;
        let mut c3 = MockCapture::new(vec![new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut c3).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_initial_start_does_not_reply_to_history() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        let model_called = Arc::new(AtomicBool::new(false));
        let mc = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "hi".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!model_called.load(Ordering::SeqCst));
        assert!(client.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_service_empty_baseline_never_turns_visible_history_into_new_messages() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.advance(test_frame(Vec::new(), "午夜咖啡")).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(client.calls.lock().unwrap().is_empty());
        assert_eq!(svc.cursor.unwrap().bubbles.len(), 2);
    }

    #[tokio::test]
    async fn test_service_frame_without_complete_bubbles_keeps_the_cursor() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let client = MockToolClient::new_default();
        for _ in 0..40 {
            let empty = test_frame(Vec::new(), "午夜咖啡");
            let mut cap = MockCapture::new(vec![empty]);
            svc.cycle(&client, &mut cap).await.unwrap();
        }
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, old.bubbles);

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let mut cap2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_tall_media_scrolling_history_off_after_send_does_not_pause() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("在的，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let scrolled = test_frame(
            vec![
                test_bubble("[图片]", "inbound_visual"),
                test_bubble("在的，我是AI客服助手。", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            scrolled.clone(),
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();
        assert!(!svc.paused);

        let mut cap2 = MockCapture::new(vec![scrolled.clone(), scrolled.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        assert!(!svc.paused);

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(svc.cursor.unwrap().bubbles, scrolled.bubbles);
    }

    #[tokio::test]
    async fn test_service_ordinary_incoming_has_no_test_prefix_and_replies_once() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].1["recipient"], "午夜咖啡");

        let mut cap2 = MockCapture::new(vec![after]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        let sends2: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends2.len(), 1);
    }

    #[tokio::test]
    async fn test_service_foreign_contact_never_calls_model() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let foreign = test_frame(vec![test_bubble("你好", "inbound_visual")], "其他人");
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![foreign]);

        assert!(svc.cycle(&client, &mut cap).await.is_err());
    }

    #[tokio::test]
    async fn test_service_identical_repeated_message_is_skipped_without_pause() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(
            vec![test_bubble("刚才发链接看了吗", "inbound_visual")],
            "午夜咖啡",
        );
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("刚才发链接看了吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![current.clone(), current.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, current.bubbles);
        assert!(client.calls.lock().unwrap().is_empty());

        let later = test_frame(
            vec![
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("在吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("在吗", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone(), later.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    fn timed(text: &str, direction: &str, label: Option<&str>) -> Bubble {
        let mut b = test_bubble(text, direction);
        if let Some(label) = label {
            b.extra.insert("time_label".into(), json!(label));
        }
        b
    }

    async fn identical_with_separators(
        old: &[Option<&str>],
        current: &[Option<&str>],
        confirm: &[Option<&str>],
    ) -> usize {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));
        let frame = |labels: &[Option<&str>]| {
            test_frame(
                labels
                    .iter()
                    .map(|l| timed("开放接口验收测试", "inbound_visual", *l))
                    .collect(),
                "午夜咖啡",
            )
        };
        svc.advance(frame(old)).unwrap();
        let mut after = frame(current);
        after
            .bubbles
            .push(test_bubble("你好，我是AI客服助手。", "outbound_visual"));
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            frame(current),
            frame(confirm),
            frame(current),
            frame(current),
            frame(current),
            after,
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();
        assert!(!svc.paused);
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .count()
    }

    #[tokio::test]
    async fn test_identical_message_under_new_time_separator_gets_one_reply() {
        let old = [Some("14:40"), Some("19:42"), Some("21:30")];
        let now = [Some("14:40"), Some("19:42"), Some("21:30"), Some("21:41")];
        assert_eq!(identical_with_separators(&old, &now, &now).await, 1);
    }

    #[tokio::test]
    async fn test_identical_message_without_new_separator_is_not_replied() {
        // Same minute: WeChat draws no new separator, so the newest cannot be told apart.
        let old = [Some("14:40"), Some("19:42"), Some("21:30")];
        let now = [Some("14:40"), Some("19:42"), Some("21:30"), None];
        assert_eq!(identical_with_separators(&old, &now, &now).await, 0);
    }

    #[tokio::test]
    async fn test_separator_must_agree_across_two_reads() {
        let old = [Some("14:40"), Some("19:42"), Some("21:30")];
        let now = [Some("14:40"), Some("19:42"), Some("21:30"), Some("21:41")];
        let other = [Some("14:40"), Some("19:42"), Some("21:30"), Some("21:47")];
        assert_eq!(identical_with_separators(&old, &now, &other).await, 0);
    }

    #[tokio::test]
    async fn test_earlier_or_unlabelled_cursor_is_not_replied() {
        let now = [Some("14:40"), Some("19:42"), Some("21:30"), Some("21:20")];
        let old = [Some("14:40"), Some("19:42"), Some("21:30")];
        assert_eq!(identical_with_separators(&old, &now, &now).await, 0);
        // A cursor saved before separators were recorded cannot be compared.
        let legacy = [None, None, None];
        let labelled = [Some("14:40"), Some("19:42"), Some("21:30"), Some("21:41")];
        assert_eq!(
            identical_with_separators(&legacy, &labelled, &labelled).await,
            0
        );
    }

    #[tokio::test]
    async fn test_service_ambiguous_frame_must_be_stable_before_skipping() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(
            vec![test_bubble("刚才发链接看了吗", "inbound_visual")],
            "午夜咖啡",
        );
        svc.advance(old.clone()).unwrap();

        let current = test_frame(
            vec![
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("刚才发链接看了吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let different = test_frame(
            vec![
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("刚才发链接看了吗", "inbound_visual"),
                test_bubble("在吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![current, different]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.unwrap().bubbles, old.bubbles);
    }

    #[tokio::test]
    async fn test_service_continuity_gap_rebaselines_without_reply_or_pause() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let cleared = test_frame(vec![test_bubble("你好 在吗", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![cleared.clone(), cleared.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, cleared.bubbles);
        assert!(client.calls.lock().unwrap().is_empty());

        let later = test_frame(
            vec![
                test_bubble("你好 在吗", "inbound_visual"),
                test_bubble("有人吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("你好 在吗", "inbound_visual"),
                test_bubble("有人吗", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone(), later.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_unrecoverable_gap_scrolls_at_most_gap_scroll_pages() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let cleared = test_frame(
            vec![test_bubble("全新的消息", "inbound_visual")],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![cleared.clone(); 8]);
        cap.scroll_pages = (0..60)
            .map(|n| {
                test_frame(
                    vec![test_bubble(&format!("更早{n}"), "inbound_visual")],
                    "午夜咖啡",
                )
            })
            .collect();

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let ups: Vec<_> = cap
            .scroll_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, _)| d == "up")
            .cloned()
            .collect();
        assert!(ups.len() <= GAP_SCROLL_PAGES as usize, "{}", ups.len());
    }

    #[tokio::test]
    async fn test_service_interrupted_gap_scroll_retries_instead_of_rebaselining() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(
            vec![
                test_bubble("a", "inbound_visual"),
                test_bubble("b", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        svc.advance(old.clone()).unwrap();

        let mut cap = MockCapture::new(vec![
            test_frame(
                vec![test_bubble("x", "inbound_visual")],
                "午夜咖啡"
            );
            3
        ]);
        // Set scroll to fail with interrupted
        cap.scroll_pages = Vec::new(); // will fallback to frame() which returns x, but we want scroll to fail:
        struct InterruptedCapture(MockCapture);
        impl Capture for InterruptedCapture {
            async fn frame(&mut self) -> Result<Frame> {
                self.0.frame().await
            }
            async fn scroll(&mut self, _d: &str, _p: u32) -> Result<Frame> {
                bail!("private visual operation failed: invalid(\"user_input_detected\")")
            }
        }
        let mut icap = InterruptedCapture(cap);
        let client = MockToolClient::new_default();
        assert!(svc.cycle(&client, &mut icap).await.is_err());
        assert_eq!(svc.cursor.unwrap().bubbles, old.bubbles);
    }

    #[tokio::test]
    async fn test_service_interrupted_media_fetch_keeps_the_turn_without_pausing() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.digest = "image:50:40".to_string();
        img.phash = Some("0f".repeat(8));
        img.media_id = Some("1".repeat(64));
        img.kind = Some("image".to_string());
        let current = test_frame(vec![test_bubble("old", "inbound_visual"), img], "午夜咖啡");

        let mut cap = MockCapture::new(vec![current.clone(); 4]);
        cap.attachments_fn = Some(Arc::new(|_, _| {
            bail!("private visual operation failed: invalid(\"user_input_detected\")")
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!svc.paused);
        assert!(svc.turn.is_some());
    }

    #[tokio::test]
    async fn test_service_tall_image_gap_recovered_by_scrolling() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let seen_prompt = Arc::new(Mutex::new(String::new()));
        let sp = seen_prompt.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *sp.lock().unwrap() = turn.text.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let c = test_bubble("c", "inbound_visual");
        let tall = test_bubble("TALL", "inbound_visual");
        let content = vec![(&a, 40), (&b, 40), (&c, 40), (&tall, 380)];

        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡"))
            .unwrap();

        let bottom = test_chat(&content, 0, 436);
        let up1 = test_chat(&content, 1, 436);
        let up2 = test_chat(&content, 2, 436);
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let mut after_b = bottom.bubbles.clone();
        after_b.push(reply);
        let after = test_frame(after_b, "午夜咖啡");

        let mut cap = MockCapture::new(vec![
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            after,
        ]);
        cap.scroll_pages = vec![up1, up2, bottom];

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*seen_prompt.lock().unwrap(), "c\nTALL");
    }

    #[tokio::test]
    async fn test_service_consecutive_tall_images_recovered_across_empty_pages() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let prompt_count = Arc::new(AtomicUsize::new(0));
        let pc = prompt_count.clone();
        svc.brain = Some(Arc::new(move |turn| {
            pc.store(turn.text.split('\n').count(), Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let mut img1 = test_bubble("[图片]", "inbound_visual");
        img1.phash = Some("0000000000000000".to_string());
        img1.digest = "image:61:61".to_string();
        img1.kind = Some("image".to_string());
        let mut img2 = test_bubble("[图片]", "inbound_visual");
        img2.phash = Some("ffffffff00000000".to_string());
        img2.digest = "image:61:61".to_string();
        img2.kind = Some("image".to_string());
        let mut img3 = test_bubble("[图片]", "inbound_visual");
        img3.phash = Some("00000000ffffffff".to_string());
        img3.digest = "image:61:61".to_string();
        img3.kind = Some("image".to_string());

        let content = vec![(&a, 40), (&b, 40), (&img1, 247), (&img2, 247), (&img3, 247)];
        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡"))
            .unwrap();

        let bottom = test_chat(&content, 0, 436);
        let ups: Vec<Frame> = (1..=8).map(|k| test_chat(&content, k, 436)).collect();
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let mut after_b = bottom.bubbles.clone();
        after_b.push(reply);
        let after = test_frame(after_b, "午夜咖啡");

        let mut cap = MockCapture::new(vec![
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            after,
        ]);
        cap.attachments_fn = Some(Arc::new(|_, _| Ok((Vec::new(), Vec::new(), Vec::new()))));
        let mut scroll_seq = ups;
        scroll_seq.push(bottom);
        cap.scroll_pages = scroll_seq;

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(prompt_count.load(Ordering::SeqCst), 3);
    }

    /// 2026-11-08 实机：一次发多张图，每张 155 高、每格滚动 90 点，原来最多 5 格翻不到之前的消息，
    /// 整批被当成"历史"跳过。微信一次最多 9 张：必须能翻回衔接点并把 9 张都交给大脑。
    #[tokio::test]
    async fn test_service_nine_images_burst_recovered_by_scrolling() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let prompt_count = Arc::new(AtomicUsize::new(0));
        let pc = prompt_count.clone();
        svc.brain = Some(Arc::new(move |turn| {
            pc.store(turn.text.split('\n').count(), Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let images: Vec<Bubble> = (0..9u64)
            .map(|i| {
                let mut img = test_bubble("[图片]", "inbound_visual");
                img.phash = Some(format!(
                    "{:016x}",
                    0x0123_4567_89ab_cdefu64.rotate_left((i * 7) as u32)
                ));
                img.digest = "image:61:38".to_string();
                img.kind = Some("image".to_string());
                img
            })
            .collect();
        let mut content: Vec<(&Bubble, i64)> = vec![(&a, 40), (&b, 40)];
        for img in &images {
            content.push((img, 155));
        }
        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡"))
            .unwrap();

        let bottom = test_chat(&content, 0, 436);
        let ups: Vec<Frame> = (1..=30).map(|k| test_chat(&content, k, 436)).collect();
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let mut after_b = bottom.bubbles.clone();
        after_b.push(reply);
        let after = test_frame(after_b, "午夜咖啡");

        let mut cap = MockCapture::new(vec![
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            after,
        ]);
        cap.attachments_fn = Some(Arc::new(|_, _| Ok((Vec::new(), Vec::new(), Vec::new()))));
        let mut scroll_seq = ups;
        scroll_seq.push(bottom);
        cap.scroll_pages = scroll_seq;

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(
            prompt_count.load(Ordering::SeqCst),
            9,
            "9 张图都必须作为新消息交给大脑"
        );
    }

    #[tokio::test]
    async fn test_service_lost_scroll_down_is_retried_before_giving_up() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let c = test_bubble("c", "inbound_visual");
        let tall = test_bubble("TALL", "inbound_visual");

        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡"))
            .unwrap();

        let bottom = test_frame(vec![c.clone(), tall.clone()], "午夜咖啡");
        let up1 = test_frame(vec![b.clone(), c.clone()], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![c.clone(), tall.clone(), reply], "午夜咖啡");

        let mut cap = MockCapture::new(vec![
            bottom.clone(),
            up1.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            after,
        ]);
        cap.scroll_pages = vec![up1.clone(), up1.clone(), bottom.clone()];

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();
    }

    #[tokio::test]
    async fn test_service_unread_jump_layout_image_is_recovered_and_answered() {
        // WeChat opened the chat at the first unread: the new image alone at the top, blank below.
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let seen_prompt = Arc::new(Mutex::new(String::new()));
        let sp = seen_prompt.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *sp.lock().unwrap() = turn.text.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));
        let placed = |b: &Bubble, y: i64, h: i64| {
            let mut b = b.clone();
            b.y = Some(y);
            b.height = Some(h);
            b
        };
        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let pic = test_bubble("PIC", "inbound_visual");
        svc.advance(test_frame(
            vec![placed(&a, 300, 40), placed(&b, 360, 40)],
            "午夜咖啡",
        ))
        .unwrap();
        let jumped = test_frame(vec![placed(&pic, 63, 247)], "午夜咖啡");
        let up = test_frame(
            vec![
                placed(&a, 33, 40),
                placed(&b, 93, 40),
                placed(&pic, 153, 247),
            ],
            "午夜咖啡",
        );
        let bottom = test_frame(
            vec![
                placed(&a, 58, 40),
                placed(&b, 118, 40),
                placed(&pic, 178, 247),
            ],
            "午夜咖啡",
        );
        let mut after_b = bottom.bubbles.clone();
        after_b.push(test_bubble("你好，我是AI客服助手。", "outbound_visual"));
        let after = test_frame(after_b, "午夜咖啡");
        let mut cap = MockCapture::new(vec![
            jumped,
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            after,
        ]);
        cap.scroll_pages = vec![up, bottom.clone(), bottom.clone(), bottom];
        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();
        let health: Value = read_json(svc.root.join("health.json")).unwrap();
        assert_ne!(health["state"], "waiting_for_stable_frame");
        assert_eq!(*seen_prompt.lock().unwrap(), "PIC");
    }

    #[tokio::test]
    async fn test_service_scroll_recovery_requires_return_to_bottom() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let old = test_frame(vec![a.clone(), b.clone()], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let content = test_frame(
            vec![
                test_bubble("c", "inbound_visual"),
                test_bubble("TALL", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let moved = test_frame(vec![test_bubble("X", "inbound_visual")], "午夜咖啡");

        let mut cap = MockCapture::new(vec![content, moved.clone(), moved.clone()]);
        cap.scroll_pages = vec![moved.clone(), moved.clone()];

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.unwrap().bubbles, old.bubbles);
        let health: Value = read_json(svc.root.join("health.json")).unwrap();
        assert_eq!(health["state"], "waiting_for_stable_frame");
    }

    #[tokio::test]
    async fn test_service_unstable_gap_frame_keeps_old_cursor() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let f1 = test_frame(vec![test_bubble("完全不同", "inbound_visual")], "午夜咖啡");
        let f2 = test_frame(
            vec![
                test_bubble("完全不同", "inbound_visual"),
                test_bubble("又变了", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![f1, f2]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.unwrap().bubbles, old.bubbles);
    }

    #[tokio::test]
    async fn test_service_own_message_is_not_input() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("我的消息", "outbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![f]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(svc.turn.is_none());
    }

    #[tokio::test]
    async fn test_service_incoming_during_send_remains_pending() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("补充问题", "inbound_visual"),
                test_bubble("你好，我是AI客服助手。", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.unwrap().bubbles, new_f.bubbles);
    }

    #[tokio::test]
    async fn test_service_new_activity_during_generation_still_sends_and_stays_pending() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let seen_prompt = Arc::new(Mutex::new(String::new()));
        let sp = seen_prompt.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *sp.lock().unwrap() = turn.text.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let changed = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("新问题", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("新问题", "inbound_visual"),
                test_bubble("你好，我是AI客服助手。", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            changed.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, new_f.bubbles);

        let final_after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("新问题", "inbound_visual"),
                test_bubble("你好，我是AI客服助手。", "outbound_visual"),
                test_bubble("好的", "outbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap2 = MockCapture::new(vec![
            after.clone(),
            after.clone(),
            after.clone(),
            final_after,
        ]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert_eq!(*seen_prompt.lock().unwrap(), "新问题");
    }

    #[tokio::test]
    async fn test_service_scrolled_echo_with_repeated_greeting_does_not_pause_or_resend() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = vec![
            test_bubble("hi", "inbound_visual"),
            test_bubble("hi", "inbound_visual"),
            test_bubble("there", "inbound_visual"),
            test_bubble("question", "inbound_visual"),
            test_bubble("received", "inbound_visual"),
        ];
        svc.advance(test_frame(old.clone(), "午夜咖啡")).unwrap();

        let mut inc_bubbles = old.clone();
        inc_bubbles.push(test_bubble("hi", "inbound_visual"));
        let incoming = test_frame(inc_bubbles.clone(), "午夜咖啡");

        let mut after_bubbles = inc_bubbles[1..].to_vec();
        after_bubbles.push(test_bubble("你好，我是AI客服助手。", "outbound_visual"));
        let after = test_frame(after_bubbles, "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            incoming.clone(),
            incoming.clone(),
            incoming.clone(),
            after.clone(),
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, after.bubbles);
        assert!(!svc.paused);

        let mut cap2 = MockCapture::new(vec![after]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_model_failure_pauses_and_survives_restart() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(Arc::new(|_| Box::pin(async { bail!("model unavailable") })));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();

        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        // With MODEL_RETRIES=2, failure happens after 3 attempts or we test restart directly
        let _ = svc.cycle(&client, &mut cap).await;

        let svc2 = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(!svc2.paused);
    }

    #[test]
    fn test_service_restart_accepts_cursor_saved_before_theme_profiles() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut legacy: Value = read_json(&svc.cursor_path).unwrap();
        legacy["profile"] = json!("wechat-4-dark-1x");
        if let Some(map) = legacy.as_object_mut() {
            map.remove("scale");
        }
        write_private(&svc.cursor_path, &legacy).unwrap();

        let restarted = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert_eq!(
            restarted.cursor.as_ref().unwrap().profile.as_deref(),
            Some("wechat-4-dark")
        );
        assert_eq!(restarted.cursor.as_ref().unwrap().scale, Some(json!(1)));
    }

    #[tokio::test]
    async fn test_service_stop_during_generation_never_sends_afterward() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let stop_flag = svc.stop.clone();
        svc.brain = Some(Arc::new(move |_| {
            stop_flag.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(client.calls.lock().unwrap().is_empty());
        let journals: Vec<_> = fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        let rec: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(rec["status"], "stopped_before_send");
    }

    #[tokio::test]
    async fn test_service_turn_stopped_before_send_is_answered_after_restart() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let stop_flag = svc.stop.clone();
        svc.brain = Some(Arc::new(move |_| {
            stop_flag.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        // Restart
        let mut svc2 = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc2.sleep = Arc::new(|_| Box::pin(async {}));
        svc2.brain = Some(default_brain("你好，我是AI客服助手。"));

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );
        let mut cap2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), after]);
        svc2.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert!(!svc2.paused);
    }

    #[tokio::test]
    async fn test_service_rate_limit_preserves_pending_input() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.max_replies_per_hour = 1;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        write_private(
            &svc.root.join("send-limit.json"),
            &json!({
                "status": "triggered",
                "attempted_at": (svc.clock)()
            }),
        )
        .unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(
            svc.turn.as_ref().unwrap().bubbles[0].text,
            "你好，介绍一下你自己"
        );
    }

    #[test]
    fn test_service_state_cannot_cross_accounts_or_contacts() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let _ = BotService::new(cfg.clone(), None, None, None, None, None).unwrap();

        let mut other_cfg = cfg;
        other_cfg.account_id = "other".to_string();
        assert!(BotService::new(other_cfg, None, None, None, None, None).is_err());
    }

    #[test]
    fn test_service_health_logs_changes_without_poll_spam_or_chat_body() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(svc.health("watching", Map::new()).is_ok());
        assert!(svc.health("watching", Map::new()).is_ok());
        let mut d = Map::new();
        d.insert("image_count".to_string(), json!(1));
        assert!(svc.health("generating", d).is_ok());
    }

    #[test]
    fn test_service_health_logs_periodic_heartbeat() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(svc.health("watching", Map::new()).is_ok());
        *svc.last_health_log_at.lock().unwrap() -= 61.0;
        assert!(svc.health("watching", Map::new()).is_ok());
    }

    // === HistoryServiceTests ===
    #[tokio::test]
    async fn test_history_service_messages_recorded_before_model_call() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&old).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();

        let order = Arc::new(Mutex::new(Vec::new()));
        let ord = order.clone();
        svc.brain = Some(Arc::new(move |_| {
            ord.lock().unwrap().push("model");
            Box::pin(async {
                Ok(BrainReply {
                    reply: "你好，我是AI客服助手。".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*order.lock().unwrap(), vec!["model"]);
        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard
            .prepare("SELECT sender FROM messages ORDER BY seq")
            .unwrap();
        let senders: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(senders, vec!["customer", "customer", "bot"]);
    }

    #[tokio::test]
    async fn test_history_service_context_uses_stored_history_limit() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.context_messages = 2;
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let bubbles: Vec<Bubble> = (0..5)
            .map(|i| test_bubble(&i.to_string(), "inbound_visual"))
            .collect();
        let base = test_frame(bubbles, "午夜咖啡");
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&base).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        svc.advance(base.clone()).unwrap();

        let mut new_bubbles = base.bubbles.clone();
        new_bubbles.push(test_bubble("新问题", "inbound_visual"));
        let new_f = test_frame(new_bubbles, "午夜咖啡");

        let context_texts = Arc::new(Mutex::new(Vec::new()));
        let ct = context_texts.clone();
        svc.brain = Some(Arc::new(move |turn| {
            let texts: Vec<String> = turn.context.iter().map(|c| c.text.clone()).collect();
            *ct.lock().unwrap() = texts;
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*context_texts.lock().unwrap(), vec!["3", "4", "新问题"]);
    }

    #[tokio::test]
    async fn test_history_service_baseline_frame_is_recorded_without_reply() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard
            .prepare("SELECT text FROM messages ORDER BY seq")
            .unwrap();
        let texts: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(texts, vec!["old", "你好，介绍一下你自己"]);
    }

    #[tokio::test]
    async fn pdf_after_image_reply_echo_is_not_already_replied() {
        let td = TestDir::new();
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let cid = history
            .ensure_contact(
                "fixture",
                &json!({"recipient": "午夜咖啡", "kind": "private"}),
                false,
            )
            .unwrap();
        let mut svc = BotService::new(
            fixture_config(td.path()),
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("PDF已收到"));
        let mut image = test_bubble("[图片]", "inbound_visual");
        image.kind = Some("image".into());
        image.phash = Some("image-phash".into());
        let baseline = test_frame(vec![image.clone()], "午夜咖啡");
        svc.advance(baseline.clone()).unwrap();
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&baseline).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        let image_ids = history
            .match_recent_inbound(&cid, &baseline.bubbles)
            .unwrap();
        let sent = "@午夜咖啡 看到这款「海带丝（香辣味）」，包装配料写着海带、饮用水、食用盐";
        let observed = "◎午夜咖啡 看到这款「海带丝（香辣味）」，包装配料写\n着海带、饮用水、食用盐";
        history
            .record_bot_reply(
                &cid,
                "image-send",
                sent,
                "triggered",
                &json!({}),
                &image_ids,
                None,
                None,
            )
            .unwrap();
        write_private(
            &svc.root.join("send-image.json"),
            &json!({"reply": sent, "status": "triggered", "attempted_at": (svc.clock)()}),
        )
        .unwrap();
        let pdf = test_bubble("岗位信息更新2.13.paf\nPDF\n220.3K 未下载", "inbound_visual");
        let echo = test_bubble(observed, "outbound_visual");
        assert!(svc.own_echo(&echo));
        let frame = test_frame(vec![image, pdf, echo], "午夜咖啡");
        let mut after = frame.clone();
        after
            .bubbles
            .push(test_bubble("PDF已收到", "outbound_visual"));
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![frame.clone(), frame.clone(), frame, after]);
        svc.cycle(&client, &mut cap).await.unwrap();
        let senders: Vec<String> = history
            .connection()
            .lock()
            .unwrap()
            .prepare("SELECT sender FROM messages ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(senders, ["customer", "bot", "customer", "bot"]);
        for journal in fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
        {
            let rec: Value = read_json(journal.path()).unwrap();
            assert_ne!(rec["status"], "already_replied");
        }
        assert_eq!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == "send_message")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn rebaseline_after_echo_does_not_mark_next_turn_replied() {
        let td = TestDir::new();
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let cid = history
            .ensure_contact(
                "fixture",
                &json!({"recipient": "午夜咖啡", "kind": "private"}),
                false,
            )
            .unwrap();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let now = Arc::new(Mutex::new(1000.0));
        let clock = now.clone();
        svc.clock = Arc::new(move || *clock.lock().unwrap());
        svc.brain = Some(default_brain("PDF已收到"));
        let mut image = test_bubble("[图片]", "inbound_visual");
        image.kind = Some("image".into());
        image.phash = Some("image-phash".into());
        let baseline = test_frame(vec![image.clone()], "午夜咖啡");
        svc.advance(baseline.clone()).unwrap();
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&baseline).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        let image_ids = history
            .match_recent_inbound(&cid, &baseline.bubbles)
            .unwrap();
        let sent = "@午夜咖啡 看到这款「海带丝（香辣味）」，包装配料写着海带、饮用水、食用盐";
        let observed = "◎午夜咖啡 看到这款「海带丝（香辣味）」，包装配料写\n着海带、饮用水、食用盐";
        history
            .record_bot_reply(
                &cid,
                "image-send",
                sent,
                "triggered",
                &json!({}),
                &image_ids,
                None,
                None,
            )
            .unwrap();
        write_private(
            &svc.root.join("send-image.json"),
            &json!({"reply": sent, "status": "triggered", "attempted_at": (svc.clock)()}),
        )
        .unwrap();
        // The first observation has already claimed this bot reply's digest.
        let first_echo = test_frame(
            vec![image.clone(), test_bubble(observed, "outbound_visual")],
            "午夜咖啡",
        );
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&first_echo).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            history.send("image-send").unwrap().unwrap()["echo_observed"],
            true
        );
        let pdf = test_bubble("岗位信息更新2.13.paf\nPDF\n220.3K 未下载", "inbound_visual");
        let echo = test_bubble(observed, "outbound_visual");
        assert!(svc.own_echo(&echo));
        let frame = test_frame(vec![image, pdf, echo], "午夜咖啡");
        let mut after = frame.clone();
        after
            .bubbles
            .push(test_bubble("PDF已收到", "outbound_visual"));
        let client = MockToolClient::new_default();
        // PDF appears before the already-claimed echo: the history cursor loses
        // continuity. The first stable frame opens the PDF turn; the second
        // records all visible bubbles via rebaseline_gap before sending it.
        let mut cap = MockCapture::new(vec![frame.clone(), frame.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();
        assert!(svc.turn.is_some());
        *now.lock().unwrap() = 1011.0;
        let mut cap = MockCapture::new(vec![frame.clone(), frame.clone(), frame, after]);
        svc.cycle(&client, &mut cap).await.unwrap();
        let capture: String = history.connection().lock().unwrap().query_row(
            "SELECT capture FROM messages WHERE text LIKE '岗位信息%' ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(capture, "rebaseline_gap");
        let senders: Vec<String> = history
            .connection()
            .lock()
            .unwrap()
            .prepare("SELECT sender FROM messages ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for journal in fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
        {
            let rec: Value = read_json(journal.path()).unwrap();
            assert_ne!(rec["status"], "already_replied");
        }
        assert!(!senders.iter().any(|s| s == "self_other"));
        assert_eq!(senders.iter().filter(|s| *s == "bot").count(), 2);
        assert_eq!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == "send_message")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn newer_message_during_send_still_gets_reply() {
        let td = TestDir::new();
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let cid = history
            .ensure_contact(
                "fixture",
                &json!({"recipient": "午夜咖啡", "kind": "private"}),
                false,
            )
            .unwrap();
        let mut svc = BotService::new(
            fixture_config(td.path()),
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.advance(test_frame(
            vec![test_bubble("old", "inbound_visual")],
            "午夜咖啡",
        ))
        .unwrap();
        let a = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("A", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("A", "inbound_visual"),
                test_bubble("B", "inbound_visual"),
                test_bubble("R_A", "outbound_visual"),
            ],
            "午夜咖啡",
        );
        let pending_history = history.clone();
        let pending_cid = cid.clone();
        let mut pending_frame = after.clone();
        pending_frame.bubbles.pop();
        svc.brain = Some(Arc::new(move |_| {
            pending_history
                .record_frame(
                    &pending_cid,
                    &serde_json::to_value(&pending_frame).unwrap(),
                    None,
                    None,
                    false,
                )
                .unwrap();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "R_A".into(),
                    metadata: HashMap::new(),
                })
            })
        }));
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![a.clone(), a.clone(), a, after.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();
        // B arrived while A was being sent: preserve the observed A -> B -> R_A order.
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&after).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        let inputs: Vec<String> = history
            .connection()
            .lock()
            .unwrap()
            .prepare("SELECT input_message_ids FROM sends ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(inputs.len(), 1);
        let ids: Vec<String> = serde_json::from_str(&inputs[0]).unwrap();
        assert_eq!(ids.len(), 1, "A's reply must cover only A");
        let texts: Vec<String> = history
            .connection()
            .lock()
            .unwrap()
            .prepare("SELECT text FROM messages WHERE text!='old' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(texts, ["A", "B", "R_A"]);
        svc.brain = Some(default_brain("R_B"));
        let mut final_frame = after.clone();
        final_frame
            .bubbles
            .push(test_bubble("R_B", "outbound_visual"));
        let mut cap = MockCapture::new(vec![after.clone(), after.clone(), after, final_frame]);
        svc.cycle(&client, &mut cap).await.unwrap();
        for journal in fs::read_dir(&svc.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
        {
            let rec: Value = read_json(journal.path()).unwrap();
            assert_ne!(rec["status"], "already_replied");
        }
        assert_eq!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == "send_message")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn test_history_service_history_reply_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        // First record the new incoming message so match_recent_inbound finds input_ids
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&new_f).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();
        let input_ids = history
            .match_recent_inbound(&cid, &new_f.bubbles[1..])
            .unwrap();
        // A bot reply blocks this turn only when it covers the same input.
        history
            .record_bot_reply(
                &cid,
                "key-other",
                "人工回答",
                "triggered",
                &json!({}),
                &input_ids,
                None,
                None,
            )
            .unwrap();

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert!(sends.is_empty());
    }

    #[tokio::test]
    async fn test_history_service_not_sent_retry_is_not_blocked_by_own_history_row() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(cfg, None, Some(history), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let attempt = Arc::new(AtomicUsize::new(0));
        let att = attempt.clone();
        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(move |name, _args| {
                if name == "send_message" {
                    let c = att.fetch_add(1, Ordering::SeqCst) + 1;
                    if c == 1 {
                        bail!("MCP tool failed: send_message: 发送尝试 s 未触发发送: 读回不一致");
                    }
                    Ok(json!({"status": "triggered", "send_id": "s", "delivery": "unknown"}))
                } else if name == "get_send_status" {
                    let c = att.load(Ordering::SeqCst);
                    if c == 1 {
                        Ok(json!({"status": "unknown", "delivery": "not_sent"}))
                    } else {
                        Ok(json!({"status": "triggered", "delivery": "unknown"}))
                    }
                } else {
                    Ok(json!({}))
                }
            }),
        };

        let mut cap = MockCapture::new(vec![
            new_f.clone(),
            new_f.clone(),
            new_f.clone(),
            after.clone(),
            after,
        ]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 2);
    }

    #[tokio::test]
    async fn test_history_service_turn_media_attached_to_history() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.digest = "image:50:40".to_string();
        img.phash = Some("0".repeat(16));
        img.media_id = Some("a".repeat(64));
        img.kind = Some("image".to_string());
        let current = test_frame(
            vec![test_bubble("old", "inbound_visual"), img.clone()],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![test_bubble("old", "inbound_visual"), img.clone(), reply],
            "午夜咖啡",
        );

        let img_path = td.path().join("x.png");
        fs::write(&img_path, b"png original").unwrap();
        let img_path_clone = img_path.clone();

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            after,
        ]);
        let img_clone = img.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((
                vec![AttachmentImage {
                    path: img_path_clone.clone(),
                    label: "第1张图片".to_string(),
                    source: "original".to_string(),
                    bubble: img_clone.clone(),
                    media_id: None,
                }],
                Vec::new(),
                Vec::new(),
            ))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard.prepare("SELECT source FROM media").unwrap();
        let sources: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(sources.contains(&"original".to_string()));
    }

    #[tokio::test]
    async fn test_history_service_failed_item_named_when_model_omits_it() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("已收到"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.kind = Some("image".to_string());
        let current = test_frame(vec![test_bubble("old", "inbound_visual"), img], "午夜咖啡");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("已收到第9张图片打不开，麻烦重发一下。", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            after,
        ]);
        cap.attachments_fn = Some(Arc::new(|_, _| {
            Ok((
                Vec::new(),
                Vec::new(),
                vec![FailedAttachmentItem {
                    label: "第9张图片".to_string(),
                    kind: "image".to_string(),
                    reason: "download_failed".to_string(),
                    detail: None,
                }],
            ))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        let msg = sends[0].1["message"].as_str().unwrap();
        assert!(msg.contains("第9张图片打不开，麻烦重发一下。"));
    }

    #[tokio::test]
    async fn test_history_service_send_uses_observed_title_for_alias() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.allowlist = vec![AllowlistEntry {
            recipient: "🍓午夜咖啡".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["午夜咖啡".to_string()]),
        }];
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "🍓午夜咖啡", "kind": "private", "aliases": ["午夜咖啡"]});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(cfg, None, Some(history), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                reply,
            ],
            "午夜咖啡",
        );

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].1["recipient"], "午夜咖啡");
    }

    #[test]
    fn test_history_service_binding_uses_contact_id() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let cid = "c-123456".to_string();
        let svc = BotService::new(cfg, None, None, Some(cid.clone()), None, None).unwrap();
        let binding: Value = read_json(svc.root.join("binding.json")).unwrap();
        assert_eq!(binding["contact_id"], cid);
        assert!(binding.get("recipient").is_none());
    }

    #[tokio::test]
    async fn test_brain_reply_action_skip() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        // Brain 返回 action=skip
        svc.brain = Some(Arc::new(|_| {
            Box::pin(async {
                let mut meta = HashMap::new();
                meta.insert("action".to_string(), json!("skip"));
                Ok(BrainReply {
                    reply: String::new(),
                    metadata: meta,
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        // 没发送任何消息
        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 0);

        // 检查日志状态为 skipped_by_brain
        let mut found_journal = false;
        for entry in fs::read_dir(td.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("send-") && name.ends_with(".json") {
                let j: Value = read_json(entry.path()).unwrap();
                assert_eq!(j["status"], "skipped_by_brain");
                found_journal = true;
            }
        }
        assert!(
            found_journal,
            "should create journal with status skipped_by_brain"
        );
    }

    #[tokio::test]
    async fn test_brain_reply_action_handoff_and_resume() {
        use std::os::unix::fs::PermissionsExt;
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        // 1. Brain 返回 action=handoff
        svc.brain = Some(Arc::new(|_| {
            Box::pin(async {
                let mut meta = HashMap::new();
                meta.insert("action".to_string(), json!("handoff"));
                Ok(BrainReply {
                    reply: String::new(),
                    metadata: meta,
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("转人工客服", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        // 没有调用 send_message
        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 0);

        // 检查日志状态为 handoff
        let mut found_journal = false;
        for entry in fs::read_dir(td.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("send-") && name.ends_with(".json") {
                let j: Value = read_json(entry.path()).unwrap();
                assert_eq!(j["status"], "handoff");
                found_journal = true;
            }
        }
        assert!(found_journal, "journal status should be handoff");

        // 检查 sessions/<id>/handoff.json 生成且权限为 0600
        let handoff_path = td.path().join("handoff.json");
        assert!(handoff_path.exists());
        let perm = fs::metadata(&handoff_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(perm, 0o600);
        let handoff_data: Value = read_json(&handoff_path).unwrap();
        assert_eq!(handoff_data["recipient"], "午夜咖啡");
        assert!(handoff_data.get("turn_id").is_some());
        assert!(handoff_data.get("at").is_some());

        // 2. 当 handoff.json 存在时，新消息到达：不交给任何大脑、不回复，health 为 handoff
        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "should not be called".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let new_f2 = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("转人工客服", "inbound_visual"),
                test_bubble("有人在吗", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap2 = MockCapture::new(vec![new_f2.clone(), new_f2.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        assert!(
            !brain_called.load(Ordering::SeqCst),
            "brain should not be called while in handoff"
        );
        let health: Value = read_json(td.path().join("health.json")).unwrap();
        assert_eq!(health["state"], "handoff");

        // 3. resume_session 删除 handoff.json 并恢复自动回复
        svc.resume_session().unwrap();
        assert!(
            !handoff_path.exists(),
            "handoff.json should be removed on resume_session"
        );

        let reply_f = test_bubble("您好，我是AI助手", "outbound_visual");
        let after_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("转人工客服", "inbound_visual"),
                test_bubble("有人在吗", "inbound_visual"),
                test_bubble("问个问题", "inbound_visual"),
                reply_f,
            ],
            "午夜咖啡",
        );
        let new_f3 = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("转人工客服", "inbound_visual"),
                test_bubble("有人在吗", "inbound_visual"),
                test_bubble("问个问题", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let mut cap3 = MockCapture::new(vec![
            new_f3.clone(),
            new_f3.clone(),
            new_f3.clone(),
            after_f.clone(),
        ]);
        svc.cycle(&client, &mut cap3).await.unwrap();
        assert!(
            brain_called.load(Ordering::SeqCst),
            "brain should be called after resume_session"
        );
    }

    #[test]
    fn test_startup_recovery_closes_generating_journals_to_not_sent() {
        let td = TestDir::new();
        let journal_path = td.path().join("send-testkey123.json");
        let record = json!({
            "status": "generating",
            "account_id": "fixture",
            "recipient": "午夜咖啡",
            "idempotency_key": "testkey123"
        });
        write_private(&journal_path, &record).unwrap();

        let cfg = fixture_config(td.path());
        let _svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let recovered: Value = read_json(&journal_path).unwrap();
        assert_eq!(recovered["status"], "not_sent");
        assert_eq!(recovered["closed_on_startup"], true);
    }

    #[tokio::test]
    async fn test_history_failure_blocks_model_and_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        // 模拟 history 写入失败（设置为只读数据库）
        history
            .connection()
            .lock()
            .unwrap()
            .execute("PRAGMA query_only = ON", [])
            .unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f]);

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!brain_called.load(Ordering::SeqCst));
        assert_eq!(client.calls.lock().unwrap().len(), 0);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, old.bubbles);
        let health: Value = read_json(td.path().join("health.json")).unwrap();
        assert_eq!(health["state"], "history_write_failed");
    }

    #[test]
    fn test_service_history_failure_blocks_model_and_send() {
        test_history_failure_blocks_model_and_send();
    }

    #[tokio::test]
    async fn test_history_read_failure_keeps_message_pending() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好，介绍一下你自己", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        // 模拟 history 读取失败（通过删除 sends 表使 recent_context 失败，但 record_frame 正常）
        history
            .connection()
            .lock()
            .unwrap()
            .execute("DROP TABLE sends", [])
            .unwrap();

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone()]);

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!brain_called.load(Ordering::SeqCst));
        assert_eq!(
            svc.turn.as_ref().unwrap().bubbles[0].text,
            "你好，介绍一下你自己"
        );

        // 没有写入 send-*.json
        let journals: Vec<_> = fs::read_dir(td.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("send-") && n.ends_with(".json")
            })
            .collect();
        assert_eq!(journals.len(), 0);

        let health: Value = read_json(td.path().join("health.json")).unwrap();
        assert_eq!(health["state"], "history_write_failed");
    }

    #[test]
    fn test_service_history_read_failure_keeps_message_pending() {
        test_history_read_failure_keeps_message_pending();
    }

    #[tokio::test]
    async fn test_scroll_recovered_messages_are_recorded_in_history() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let c = test_bubble("c", "inbound_visual");
        let tall = test_bubble("TALL", "inbound_visual");

        let content = vec![(&a, 40i64), (&b, 40), (&c, 40), (&tall, 380)];
        let base = test_frame(vec![a.clone(), b.clone()], "午夜咖啡");
        svc.advance(base.clone()).unwrap();
        history
            .record_frame(
                &cid,
                &serde_json::to_value(&base).unwrap(),
                None,
                None,
                false,
            )
            .unwrap();

        let bottom = test_chat(&content, 0, 500);
        let mut reply_bubbles = bottom.bubbles.clone();
        reply_bubbles.push(test_bubble("你好，我是AI客服助手。", "outbound_visual"));
        let reply_frame = test_frame(reply_bubbles, "午夜咖啡");

        let p1 = test_chat(&content, 1, 500);
        let p2 = test_chat(&content, 2, 500);

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            bottom.clone(),
            reply_frame,
        ]);
        cap.scroll_pages = vec![p1, p2, bottom];

        svc.cycle(&client, &mut cap).await.unwrap();

        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard
            .prepare("SELECT text FROM messages WHERE contact_id=? ORDER BY seq")
            .unwrap();
        let texts: Vec<String> = stmt
            .query_map([&cid], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(texts.len() >= 4);
        assert_eq!(&texts[..4], &["a", "b", "c", "TALL"]);
    }

    #[test]
    fn test_service_scroll_recovered_messages_are_recorded_in_history() {
        test_scroll_recovered_messages_are_recorded_in_history();
    }

    #[tokio::test]
    async fn test_nine_image_turn_sends_one_reply() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.model_config.backend = "openai-compatible".to_string();
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let mut imgs = Vec::new();
        let mut items = Vec::new();
        for n in 1..=9 {
            let mut b = test_bubble("[图片]", "inbound_visual");
            b.kind = Some("image".to_string());
            b.digest = "image:50:40".to_string();
            b.phash = Some(format!("{:016x}", (n as u64) * 0x0f0f0f0f0fu64));
            let img_path = td.path().join(format!("{n}.png"));
            fs::write(&img_path, format!("png-{n}")).unwrap();
            items.push(AttachmentImage {
                path: img_path,
                label: format!("第{n}张图片"),
                source: "original".to_string(),
                bubble: b.clone(),
                media_id: None,
            });
            imgs.push(b);
        }

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut current_bubbles = vec![test_bubble("old", "inbound_visual")];
        current_bubbles.extend(imgs);
        let current = test_frame(current_bubbles.clone(), "午夜咖啡");

        let mut sent_bubbles = current_bubbles.clone();
        sent_bubbles.push(test_bubble("已收到", "outbound_visual"));
        let sent_frame = test_frame(sent_bubbles, "午夜咖啡");

        let understand_called = Arc::new(AtomicUsize::new(0));
        let uc = understand_called.clone();
        svc.understand_fn = Some(Arc::new(move |images, _, _, _| {
            uc.fetch_add(1, Ordering::SeqCst);
            let notes = images
                .iter()
                .map(|i| Some(format!("要点{}", i.label)))
                .collect();
            Box::pin(async move { notes })
        }));

        let received_images = Arc::new(Mutex::new(Vec::new()));
        let received_notes = Arc::new(Mutex::new(Vec::new()));
        let ri = received_images.clone();
        let rn = received_notes.clone();

        svc.brain = Some(Arc::new(move |turn| {
            *ri.lock().unwrap() = turn.images.clone();
            *rn.lock().unwrap() = turn.notes.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "已收到".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent_frame,
        ]);
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((items.clone(), Vec::new(), Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(understand_called.load(Ordering::SeqCst), 1);
        assert_eq!(received_images.lock().unwrap().len(), 0);
        assert_eq!(received_notes.lock().unwrap().len(), 9);
    }

    #[test]
    fn test_service_nine_image_turn_sends_one_reply() {
        test_nine_image_turn_sends_one_reply();
    }

    #[tokio::test]
    async fn test_few_images_go_to_the_model_directly() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.model_config.backend = "openai-compatible".to_string();
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let mut imgs = Vec::new();
        let mut items = Vec::new();
        for n in 1..=3 {
            let mut b = test_bubble("[图片]", "inbound_visual");
            b.kind = Some("image".to_string());
            b.digest = "image:50:40".to_string();
            b.phash = Some(format!("{:016x}", (n as u64) * 0x0f0f0f0f0fu64));
            let img_path = td.path().join(format!("{n}.png"));
            fs::write(&img_path, format!("png-{n}")).unwrap();
            items.push(AttachmentImage {
                path: img_path,
                label: format!("第{n}张图片"),
                source: "original".to_string(),
                bubble: b.clone(),
                media_id: None,
            });
            imgs.push(b);
        }

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut current_bubbles = vec![test_bubble("old", "inbound_visual")];
        current_bubbles.extend(imgs);
        let current = test_frame(current_bubbles.clone(), "午夜咖啡");

        let mut sent_bubbles = current_bubbles.clone();
        sent_bubbles.push(test_bubble("已收到", "outbound_visual"));
        let sent_frame = test_frame(sent_bubbles, "午夜咖啡");

        let understand_called = Arc::new(AtomicUsize::new(0));
        let uc = understand_called.clone();
        svc.understand_fn = Some(Arc::new(move |images, _, _, _| {
            uc.fetch_add(1, Ordering::SeqCst);
            let notes = images
                .iter()
                .map(|i| Some(format!("要点{}", i.label)))
                .collect();
            Box::pin(async move { notes })
        }));

        let received_images = Arc::new(Mutex::new(Vec::new()));
        let ri = received_images.clone();

        svc.brain = Some(Arc::new(move |turn| {
            *ri.lock().unwrap() = turn.images.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "已收到".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent_frame,
        ]);
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((items.clone(), Vec::new(), Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(understand_called.load(Ordering::SeqCst), 0);
        assert_eq!(received_images.lock().unwrap().len(), 3);
    }

    #[test]
    fn test_service_few_images_go_to_the_model_directly() {
        test_few_images_go_to_the_model_directly();
    }

    #[tokio::test]
    async fn test_unsupported_file_reason_reaches_the_model() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let mut file_b = test_bubble("报价.xls", "inbound_visual");
        file_b.kind = Some("file".to_string());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![test_bubble("old", "inbound_visual"), file_b.clone()],
            "午夜咖啡",
        );
        let sent = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                file_b,
                test_bubble("已收到", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let failed_received = Arc::new(Mutex::new(Vec::new()));
        let fr = failed_received.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *fr.lock().unwrap() = turn.failed.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "已收到".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((
                Vec::new(),
                Vec::new(),
                vec![FailedAttachmentItem {
                    label: "报价.xls".to_string(),
                    kind: "file".to_string(),
                    reason: "unsupported_type".to_string(),
                    detail: Some("unsupported_file_type".to_string()),
                }],
            ))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let f = failed_received.lock().unwrap();
        assert_eq!(
            f.last().unwrap().reason.as_deref(),
            Some("unsupported_type")
        );
        let sends: Vec<_> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .cloned()
            .collect();
        assert_eq!(sends.len(), 1);
    }

    #[test]
    fn test_service_unsupported_file_reason_reaches_the_model() {
        test_unsupported_file_reason_reaches_the_model();
    }

    #[tokio::test]
    async fn test_final_generation_failure_still_pauses() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.model_retries = Some(0);

        svc.brain = Some(Arc::new(|_| {
            Box::pin(async { bail!("reply generation failed") })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        let client = MockToolClient::new_default();

        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_err());
        assert!(svc.paused);
    }

    #[test]
    fn test_service_final_generation_failure_still_pauses() {
        test_final_generation_failure_still_pauses();
    }

    #[tokio::test]
    async fn test_media_is_fetched_from_the_current_frame() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.turn_quiet_seconds = 10;
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("好的"));

        let mut pdf = test_bubble("报价.pdf\n12KB", "inbound_visual");
        pdf.kind = Some("file".to_string());
        pdf.digest = "file:报价.pdf:12KB".to_string();
        pdf.media_id = Some("1".repeat(64));
        pdf.filename = Some("报价.pdf".to_string());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let first = test_frame(
            vec![test_bubble("old", "inbound_visual"), pdf.clone()],
            "午夜咖啡",
        );
        let later = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                pdf.clone(),
                test_bubble("帮我看看", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        let mut pdf_now = pdf.clone();
        pdf_now.media_id = Some("2".repeat(64));
        let mut now = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                pdf_now,
                test_bubble("帮我看看", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        now.snapshot_id = Some("s-now".to_string());

        let mut sent_bubbles = now.bubbles.clone();
        sent_bubbles.push(test_bubble("好的", "outbound_visual"));
        let sent = test_frame(sent_bubbles, "午夜咖啡");

        let now_s = now.clone();
        // at t=0
        svc.clock = Arc::new(|| 0.0);
        let client = MockToolClient::new_default();
        let mut cap1 = MockCapture::new(vec![first.clone(), first]);
        svc.cycle(&client, &mut cap1).await.unwrap();

        // at t=3
        svc.clock = Arc::new(|| 3.0);
        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        // at t=14
        svc.clock = Arc::new(|| 14.0);
        let seen_items = Arc::new(Mutex::new(Vec::new()));
        let seen_snapshot = Arc::new(Mutex::new(None));
        let si = seen_items.clone();
        let ss = seen_snapshot.clone();

        let mut cap3 = MockCapture::new(vec![later.clone(), now_s.clone(), now_s, sent]);
        cap3.attachments_fn = Some(Arc::new(move |items, snap| {
            *si.lock().unwrap() = items.to_vec();
            *ss.lock().unwrap() = snap.snapshot_id.clone();
            Ok((Vec::new(), Vec::new(), Vec::new()))
        }));

        svc.cycle(&client, &mut cap3).await.unwrap();

        let items = seen_items.lock().unwrap();
        let file_mids: Vec<_> = items
            .iter()
            .filter(|i| i.kind.as_deref() == Some("file"))
            .map(|i| i.media_id.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(file_mids, vec!["2".repeat(64).as_str()]);
        assert_eq!(seen_snapshot.lock().unwrap().as_deref(), Some("s-now"));
    }

    #[test]
    fn test_service_media_is_fetched_from_the_current_frame() {
        test_media_is_fetched_from_the_current_frame();
    }

    #[tokio::test]
    async fn test_model_is_told_this_turns_image_count_separately() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let mut imgs = Vec::new();
        let mut items = Vec::new();
        for n in 1..=3 {
            let mut b = test_bubble("[图片]", "inbound_visual");
            b.kind = Some("image".to_string());
            b.digest = "image:50:40".to_string();
            let p = td.path().join(format!("{n}.png"));
            fs::write(&p, format!("png-{n}")).unwrap();
            items.push(AttachmentImage {
                path: p,
                label: format!("第{n}张图片"),
                source: "original".to_string(),
                bubble: b.clone(),
                media_id: None,
            });
            imgs.push(b);
        }

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut current_bubbles = vec![test_bubble("old", "inbound_visual")];
        current_bubbles.extend(imgs);
        let current = test_frame(current_bubbles.clone(), "午夜咖啡");

        let mut sent_bubbles = current_bubbles;
        sent_bubbles.push(test_bubble("好的", "outbound_visual"));
        let sent = test_frame(sent_bubbles, "午夜咖啡");

        let turn_img_count = Arc::new(AtomicUsize::new(0));
        let turn_earlier_count = Arc::new(AtomicUsize::new(0));
        let tc = turn_img_count.clone();
        let te = turn_earlier_count.clone();

        svc.brain = Some(Arc::new(move |turn| {
            tc.store(turn.image_count.unwrap_or(0), Ordering::SeqCst);
            te.store(turn.earlier_images, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((items.clone(), Vec::new(), Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(turn_img_count.load(Ordering::SeqCst), 3);
        assert_eq!(turn_earlier_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_service_model_is_told_this_turns_image_count_separately() {
        test_model_is_told_this_turns_image_count_separately();
    }

    #[tokio::test]
    async fn test_earlier_context_images_follow_this_turns_and_are_counted() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let this_path = td.path().join("this.png");
        let old_path = td.path().join("old.png");
        fs::write(&this_path, b"xxxxxxxxxx").unwrap();
        fs::write(&old_path, b"yyyyyyyyyy").unwrap();

        let img = AttachmentImage {
            path: this_path.clone(),
            label: "第1张图片".to_string(),
            source: "original".to_string(),
            bubble: test_bubble("[图片]", "inbound_visual"),
            media_id: None,
        };

        let old_context = ContextItem {
            role: "customer".to_string(),
            text: "[图片]".to_string(),
            media: Some(crate::brain::ContextMedia {
                kind: Some("image".to_string()),
                path: Some(old_path.clone()),
                understanding: None,
                filename: None,
            }),
        };

        let mut failed = Vec::new();
        let (_notes, model_images, earlier) = svc
            .understand_images("[图片]", &[img], &mut failed, &[old_context])
            .await
            .unwrap();

        assert_eq!(model_images, vec![this_path, old_path]);
        assert_eq!(earlier, 1);
    }

    #[test]
    fn test_service_earlier_context_images_follow_this_turns_and_are_counted() {
        test_earlier_context_images_follow_this_turns_and_are_counted();
    }

    #[tokio::test]
    async fn test_images_over_the_request_size_are_named_not_paused() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.max_model_image_bytes = Some(25); // 限制 25 字节

        let mut imgs = Vec::new();
        let mut items = Vec::new();
        for n in 1..=3 {
            let mut b = test_bubble("[图片]", "inbound_visual");
            b.kind = Some("image".to_string());
            b.digest = "image:50:40".to_string();
            let p = td.path().join(format!("{n}.png"));
            // 写入 11 字节：前两张共 22 字节 <= 25，第三张加上后 33 字节 > 25
            fs::write(&p, format!("PNG_HDR_{n:03}")).unwrap();
            items.push(AttachmentImage {
                path: p,
                label: format!("第{n}张图片"),
                source: "original".to_string(),
                bubble: b.clone(),
                media_id: None,
            });
            imgs.push(b);
        }

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut current_bubbles = vec![test_bubble("old", "inbound_visual")];
        current_bubbles.extend(imgs);
        let current = test_frame(current_bubbles.clone(), "午夜咖啡");

        let mut sent_bubbles = current_bubbles;
        sent_bubbles.push(test_bubble("好的", "outbound_visual"));
        let sent = test_frame(sent_bubbles, "午夜咖啡");

        let model_imgs = Arc::new(Mutex::new(Vec::new()));
        let model_failed = Arc::new(Mutex::new(Vec::new()));
        let mi = model_imgs.clone();
        let mf = model_failed.clone();

        svc.brain = Some(Arc::new(move |turn| {
            *mi.lock().unwrap() = turn.images.clone();
            *mf.lock().unwrap() = turn.failed.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((items.clone(), Vec::new(), Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(model_imgs.lock().unwrap().len(), 2);
        let failed = model_failed.lock().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].label, "第3张图片");
        assert_eq!(failed[0].kind.as_deref(), Some("image"));
        assert_eq!(failed[0].reason.as_deref(), Some("too_large"));
    }

    #[test]
    fn test_service_images_over_the_request_size_are_named_not_paused() {
        test_images_over_the_request_size_are_named_not_paused();
    }

    #[tokio::test]
    async fn test_attempting_journal_failure_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f]);

        let state_path = td.path().to_path_buf();
        svc.brain = Some(Arc::new(move |_| {
            // 在模型生成后、写入 attempting journal 之前将状态目录设为只读，导致 write_journal 失败
            let _ = fs::set_permissions(&state_path, fs::Permissions::from_mode(0o500));
            Box::pin(async {
                Ok(BrainReply {
                    reply: "回复内容".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let res = svc.cycle(&client, &mut cap).await;

        // 恢复目录权限供清理
        let _ = fs::set_permissions(td.path(), fs::Permissions::from_mode(0o700));

        // write_journal 失败必须向上传播错误并阻断发送，且 client 绝不能调用 send_message
        assert!(res.is_err(), "write_journal 失败必须向上传播错误并阻断发送");
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(
            !sent,
            "attempting journal 写入失败时绝对不能调用 send_message"
        );
    }

    #[tokio::test]
    async fn test_history_record_bot_reply_failure_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f]);

        let hist_clone = history.clone();
        svc.brain = Some(Arc::new(move |_| {
            // 在模型生成成功后、写入 record_bot_reply 之前将历史数据库设为只读
            hist_clone
                .connection()
                .lock()
                .unwrap()
                .execute("PRAGMA query_only = ON", [])
                .unwrap();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "回复内容".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_err(), "record_bot_reply 失败必须阻断发送并返回错误");
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(!sent, "record_bot_reply 失败时绝对不能调用 send_message");
    }

    #[tokio::test]
    async fn test_history_replied_after_failure_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let f = test_frame(vec![test_bubble("你好", "inbound_visual")], "午夜咖啡");
        // 损坏数据库破坏 replied_after 查询
        let conn_arc = history.connection();
        conn_arc
            .lock()
            .unwrap()
            .execute("DROP TABLE messages", [])
            .unwrap();

        let res = svc.replied_meanwhile(&f, &f, &["msg1".to_string()], Some("key1"));
        assert!(
            res.is_err(),
            "replied_after 数据库查询失败必须向上传播 Err 而不能吞掉当作 false"
        );
    }

    #[tokio::test]
    async fn test_brain_files_strips_path_and_bubble() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut file_bubble = test_bubble("[文件] 财务报表.xlsx", "inbound_visual");
        file_bubble.kind = Some("file".to_string());

        let current = test_frame(
            vec![test_bubble("old", "inbound_visual"), file_bubble.clone()],
            "午夜咖啡",
        );
        let sent = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                file_bubble.clone(),
                test_bubble("已收到", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let secret_file_path = td.path().join("confidential_local_file.xlsx");
        fs::write(&secret_file_path, "confidential bytes").unwrap();

        let mut content = serde_json::Map::new();
        content.insert("text".to_string(), json!("表格第1行数据"));
        content.insert("truncated".to_string(), json!(false));

        let attachment_file = AttachmentFile {
            filename: "财务报表.xlsx".to_string(),
            label: "第1个文件".to_string(),
            original_bytes: false,
            path: secret_file_path,
            bubble: file_bubble,
            content,
        };

        let captured_files = Arc::new(Mutex::new(Vec::new()));
        let cf = captured_files.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *cf.lock().unwrap() = turn.files.clone();
            Box::pin(async {
                Ok(BrainReply {
                    reply: "已收到".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        let af = attachment_file.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((Vec::new(), vec![af.clone()], Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let files = captured_files.lock().unwrap();
        assert_eq!(files.len(), 1, "必须向模型传递附件文件视图");
        let file_obj = files[0].as_object().expect("file view 必须是 JSON 对象");
        assert_eq!(
            file_obj.get("filename").and_then(|v| v.as_str()),
            Some("财务报表.xlsx")
        );
        assert_eq!(
            file_obj.get("text").and_then(|v| v.as_str()),
            Some("表格第1行数据")
        );
        assert!(
            !file_obj.contains_key("path"),
            "模型文件视图中绝对不能包含本机文件路径 path"
        );
        assert!(
            !file_obj.contains_key("bubble"),
            "模型文件视图中绝对不能包含屏幕气泡坐标与视觉几何 bubble"
        );
    }

    #[tokio::test]
    async fn turns_populates_media_refs() {
        let td = TestDir::new();
        let mut cfg = fixture_config(td.path());
        cfg.images_per_model_call = 0;
        cfg.model_config.backend = "openai-compatible".to_string();

        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut img_bubble = test_bubble("[图片]", "inbound_visual");
        img_bubble.kind = Some("image".to_string());
        img_bubble.media_id = Some("img_media_id_111".to_string());

        let mut file_bubble = test_bubble("[文件] 财务报表.xlsx", "inbound_visual");
        file_bubble.kind = Some("file".to_string());
        file_bubble.media_id = Some("file_media_id_222".to_string());

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                img_bubble.clone(),
                file_bubble.clone(),
            ],
            "午夜咖啡",
        );
        let sent = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                img_bubble.clone(),
                file_bubble.clone(),
                test_bubble("好的", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let img_path = td.path().join("test_img.png");
        fs::write(&img_path, "dummy image bytes").unwrap();
        let file_path = td.path().join("财务报表.xlsx");
        fs::write(&file_path, "dummy file bytes").unwrap();

        let attachment_image = AttachmentImage {
            path: img_path,
            label: "第1张图片".to_string(),
            source: "inbound_visual".to_string(),
            bubble: img_bubble,
            media_id: Some("img_media_id_111".to_string()),
        };

        let attachment_file = AttachmentFile {
            filename: "财务报表.xlsx".to_string(),
            label: "第1个文件".to_string(),
            original_bytes: false,
            path: file_path,
            bubble: file_bubble,
            content: serde_json::Map::new(),
        };

        svc.understand_fn = Some(Arc::new(move |_images, _, _, _| {
            Box::pin(async move { vec![Some("这是图片理解文字".to_string())] })
        }));

        let captured_turn = Arc::new(Mutex::new(None));
        let ct = captured_turn.clone();
        svc.brain = Some(Arc::new(move |turn| {
            *ct.lock().unwrap() = Some(turn.clone());
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        let ai = attachment_image.clone();
        let af = attachment_file.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((vec![ai.clone()], vec![af.clone()], Vec::new()))
        }));

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let turn = captured_turn
            .lock()
            .unwrap()
            .take()
            .expect("Turn 必须被交付给大脑");
        assert_eq!(turn.media_refs.len(), 2, "media_refs 必须包含图片和文件");

        let img_ref = &turn.media_refs[0];
        assert_eq!(img_ref.kind, "image");
        assert_eq!(
            img_ref.media_id,
            Some("77f1d16d5bb6f93c7366a352f518efc7dd1a492da3c4882959b266ac03560210".to_string())
        );
        assert_eq!(img_ref.understanding, Some("这是图片理解文字".to_string()));
        assert_eq!(img_ref.filename, None);

        let file_ref = &turn.media_refs[1];
        assert_eq!(file_ref.kind, "file");
        assert_eq!(file_ref.media_id, Some("file_media_id_222".to_string()));
        assert_eq!(file_ref.filename, Some("财务报表.xlsx".to_string()));
        assert_eq!(file_ref.understanding, None);
    }

    #[tokio::test]
    async fn test_model_provenance_persisted_to_journal_and_history() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc =
            BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let sent = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("好的，收到", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let mut brain_metadata = HashMap::new();
        brain_metadata.insert("backend".to_string(), json!("codex-exec"));
        brain_metadata.insert("model".to_string(), json!("gpt-4o"));
        brain_metadata.insert("provider".to_string(), json!("openai"));
        brain_metadata.insert("seconds".to_string(), json!(1.45));
        brain_metadata.insert("action".to_string(), json!("send"));

        svc.brain = Some(Arc::new(move |_| {
            let meta = brain_metadata.clone();
            Box::pin(async move {
                Ok(BrainReply {
                    reply: "好的，收到".to_string(),
                    metadata: meta,
                })
            })
        }));

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        // 检查 journal 中的 model provenance
        let mut found_journal = false;
        for entry in fs::read_dir(td.path()).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("send-") && s.ends_with(".json"))
                .unwrap_or(false)
            {
                let data: Value = read_json(&path).unwrap();
                let model = data.get("model").expect("journal 中必须记录 model 字段");
                assert_eq!(
                    model.get("backend").and_then(|v| v.as_str()),
                    Some("codex-exec")
                );
                assert_eq!(model.get("model").and_then(|v| v.as_str()), Some("gpt-4o"));
                assert_eq!(
                    model.get("provider").and_then(|v| v.as_str()),
                    Some("openai")
                );
                assert_eq!(model.get("seconds").and_then(|v| v.as_f64()), Some(1.45));
                assert!(
                    !model.as_object().unwrap().contains_key("action"),
                    "model provenance 中不应包含控制字段 action"
                );
                assert_eq!(
                    data.get("timings")
                        .and_then(|t| t.get("model_seconds"))
                        .and_then(|v| v.as_f64()),
                    Some(1.45)
                );
                found_journal = true;
                break;
            }
        }
        assert!(found_journal, "必须生成 send-*.json journal");

        // 检查 SQLite history 库中的 record_bot_reply (sends 表)
        let conn_arc = history.connection();
        let conn = conn_arc.lock().unwrap();
        let saved_model_json: String = conn
            .query_row("SELECT model FROM sends", [], |r| r.get(0))
            .unwrap();
        let saved_model: Value = serde_json::from_str(&saved_model_json).unwrap();
        assert_eq!(
            saved_model.get("backend").and_then(|v| v.as_str()),
            Some("codex-exec")
        );
        assert_eq!(
            saved_model.get("model").and_then(|v| v.as_str()),
            Some("gpt-4o")
        );
        assert_eq!(
            saved_model.get("provider").and_then(|v| v.as_str()),
            Some("openai")
        );
        assert_eq!(
            saved_model.get("seconds").and_then(|v| v.as_f64()),
            Some(1.45)
        );
    }

    #[tokio::test]
    async fn test_echo_frame_title_mismatch_never_marked_seen_or_cursor_advanced() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );

        // 构造一个标题不匹配的恶意/错误回显帧：它虽然包含匹配的出站文本 "好的"，但来自其他群聊 "闲聊群"
        let bad_title_frame = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("好的", "outbound_visual"),
            ],
            "闲聊群", // 标题不匹配 policy！
        );

        svc.brain = Some(Arc::new(|_| {
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|tool, _| {
                if tool == "send_message" {
                    // 模拟网关返回错误或非 triggered，使 turns 进入 echo_check 路径
                    anyhow::bail!("send_message network timeout");
                }
                if tool == "get_send_status" {
                    Ok(json!({"status": "unknown"}))
                } else {
                    Ok(json!({}))
                }
            }),
        };

        // capture 在发送后连续 3 次返回 bad_title_frame
        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            bad_title_frame.clone(),
            bad_title_frame.clone(),
            bad_title_frame.clone(),
        ]);

        let _ = svc.cycle(&client, &mut cap).await;
        // 绝不能标记为 triggered/Seen，游标绝不能推进到包含坏帧
        assert_ne!(
            svc.cursor.as_ref().unwrap().title_lines,
            vec!["闲聊群".to_string()]
        );
        // 检查 journal 中的 status 必须为 unknown 而非 triggered
        for entry in fs::read_dir(td.path()).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("send-") && s.ends_with(".json"))
                .unwrap_or(false)
            {
                let data: Value = read_json(&path).unwrap();
                assert_eq!(data.get("status").and_then(|v| v.as_str()), Some("unknown"));
                assert_ne!(
                    data.get("client_echo_observed").and_then(|v| v.as_bool()),
                    Some(true)
                );
            }
        }
    }

    #[tokio::test]
    async fn test_sigterm_arriving_during_model_generation_aborts_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current]);

        let stop_flag = svc.stop.clone();
        svc.brain = Some(Arc::new(move |_| {
            // 模拟模型生成期间外部传入停止信号（SIGTERM/SIGINT），实时修改了共享 stop 标志
            stop_flag.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "回复内容".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_ok(), "收到停止信号应安全退出并保留 turn");

        // 验证 client 绝对没有发送消息
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(!sent, "停止信号到达后绝对不能发送消息");

        // 验证 journal 记录为 stopped_before_send
        let mut found_journal = false;
        for entry in fs::read_dir(td.path()).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("send-") && s.ends_with(".json"))
                .unwrap_or(false)
            {
                let data: Value = read_json(&path).unwrap();
                assert_eq!(
                    data.get("status").and_then(|v| v.as_str()),
                    Some("stopped_before_send")
                );
                found_journal = true;
                break;
            }
        }
        assert!(found_journal, "必须生成 stopped_before_send journal");

        // 验证 pending turn 被 keep_turn 保留在磁盘 turn.json 中
        let turn_path = td.path().join("turn.json");
        assert!(
            turn_path.exists(),
            "pending turn 必须保留在磁盘以便下次恢复"
        );
    }

    #[tokio::test]
    async fn test_delayed_echo_polling_rejects_mismatched_title_frame() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let after_no_echo = current.clone();

        // 构造一个包含 echo 但标题不匹配的坏帧
        let bad_title_echo_frame = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("好的", "outbound_visual"),
            ],
            "其他联系人", // 标题不匹配
        );

        svc.brain = Some(Arc::new(|_| {
            Box::pin(async {
                Ok(BrainReply {
                    reply: "好的".to_string(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let client = MockToolClient::new_default();
        // 前4帧为正常接收、确认、发送前核对及初次回显检查帧，后续轮询帧为标题不匹配的坏帧
        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            after_no_echo.clone(),
            bad_title_echo_frame.clone(),
            bad_title_echo_frame.clone(),
            bad_title_echo_frame.clone(),
        ]);

        let res = svc.cycle(&client, &mut cap).await;

        assert!(
            res.is_ok(),
            "延迟轮询中即使外部窗口标题不符，也应忽略坏帧继续完成本轮"
        );

        // 检查 journal 中的 client_echo_observed 绝不能为 true（因为回显帧标题不符，被校验拒绝）
        for entry in fs::read_dir(td.path()).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with("send-") && s.ends_with(".json"))
                .unwrap_or(false)
            {
                let data: Value = read_json(&path).unwrap();
                assert_ne!(
                    data.get("client_echo_observed").and_then(|v| v.as_bool()),
                    Some(true),
                    "标题不符的回显帧绝不能被认定为 client_echo_observed"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_external_mode_hot_added_session_uses_external_brain_on_first_sweep() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());

        let external_called = Arc::new(AtomicBool::new(false));
        let ec = external_called.clone();
        let external_brain: BrainFn = Arc::new(move |_turn| {
            ec.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "来自外部大脑的回答".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });

        // 热加会话由协调器注入 default_brain（见 scheduler 测试 test_hot_added_session_inherits_coordinator_default_brain）
        let mut new_child = BotService::new(cfg, None, None, None, None, None).unwrap();
        new_child.sleep = Arc::new(|_| Box::pin(async {}));
        new_child.brain = Some(external_brain);

        // 模拟该会话从磁盘恢复了 pending turn，在首个 sweep 立即触发回复
        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        new_child.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let sent = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("来自外部大脑的回答", "outbound_visual"),
            ],
            "午夜咖啡",
        );

        let mut cap = MockCapture::new(vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            sent,
        ]);
        let client = MockToolClient::new_default();

        let res = new_child.cycle(&client, &mut cap).await;
        assert!(res.is_ok());

        assert!(
            external_called.load(Ordering::SeqCst),
            "首个 sweep 中热加会话必须调用外部大脑，绝不能越过外部大脑退化到 builtin"
        );
    }

    #[tokio::test]
    async fn test_brain_snapshot_provenance_and_failure_injection() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let state_dir = cfg.state_dir.clone();
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let history = Arc::new(HistoryStore::open(&state_dir).unwrap());
        history.migrate().unwrap();

        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        // 1. 初始化 Store 并激活一个角色
        let store = crate::core::brain_profiles::Store::open(&state_dir).unwrap();
        let ctx = crate::core::brain_profiles::AccountContext::new("fixture".into()).unwrap();
        let mut r = crate::core::brain_profiles::legacy_seed(&ctx, true, 1).unwrap();
        r.role_id = "role_turn".into();
        r.revision_id = "rev_t1".into();
        r.persona = "运行测试角色".into();
        let sealed = r.seal().unwrap();

        let access = crate::core::control::ControlContactAccess {
            account_id: "fixture".into(),
            brain_access: None,
            conn: None,
        };

        let write_req = crate::core::brain_profiles::WriteRequest {
            expected_generation: 0,
            request_id: uuid::Uuid::new_v4().to_string(),
            mutation: crate::core::brain_profiles::Mutation::SaveRole(sealed),
        };
        store.write(&ctx, &write_req, &access, 100).unwrap();

        let bindings = crate::core::brain_profiles::BindingSet {
            default: Some(crate::core::brain_profiles::RoleRef {
                role_id: "role_turn".into(),
                revision_id: "rev_t1".into(),
            }),
            overrides: vec![],
        };
        let write_bind = crate::core::brain_profiles::WriteRequest {
            expected_generation: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            mutation: crate::core::brain_profiles::Mutation::SaveBindings(bindings),
        };
        store.write(&ctx, &write_bind, &access, 101).unwrap();

        let write_act = crate::core::brain_profiles::WriteRequest {
            expected_generation: 2,
            request_id: uuid::Uuid::new_v4().to_string(),
            mutation: crate::core::brain_profiles::Mutation::Activate,
        };
        store.write(&ctx, &write_act, &access, 102).unwrap();

        let mut svc = BotService::new(
            cfg,
            None,
            Some(history.clone()),
            Some(cid.clone()),
            None,
            None,
        )
        .unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_turn| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "收到测试消息".into(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f]);

        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_ok());
        assert!(brain_called.load(Ordering::SeqCst));

        // 验证持久化到了 brain_turns 表中
        let turns: i64 = history
            .connection()
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM brain_turns WHERE account_id='fixture'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(turns, 1);

        // 验证失败注入：当快照持久化失败时，不调用模型也不发送
        history
            .connection()
            .lock()
            .unwrap()
            .execute("DROP TABLE brain_turns", [])
            .unwrap();
        let brain_called_2 = Arc::new(AtomicBool::new(false));
        let bc2 = brain_called_2.clone();
        svc.brain = Some(Arc::new(move |_turn| {
            bc2.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "这条绝不能生成".into(),
                    metadata: HashMap::new(),
                })
            })
        }));
        let new_f2 = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("你好", "inbound_visual"),
                test_bubble("故障注入测试", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap2 = MockCapture::new(vec![new_f2.clone(), new_f2.clone(), new_f2]);
        let client2 = MockToolClient::new_default();
        let res2 = svc.cycle(&client2, &mut cap2).await;
        assert!(res2.is_err(), "快照写入失败时必须阻断并报错");
        assert!(
            !brain_called_2.load(Ordering::SeqCst),
            "快照写入失败时绝对不能调用模型"
        );
        let calls = client2.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(!sent, "快照写入失败时绝对不能调用发送");
    }

    #[tokio::test]
    async fn test_fresh_managed_mode_no_role_records_role_required() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let state_dir = cfg.state_dir.clone();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let store = crate::core::brain_profiles::Store::open(&state_dir).unwrap();
        let ctx = crate::core::brain_profiles::AccountContext::new("fixture".into()).unwrap();
        // Fresh 初始化：处于 Managed 模式但无配置/激活角色
        store
            .initialize(
                &ctx,
                crate::core::brain_profiles::Installation::Fresh,
                b"{}",
                true,
                0,
                None,
            )
            .unwrap();

        let mut svc = BotService::new(cfg, None, None, Some("c1".into()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_turn| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "这条绝不能生成".into(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("客户新发的一条消息", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current]);
        let client = MockToolClient::new_default();
        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_ok(), "cycle 本身安全返回 Ok");
        assert!(
            !brain_called.load(Ordering::SeqCst),
            "Managed 模式无角色绝对不调用模型"
        );
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(!sent, "Managed 模式无角色绝对不调用发送");

        // 验证 journal 记录了 role_required
        let journal_file = std::fs::read_dir(&svc.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .map(|e| e.path())
            .expect("journal file exists");
        let content = std::fs::read_to_string(journal_file).unwrap();
        let journal: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(journal["status"], "role_required");
    }

    #[tokio::test]
    async fn test_broken_root_blocks_and_records_integrity_mismatch() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let state_dir = cfg.state_dir.clone();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        // 写入损坏的根文件（不支持的 schema 版本或非法的 JSON 内容）
        let bindings_path = state_dir.join("brain-bindings.json");
        std::fs::write(&bindings_path, b"{\"schema_version\": 999}").unwrap();

        let mut svc = BotService::new(cfg, None, None, Some("c1".into()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_turn| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "损坏根绝不能调用模型".into(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("客户新消息", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current]);
        let client = MockToolClient::new_default();
        let res = svc.cycle(&client, &mut cap).await;
        assert!(res.is_err(), "损坏根必须 fail-closed 阻断并报错");
        assert!(
            !brain_called.load(Ordering::SeqCst),
            "损坏根绝不调用模型，绝不降级为 Legacy"
        );
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(!sent, "损坏根绝不调用发送");

        // 验证 journal 记录了 integrity_mismatch
        let journal_file = std::fs::read_dir(&svc.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .map(|e| e.path())
            .expect("journal file exists");
        let content = std::fs::read_to_string(journal_file).unwrap();
        let journal: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(journal["status"], "integrity_mismatch");
    }

    #[tokio::test]
    async fn test_legacy_mode_no_role_preserves_old_pipeline() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let state_dir = cfg.state_dir.clone();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        // 未初始化（没有 brain-bindings.json），Legacy 模式

        let mut svc = BotService::new(cfg, None, None, Some("c1".into()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let brain_called = Arc::new(AtomicBool::new(false));
        let bc = brain_called.clone();
        svc.brain = Some(Arc::new(move |_turn| {
            bc.store(true, Ordering::SeqCst);
            Box::pin(async {
                Ok(BrainReply {
                    reply: "Legacy 正常回复".into(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("客户新消息", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current]);
        let client = MockToolClient::new_default();
        let res = svc.cycle(&client, &mut cap).await;

        assert!(res.is_ok(), "Legacy 模式正常执行");
        assert!(
            brain_called.load(Ordering::SeqCst),
            "Legacy 模式保留原有链路调用模型"
        );
        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(sent, "Legacy 模式正常调用发送");
    }

    #[tokio::test]
    async fn test_turn_model_retry_pins_initial_snapshot_version() {
        use crate::core::brain_profiles::{
            AccountContext, BindingSet, Installation, Mutation, Store, WriteRequest,
        };

        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let state_dir = cfg.state_dir.clone();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let history_dir = state_dir.join("history");
        std::fs::create_dir_all(&history_dir).unwrap();
        std::fs::set_permissions(&history_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _history_store = crate::core::history::store::HistoryStore::open(&state_dir).unwrap();
        let db_path = history_dir.join("history.sqlite3");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            -- recipient 姓名 "午夜咖啡" 与 contact_id "c1" 严格不同
            INSERT INTO contacts VALUES('c1', 'fixture', 'private', '午夜咖啡真实名', NULL, 100, 100);
            INSERT INTO contact_names VALUES(1, 'c1', 'fixture', '午夜咖啡', '午夜咖啡', 'config', 100, 100);
            "#,
        )
        .unwrap();

        let brain_access = crate::core::open_api::access::BrainAccess::load(&cfg, &conn).unwrap();
        let access = crate::core::control::ControlContactAccess {
            account_id: "fixture".into(),
            brain_access: Some(brain_access),
            conn: Some(conn),
        };

        let ctx = AccountContext::new("fixture".into()).unwrap();
        let store = Store::open(&state_dir).unwrap();

        // 1. 初始化并激活 legacy 角色 v1
        let cfg_bytes = serde_json::to_vec(&cfg).unwrap();
        store
            .migrate(&ctx, Installation::LegacyUpgrade, &cfg_bytes, true, 0)
            .unwrap();

        let r1 = store
            .resolve(
                &ctx,
                "c1",
                &crate::core::brain_profiles::ChatKind::Private,
                &access,
            )
            .unwrap();
        let r1_rev = r1.revision_id.clone();

        let mut svc =
            BotService::new(cfg.clone(), None, None, Some("c1".into()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.model_retries = Some(2);

        let captured_snapshots = Arc::new(Mutex::new(Vec::new()));
        let cs = captured_snapshots.clone();
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        svc.brain = Some(Arc::new(move |turn| {
            let attempt = cc.fetch_add(1, Ordering::SeqCst);
            if let Some(snap) = &turn.snapshot {
                cs.lock().unwrap().push(snap.clone());
            }
            Box::pin(async move {
                if attempt == 0 {
                    bail!("simulated temporary model timeout");
                }
                Ok(BrainReply {
                    reply: "重试成功回复".into(),
                    metadata: HashMap::new(),
                })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(
            vec![
                test_bubble("old", "inbound_visual"),
                test_bubble("新消息咨询", "inbound_visual"),
            ],
            "午夜咖啡",
        );
        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current.clone()]);
        let client = MockToolClient::new_default();

        // 第一次 cycle：由于模型失败，应当进入 model_retry_wait 并保存 fixed_snapshot
        let res1 = svc.cycle(&client, &mut cap).await;
        assert!(res1.is_ok(), "模型重试返回 Ok 并处于等待重试状态");
        assert_eq!(call_count.load(Ordering::SeqCst), 1, "已尝试第一次调用");

        let turn_data = svc.turn.as_ref().expect("轮次依然保留在 pending 中");
        assert_eq!(turn_data.model_failures, 1, "已记录 1 次模型失败");
        let initial_snapshot = turn_data
            .fixed_snapshot
            .as_ref()
            .expect("必须锁定初始快照 fixed_snapshot")
            .clone();
        assert_eq!(
            initial_snapshot["role"]["revision_id"], r1_rev,
            "初始锁定的快照必须是 role-v1 的 revision"
        );

        // 2. 关键破坏测试：在第一次调用失败后，管理员更新并激活全新角色 role-v2！
        let mut r2 = r1.clone();
        r2.parent_revision_id = Some(r1.revision_id.clone());
        r2.revision_id = "role_rev_v2_new".into();
        r2.persona = "我是客服V2，完全不同的新性格".into();
        let r2 = r2.seal().unwrap();
        let generation = store.generation().unwrap();
        store
            .write(
                &ctx,
                &WriteRequest {
                    expected_generation: generation,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    mutation: Mutation::SaveRole(r2.clone()),
                },
                &access,
                0,
            )
            .unwrap();
        let b2 = BindingSet {
            default: Some(r2.reference()),
            overrides: vec![],
        };
        store
            .write(
                &ctx,
                &WriteRequest {
                    expected_generation: generation + 1,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    mutation: Mutation::SaveBindings(b2),
                },
                &access,
                0,
            )
            .unwrap();
        store
            .write(
                &ctx,
                &WriteRequest {
                    expected_generation: generation + 2,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    mutation: Mutation::Activate,
                },
                &access,
                0,
            )
            .unwrap();

        let active_now = store
            .resolve(
                &ctx,
                "c1",
                &crate::core::brain_profiles::ChatKind::Private,
                &access,
            )
            .unwrap();
        assert_eq!(
            active_now.revision_id, "role_rev_v2_new",
            "Store 中当前最新 active 角色已变为 v2"
        );

        // 3. 推进时钟至重试时间到达
        if let Some(t) = svc.turn.as_mut() {
            t.retry_at = 0.0;
        }

        // 第二次 cycle：重试触发
        let mut cap2 = MockCapture::new(vec![current.clone(), current.clone(), current]);
        let res2 = svc.cycle(&client, &mut cap2).await;
        assert!(res2.is_ok(), "重试 cycle 执行成功");
        assert_eq!(call_count.load(Ordering::SeqCst), 2, "模型被二次调用");

        // 4. 核心断言：第二次调用发给模型的快照必须与第一次相同（锁定 v1），绝不能变成新激活的 v2！
        let snaps = captured_snapshots.lock().unwrap();
        assert_eq!(snaps.len(), 2, "总共捕获到两次模型调用的快照");
        assert_eq!(
            snaps[0]["role"]["revision_id"], r1_rev,
            "第一次调用为版本 1"
        );
        assert_eq!(
            snaps[1]["role"]["revision_id"], r1_rev,
            "第二次重试调用必须固定为初始版本 1，严禁混用新激活的 revision 2"
        );
        assert_eq!(
            snaps[0]["snapshot_id"], snaps[1]["snapshot_id"],
            "两次调用的 snapshot_id 必须完全一致"
        );

        let calls = client.calls.lock().unwrap();
        let sent = calls.iter().any(|(tool, _)| tool == "send_message");
        assert!(sent, "重试成功后正常调用 send_message");
    }
}
