
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
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use crate::brain::{Brain, BrainReply, ContactInfo, ContextItem, FailedAttachment, Turn as BrainTurn};
use crate::core::config::{candidate_names, clean_title, BotConfig};
use crate::core::delivery::{is_not_sent, EchoOutcome, ToolClient};
use crate::core::history::store::{similar, HistoryStore};
use crate::core::model::{Bubble, Frame};
use crate::core::policy::{
    echo_matches, fingerprint, merge_pages, new_bubbles, reconcile, relocate, strip_member_count,
    validate_reply, ContinuityError, Policy,
};
use crate::core::state::{read_json, write_private};
use crate::core::worker_client::action_interrupted;

pub const MODEL_RETRIES: usize = 2;
pub const MODEL_RETRY_SECONDS: f64 = 30.0;
pub const UPLOAD_EXTRA_SECONDS: f64 = 120.0;
pub const MAX_MODEL_IMAGE_BYTES: u64 = 30 * 1024 * 1024;
pub const GAP_SCROLL_PAGES: u32 = 5;

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub type BrainFn = Arc<dyn Fn(&BrainTurn) -> Pin<Box<dyn Future<Output = Result<BrainReply>> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentImage {
    pub path: PathBuf,
    pub label: String,
    pub source: String,
    pub bubble: Bubble,
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
    fn scroll(&mut self, _direction: &str, _pages: u32) -> impl Future<Output = Result<Frame>> + Send {
        async move { bail!("scroll not supported") }
    }
    fn download(&mut self, _media_id: &str) -> impl Future<Output = Result<PathBuf>> + Send {
        async move { bail!("download not supported") }
    }
    fn attachments(
        &mut self,
        _items: &[Bubble],
        _frame: &Frame,
    ) -> impl Future<Output = Result<(Vec<AttachmentImage>, Vec<AttachmentFile>, Vec<FailedAttachmentItem>)>> + Send {
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
}

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
        ).map_err(|e| anyhow::anyhow!("{e:?}"))?;

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
                bail!("state belongs to a different account or contact; use a separate state directory");
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
            policy.check_frame(c).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }

        // 启动时处理遗留的日志文件
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if file_name.starts_with("send-") && file_name.ends_with(".json") {
                        if let Ok(mut data) = read_json::<Value>(&path) {
                            let status = data.get("status").and_then(|v| v.as_str());
                            if matches!(status, Some("preparing_media" | "generating" | "generated")) {
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
        images: &[AttachmentImage],
        files: &[AttachmentFile],
    ) {
        let Some(history) = &self.history else {
            return;
        };
        if input_ids.len() != incoming.len() {
            return;
        }
        for item in images {
            let mid = incoming.iter().zip(input_ids.iter()).find(|(b, _)| **b == item.bubble).map(|(_, m)| m);
            if let Some(message_id) = mid {
                let _ = history.attach_media(message_id, &item.path, "image", None, &item.source, None);
            }
        }
        for item in files {
            let mid = incoming.iter().zip(input_ids.iter()).find(|(b, _)| **b == item.bubble).map(|(_, m)| m);
            if let Some(message_id) = mid {
                let _ = history.attach_media(message_id, &item.path, "file", Some(&item.filename), "original", None);
                let _ = history.finish_media_job(message_id, "done", None);
            }
        }
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
        let recent: Vec<f64> = attempts
            .into_iter()
            .filter(|&t| now - t < 3600.0)
            .collect();
        let max_recent = recent.iter().copied().fold(0.0f64, f64::max);
        let count_limited = recent.len() >= self.config.max_replies_per_hour as usize;
        let cooldown_limited = !recent.is_empty() && (now - max_recent < self.config.cooldown_seconds as f64);
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
        let model_backend = self
            .config
            .model_config
            .backend
            .clone();
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
            tracing::info!(event = "state", state = state, account_id = self.account.as_str());
            *last_log = Some(sig);
            *last_at = now;
        }

        let health_path = self.root.join("health.json");
        write_private(&health_path, &Value::Object(map))?;
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
        d.insert("reason".to_string(), json!(format!("already replied ({source})")));
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
                                let attempted_at = data.get("attempted_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
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
    ) -> bool {
        let mut source = None;
        if let Some(history) = &self.history {
            if let Some(cid) = &self.contact_id {
                if let Ok(replied) = history.replied_after(cid, input_ids, send_key) {
                    if replied {
                        source = Some("history");
                    }
                }
            }
        }

        if source.is_none() {
            let reconciled = reconcile(&confirmed.bubbles, &latest.bubbles);
            let later = new_bubbles(&confirmed.bubbles, &reconciled).unwrap_or_default();
            if later.iter().any(|b| b.direction == "outbound_visual" && !self.own_echo(b)) {
                source = Some("screen");
            }
        }

        if let Some(s) = source {
            let mut d = Map::new();
            d.insert("reason".to_string(), json!(format!("already replied ({s})")));
            let _ = self.health("watching", d);
            true
        } else {
            false
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

    pub async fn scroll_to_bottom(&mut self, capture: &mut impl Capture, mut pages: u32) -> Result<Frame> {
        while pages > 0 {
            let chunk = std::cmp::min(40, pages);
            let _ = capture.scroll("down", chunk).await?;
            pages -= chunk;
        }
        let bottom = capture.frame().await?;
        self.policy.check_frame(&bottom).map_err(|e| anyhow::anyhow!("{e:?}"))?;
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

        let mut bottom = self.scroll_to_bottom(capture, pages.len() as u32 + 2).await?;
        let rec_bottom = reconcile(&frame.bubbles, &bottom.bubbles);
        let fp_frame: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
        let fp_rec: Vec<_> = rec_bottom.iter().map(fingerprint).collect();

        if fp_frame != fp_rec {
            bottom = self.scroll_to_bottom(capture, 40).await?;
            let rec_bottom2 = reconcile(&frame.bubbles, &bottom.bubbles);
            let fp_rec2: Vec<_> = rec_bottom2.iter().map(fingerprint).collect();
            if changes.is_some() && fp_frame != fp_rec2 {
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
                d.insert("error".to_string(), json!(err_str.chars().take(200).collect::<String>()));
                let _ = self.health("history_write_failed", d);
                return Ok(false);
            }
        };

        if let Some(title) = frame.title_lines.first() {
            let _ = history.observe_name(cid, title);
        }

        for row in rows {
            let kind = row.get("kind").and_then(|v| v.as_str());
            let mid = row.get("bubble").and_then(|b| b.get("media_id")).and_then(|v| v.as_str());
            let message_id = row.get("message_id").and_then(|v| v.as_str());
            if kind == Some("image") && mid.is_some() && message_id.is_some() {
                if let Ok(path) = capture.download(mid.unwrap()).await {
                    let _ = history.attach_media(message_id.unwrap(), &path, "image", None, "chat_thumbnail", None);
                }
            }
        }
        Ok(true)
    }

    pub async fn cycle(&mut self, client: &impl ToolClient, capture: &mut impl Capture) -> Result<()> {
        self.had_incoming = false;
        if !self.target_allowed() {
            return Ok(());
        }

        let mut frame = capture.frame().await?;
        if frame.clipped_bubble == Some(true) {
            frame = capture.scroll("down", 40).await?;
        }
        self.policy.check_frame(&frame).map_err(|e| anyhow::anyhow!("{e:?}"))?;

        if !self.record(&frame, capture).await? {
            return Ok(());
        }

        let legacy = self.cursor.as_ref().map_or(false, |c| {
            c.bubbles.iter().any(|b| b.kind.as_deref() == Some("image") && b.phash.is_none())
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
                certain.clone()
            }
            Err(ContinuityError::ContinuityGap) => {
                match self.recover_gap(capture, &frame, GAP_SCROLL_PAGES).await? {
                    Some(recovered) => recovered,
                    None => {
                        if self.recovery_unconfirmed {
                            let mut d = Map::new();
                            d.insert("reason".to_string(), json!("scrolled back but bottom frame changed"));
                            self.health("waiting_for_stable_frame", d)?;
                            return Ok(());
                        }
                        ambiguous = Some("visual continuity gap".to_string());
                        Vec::new()
                    }
                }
            }
        };

        if ambiguous.is_some() && changes.is_empty() {
            let confirmed = capture.frame().await?;
            self.policy.check_frame(&confirmed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let fp1: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
            let fp2: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
            if fp1 != fp2 {
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(ambiguous.unwrap()));
                self.health("waiting_for_stable_frame", d)?;
                return Ok(());
            }
            let reason_text = format!("{}: visible history rebaselined without reply", ambiguous.unwrap());
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
            self.policy.check_frame(&confirmed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            confirmed.bubbles = reconcile(&frame.bubbles, &confirmed.bubbles);
            let fp1: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
            let fp2: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
            if fp1 != fp2 {
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
        self.policy.check_frame(&f).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if self.replied_meanwhile(confirmed, &f, input_ids, Some(key)) {
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
    ) -> (Option<Value>, Value, Option<String>) {
        if let Some(map) = record.as_object_mut() {
            map.insert("status".to_string(), json!("attempting"));
            map.insert("attempted_at".to_string(), json!((self.clock)()));
        }
        let _ = self.write_journal(journal_path, record);
        let mut d = Map::new();
        d.insert("idempotency_key".to_string(), json!(key));
        let _ = self.health("sending", d);

        let res_result = client.tool(
            "send_message",
            json!({
                "recipient": self.clean_recipient(confirmed),
                "message": reply,
                "idempotency_key": key,
            }),
        ).await;

        let rec_result = client.tool(
            "get_send_status",
            json!({"idempotency_key": key}),
        ).await;

        let send_err_msg = res_result.as_ref().err().map(|e| format!("{e:#}"));
        let receipt = rec_result.unwrap_or_else(|_| json!({}));
        let res = res_result.ok();
        (res, receipt, send_err_msg)
    }

    pub async fn process_turn(
        &mut self,
        client: &impl ToolClient,
        capture: &mut impl Capture,
    ) -> Result<()> {
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
                                        role: item.get("role").and_then(|v| v.as_str()).unwrap_or("user").to_string(),
                                        text: item.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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
                                d.insert("error".to_string(), json!(e.to_string().chars().take(200).collect::<String>()));
                                self.health("history_write_failed", d)?;
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        let mut d = Map::new();
                        d.insert("error".to_string(), json!(e.to_string().chars().take(200).collect::<String>()));
                        self.health("history_write_failed", d)?;
                        return Ok(());
                    }
                }
            }
        } else {
            let start = confirmed.bubbles.len().saturating_sub(12);
            for b in &confirmed.bubbles[start..] {
                context_items.push(ContextItem {
                    role: if b.direction == "inbound_visual" { "user".to_string() } else { "assistant".to_string() },
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
                    d.insert("reason".to_string(), json!("existing batch requires review"));
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

        let result = self.run_generation_and_send(client, capture, &key, &journal_path, &pending, &incoming, &confirmed, &context_items, &input_ids, &mut record).await;

        match result {
            Ok(Some(after)) => {
                let echoed = after.bubbles.iter().any(|b| {
                    b.direction == "outbound_visual"
                        && echo_matches(&b.text, record.get("reply").and_then(|v| v.as_str()).unwrap_or(""))
                });
                if let Some(map) = record.as_object_mut() {
                    map.insert("client_echo_observed".to_string(), json!(echoed));
                }
                self.write_journal(&journal_path, &record)?;

                let unseen = match new_bubbles(&confirmed.bubbles, &after.bubbles) {
                    Ok(un) => Some(un),
                    Err(ref err @ ContinuityError::ContinuityGap) | Err(ref err @ ContinuityError::AmbiguousAnchor { .. }) => {
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
                let status_str = record.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                if let Some(map) = record.as_object_mut() {
                    map.insert("error".to_string(), json!(err_str.chars().take(400).collect::<String>()));
                }

                let failures = pending.model_failures;
                if status_str == "preparing_media" && action_interrupted(&err_str) {
                    let _ = fs::remove_file(&journal_path);
                    self.keep_turn(pending)?;
                    let mut d = Map::new();
                    d.insert("reason".to_string(), json!(err_str.chars().take(200).collect::<String>()));
                    self.health("waiting_for_wechat_window", d)?;
                    return Ok(());
                }

                if status_str == "generating" && failures < MODEL_RETRIES {
                    let _ = fs::remove_file(&journal_path);
                    let mut retrying_turn = pending;
                    retrying_turn.model_failures = failures + 1;
                    retrying_turn.retry_at = (self.clock)() + MODEL_RETRY_SECONDS;
                    self.keep_turn(retrying_turn)?;
                    let mut d = Map::new();
                    d.insert("reason".to_string(), json!(err_str.chars().take(200).collect::<String>()));
                    d.insert("attempt".to_string(), json!(failures + 1));
                    self.health("model_retry_wait", d)?;
                    return Ok(());
                }

                self.write_journal(&journal_path, &record)?;
                self.paused = true;
                self.pause_reason = Some(format!("model or send attempt requires review: {}", err_str));
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(self.pause_reason.clone().unwrap()));
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
        let (fetch, images, files, failed) = if incoming.iter().any(|b| b.kind.as_deref().unwrap_or("text") != "text") {
            let snapshot = capture.frame().await?;
            self.policy.check_frame(&snapshot).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let fetch = relocate(incoming, &snapshot.bubbles);
            let (imgs, fls, fld) = capture.attachments(&fetch, &snapshot).await?;
            (fetch, imgs, fls, fld)
        } else {
            (incoming.to_vec(), Vec::new(), Vec::new(), Vec::new())
        };
        self.archive_media(&fetch, input_ids, &images, &files);

        let media_meta: Vec<Value> = files.iter().map(|f| {
            json!({
                "kind": f.content.get("kind").unwrap_or(&json!("document")),
                "filename": f.filename,
                "original_bytes": f.original_bytes,
                "truncated": f.content.get("truncated").unwrap_or(&json!(false)),
            })
        }).collect();
        let images_meta: Vec<Value> = images.iter().map(|i| {
            json!({
                "label": i.label,
                "source": i.source,
            })
        }).collect();

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
            let prefix = if self.group && b.sender.is_some() {
                format!("{}：", b.sender.as_ref().unwrap())
            } else {
                String::new()
            };
            text_lines.push(format!("{}{}", prefix, b.text));
        }
        let full_text = text_lines.join("
");

        let brain_turn = BrainTurn {
            turn_id: key.to_string(),
            account_id: self.account.clone(),
            contact: ContactInfo {
                name: self.recipient.clone(),
                kind: if self.group { "group".to_string() } else { "private".to_string() },
            },
            text: full_text,
            mode: "service".to_string(),
            chat_kind: if self.group { "group".to_string() } else { "private".to_string() },
            context: context_items.to_vec(),
            images: images.iter().map(|i| i.path.clone()).collect(),
            files: files.iter().map(|f| serde_json::to_value(f).unwrap()).collect(),
            notes: Vec::new(),
            failed: failed.iter().map(|f| FailedAttachment {
                label: f.label.clone(),
                kind: Some(f.kind.clone()),
                reason: Some(f.reason.clone()),
                hint: None,
            }).collect(),
            image_count: Some(images.len()),
            earlier_images: 0,
            max_attachment_mb: self.config.max_attachment_mb,
        };

        let generated_reply = if let Some(brain) = &self.brain {
            brain(&brain_turn).await?.reply
        } else {
            let builtin = crate::brain::builtin::BuiltinBrain::new(
                self.config.model_config.clone(),
            );
            builtin.reply(&brain_turn).await?.reply
        };

        let validated = validate_reply(&generated_reply).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let reply = ensure_unreadable_named(&validated, &failed)?;

        if let Some(map) = record.as_object_mut() {
            map.insert("status".to_string(), json!("generated"));
            map.insert("reply".to_string(), json!(reply));
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

        let (check_res, _latest) = self.verify_before_send(capture, confirmed, input_ids, key).await?;
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

        if let Some(history) = &self.history {
            if let Some(cid) = &self.contact_id {
                let _ = history.record_bot_reply(
                    cid,
                    key,
                    &reply,
                    "generated",
                    &json!({}),
                    input_ids,
                    None,
                    None,
                );
            }
        }

        let (mut result, mut receipt, mut send_err) = self.try_send_once(client, journal_path, record, confirmed, &reply, key).await;

        if send_err.is_none() && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered") {
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
                (self.sleep)(Duration::from_secs(delay)).await;
                let (check_res, _) = self.verify_before_send(capture, confirmed, input_ids, key).await?;
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
                let (r, rec, err) = self.try_send_once(client, journal_path, record, confirmed, &reply, key).await;
                result = r;
                receipt = rec;
                send_err = err;
                if send_err.is_none() && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered") {
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
                    map.insert("error".to_string(), json!(err_msg.chars().take(400).collect::<String>()));
                    map.insert("durable_status".to_string(), receipt);
                    map.insert("delivery".to_string(), json!("not_sent"));
                }
                self.write_journal(journal_path, record)?;
                let short_err = err_msg.lines().next().unwrap_or("").chars().take(80).collect::<String>();
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(format!("last reply not sent: {short_err}")));
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
                        let _ = self.policy.check_frame(&f);
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
                        map.insert("error".to_string(), json!(err_msg.chars().take(400).collect::<String>()));
                        map.insert("durable_status".to_string(), receipt);
                    }
                    self.write_journal(journal_path, record)?;
                    return Ok(None);
                }
                EchoOutcome::NotSeen(_) => {
                    let err_msg = send_err.clone().unwrap_or_default();
                    if let Some(map) = record.as_object_mut() {
                        map.insert("status".to_string(), json!("not_sent"));
                        map.insert("error".to_string(), json!(err_msg.chars().take(400).collect::<String>()));
                        map.insert("durable_status".to_string(), receipt.clone());
                    }
                    self.write_journal(journal_path, record)?;

                    (self.sleep)(Duration::from_secs(5)).await;
                    let (check_res, _) = self.verify_before_send(capture, confirmed, input_ids, key).await?;
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

                    let (r, rec, err) = self.try_send_once(client, journal_path, record, confirmed, &reply, key).await;
                    result = r;
                    receipt = rec;
                    send_err = err;

                    if send_err.is_none() && receipt.get("status").and_then(|v| v.as_str()) == Some("triggered") {
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
                            map.insert("error".to_string(), json!(err_msg2.chars().take(400).collect::<String>()));
                            map.insert("durable_status".to_string(), receipt);
                            map.insert("delivery".to_string(), json!("not_sent"));
                        }
                        self.write_journal(journal_path, record)?;
                        let short_err = err_msg2.lines().next().unwrap_or("").chars().take(80).collect::<String>();
                        let mut d = Map::new();
                        d.insert("reason".to_string(), json!(format!("last reply not sent: {short_err}")));
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
            f
        } else {
            let f = capture.frame().await?;
            self.policy.check_frame(&f).map_err(|e| anyhow::anyhow!("{e:?}"))?;
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
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    
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
        pub download_fn: Option<Arc<dyn Fn(&str) -> Result<PathBuf> + Send + Sync>>,
        pub attachments_fn: Option<Arc<dyn Fn(&[Bubble], &Frame) -> Result<(Vec<AttachmentImage>, Vec<AttachmentFile>, Vec<FailedAttachmentItem>)> + Send + Sync>>,
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
            self.scroll_calls.lock().unwrap().push((direction.to_string(), pages));
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
            if let Some(dfn) = &self.download_fn {
                dfn(media_id)
            } else {
                bail!("download not mocked");
            }
        }

        async fn attachments(
            &mut self,
            items: &[Bubble],
            frame: &Frame,
        ) -> Result<(Vec<AttachmentImage>, Vec<AttachmentFile>, Vec<FailedAttachmentItem>)> {
            if let Some(afn) = &self.attachments_fn {
                afn(items, frame)
            } else {
                Ok((Vec::new(), Vec::new(), Vec::new()))
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
                        Ok(json!({"status": "triggered", "send_id": "fixture-send", "delivery": "unknown"}))
                    } else if name == "get_send_status" {
                        Ok(json!({"status": "triggered", "send_id": "fixture-send", "delivery": "unknown"}))
                    } else {
                        Ok(json!({}))
                    }
                }),
            }
        }
    }

    impl ToolClient for MockToolClient {
        async fn tool(&self, name: &str, args: Value) -> Result<Value> {
            self.calls.lock().unwrap().push((name.to_string(), args.clone()));
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
        assert_eq!(key, "bot-service-d5143f967016e38026a4ea02061965c5b5584395538205585b4962862fb73666");

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
        assert_eq!(key2, "bot-service-a13abb560bb7ac5df1dacb0766724f0d99e5fed4530e4280a4fe3376f0c0361f");
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

        let f_long: Vec<FailedAttachmentItem> = (0..15).map(|i| FailedAttachmentItem {
            label: format!("长长长长长长长长文件_{i}.pdf"),
            kind: "file".to_string(),
            reason: "unsupported".to_string(),
            detail: None,
        }).collect();
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

    // === SendRetryTests (8 tests) ===
    #[tokio::test]
    async fn test_send_retry_not_sent_click_failure_retries_and_does_not_pause() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");

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
                        bail!("MCP tool failed: send_message: 发送尝试 send-1 未触发发送: wechat_click 退出状态异常: Some(1)");
                    }
                    Ok(json!({"status": "triggered", "send_id": "send-1", "delivery": "unknown"}))
                } else if name == "get_send_status" {
                    let att = attempt_clone.load(Ordering::SeqCst);
                    if att == 1 {
                        Ok(json!({"status": "unknown", "delivery": "not_sent", "send_id": "send-1"}))
                    } else {
                        Ok(json!({"status": "triggered", "delivery": "unknown", "send_id": "send-1"}))
                    }
                } else {
                    Ok(json!({}))
                }
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after.clone(), after.clone()]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);
        assert_eq!(send_calls[0]["idempotency_key"], send_calls[1]["idempotency_key"]);

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!("MCP tool failed: send_message: 发送尝试 fail 未触发发送: wechat_click 退出状态异常: Some(1)");
                }
                if name == "get_send_status" {
                    return Ok(json!({"status": "unknown", "delivery": "not_sent", "send_id": "fail"}));
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(); 10]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 3);
        assert_eq!(send_calls[0]["idempotency_key"], send_calls[1]["idempotency_key"]);
        assert_eq!(send_calls[1]["idempotency_key"], send_calls[2]["idempotency_key"]);

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("send-"))
            .collect();
        assert_eq!(journals.len(), 1);
        let data: Value = read_json(journals[0].path()).unwrap();
        assert_eq!(data["status"], "not_sent");

        let health: Value = read_json(svc.root.join("health.json")).unwrap();
        assert_eq!(health["state"], "watching");
        assert!(health["reason"].as_str().unwrap().contains("last reply not sent"));
    }

    #[tokio::test]
    async fn test_send_retry_unknown_with_echo_marks_triggered_no_resend() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let frame_with_echo = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!("MCP tool failed: send_message: 结果未知: connection closed");
                }
                if name == "get_send_status" {
                    return Ok(json!({"status": "unknown", "delivery": "unknown", "send_id": "unk-1"}));
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(), new_f.clone(), new_f.clone(),
            frame_with_echo.clone(), frame_with_echo.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 1);

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");
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
                        Ok(json!({"status": "triggered", "delivery": "unknown", "send_id": "retry-1"}))
                    }
                } else {
                    Ok(json!({}))
                }
            }),
        };

        let mut capture = MockCapture::new(vec![
            new_f.clone(), new_f.clone(),
            new_f.clone(), new_f.clone(), new_f.clone(), // 3 screen checks without echo
            new_f.clone(), // pre-send retry
            after.clone(), after.clone(),
        ]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let client = MockToolClient {
            calls: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(|name, _args| {
                if name == "send_message" {
                    bail!("MCP tool failed: send_message: 结果未知: osascript timeout");
                }
                if name == "get_send_status" {
                    return Ok(json!({"status": "unknown", "delivery": "unknown", "send_id": "unk-err"}));
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone()]);
        capture.frame_sequence.push(MockFrameItem::Err("capture device error 1".to_string()));
        capture.frame_sequence.push(MockFrameItem::Err("capture device error 2".to_string()));
        capture.frame_sequence.push(MockFrameItem::Err("capture device error 3".to_string()));

        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 1);

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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
        write_private(&attempting, &json!({
            "status": "attempting",
            "reply": "你好，我是AI客服助手。",
            "idempotency_key": "key-attempting"
        })).unwrap();

        let cfg = fixture_config(&state_dir);
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        assert!(!svc.paused);

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let frame_with_echo = test_frame(vec![test_bubble("old", "inbound_visual"), reply], "午夜咖啡");
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
        write_private(&gen_path, &json!({
            "status": "generated",
            "reply": "你好",
            "idempotency_key": "key-gen"
        })).unwrap();
        let prep = state_dir.join("send-preparing.json");
        write_private(&prep, &json!({
            "status": "preparing_media",
            "idempotency_key": "key-prep"
        })).unwrap();

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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");
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
                        bail!("MCP tool failed: send_message: 发送尝试 sim-1 未触发发送: wechat_click 退出状态异常: Some(1)");
                    }
                    let success = json!({"status": "triggered", "delivery": "unknown", "send_id": "sim-1"});
                    box_map.insert(key, success.clone());
                    return Ok(success);
                } else if name == "get_send_status" {
                    let key = args["idempotency_key"].as_str().unwrap().to_string();
                    let box_map = outbox_clone.lock().unwrap();
                    return Ok(box_map.get(&key).cloned().unwrap_or(json!({"status": "unknown", "delivery": "unknown", "send_id": "sim-1"})));
                }
                Ok(json!({}))
            }),
        };

        let mut capture = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after.clone(), after.clone()]);
        svc.cycle(&client, &mut capture).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(dispatches.load(Ordering::SeqCst), 2);
        let send_calls: Vec<Value> = client.calls.lock().unwrap()
            .iter()
            .filter(|(n, _)| n == "send_message")
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(send_calls.len(), 2);
        assert_eq!(send_calls[0]["idempotency_key"], send_calls[1]["idempotency_key"]);
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
            Box::pin(async { Ok(BrainReply { reply: "hi".to_string(), metadata: HashMap::new() }) })
        }));

        let mut legacy_img = test_bubble("[图片]", "inbound_visual");
        legacy_img.digest = "pixels".to_string();
        legacy_img.kind = Some("image".to_string());
        let legacy = test_frame(vec![test_bubble("old", "inbound_visual"), legacy_img], "午夜咖啡");
        write_private(&svc.cursor_path, &legacy).unwrap();

        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        let mut curr_img = test_bubble("[图片]", "inbound_visual");
        curr_img.digest = "image:50:40".to_string();
        curr_img.kind = Some("image".to_string());
        curr_img.phash = Some("0".repeat(16));
        let current = test_frame(vec![test_bubble("old", "inbound_visual"), curr_img, test_bubble("新消息", "inbound_visual")], "午夜咖啡");

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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        write_private(&svc.cursor_path, &old).unwrap();

        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        let mc2 = model_called.clone();
        svc.brain = Some(Arc::new(move |_| {
            mc2.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        let mut capture = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), new_f.clone()]);
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let one = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("图片说明", "inbound_visual")], "午夜咖啡");
        let two = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("图片说明", "inbound_visual"), test_bubble("这是什么", "inbound_visual")], "午夜咖啡");
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
        let after = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("图片说明", "inbound_visual"),
            test_bubble("这是什么", "inbound_visual"),
            reply,
        ], "午夜咖啡");
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.digest = "image:50:40".to_string();
        img.phash = Some("0".repeat(16));
        img.media_id = Some("a".repeat(64));
        img.kind = Some("image".to_string());

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(vec![test_bubble("old", "inbound_visual"), img.clone()], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), img.clone(), reply], "午夜咖啡");

        let shot_path = td.path().join("x.png");
        fs::write(&shot_path, b"fake png").unwrap();
        let shot_path_clone = shot_path.clone();

        let mut cap = MockCapture::new(vec![
            current.clone(), current.clone(), current.clone(), current.clone(), after
        ]);
        let img_clone = img.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((
                vec![AttachmentImage {
                    path: shot_path_clone.clone(),
                    label: "第1张图片".to_string(),
                    source: "chat_thumbnail".to_string(),
                    bubble: img_clone.clone(),
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        *current_time.lock().unwrap() = 1000.0;
        let mut cap1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap1).await.unwrap();

        let answered = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好，介绍一下你自己", "inbound_visual"),
            test_bubble("人工：我是店长，马上处理", "outbound_visual"),
        ], "午夜咖啡");

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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let answered = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好，介绍一下你自己", "inbound_visual"),
            test_bubble("人工：我是店长，马上处理", "outbound_visual"),
        ], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), answered.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let send_calls: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
        assert!(send_calls.is_empty());

        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        *current_time.lock().unwrap() = 1000.0;
        let mut cap1 = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap1).await.unwrap();

        let key = svc.batch_key(&svc.turn.as_ref().unwrap().baseline, &svc.turn.as_ref().unwrap().bubbles);
        write_private(&svc.root.join(format!("send-{key}.json")), &json!({"status": "triggered", "idempotency_key": key})).unwrap();

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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let again = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好，介绍一下你自己", "inbound_visual"),
            test_bubble("第二个问题", "inbound_visual"),
            reply,
        ], "午夜咖啡");

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

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let noisy = test_bubble("你好，我是Al客服助手。", "outbound_visual");
        let again = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好，介绍一下你自己", "inbound_visual"),
            test_bubble("第二个问题", "inbound_visual"),
            noisy,
        ], "午夜咖啡");

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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
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

        let uploading = test_frame(vec![test_bubble("old", "inbound_visual"), card.clone()], "午夜咖啡");
        let mut done_card = card.clone();
        done_card.text = "报价.pdf\nPDF\n220.3K".to_string();
        done_card.transfer_pending = Some(false);
        let done = test_frame(vec![test_bubble("old", "inbound_visual"), done_card], "午夜咖啡");

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
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");

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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let mut clipped = new_f.clone();
        clipped.clipped_bubble = Some(true);

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");

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
            Box::pin(async { Ok(BrainReply { reply: "有的".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "测试群 (3)");
        svc.advance(old).unwrap();

        let mut asked = test_bubble("@享味来 有货吗", "inbound_visual");
        asked.sender = Some("午夜咖啡".to_string());
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), asked.clone()], "测试群 (3)");
        let reply = test_bubble("有的", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), asked, reply], "测试群 (3)");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*seen_prompt.lock().unwrap(), "午夜咖啡：@享味来 有货吗");
        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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
                Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() })
            })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");

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

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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
            Box::pin(async { Ok(BrainReply { reply: "hi".to_string(), metadata: HashMap::new() }) })
        }));

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual"), reply], "午夜咖啡");

        let mut cap2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let scrolled = test_frame(vec![
            test_bubble("[图片]", "inbound_visual"),
            test_bubble("在的，我是AI客服助手。", "outbound_visual"),
        ], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), scrolled.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();
        assert!(!svc.paused);

        let mut cap2 = MockCapture::new(vec![scrolled.clone(), scrolled.clone()]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        assert!(!svc.paused);

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].1["recipient"], "午夜咖啡");

        let mut cap2 = MockCapture::new(vec![after]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        let sends2: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let old = test_frame(vec![test_bubble("刚才发链接看了吗", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let current = test_frame(vec![
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("刚才发链接看了吗", "inbound_visual"),
        ], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![current.clone(), current.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(!svc.paused);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, current.bubbles);
        assert!(client.calls.lock().unwrap().is_empty());

        let later = test_frame(vec![
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("在吗", "inbound_visual"),
        ], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("在吗", "inbound_visual"),
            reply,
        ], "午夜咖啡");

        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone(), later.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_ambiguous_frame_must_be_stable_before_skipping() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("刚才发链接看了吗", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let current = test_frame(vec![
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("刚才发链接看了吗", "inbound_visual"),
        ], "午夜咖啡");
        let different = test_frame(vec![
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("刚才发链接看了吗", "inbound_visual"),
            test_bubble("在吗", "inbound_visual"),
        ], "午夜咖啡");

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

        let later = test_frame(vec![test_bubble("你好 在吗", "inbound_visual"), test_bubble("有人吗", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("你好 在吗", "inbound_visual"), test_bubble("有人吗", "inbound_visual"), reply], "午夜咖啡");

        let mut cap2 = MockCapture::new(vec![later.clone(), later.clone(), later.clone(), after]);
        svc.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
        assert_eq!(sends.len(), 1);
    }

    #[tokio::test]
    async fn test_service_unrecoverable_gap_scrolls_at_most_five_pages() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let cleared = test_frame(vec![test_bubble("全新的消息", "inbound_visual")], "午夜咖啡");
        let mut cap = MockCapture::new(vec![cleared.clone(); 8]);
        cap.scroll_pages = (0..60).map(|n| test_frame(vec![test_bubble(&format!("更早{n}"), "inbound_visual")], "午夜咖啡")).collect();

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        let ups: Vec<_> = cap.scroll_calls.lock().unwrap().iter().filter(|(d, _)| d == "up").cloned().collect();
        assert!(ups.len() <= 5);
    }

    #[tokio::test]
    async fn test_service_interrupted_gap_scroll_retries_instead_of_rebaselining() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let mut svc = BotService::new(cfg, None, None, None, None, None).unwrap();

        let old = test_frame(vec![test_bubble("a", "inbound_visual"), test_bubble("b", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();

        let mut cap = MockCapture::new(vec![test_frame(vec![test_bubble("x", "inbound_visual")], "午夜咖啡"); 3]);
        // Set scroll to fail with interrupted
        cap.scroll_pages = Vec::new(); // will fallback to frame() which returns x, but we want scroll to fail:
        struct InterruptedCapture(MockCapture);
        impl Capture for InterruptedCapture {
            async fn frame(&mut self) -> Result<Frame> { self.0.frame().await }
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let c = test_bubble("c", "inbound_visual");
        let tall = test_bubble("TALL", "inbound_visual");
        let content = vec![(&a, 40), (&b, 40), (&c, 40), (&tall, 380)];

        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡")).unwrap();

        let bottom = test_chat(&content, 0, 436);
        let up1 = test_chat(&content, 1, 436);
        let up2 = test_chat(&content, 2, 436);
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let mut after_b = bottom.bubbles.clone();
        after_b.push(reply);
        let after = test_frame(after_b, "午夜咖啡");

        let mut cap = MockCapture::new(vec![bottom.clone(), bottom.clone(), bottom.clone(), bottom.clone(), after]);
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let a = test_bubble("a", "inbound_visual");
        let b = test_bubble("b", "inbound_visual");
        let mut img1 = test_bubble("[图片]", "inbound_visual"); img1.phash = Some("0000000000000000".to_string()); img1.digest = "image:61:61".to_string(); img1.kind = Some("image".to_string());
        let mut img2 = test_bubble("[图片]", "inbound_visual"); img2.phash = Some("ffffffff00000000".to_string()); img2.digest = "image:61:61".to_string(); img2.kind = Some("image".to_string());
        let mut img3 = test_bubble("[图片]", "inbound_visual"); img3.phash = Some("00000000ffffffff".to_string()); img3.digest = "image:61:61".to_string(); img3.kind = Some("image".to_string());

        let content = vec![(&a, 40), (&b, 40), (&img1, 247), (&img2, 247), (&img3, 247)];
        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡")).unwrap();

        let bottom = test_chat(&content, 0, 436);
        let ups: Vec<Frame> = (1..=8).map(|k| test_chat(&content, k, 436)).collect();
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let mut after_b = bottom.bubbles.clone();
        after_b.push(reply);
        let after = test_frame(after_b, "午夜咖啡");

        let mut cap = MockCapture::new(vec![bottom.clone(), bottom.clone(), bottom.clone(), bottom.clone(), bottom.clone(), after]);
        let mut scroll_seq = ups;
        scroll_seq.push(bottom);
        cap.scroll_pages = scroll_seq;

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(prompt_count.load(Ordering::SeqCst), 3);
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

        svc.advance(test_frame(vec![a.clone(), b.clone()], "午夜咖啡")).unwrap();

        let bottom = test_frame(vec![c.clone(), tall.clone()], "午夜咖啡");
        let up1 = test_frame(vec![b.clone(), c.clone()], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![c.clone(), tall.clone(), reply], "午夜咖啡");

        let mut cap = MockCapture::new(vec![bottom.clone(), up1.clone(), bottom.clone(), bottom.clone(), bottom.clone(), after]);
        cap.scroll_pages = vec![up1.clone(), up1.clone(), bottom.clone()];

        let client = MockToolClient::new_default();
        svc.cycle(&client, &mut cap).await.unwrap();
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

        let content = test_frame(vec![test_bubble("c", "inbound_visual"), test_bubble("TALL", "inbound_visual")], "午夜咖啡");
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
        let f2 = test_frame(vec![test_bubble("完全不同", "inbound_visual"), test_bubble("又变了", "inbound_visual")], "午夜咖啡");

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

        let f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("我的消息", "outbound_visual")], "午夜咖啡");
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let after = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好", "inbound_visual"),
            test_bubble("补充问题", "inbound_visual"),
            test_bubble("你好，我是AI客服助手。", "outbound_visual"),
        ], "午夜咖啡");

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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let changed = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual"), test_bubble("新问题", "inbound_visual")], "午夜咖啡");
        let after = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好", "inbound_visual"),
            test_bubble("新问题", "inbound_visual"),
            test_bubble("你好，我是AI客服助手。", "outbound_visual"),
        ], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), changed.clone(), after.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, new_f.bubbles);

        let final_after = test_frame(vec![
            test_bubble("old", "inbound_visual"),
            test_bubble("你好", "inbound_visual"),
            test_bubble("新问题", "inbound_visual"),
            test_bubble("你好，我是AI客服助手。", "outbound_visual"),
            test_bubble("好的", "outbound_visual"),
        ], "午夜咖啡");
        let mut cap2 = MockCapture::new(vec![after.clone(), after.clone(), after.clone(), final_after]);
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

        let old = vec![test_bubble("hi", "inbound_visual"), test_bubble("hi", "inbound_visual"), test_bubble("there", "inbound_visual"), test_bubble("question", "inbound_visual"), test_bubble("received", "inbound_visual")];
        svc.advance(test_frame(old.clone(), "午夜咖啡")).unwrap();

        let mut inc_bubbles = old.clone();
        inc_bubbles.push(test_bubble("hi", "inbound_visual"));
        let incoming = test_frame(inc_bubbles.clone(), "午夜咖啡");

        let mut after_bubbles = inc_bubbles[1..].to_vec();
        after_bubbles.push(test_bubble("你好，我是AI客服助手。", "outbound_visual"));
        let after = test_frame(after_bubbles, "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![incoming.clone(), incoming.clone(), incoming.clone(), after.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.cursor.as_ref().unwrap().bubbles, after.bubbles);
        assert!(!svc.paused);

        let mut cap2 = MockCapture::new(vec![after]);
        svc.cycle(&client, &mut cap2).await.unwrap();
        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
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
        assert_eq!(restarted.cursor.as_ref().unwrap().profile.as_deref(), Some("wechat-4-dark"));
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert!(client.calls.lock().unwrap().is_empty());
        let journals: Vec<_> = fs::read_dir(&svc.root).unwrap()
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
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        // Restart
        let mut svc2 = BotService::new(cfg, None, None, None, None, None).unwrap();
        svc2.sleep = Arc::new(|_| Box::pin(async {}));
        svc2.brain = Some(default_brain("你好，我是AI客服助手。"));

        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual"), reply], "午夜咖啡");
        let mut cap2 = MockCapture::new(vec![new_f.clone(), new_f.clone(), after]);
        svc2.cycle(&client, &mut cap2).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        write_private(&svc.root.join("send-limit.json"), &json!({
            "status": "triggered",
            "attempted_at": (svc.clock)()
        })).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(svc.turn.as_ref().unwrap().bubbles[0].text, "你好，介绍一下你自己");
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

        let mut svc = BotService::new(cfg, None, Some(history.clone()), Some(cid.clone()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old.clone()).unwrap();
        history.record_frame(&cid, &serde_json::to_value(&old).unwrap(), None, None, false).unwrap();

        let order = Arc::new(Mutex::new(Vec::new()));
        let ord = order.clone();
        svc.brain = Some(Arc::new(move |_| {
            ord.lock().unwrap().push("model");
            Box::pin(async { Ok(BrainReply { reply: "你好，我是AI客服助手。".to_string(), metadata: HashMap::new() }) })
        }));

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual"), reply], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        assert_eq!(*order.lock().unwrap(), vec!["model"]);
        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard.prepare("SELECT sender FROM messages ORDER BY seq").unwrap();
        let senders: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
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

        let mut svc = BotService::new(cfg, None, Some(history.clone()), Some(cid.clone()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));

        let bubbles: Vec<Bubble> = (0..5).map(|i| test_bubble(&i.to_string(), "inbound_visual")).collect();
        let base = test_frame(bubbles, "午夜咖啡");
        history.record_frame(&cid, &serde_json::to_value(&base).unwrap(), None, None, false).unwrap();
        svc.advance(base.clone()).unwrap();

        let mut new_bubbles = base.bubbles.clone();
        new_bubbles.push(test_bubble("新问题", "inbound_visual"));
        let new_f = test_frame(new_bubbles, "午夜咖啡");

        let context_texts = Arc::new(Mutex::new(Vec::new()));
        let ct = context_texts.clone();
        svc.brain = Some(Arc::new(move |turn| {
            let texts: Vec<String> = turn.context.iter().map(|c| c.text.clone()).collect();
            *ct.lock().unwrap() = texts;
            Box::pin(async { Ok(BrainReply { reply: "好的".to_string(), metadata: HashMap::new() }) })
        }));

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), new_f.clone()]);
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

        let mut svc = BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let conn = history.connection();
        let conn_guard = conn.lock().unwrap();
        let mut stmt = conn_guard.prepare("SELECT text FROM messages ORDER BY seq").unwrap();
        let texts: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(texts, vec!["old", "你好，介绍一下你自己"]);
    }

    #[tokio::test]
    async fn test_history_service_history_reply_blocks_send() {
        let td = TestDir::new();
        let cfg = fixture_config(td.path());
        let history = Arc::new(HistoryStore::open(td.path().join("h")).unwrap());
        history.migrate().unwrap();
        let entry = json!({"recipient": "午夜咖啡", "kind": "private"});
        let cid = history.ensure_contact("fixture", &entry, false).unwrap();

        let mut svc = BotService::new(cfg, None, Some(history.clone()), Some(cid.clone()), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        // First record the new incoming message so match_recent_inbound finds input_ids
        history.record_frame(&cid, &serde_json::to_value(&new_f).unwrap(), None, None, false).unwrap();
        // Then record an outbound reply so replied_after returns true
        history.record_bot_reply(&cid, "key-other", "人工回答", "triggered", &json!({}), &[], None, None).unwrap();

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone()]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好，介绍一下你自己", "inbound_visual"), reply], "午夜咖啡");

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

        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let mut svc = BotService::new(cfg, None, Some(history.clone()), Some(cid), None, None).unwrap();
        svc.sleep = Arc::new(|_| Box::pin(async {}));
        svc.brain = Some(default_brain("你好，我是AI客服助手。"));

        let old = test_frame(vec![test_bubble("old", "inbound_visual")], "午夜咖啡");
        svc.advance(old).unwrap();

        let mut img = test_bubble("[图片]", "inbound_visual");
        img.digest = "image:50:40".to_string();
        img.phash = Some("0".repeat(16));
        img.media_id = Some("a".repeat(64));
        img.kind = Some("image".to_string());
        let current = test_frame(vec![test_bubble("old", "inbound_visual"), img.clone()], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), img.clone(), reply], "午夜咖啡");

        let img_path = td.path().join("x.png");
        fs::write(&img_path, b"png original").unwrap();
        let img_path_clone = img_path.clone();

        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current.clone(), current.clone(), after]);
        let img_clone = img.clone();
        cap.attachments_fn = Some(Arc::new(move |_, _| {
            Ok((
                vec![AttachmentImage {
                    path: img_path_clone.clone(),
                    label: "第1张图片".to_string(),
                    source: "original".to_string(),
                    bubble: img_clone.clone(),
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
        let sources: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
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
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("已收到第9张图片打不开，麻烦重发一下。", "outbound_visual")], "午夜咖啡");

        let mut cap = MockCapture::new(vec![current.clone(), current.clone(), current.clone(), current.clone(), after]);
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

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

        let new_f = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual")], "午夜咖啡");
        let reply = test_bubble("你好，我是AI客服助手。", "outbound_visual");
        let after = test_frame(vec![test_bubble("old", "inbound_visual"), test_bubble("你好", "inbound_visual"), reply], "午夜咖啡");

        let client = MockToolClient::new_default();
        let mut cap = MockCapture::new(vec![new_f.clone(), new_f.clone(), new_f.clone(), after]);
        svc.cycle(&client, &mut cap).await.unwrap();

        let sends: Vec<_> = client.calls.lock().unwrap().iter().filter(|(n, _)| n == "send_message").cloned().collect();
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

}
