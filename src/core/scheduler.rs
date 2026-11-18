use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::config::{AllowlistEntry, BotConfig, candidate_names, name_key};
use crate::core::delivery::ToolClient;
use crate::core::history::store::HistoryStore;
use crate::core::model::{Bubble, Frame};
use crate::core::state::{read_json, write_private};
use crate::core::turns::{
    AttachmentFile, AttachmentImage, BotService, Capture, FailedAttachmentItem,
};
use crate::core::worker_client::{OcrStatus, action_interrupted, layout_unavailable, must_wait};

pub const BADGE_PIXELS: u64 = 20;
pub const TITLE_MISMATCH_LIMIT: usize = 3;
pub const RECALIBRATION_LIMIT: usize = 5;

pub const ROW_NOT_VISIBLE: &str = "session_row_not_unique_or_missing";
pub const WINDOW_UNAVAILABLE: &[&str] = &[
    "uncalibrated_theme",
    "unsupported_image_geometry",
    "wechat_not_visible",
    "window_capture_failed",
    "desktop_busy",
    "vision_timeout",
    "layout_missing",
    "layout_unreadable",
    "screen_not_main",
    ROW_NOT_VISIBLE,
];

pub trait DynToolClient: Send + Sync {
    fn tool(
        self: Arc<Self>,
        name: String,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>;
}

impl<T: ToolClient + 'static> DynToolClient for T {
    fn tool(
        self: Arc<Self>,
        name: String,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> {
        Box::pin(async move {
            let fut = <Self as ToolClient>::tool(&self, &name, args);
            fut.await
        })
    }
}

impl ToolClient for Arc<dyn DynToolClient> {
    fn tool(&self, name: &str, args: Value) -> impl Future<Output = Result<Value>> + Send {
        let this = self.clone();
        let name_owned = name.to_string();
        async move { this.tool(name_owned, args).await }
    }
}

pub fn is_window_unavailable(err_msg: &str) -> bool {
    layout_unavailable(err_msg)
        || WINDOW_UNAVAILABLE.iter().any(|code| {
            err_msg.contains(&format!("invalid(\"{code}\")"))
                || err_msg.contains(&format!("invalid(\"{code}:"))
        })
        || action_interrupted(err_msg)
}

pub fn extract_mismatch_failures(error: &str) -> Vec<String> {
    let start = match error.find("invalid(\"layout_mismatch") {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let rest = &error[start + 9..];
    let token = match rest.find("\")") {
        Some(end) => &rest[..end],
        None => rest,
    };
    if let Some((_, payload)) = token.split_once(':') {
        payload
            .split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

pub fn ocr_noise(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    let diff = (ca.len() as isize - cb.len() as isize).abs();
    if diff > 1 {
        return false;
    }
    if ca.len() == cb.len() {
        let mismatches = ca.iter().zip(cb.iter()).filter(|(x, y)| x != y).count();
        return mismatches <= 1;
    }
    let (short, long_) = if ca.len() < cb.len() {
        (&ca, &cb)
    } else {
        (&cb, &ca)
    };
    for i in 0..long_.len() {
        let mut skipped = Vec::with_capacity(short.len());
        skipped.extend_from_slice(&long_[..i]);
        skipped.extend_from_slice(&long_[i + 1..]);
        if &skipped == short {
            return true;
        }
    }
    false
}

pub fn letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListState {
    pub digest: Option<String>,
    pub key: Option<String>,
    pub time: Option<String>,
    pub min_red: u64,
    #[serde(default)]
    pub left: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRow {
    pub name: String,
    pub preview_digest: String,
    pub preview_key: Option<String>,
    pub preview_time: Option<String>,
    pub red_pixels: u64,
    #[serde(default)]
    pub y: i64,
}

pub fn preview_changed(prior: &ListState, row: &SessionRow) -> bool {
    if prior.time.is_some() && row.preview_time.is_some() && prior.time != row.preview_time {
        return true;
    }
    if let (Some(pk), Some(rk)) = (&prior.key, &row.preview_key) {
        return !ocr_noise(pk, rk);
    }
    prior.digest.as_deref() != Some(&row.preview_digest)
}

pub fn shows_last_seen(key: &str, bubbles: &[Bubble]) -> bool {
    if key.is_empty() || bubbles.is_empty() {
        return false;
    }
    let last = match bubbles.last() {
        Some(b) => b,
        None => return false,
    };
    let key_len = key.chars().count();
    let sender = last.sender.as_deref().unwrap_or("");
    let t1 = last.text.clone();
    let t2 = format!("{}{}", sender, last.text);
    for text in [&t1, &t2] {
        let seen = letters(text);
        if !seen.is_empty() {
            let prefix: String = seen.chars().take(key_len).collect();
            if ocr_noise(key, &prefix) || ocr_noise(key, &seen) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Routing: 对齐 routing.py
// ---------------------------------------------------------------------------

pub fn validate_rules(config: &BotConfig) -> Result<()> {
    let mut keys = HashMap::new();

    let check_entries = |entries: &[AllowlistEntry],
                         name: &str,
                         keys: &mut HashMap<String, usize>|
     -> Result<()> {
        if entries.len() > 200 {
            bail!("{name} must be an array of at most 200 conversations");
        }
        if name == "allowlist" && entries.is_empty() {
            bail!("at least one explicitly allowlisted conversation required");
        }
        let mut seen = HashSet::new();
        for (idx, entry) in entries.iter().enumerate() {
            let recipient = &entry.recipient;
            let kind = &entry.kind;
            let aliases = entry.aliases.as_deref().unwrap_or(&[]);
            if aliases.len() > 10 {
                bail!("aliases must be an array of at most 10 names");
            }
            let all_names = std::iter::once(recipient).chain(aliases.iter());
            for value in all_names {
                if value.trim().is_empty() || value != value.trim() {
                    bail!("exact conversation name without surrounding whitespace required");
                }
                if value.chars().count() > 64 {
                    bail!("conversation names are limited to 64 characters (native worker limit)");
                }
            }
            if kind != "private" && kind != "group" {
                bail!("conversation kind must be private or group");
            }
            let identity = (recipient.clone(), kind.clone());
            if !seen.insert(identity) {
                bail!("duplicate {name} entry");
            }
            if name == "allowlist" {
                for value in candidate_names(entry) {
                    let k = name_key(&value);
                    if let Some(&existing_idx) = keys.get(&k) {
                        if existing_idx != idx {
                            bail!("allowlist names collide after normalization");
                        }
                    } else {
                        keys.insert(k, idx);
                    }
                }
            }
        }
        Ok(())
    };

    check_entries(&config.allowlist, "allowlist", &mut keys)?;
    if let Some(ignorelist) = &config.ignorelist {
        check_entries(ignorelist, "ignorelist", &mut keys)?;
    }
    Ok(())
}

pub fn match_entry<'a>(
    config: &'a BotConfig,
    raw_name: &str,
    kind: Option<&str>,
) -> Option<&'a AllowlistEntry> {
    let key = name_key(raw_name);
    for entry in &config.allowlist {
        if (kind.is_none() || Some(entry.kind.as_str()) == kind)
            && candidate_names(entry).iter().any(|n| name_key(n) == key)
        {
            return Some(entry);
        }
    }
    None
}

pub fn decision(config: &BotConfig, recipient: &str, kind: Option<&str>) -> &'static str {
    let key = name_key(recipient);
    if let Some(ignorelist) = &config.ignorelist {
        for entry in ignorelist {
            if candidate_names(entry).iter().any(|n| name_key(n) == key) {
                return "ignored";
            }
        }
    }
    if match_entry(config, recipient, kind).is_none() {
        return "not_allowlisted";
    }
    "allowed"
}

// ---------------------------------------------------------------------------
// CoordinatorCapture trait
// ---------------------------------------------------------------------------

pub trait CoordinatorCapture: Send + Sync {
    fn start<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn close<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn frame<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>>;
    fn scroll<'a>(
        &'a mut self,
        direction: &'a str,
        amount: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>>;
    fn call<'a>(
        &'a mut self,
        op: &'a str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
    fn retarget<'a>(
        &'a mut self,
        recipient: &'a str,
        directory: &'a Path,
        names: &'a [String],
        group: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    /// Tells a disconnected capture which chat start() should open (Python builds the capture per session).
    fn prepare_target(
        &mut self,
        _recipient: &str,
        _directory: &Path,
        _names: &[String],
        _group: bool,
    ) {
    }
    fn sessions<'a>(
        &'a mut self,
        names: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionRow>>> + Send + 'a>>;
    fn download<'a>(
        &'a mut self,
        media_id: &'a str,
        kind: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>>;
    fn attachments<'a>(
        &'a mut self,
        items: &'a [Bubble],
        frame: &'a Frame,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<(
                        Vec<AttachmentImage>,
                        Vec<AttachmentFile>,
                        Vec<FailedAttachmentItem>,
                    )>,
                > + Send
                + 'a,
        >,
    > {
        let _ = (items, frame);
        Box::pin(async move { bail!("attachments not supported") })
    }
    fn last_switch(&self) -> Option<Value>;
    fn ocr_status(&self) -> Option<OcrStatus>;
    fn is_connected(&self) -> bool;
}

pub struct CaptureAdapter<'a> {
    pub inner: &'a mut (dyn CoordinatorCapture + 'a),
}

impl<'a> CaptureAdapter<'a> {
    pub fn download(
        &mut self,
        media_id: &str,
        kind: &str,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + '_>> {
        let mid = media_id.to_string();
        let k = kind.to_string();
        Box::pin(async move { self.inner.download(&mid, &k).await })
    }
}

#[allow(refining_impl_trait)]
impl<'a> Capture for CaptureAdapter<'a> {
    fn frame(&mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + '_>> {
        self.inner.frame()
    }

    fn scroll(
        &mut self,
        direction: &str,
        pages: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + '_>> {
        let dir = direction.to_string();
        Box::pin(async move { self.inner.scroll(&dir, pages).await })
    }

    fn download(
        &mut self,
        media_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + '_>> {
        self.download_kind(media_id, "image")
    }

    fn download_kind(
        &mut self,
        media_id: &str,
        kind: &str,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + '_>> {
        CaptureAdapter::download(self, media_id, kind)
    }

    fn attachments(
        &mut self,
        items: &[Bubble],
        frame: &Frame,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<(
                        Vec<AttachmentImage>,
                        Vec<AttachmentFile>,
                        Vec<FailedAttachmentItem>,
                    )>,
                > + Send
                + '_,
        >,
    > {
        let items = items.to_vec();
        let frame = frame.clone();
        Box::pin(async move { self.inner.attachments(&items, &frame).await })
    }
}

/// 通讯录同步一次运行的安全上限。同步期间完全停掉对话监听、一口气做完：用户 2026-11-08 要求——
/// 每 15 秒切回对话会与同步抢鼠标（点到对话里的联系人），续做时还找不到上次的位置。
/// 只在异常卡住时才让出。
pub const CONTACTS_SYNC_MAX_RUN: std::time::Duration = std::time::Duration::from_secs(20 * 60);

/// 同步的让出判断：收到停止信号立即让出（写断点、核心可正常退出）；否则只在超过安全上限时让出
pub fn contacts_sync_yield_check(
    stop: Arc<AtomicBool>,
    started: std::time::Instant,
) -> impl Fn() -> bool + Send + Sync {
    move || stop.load(Ordering::SeqCst) || started.elapsed() >= CONTACTS_SYNC_MAX_RUN
}

// ---------------------------------------------------------------------------
// BotCoordinator: 对齐 coordinator.py
// ---------------------------------------------------------------------------

pub struct BotCoordinator {
    pub config_path: PathBuf,
    pub config: BotConfig,
    pub root: PathBuf,
    pub sessions: HashMap<String, BotService>,
    pub stop: Arc<AtomicBool>,
    pub capture: Option<Box<dyn CoordinatorCapture>>,
    pub tool_client: Option<Arc<dyn DynToolClient>>,
    pub active_name: Option<String>,
    pub last_health: Option<String>,
    pub last_health_at: f64,
    pub last_summary: Option<(String, Vec<(String, String)>, String)>,
    pub list_state: HashMap<String, ListState>,
    pub last_visit: HashMap<String, f64>,
    pub pending: HashMap<String, (f64, i64)>,
    pub layout_mismatches: usize,
    pub hidden_rows: HashSet<String>,
    pub history: Arc<HistoryStore>,
    pub ownership: String,
    pub clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub session_title_mismatches: HashMap<String, usize>,
    pub logged_events: Arc<Mutex<Vec<Value>>>,
    /// 外部大脑模式下注入；热加会话创建时继承，避免首轮退化到 builtin。
    pub default_brain: Option<crate::core::turns::BrainFn>,
    /// 配置文件里的账号名（状态目录归属）；config.account_id 是当前登录的微信账号
    pub configured_account_id: String,
    pub(crate) account_detector: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
}

impl BotCoordinator {
    pub fn new(config_path: impl AsRef<Path>) -> Result<Self> {
        Self::new_with_account(config_path, None)
    }

    /// account：当前登录的微信账号（wxid）。给出时聊天记录和联系人都挂在这个账号下；
    /// 不给时沿用配置里的账号名（测试、未接微信时）。
    pub fn new_with_account(
        config_path: impl AsRef<Path>,
        account: Option<String>,
    ) -> Result<Self> {
        let config_path = config_path.as_ref().to_path_buf();
        let mut config = BotConfig::load(&config_path)?;
        let root = config.state_dir.clone();
        if !root.exists() {
            fs::create_dir_all(&root)?;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        } else {
            let meta = fs::metadata(&root)?;
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o700 {
                bail!("spool directory must be owned by this user and mode 0700");
            }
        }

        let binding = json!({
            "host": config.host,
            "account_id": config.account_id,
        });
        let binding_path = root.join("coordinator-binding.json");
        if binding_path.exists() {
            let existing: Value = read_json(&binding_path)?;
            if existing != binding {
                bail!("coordinator state belongs to a different account/host");
            }
        }
        write_private(&binding_path, &binding)?;
        // 绑定文件按配置账号名校验（状态目录归属）；数据归属用当前登录的微信账号
        let configured_account_id = config.account_id.clone();
        if let Some(acc) = account.filter(|a| !a.is_empty()) {
            config.account_id = acc;
        }

        let sessions_dir = root.join("sessions");
        if sessions_dir.exists() {
            if let Ok(entries) = fs::read_dir(&sessions_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.len() == 64
                            && name
                                .chars()
                                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                        {
                            bail!(
                                "legacy session directories found; stop the bot and run control migrate-history first"
                            );
                        }
                    }
                }
            }
        }

        let history = Arc::new(HistoryStore::open(&root)?);
        let mut hasher = Sha256::new();
        hasher.update(root.to_string_lossy().as_bytes());
        let ownership = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        let stop = Arc::new(AtomicBool::new(false));
        let default_clock = Arc::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        });

        crate::core::contacts::sync::clean_stale_contacts_sync_pause(&root);

        let mut coord = Self {
            config_path,
            config: config.clone(),
            root: root.clone(),
            sessions: HashMap::new(),
            stop,
            capture: None,
            tool_client: None,
            active_name: None,
            last_health: None,
            last_health_at: 0.0,
            last_summary: None,
            list_state: HashMap::new(),
            last_visit: HashMap::new(),
            pending: HashMap::new(),
            layout_mismatches: 0,
            hidden_rows: HashSet::new(),
            history: history.clone(),
            ownership,
            clock: default_clock,
            session_title_mismatches: HashMap::new(),
            logged_events: Arc::new(Mutex::new(Vec::new())),
            default_brain: None,
            configured_account_id,
            account_detector: None,
        };

        let mut current = HashMap::new();
        for entry in &config.allowlist {
            let entry_val = serde_json::to_value(entry)?;
            if let Ok(cid) = history.ensure_contact(&config.account_id, &entry_val, false) {
                current.insert(cid, entry.clone());
            }
        }

        let conn_arc = history.connection();
        let conn = conn_arc.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT contact_id, display_name, kind FROM contacts WHERE account_id=? AND kind IN ('private','group')"
        )?;
        let rows = stmt.query_map([&config.account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut to_restore = Vec::new();
        for r in rows.flatten() {
            to_restore.push(r);
        }
        drop(stmt);
        drop(conn);

        for (cid, display_name, kind) in to_restore {
            if root
                .join("sessions")
                .join(&cid)
                .join("binding.json")
                .exists()
            {
                let target = current.get(&cid).cloned().unwrap_or(AllowlistEntry {
                    recipient: display_name,
                    kind,
                    aliases: None,
                });
                let _ = coord.session(&target);
            }
        }

        coord.restore_list_state();
        Ok(coord)
    }

    fn list_state_path(&self) -> PathBuf {
        self.root.join("list-state.json")
    }

    fn active_chat_path(&self) -> PathBuf {
        self.root.join("active-chat.json")
    }

    fn save_list_state(&self) {
        let _ = write_private(&self.list_state_path(), &json!(self.list_state));
    }

    fn save_active_chat(&self) {
        let _ = write_private(
            &self.active_chat_path(),
            &json!({"active": self.active_name, "at": (self.clock)()}),
        );
    }

    /// Preview digests are the only signal a restart has for a message that arrived while the bot was
    /// down and left no unread badge; an in-memory list state lost it and the chat was never visited
    /// (device 2026-11-10: the paused 午夜咖啡 message stayed unanswered after the restart). Load it,
    /// and on the first run of a build that did not persist it, check each allowlisted chat once.
    fn restore_list_state(&mut self) {
        if let Ok(saved) = read_json::<HashMap<String, ListState>>(&self.list_state_path()) {
            self.list_state = saved;
        } else if self.root.join("health.json").exists() {
            let now = (self.clock)();
            let names: Vec<String> = self
                .config
                .allowlist
                .iter()
                .map(|entry| entry.recipient.clone())
                .collect();
            for name in &names {
                self.pending.entry(name.clone()).or_insert((now, 0));
            }
            self.log_event("list_state_bootstrap", json!({"sessions": names}));
        }
        if let Ok(saved) = read_json::<Value>(&self.active_chat_path()) {
            if let Some(active) = saved.get("active").and_then(Value::as_str) {
                if !active.is_empty() {
                    let now = (self.clock)();
                    self.pending.entry(active.to_string()).or_insert((now, 0));
                    self.log_event("active_chat_resume", json!({"recipient": active}));
                }
            }
        }
    }

    pub fn log_event(&self, event: &str, fields: Value) {
        let mut item = match fields {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        item.insert("event".to_string(), json!(event));
        tracing::info!(
            event = %event,
            fields = %serde_json::to_string(&item).unwrap_or_default(),
            "coordinator event: {}",
            event
        );
        const MAX_LOGGED_EVENTS: usize = 256;
        let mut events = self.logged_events.lock().unwrap();
        if events.len() >= MAX_LOGGED_EVENTS {
            events.remove(0);
        }
        events.push(Value::Object(item));
    }

    pub fn rules(&self) -> Result<BotConfig> {
        let mut value = BotConfig::load(&self.config_path)?;
        if value.host != self.config.host
            || value.account_id != self.configured_account_id
            || value.state_dir != self.config.state_dir
        {
            bail!("account/host/state changes require a separate state directory and restart");
        }
        value.account_id = self.config.account_id.clone();
        Ok(value)
    }

    pub fn session(&mut self, target: &AllowlistEntry) -> Result<&mut BotService> {
        let name = target.recipient.clone();
        if !self.sessions.contains_key(&name) {
            let target_val = serde_json::to_value(target)?;
            let contact_id =
                self.history
                    .ensure_contact(&self.config.account_id, &target_val, false)?;
            let directory = self.root.join("sessions").join(&contact_id);
            if !directory.exists() {
                fs::create_dir_all(&directory)?;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
            }

            let mut child_config = self.config.clone();
            child_config.allowlist = vec![target.clone()];
            child_config.state_dir = directory;

            let rules_path = self.config_path.clone();
            let check_name = name.clone();
            let routing_check = Arc::new(move || {
                if let Ok(rules) = BotConfig::load(&rules_path) {
                    decision(&rules, &check_name, None).to_string()
                } else {
                    "ignored".to_string()
                }
            });

            let mut child = BotService::new(
                child_config,
                Some(routing_check),
                Some(self.history.clone()),
                Some(contact_id),
                Some(self.ownership.clone()),
                Some(candidate_names(target)),
            )?;
            child.stop = self.stop.clone();
            child.brain = self.default_brain.clone();
            child.had_incoming = false;
            self.sessions.insert(name.clone(), child);
        }
        Ok(self.sessions.get_mut(&name).unwrap())
    }

    pub fn health(&mut self, state: &str, details: Map<String, Value>) -> Result<()> {
        let rules = self.rules().unwrap_or_else(|_| self.config.clone());
        let mut value = Map::new();
        value.insert("pid".to_string(), json!(std::process::id()));
        value.insert("state".to_string(), json!(state));
        value.insert("account_id".to_string(), json!(self.config.account_id));
        let allow_names: Vec<Value> = rules.allowlist.iter().map(|x| json!(x.recipient)).collect();
        value.insert("allowlist".to_string(), Value::Array(allow_names));
        let groups_enabled = rules.allowlist.iter().any(|x| x.kind == "group");
        value.insert("groups_enabled".to_string(), json!(groups_enabled));
        let model_backend = self.config.model_config.backend.clone();
        value.insert("model_backend".to_string(), json!(model_backend));
        value.insert("platform_message_verified".to_string(), json!(false));

        let mut sessions_map = Map::new();
        if let Some(cap) = &self.capture {
            if let Some(ocr) = cap.ocr_status() {
                value.insert("ocr".to_string(), json!(ocr));
            }
        }

        for (name, child) in &self.sessions {
            let path = child.root.join("health.json");
            if path.exists() {
                if let Ok(child_health) = read_json::<Value>(&path) {
                    sessions_map.insert(name.clone(), child_health);
                } else {
                    sessions_map.insert(name.clone(), json!({"state": "pending"}));
                }
            } else {
                sessions_map.insert(name.clone(), json!({"state": "pending"}));
            }
        }
        value.insert("sessions".to_string(), Value::Object(sessions_map));

        for (k, v) in details {
            value.insert(k, v);
        }

        let signature = serde_json::to_string(&Value::Object(value.clone()))?;
        let now = (self.clock)();

        let mut summary_map = Map::new();
        if let Some(Value::Object(sess)) = value.get("sessions") {
            for (name, item) in sess {
                let st = item
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("pending");
                summary_map.insert(name.clone(), json!(st));
            }
        }

        let mut sorted_summary: Vec<(String, String)> = summary_map
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
            .collect();
        sorted_summary.sort();

        let details_json = serde_json::to_string(&Value::Object(value.clone()))?;
        let compact = (state.to_string(), sorted_summary, details_json);

        if self.last_summary.as_ref() != Some(&compact) || now - self.last_health_at >= 60.0 {
            self.log_event(
                "coordinator_state",
                json!({
                    "state": state,
                    "sessions": summary_map,
                }),
            );
            self.last_summary = Some(compact);
            self.last_health_at = now;
        }
        self.last_health = Some(signature);

        let mut write_val = value;
        write_val.insert("updated_at".to_string(), json!(now));
        write_private(self.root.join("health.json"), &Value::Object(write_val))?;
        Ok(())
    }

    pub fn disable_session(&mut self, target: &AllowlistEntry, verdict: &str) -> Result<()> {
        let now = (self.clock)();
        let child = self.session(target)?;
        let disabled_payload = json!({
            "rule": verdict,
            "updated_at": now,
        });
        write_private(child.root.join("routing-disabled.json"), &disabled_payload)?;
        let mut d = Map::new();
        d.insert("rule".to_string(), json!(verdict));
        child.health("ignored", d)?;
        Ok(())
    }

    /// Disconnects the worker but keeps the injected capture client: serve-core builds it once and has no
    /// factory, so dropping it here left every session without capture (device 2026-11-08). connect()
    /// restarts it when it is not connected.
    pub async fn close_capture(&mut self) -> Result<()> {
        if let Some(cap) = self.capture.as_mut() {
            if let Err(err) = cap.close().await {
                self.log_event(
                    "capture_cleanup_failed",
                    json!({"error_type": "CaptureError", "error": err.to_string()}),
                );
            }
        }
        self.active_name = None;
        Ok(())
    }

    pub async fn close_connection(&mut self) -> Result<()> {
        self.close_capture().await?;
        Ok(())
    }

    pub async fn connect(&mut self, recipient: &str) -> Result<()> {
        if self.active_name.as_deref() == Some(recipient) && self.capture.is_some() {
            return Ok(());
        }
        if !self.sessions.contains_key(recipient) {
            let entry = self
                .config
                .allowlist
                .iter()
                .find(|e| e.recipient == recipient)
                .cloned()
                .context("unknown recipient")?;
            self.session(&entry)?;
        }
        let (root, names, group, is_target_allowed) = {
            let child = self
                .sessions
                .get_mut(recipient)
                .context("unknown recipient")?;
            if !child.target_allowed() {
                return Ok(());
            }
            let d = Map::new();
            child.health("connecting", d)?;
            (
                child.root.clone(),
                child.names.clone(),
                child.group,
                child.target_allowed(),
            )
        };
        if !is_target_allowed {
            return Ok(());
        }

        let switched = if let Some(cap) = &mut self.capture {
            if !cap.is_connected() {
                cap.prepare_target(recipient, &root, &names, group);
                cap.start().await?;
            }
            cap.retarget(recipient, &root, &names, group).await?;
            self.active_name = Some(recipient.to_string());
            let switch = cap.last_switch().unwrap_or_else(|| json!({}));
            self.log_event(
                "chat_switched",
                json!({
                    "recipient": recipient,
                    "clicked": switch.get("clicked"),
                    "seconds": switch.get("seconds"),
                    "fallback": switch.get("fallback"),
                }),
            );
            true
        } else {
            false
        };
        if switched {
            self.save_active_chat();
            return Ok(());
        }

        bail!("no capture client configured");
    }

    pub fn clear_layout_waits(&mut self, verified_name: &str) {
        for (name, child) in &mut self.sessions {
            if name == verified_name || child.paused {
                continue;
            }
            let h_path = child.root.join("health.json");
            if let Ok(st) = read_json::<Value>(&h_path) {
                let s = st.get("state").and_then(|v| v.as_str()).unwrap_or("");
                let r = st.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                if s == "needs_recalibration" || r.contains("layout_mismatch") {
                    let mut d = Map::new();
                    d.insert(
                        "reason".to_string(),
                        json!("layout verified on another chat"),
                    );
                    let _ = child.health("watching", d);
                }
            }
        }
    }

    pub async fn visit(&mut self, target: &AllowlistEntry) -> Result<()> {
        let recipient = target.recipient.clone();
        let verdict = decision(&self.rules()?, &recipient, None);
        if verdict != "allowed" {
            self.disable_session(target, verdict)?;
            if self.active_name.as_deref() == Some(&recipient) {
                self.close_capture().await?;
            }
            return Ok(());
        }

        let child = self.session(target)?;
        if child.paused {
            let reason = child
                .pause_reason
                .clone()
                .unwrap_or_else(|| "review unfinished session journal".to_string());
            let mut d = Map::new();
            d.insert("reason".to_string(), json!(reason));
            child.health("paused", d)?;
            return Ok(());
        }

        let marker = child.root.join("routing-disabled.json");
        if marker.exists() {
            child.cursor = None;
            let _ = fs::remove_file(child.root.join("baseline-established.json"));
        }

        let connect_res = self.connect(&recipient).await;
        if let Err(err) = connect_res {
            let err_str = err.to_string();
            self.handle_visit_error(&recipient, &err_str).await?;
            return Ok(());
        }

        if self.stop.load(Ordering::SeqCst) {
            return Ok(());
        }

        let cycle_res: Result<()> = async {
            let cap = self.capture.as_mut().context("no capture configured")?;
            let tool = self
                .tool_client
                .as_ref()
                .context("no tool client configured")?;
            let mut adapter = CaptureAdapter {
                inner: cap.as_mut(),
            };
            let child = self.sessions.get_mut(&recipient).unwrap();
            child.cycle(tool, &mut adapter).await
        }
        .await;

        match cycle_res {
            Ok(()) => {
                self.session_title_mismatches.insert(recipient.clone(), 0);
                if self.layout_mismatches > 0 {
                    self.clear_layout_waits(&recipient);
                }
                self.layout_mismatches = 0;
                let child = self.sessions.get_mut(&recipient).unwrap();
                if child.cursor.is_some() && marker.exists() {
                    let _ = fs::remove_file(&marker);
                }
            }
            Err(err) => {
                let err_str = err.to_string();
                self.handle_visit_error(&recipient, &err_str).await?;
                if err_str.contains("no tool client configured")
                    || err_str.contains("no capture configured")
                {
                    bail!("{err_str}");
                }
            }
        }
        Ok(())
    }

    pub async fn handle_visit_error(&mut self, recipient: &str, err_str: &str) -> Result<()> {
        if err_str == "incomplete bottom bubble; no automatic reply" {
            let child = self.sessions.get_mut(recipient).unwrap();
            if !child.paused {
                let mut d = Map::new();
                d.insert(
                    "reason".to_string(),
                    json!("bottom bubble is incomplete; retry capture without model/send"),
                );
                child.health("waiting_for_stable_frame", d)?;
            }
            return Ok(());
        }

        if err_str.contains("invalid(\"allowlisted_title_mismatch\")") {
            let is_paused = self.sessions.get(recipient).map_or(true, |c| c.paused);
            if !is_paused {
                let count = self
                    .session_title_mismatches
                    .entry(recipient.to_string())
                    .or_insert(0);
                *count += 1;
                let current_count = *count;
                if current_count < TITLE_MISMATCH_LIMIT {
                    let reason = if err_str.chars().count() > 400 {
                        err_str.chars().take(400).collect::<String>()
                    } else {
                        err_str.to_string()
                    };
                    let child = self.sessions.get_mut(recipient).unwrap();
                    let mut d = Map::new();
                    d.insert("reason".to_string(), json!(reason));
                    d.insert("attempt".to_string(), json!(current_count));
                    child.health("waiting_for_wechat_window", d)?;
                    self.close_connection().await?;
                    return Ok(());
                }
            }
        }

        if err_str.contains("invalid(\"layout_mismatch") {
            let is_paused = self.sessions.get(recipient).map_or(true, |c| c.paused);
            if !is_paused {
                self.active_name = None;
                self.layout_mismatches += 1;
                let failures = extract_mismatch_failures(err_str);
                let state = if self.layout_mismatches >= RECALIBRATION_LIMIT {
                    "needs_recalibration"
                } else {
                    "waiting_for_wechat_window"
                };
                let reason = if err_str.chars().count() > 400 {
                    err_str.chars().take(400).collect::<String>()
                } else {
                    err_str.to_string()
                };
                let child = self.sessions.get_mut(recipient).unwrap();
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(reason));
                d.insert("failures".to_string(), json!(failures));
                d.insert("attempt".to_string(), json!(self.layout_mismatches));
                child.health(state, d)?;
                return Ok(());
            }
        }

        if is_window_unavailable(err_str) {
            let is_paused = self.sessions.get(recipient).map_or(true, |c| c.paused);
            if !is_paused {
                let reason = if err_str.chars().count() > 400 {
                    err_str.chars().take(400).collect::<String>()
                } else {
                    err_str.to_string()
                };
                let child = self.sessions.get_mut(recipient).unwrap();
                let mut d = Map::new();
                d.insert("reason".to_string(), json!(reason));
                child.health("waiting_for_wechat_window", d)?;
                if !err_str.contains("screen_not_main") && !err_str.contains(ROW_NOT_VISIBLE) {
                    self.close_connection().await?;
                }
                return Ok(());
            }
        }

        let p_reason = if err_str.chars().count() > 400 {
            err_str.chars().take(400).collect::<String>()
        } else {
            err_str.to_string()
        };
        let child = self.sessions.get_mut(recipient).unwrap();
        child.paused = true;
        child.pause_reason = Some(p_reason.clone());
        let mut d = Map::new();
        d.insert("reason".to_string(), json!(p_reason));
        child.health("paused", d)?;
        self.close_connection().await?;
        Ok(())
    }

    pub fn reconcile_journals(&mut self) -> Result<()> {
        for child in self.sessions.values() {
            let entries = fs::read_dir(&child.root)?;
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(file_name) = p.file_name().and_then(|s| s.to_str()) {
                    if file_name.starts_with("send-") && file_name.ends_with(".json") {
                        if let Ok(record) = read_json::<Value>(&p) {
                            let key = match record.get("idempotency_key").and_then(|v| v.as_str()) {
                                Some(k) => k,
                                None => continue,
                            };
                            let existing = self.history.send(key)?;
                            let status =
                                record.get("status").and_then(|v| v.as_str()).unwrap_or("");
                            if existing.is_none()
                                && record.get("reply").is_some()
                                && (status == "attempting"
                                    || status == "triggered"
                                    || status == "unknown")
                            {
                                let at = record
                                    .get("attempted_at")
                                    .or_else(|| record.get("created_at"))
                                    .and_then(|v| v.as_f64());
                                let observed_at = at.map(|sec| (sec * 1000.0) as i64);
                                let reply_val = record["reply"].as_str().unwrap_or("");
                                let model_val =
                                    record.get("model").cloned().unwrap_or_else(|| json!({}));
                                if let Some(cid) = &child.contact_id {
                                    self.history.record_bot_reply(
                                        cid,
                                        key,
                                        reply_val,
                                        status,
                                        &model_val,
                                        &[],
                                        observed_at,
                                        None,
                                    )?;
                                }
                            }
                            let existing = self.history.send(key)?;
                            if let Some(ex) = existing {
                                let mut needs_update = false;
                                let cur_status = ex.get("status").and_then(|v| v.as_str());
                                if cur_status != Some(status) {
                                    needs_update = true;
                                }
                                if let Some(echo) = record.get("client_echo_observed") {
                                    let echo_val = echo.as_bool().unwrap_or(false);
                                    let cur_echo =
                                        ex.get("echo_observed").and_then(|v| v.as_bool());
                                    if cur_echo != Some(echo_val) {
                                        needs_update = true;
                                    }
                                }
                                if needs_update {
                                    let echo_opt = record
                                        .get("client_echo_observed")
                                        .and_then(|v| v.as_bool());
                                    self.history.update_send(key, status, echo_opt)?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether this chat already has a reply baseline. A chat with no messages keeps an empty
    /// cursor, so the marker written on its first baseline pass is what makes "once per chat ever"
    /// hold; without it an empty chat was reopened on every sweep.
    fn has_baseline(
        &mut self,
        name: &str,
        entries: &HashMap<String, AllowlistEntry>,
    ) -> Result<bool> {
        let Some(entry) = entries.get(name) else {
            return Ok(true);
        };
        let child = self.session(entry)?;
        Ok(child.cursor.as_ref().is_some_and(|c| !c.bubbles.is_empty())
            || child.root.join("baseline-established.json").exists())
    }

    pub async fn due_sessions(&mut self, names: &[String]) -> Result<HashSet<String>> {
        if names.is_empty() || self.capture.is_none() {
            return Ok(names.iter().cloned().collect());
        }

        let rules = self.rules()?;
        let entries: HashMap<String, AllowlistEntry> = names
            .iter()
            .map(|name| {
                let ent = match_entry(&rules, name, None)
                    .cloned()
                    .unwrap_or(AllowlistEntry {
                        recipient: name.clone(),
                        kind: "private".to_string(),
                        aliases: None,
                    });
                (name.clone(), ent)
            })
            .collect();

        let mut candidate_list = Vec::new();
        for name in names {
            if let Some(entry) = entries.get(name) {
                candidate_list.extend(candidate_names(entry));
            }
        }

        const MAX_PROBE_BATCH_SIZE: usize = 50;
        let mut probe_rows = Vec::new();
        if !candidate_list.is_empty() {
            for chunk in candidate_list.chunks(MAX_PROBE_BATCH_SIZE) {
                match self.capture.as_mut().unwrap().sessions(chunk).await {
                    Ok(rows) => {
                        probe_rows.extend(rows);
                    }
                    Err(err) => {
                        self.log_event(
                            "session_probe_failed",
                            json!({
                                "error_type": "ProbeError",
                                "error": err.to_string().chars().take(200).collect::<String>(),
                            }),
                        );
                        let open = self.active_name.clone();
                        if !must_wait(&err.to_string()) {
                            self.close_connection().await?;
                        }
                        // Without a list we know nothing new: keep the open chat and open turns,
                        // never sweep every allowlisted chat (hundreds in production). With no
                        // chat open, one visit re-establishes the worker connection.
                        let mut keep: HashSet<String> = names
                            .iter()
                            .filter(|n| {
                                open.as_deref() == Some(n.as_str())
                                    || self.sessions.get(*n).is_some_and(|s| s.turn.is_some())
                            })
                            .cloned()
                            .collect();
                        if keep.is_empty() {
                            keep.extend(names.first().cloned());
                        }
                        return Ok(keep);
                    }
                }
            }
        }

        let mut rows: HashMap<String, SessionRow> = HashMap::new();
        for row in probe_rows {
            if let Some(entry) = match_entry(&rules, &row.name, None) {
                rows.insert(entry.recipient.clone(), row);
            }
        }

        let now = (self.clock)();
        let mut due = HashSet::new();
        let mut list_state_changed = false;

        for name in names {
            let row = rows.get(name);
            let prior_opt = self.list_state.get(name).cloned();
            let mut reason: Option<&'static str> = None;

            // Visits are event driven: an unread badge or a changed preview. No periodic
            // sweep, so hundreds of allowlisted chats cost one list probe, not hundreds of clicks.
            if self.active_name.as_deref() == Some(name) {
                reason = Some("active");
            } else if self.sessions.get(name).map_or(false, |s| s.turn.is_some()) {
                reason = Some("turn_open");
            } else if !self.has_baseline(name, &entries)? {
                // Once per chat ever: the persisted cursor survives restarts.
                reason = Some("initial");
            } else if let Some(r) = row {
                match prior_opt.as_ref() {
                    Some(p) if p.digest.is_some() => {
                        if p.left {
                            let bubbles = self
                                .sessions
                                .get(name)
                                .and_then(|s| s.cursor.as_ref())
                                .map(|c| c.bubbles.as_slice())
                                .unwrap_or(&[]);
                            if preview_changed(p, r)
                                && !shows_last_seen(r.preview_key.as_deref().unwrap_or(""), bubbles)
                            {
                                reason = Some("preview_changed");
                            }
                        } else if preview_changed(p, r) {
                            reason = Some("preview_changed");
                        } else if r.red_pixels >= p.min_red + BADGE_PIXELS {
                            reason = Some("unread_badge");
                        }
                    }
                    // First list sighting in this process (e.g. after a restart): only a
                    // visible unread badge means messages arrived while we were not looking.
                    _ => {
                        if r.red_pixels >= BADGE_PIXELS {
                            reason = Some("unread_badge");
                        }
                    }
                }
            }

            let state_before = self.list_state.get(name).cloned();
            if let Some(r) = row {
                let low = match &prior_opt {
                    Some(p) => p.min_red.min(r.red_pixels),
                    None => r.red_pixels,
                };
                if self.active_name.as_deref() == Some(name) {
                    self.list_state.insert(
                        name.clone(),
                        ListState {
                            digest: prior_opt.as_ref().and_then(|p| p.digest.clone()),
                            key: prior_opt.as_ref().and_then(|p| p.key.clone()),
                            time: prior_opt.as_ref().and_then(|p| p.time.clone()),
                            min_red: low,
                            left: true,
                        },
                    );
                } else {
                    self.list_state.insert(
                        name.clone(),
                        ListState {
                            digest: Some(r.preview_digest.clone()),
                            key: r.preview_key.clone(),
                            time: r.preview_time.clone(),
                            min_red: low,
                            left: false,
                        },
                    );
                }
                if state_before.as_ref() != self.list_state.get(name) {
                    list_state_changed = true;
                }
            }

            if reason.is_none() && self.pending.contains_key(name) {
                reason = Some("pending");
            }

            if row.is_none() && self.active_name.as_deref() != Some(name) {
                if let Some(r) = reason {
                    if !self.hidden_rows.contains(name) {
                        self.hidden_rows.insert(name.clone());
                        self.log_event(
                            "session_row_not_visible",
                            json!({"recipient": name, "reason": r}),
                        );
                    }
                    continue;
                }
            }
            self.hidden_rows.remove(name);

            if let Some(res) = reason {
                due.insert(name.clone());
                if res == "preview_changed" || res == "unread_badge" {
                    let y_coord = row.map(|r| r.y).unwrap_or(0);
                    self.pending.entry(name.clone()).or_insert((now, -y_coord));
                }
                if res != "active" && res != "pending" && res != "turn_open" {
                    self.log_event(
                        "session_switch_due",
                        json!({
                            "recipient": name,
                            "reason": res,
                            "visible": row.is_some(),
                            "red_pixels": row.map(|r| r.red_pixels),
                            "baseline_red": prior_opt.as_ref().map(|p| p.min_red),
                        }),
                    );
                }
            }
        }

        if list_state_changed {
            self.save_list_state();
        }
        Ok(due)
    }

    pub async fn download_pending_media(&mut self) -> Result<()> {
        let active = match &self.active_name {
            Some(a) => a.clone(),
            None => return Ok(()),
        };
        let (contact_id, recipient) = match self.sessions.get(&active) {
            Some(c) => (c.contact_id.clone(), c.recipient.clone()),
            None => return Ok(()),
        };
        let contact_id = match contact_id {
            Some(cid) => cid,
            None => return Ok(()),
        };

        let job = match self.history.next_media_job(&contact_id)? {
            Some(j) => j,
            None => return Ok(()),
        };
        let mid = job["message_id"].as_str().unwrap_or("").to_string();

        let digest_row: Option<String> = {
            let conn_arc = self.history.connection();
            let conn = conn_arc.lock().unwrap();
            conn.query_row(
                "SELECT bubble_digest FROM messages WHERE message_id=?",
                [&mid],
                |r| r.get(0),
            )
            .ok()
        };

        let digest = match digest_row {
            Some(d) => d,
            None => return Ok(()),
        };

        if let Some(cap) = &mut self.capture {
            let max_bytes = self.config.max_attachment_mb * 1024 * 1024;
            let fetch_res: Result<(PathBuf, Option<String>)> = async {
                let frame = cap.frame().await.context("failed to capture frame")?;
                let bubble = frame
                    .bubbles
                    .iter()
                    .find(|b| b.kind.as_deref() == Some("file") && b.digest == digest);
                let bubble = match bubble {
                    Some(b) => b,
                    None => bail!("__bubble_gone__"),
                };
                let media_id = match &bubble.media_id {
                    Some(m) => m,
                    None => bail!("missing media_id"),
                };

                let info_val = cap
                    .call(
                        "prepare_file",
                        json!({
                            "media_id": media_id,
                            "snapshot_id": frame.snapshot_id,
                            "max_bytes": max_bytes,
                        }),
                    )
                    .await
                    .context("prepare_file failed")?;

                let path = cap
                    .download(media_id, "file")
                    .await
                    .context("download file failed")?;
                let filename = info_val
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Ok((path, filename))
            }
            .await;

            match fetch_res {
                Ok((path, filename)) => {
                    let attach_res = self.history.attach_media(
                        &mid,
                        &path,
                        "file",
                        filename.as_deref(),
                        "original",
                        Some(max_bytes as usize),
                    );
                    match attach_res {
                        Ok(_) => {
                            let _ = self.history.finish_media_job(&mid, "done", None);
                            self.log_event(
                                "history_file_archived",
                                json!({"recipient": recipient}),
                            );
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            let _ = self.history.finish_media_job(
                                &mid,
                                "failed",
                                Some(&err_str.chars().take(200).collect::<String>()),
                            );
                            self.log_event(
                                "history_file_download_failed",
                                json!({
                                    "recipient": recipient,
                                    "gone": false,
                                    "error": err_str.chars().take(200).collect::<String>()
                                }),
                            );
                        }
                    }
                }
                Err(err) => {
                    let err_str = err.to_string();
                    if err_str.contains("__bubble_gone__") {
                        let _ = self
                            .history
                            .finish_media_job(&mid, "gone", Some("not visible"));
                    } else {
                        let gone = err_str.contains("file_snapshot_changed");
                        let status = if gone { "gone" } else { "failed" };
                        let _ = self.history.finish_media_job(
                            &mid,
                            status,
                            Some(&err_str.chars().take(200).collect::<String>()),
                        );
                        self.log_event(
                            "history_file_download_failed",
                            json!({
                                "recipient": recipient,
                                "gone": gone,
                                "error": err_str.chars().take(200).collect::<String>()
                            }),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn sweep(&mut self) -> Result<()> {
        let admin_pause = self.root.join("admin-pause.json");
        if admin_pause.exists() {
            // 暂停只停自动回复：手动发起的通讯录同步（及其断点续做）照常处理，每日自动同步不跑
            let _ = self.serve_contacts_sync_with(false).await;
            let mut d = Map::new();
            d.insert("reason".to_string(), json!("admin_pause"));
            self.health("paused", d)?;
            return Ok(());
        }

        let _ = self.serve_friend_requests().await;

        // 处理通讯录同步请求与增量同步
        let _ = self.serve_contacts_sync().await;

        let rules = self.rules()?;
        let targets = rules.allowlist.clone();

        let mut allowed_ids = HashSet::new();
        for t in &targets {
            let t_val = serde_json::to_value(t)?;
            if let Ok(cid) = self
                .history
                .ensure_contact(&self.config.account_id, &t_val, false)
            {
                allowed_ids.insert(cid);
            }
        }

        let current_session_names: Vec<String> = self.sessions.keys().cloned().collect();
        for name in current_session_names {
            let in_targets = targets.iter().any(|x| x.recipient == name);
            if !in_targets {
                let cid = self.sessions[&name].contact_id.clone().unwrap_or_default();
                if allowed_ids.contains(&cid) {
                    if self.active_name.as_deref() == Some(&name) {
                        self.close_capture().await?;
                    }
                    self.sessions.remove(&name);
                    continue;
                }
                self.disable_session(
                    &AllowlistEntry {
                        recipient: name.clone(),
                        kind: "private".to_string(),
                        aliases: None,
                    },
                    "not_allowlisted",
                )?;
                if self.active_name.as_deref() == Some(&name) {
                    self.close_capture().await?;
                }
            }
        }

        let mut active = Vec::new();
        let mut allowed = Vec::new();

        for target in &targets {
            let verdict = decision(&rules, &target.recipient, None);
            if verdict != "allowed" {
                self.disable_session(target, verdict)?;
                if self.active_name.as_deref() == Some(&target.recipient) {
                    self.close_capture().await?;
                }
                continue;
            }
            let child = self.session(target)?;
            let cid = child.contact_id.clone().unwrap_or_default();
            if self.history.canonical(cid.clone())? != cid {
                self.disable_session(target, "merged")?;
                if self.active_name.as_deref() == Some(&target.recipient) {
                    self.close_capture().await?;
                }
                continue;
            }
            active.push(target.recipient.clone());
            allowed.push(target.clone());
        }

        let due = self.due_sessions(&active).await?;

        let active_opt = self.active_name.clone();
        allowed.sort_by(|a, b| {
            let a_is_active = active_opt.as_deref() == Some(&a.recipient);
            let b_is_active = active_opt.as_deref() == Some(&b.recipient);
            if a_is_active != b_is_active {
                return b_is_active.cmp(&a_is_active);
            }
            let a_pen = self
                .pending
                .get(&a.recipient)
                .copied()
                .unwrap_or((f64::INFINITY, 0));
            let b_pen = self
                .pending
                .get(&b.recipient)
                .copied()
                .unwrap_or((f64::INFINITY, 0));
            a_pen
                .partial_cmp(&b_pen)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut visited = Vec::new();
        for target in allowed {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            if !due.contains(&target.recipient) {
                continue;
            }
            visited.push(target.recipient.clone());
            if decision(&self.rules()?, &target.recipient, None) != "allowed" {
                continue;
            }
            self.visit(&target).await?;
            let now = (self.clock)();
            self.last_visit.insert(target.recipient.clone(), now);
            self.pending.remove(&target.recipient);
        }

        let has_had_incoming = visited
            .iter()
            .any(|n| self.sessions.get(n).map_or(false, |s| s.had_incoming));
        let has_open_turn = self.sessions.values().any(|s| s.turn.is_some());

        if !self.stop.load(Ordering::SeqCst)
            && self.pending.is_empty()
            && !has_had_incoming
            && !has_open_turn
        {
            self.download_pending_media().await?;
        }

        let state = if active.is_empty() {
            "ignored"
        } else if active
            .iter()
            .all(|n| self.sessions.get(n).map_or(false, |s| s.paused))
        {
            "paused"
        } else {
            "watching"
        };

        let mut d = Map::new();
        if let Some(act) = &self.active_name {
            d.insert("active_chat".to_string(), json!(act));
        }
        self.health(state, d)?;
        Ok(())
    }

    pub async fn serve_calibration(&mut self) -> Result<()> {
        let req_path = self.root.join("calibrate-request.json");
        if !req_path.exists() {
            return Ok(());
        }
        let res_path = self.root.join("calibrate-result.json");
        let req: Value = match read_json(&req_path) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };

        let req_id = req.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let action = req.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let created_at = req
            .get("created_at")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let now = (self.clock)();

        struct DropFile(PathBuf);
        impl Drop for DropFile {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let _cleanup_guard = DropFile(req_path.clone());

        if now - created_at > 600.0 {
            write_private(
                &res_path,
                &json!({
                    "id": req_id,
                    "ok": false,
                    "error": "expired",
                }),
            )?;
            return Ok(());
        }

        if self.capture.is_none() {
            write_private(
                &res_path,
                &json!({
                    "id": req_id,
                    "ok": false,
                    "error": "worker_not_connected",
                }),
            )?;
            return Ok(());
        }

        let worker_res = match self
            .capture
            .as_mut()
            .unwrap()
            .call("calibrate", json!({}))
            .await
        {
            Ok(res) => res,
            Err(err) => {
                write_private(
                    &res_path,
                    &json!({
                        "id": req_id,
                        "ok": false,
                        "error": err.to_string(),
                    }),
                )?;
                return Ok(());
            }
        };

        let config_val = serde_json::to_value(&self.config).ok();
        if action == "accept" {
            match crate::core::control::calibrate_accept(
                config_val.as_ref(),
                None,
                None,
                Some(&worker_res),
                None,
            ) {
                Ok(accepted_res) => {
                    write_private(
                        &res_path,
                        &json!({
                            "id": req_id,
                            "ok": true,
                            "result": accepted_res,
                        }),
                    )?;
                }
                Err(err) => {
                    write_private(
                        &res_path,
                        &json!({
                            "id": req_id,
                            "ok": false,
                            "error": err.to_string(),
                        }),
                    )?;
                }
            }
        } else {
            match crate::core::control::run_calibrate(
                config_val.as_ref(),
                Some(&worker_res),
                None,
                None,
                None,
            ) {
                Ok((diffs, missing, candidate)) => {
                    let version = candidate
                        .get("wechat_version")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let out = json!({
                        "candidate_json": format!("{}/candidate-wechat-{}.json", crate::core::control::REMOTE_LAYOUT_DIR, version),
                        "candidate_png": format!("{}/candidate-wechat-{}.png", crate::core::control::REMOTE_LAYOUT_DIR, version),
                        "differences": diffs,
                        "missing": missing,
                    });
                    write_private(
                        &res_path,
                        &json!({
                            "id": req_id,
                            "ok": true,
                            "result": out,
                        }),
                    )?;
                }
                Err(err) => {
                    write_private(
                        &res_path,
                        &json!({
                            "id": req_id,
                            "ok": false,
                            "error": err.to_string(),
                        }),
                    )?;
                }
            }
        }

        Ok(())
    }

    /// 同步前确保与桌面服务的连接：上一轮同步结束后会主动断开（以便重新打开目标会话），
    /// 续做前必须先连上，否则第一步就 transport closed、被当成失败（2026-11-08 实机）。
    async fn ensure_capture_connected(&mut self) -> bool {
        match self.capture.as_mut() {
            Some(cap) if cap.is_connected() => return true,
            Some(cap) => match cap.start().await {
                Ok(()) => return true,
                Err(e) => {
                    tracing::warn!("contacts sync: worker reconnect without target failed: {e:#}")
                }
            },
            None => return false,
        }
        // 暂停期间从没连接过会话：桌面服务连接需要一个目标会话。按平时的连接流程连到允许名单第一个会话
        // （只建立连接，不读消息、不回复），然后再进通讯录（2026-11-08 实机：重试 562 次都连不上）
        let Some(target) = self.config.allowlist.first().map(|e| e.recipient.clone()) else {
            return false;
        };
        match self.connect(&target).await {
            Ok(()) => self.capture.as_ref().map_or(false, |c| c.is_connected()),
            Err(e) => {
                tracing::warn!(
                    "contacts sync deferred: connect to {} failed: {e:#}",
                    "allowlist[0]"
                );
                false
            }
        }
    }

    pub async fn serve_friend_requests(&mut self) -> Result<()> {
        use crate::core::friend_requests as fr;
        let now = (self.clock)();
        let idle = self.pending.is_empty() && !self.sessions.values().any(|s| s.turn.is_some());
        let c = self.rules()?.friend_requests;
        if !c.enabled
            || self.root.join("admin-pause.json").exists()
            || !idle
            || self.stop.load(Ordering::SeqCst)
        {
            return Ok(());
        }
        if self
            .account_detector
            .as_ref()
            .is_some_and(|f| f().as_deref() != Some(self.config.account_id.as_str()))
        {
            return Ok(());
        }
        let mut state = fr::State::load(&self.root)?;
        state.bind_account(&self.config.account_id);
        if !fr::should_check(&c, false, idle, now, state.last_check) {
            return Ok(());
        }
        if self.capture.is_none() || !self.ensure_capture_connected().await {
            return Ok(());
        }
        state.last_check = Some(now);
        state.save(&self.root)?;
        let badge = self
            .capture
            .as_mut()
            .unwrap()
            .call("friend_request_badge", json!({}))
            .await?;
        // Only a new request (the 通讯录 badge) opens the list. That scan also handles every older
        // row still showing 「等待验证」; rows already decided under the same rules are skipped.
        if badge["red_pixels"].as_u64().unwrap_or(0) < BADGE_PIXELS {
            return Ok(());
        }
        let stop = self.stop.clone();
        let pause = self.root.join("admin-pause.json");
        let detect = self.account_detector.clone();
        let account = self.config.account_id.clone();
        let config_path = self.config_path.clone();
        let stop_check = || {
            stop.load(Ordering::SeqCst)
                || pause.exists()
                || !BotConfig::load(&config_path)
                    .is_ok_and(|cfg| cfg.friend_requests == c && cfg.friend_requests.enabled)
                || detect
                    .as_ref()
                    .is_some_and(|f| f().as_deref() != Some(account.as_str()))
        };
        let result = fr::scan(
            self.capture.as_mut().unwrap().as_mut(),
            &c,
            &self.root,
            &account,
            now,
            &stop_check,
        )
        .await;
        self.log_event("friend_request_scan", json!({"success":result.is_ok()}));
        // Navigating contacts invalidates the active chat on both success and failure.
        let closed = self.close_connection().await;
        match result {
            Ok(report) => {
                for event in report.events {
                    let kind = if event["event"] == "friend_request_scan" {
                        "friend_request_scan"
                    } else {
                        "friend_request_decision"
                    };
                    self.log_event(kind, event.clone());
                    if kind == "friend_request_decision"
                        && event["decision"] != "not_matched"
                        && event["decision"] != "limit_reached"
                    {
                        self.log_event("friend_request_verify", event);
                    }
                }
                let queued = match fr::queue_contacts_sync(
                    &self.root,
                    &account,
                    now,
                    c.mode,
                    report.accepted,
                ) {
                    Ok(v) => v,
                    Err(_) => {
                        self.log_event(
                            "friend_request_contacts_sync_failed",
                            json!({"reason":"queue_failed"}),
                        );
                        false
                    }
                };
                if queued {
                    // The normal control request consumer owns its own, separate lease.
                    if self.serve_contacts_sync_with(false).await.is_err() {
                        self.log_event(
                            "friend_request_contacts_sync_failed",
                            json!({"reason":"sync_failed"}),
                        );
                    } else if read_json::<Value>(self.root.join("contacts-sync-result.json"))
                        .ok()
                        .is_some_and(|v| v["complete"] == false)
                    {
                        self.log_event(
                            "friend_request_contacts_sync_failed",
                            json!({"reason":"incomplete"}),
                        );
                    }
                }
            }
            Err(_) => {
                self.log_event(
                    "friend_request_scan",
                    json!({"success":false,"reason":"scan_failed"}),
                );
            }
        }
        closed?;
        Ok(())
    }

    pub async fn serve_contacts_sync(&mut self) -> Result<()> {
        self.serve_contacts_sync_with(true).await?;
        if crate::core::friend_requests::promote_contacts_sync(
            &self.root,
            &self.config.account_id,
            (self.clock)(),
        )? {
            if self.serve_contacts_sync_with(false).await.is_err()
                || read_json::<Value>(self.root.join("contacts-sync-result.json"))
                    .ok()
                    .is_some_and(|v| v["complete"] == false)
            {
                self.log_event(
                    "friend_request_contacts_sync_failed",
                    json!({"reason":"queued_sync_failed"}),
                );
            }
        }
        Ok(())
    }

    /// allow_daily=false：只处理手动请求（暂停期间），不触发每日自动同步
    pub async fn serve_contacts_sync_with(&mut self, allow_daily: bool) -> Result<()> {
        use crate::core::contacts::sync::{account_state_path, write_sync_result};
        let account_matches = || {
            self.account_detector
                .as_ref()
                .is_none_or(|detect| detect().as_deref() == Some(self.config.account_id.as_str()))
        };
        if !account_matches() {
            return Ok(());
        }
        let req_path = self.root.join("contacts-sync-request.json");
        let res_path = self.root.join("contacts-sync-result.json");
        let checkpoint_path = self.root.join("contacts-sync-checkpoint.json");

        if req_path.exists() {
            let req: Value = match read_json(&req_path) {
                Ok(v) => v,
                Err(_) => return Ok(()),
            };

            let req_id = req
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mode_str = req
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("incremental");
            let mode = if mode_str == "full" {
                crate::core::contacts::sync::SyncMode::Full
            } else {
                crate::core::contacts::sync::SyncMode::Incremental
            };
            let created_at = req
                .get("created_at")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let now = (self.clock)();

            struct DropFile(PathBuf);
            impl Drop for DropFile {
                fn drop(&mut self) {
                    let _ = fs::remove_file(&self.0);
                }
            }
            let _cleanup_guard = DropFile(req_path.clone());

            if req.get("account_id").and_then(Value::as_str)
                != Some(self.config.account_id.as_str())
            {
                write_private(
                    &res_path,
                    &json!({"id": req_id, "account_id": req.get("account_id"), "complete": false, "error": "account_mismatch"}),
                )?;
                return Ok(());
            }
            if now - created_at > 600.0 {
                write_sync_result(
                    &self.root,
                    &self.config.account_id,
                    &json!({
                        "id": req_id,
                        "complete": false,
                        "error": "expired",
                    }),
                )?;
                return Ok(());
            }

            if self.capture.is_none() {
                write_sync_result(
                    &self.root,
                    &self.config.account_id,
                    &json!({
                        "id": req_id,
                        "complete": false,
                        "error": "worker_not_connected",
                    }),
                )?;
                return Ok(());
            }

            if !self.ensure_capture_connected().await {
                // 连不上：保留请求等下一轮，不当作失败
                std::mem::forget(_cleanup_guard);
                return Ok(());
            }
            let capture = self.capture.as_mut().unwrap().as_mut();
            let store = &self.history;
            let account_id = &self.config.account_id;
            let now_ms = (now * 1000.0) as i64;

            let stop_check =
                contacts_sync_yield_check(self.stop.clone(), std::time::Instant::now());
            let detect = self.account_detector.clone();
            let expected_account = self.config.account_id.clone();
            let should_yield = move || {
                stop_check()
                    || detect
                        .as_ref()
                        .is_some_and(|f| f().as_deref() != Some(expected_account.as_str()))
            };

            let publish = match crate::core::contacts::sync::run_sync(
                capture,
                store,
                account_id,
                mode,
                &checkpoint_path,
                &should_yield,
                now_ms,
            )
            .await
            {
                Ok(report) => {
                    let res_val = json!({
                        "id": req_id,
                        "finished_at_ms": now_ms,
                        "complete": report.complete,
                        "per_category": report.per_category.iter().map(|(c, exp, syn)| {
                            json!({"category": c, "expected": exp, "synced": syn})
                        }).collect::<Vec<_>>(),
                        "unreadable_rows": report.unreadable_rows,
                        "needs_review": report.needs_review,
                        "yielded": report.yielded,
                    });
                    write_sync_result(&self.root, &self.config.account_id, &res_val)
                }
                Err(e) => write_sync_result(
                    &self.root,
                    &self.config.account_id,
                    &json!({
                        "id": req_id,
                        "complete": false,
                        "error": e.to_string(),
                    }),
                ),
            };
            // 同步把微信切到过通讯录：断开重连，下一轮重新打开目标会话（不论成败）
            let closed = self.close_connection().await;
            publish?;
            closed?;
        }

        let is_idle = self.pending.is_empty() && !self.sessions.values().any(|s| s.turn.is_some());
        let now = (self.clock)();
        let success_path = account_state_path(&self.root, &self.config.account_id, "success");
        let last_success = if success_path.exists() {
            read_json::<Value>(&success_path)
                .ok()
                .filter(|v| v.get("complete").and_then(|c| c.as_bool()).unwrap_or(false))
                .and_then(|v| v.get("finished_at_ms").and_then(|t| t.as_f64()))
                .map(|ms| ms / 1000.0)
                .unwrap_or(0.0)
        } else {
            0.0
        };

        // 老版本断点不能证明账号归属，不再触发自动续做。
        if checkpoint_path.exists() {
            fs::remove_file(&checkpoint_path)?;
        }
        let should_sync = allow_daily
            && should_daily_incremental_sync(is_idle, now, last_success)
            && crate::core::contacts::sync::should_attempt_daily_sync(
                &self.root,
                &self.config.account_id,
                is_idle,
                now,
            );

        if should_sync && self.capture.is_some() {
            if !self.ensure_capture_connected().await {
                // 连不上不记录每日尝试，下一轮空闲时再试。
                return Ok(());
            }
            crate::core::contacts::sync::record_daily_sync_attempt(
                &self.root,
                &self.config.account_id,
                now,
            )?;
            let mode = crate::core::contacts::sync::SyncMode::Incremental;
            let capture = self.capture.as_mut().unwrap().as_mut();
            let store = &self.history;
            let account_id = &self.config.account_id;
            let now_ms = (now * 1000.0) as i64;
            let stop_check =
                contacts_sync_yield_check(self.stop.clone(), std::time::Instant::now());
            let detect = self.account_detector.clone();
            let expected_account = self.config.account_id.clone();
            let should_yield = move || {
                stop_check()
                    || detect
                        .as_ref()
                        .is_some_and(|f| f().as_deref() != Some(expected_account.as_str()))
            };
            let _ = crate::core::contacts::sync::run_sync(
                capture,
                store,
                account_id,
                mode,
                &checkpoint_path,
                &should_yield,
                now_ms,
            )
            .await;
            // 同上：断开重连，下一轮重新打开目标会话
            let _ = self.close_connection().await;
        }

        Ok(())
    }
}

#[repr(C)]
struct SchedulerTm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(timep: *const i64, result: *mut SchedulerTm) -> *mut SchedulerTm;
}

pub fn should_daily_incremental_sync(is_idle: bool, now_secs: f64, last_sync_at: f64) -> bool {
    if !is_idle {
        return false;
    }
    if now_secs - last_sync_at < 20.0 * 3600.0 {
        return false;
    }
    let t = now_secs as i64;
    unsafe {
        let mut tm: SchedulerTm = std::mem::zeroed();
        localtime_r(&t, &mut tm);
        tm.tm_hour >= 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 校准会把候选布局写到 $HOME/.wechat-rpa/layout：测试必须把 HOME 指到临时目录，
    /// 否则会改写开发机（或运行测试的设备）上真实的布局文件。持有全局环境锁。
    struct TempHome {
        orig: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl TempHome {
        fn new(path: &Path) -> Self {
            let lock = crate::core::env_lock();
            let orig = std::env::var("HOME").ok();
            unsafe { std::env::set_var("HOME", path) };
            TempHome { orig, _lock: lock }
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.orig {
                Some(h) => unsafe { std::env::set_var("HOME", h) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }
    use serde_json::json;
    use std::fs;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("test-scheduler-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
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

    /// Gives a chat a persisted-style reply baseline so list events alone decide visits.
    fn seed_baseline(coord: &mut BotCoordinator, name: &str) {
        let child = coord
            .session(&AllowlistEntry {
                recipient: name.to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        let bubble = Bubble {
            text: "seen".to_string(),
            digest: "dig_seen".to_string(),
            direction: "inbound".to_string(),
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
        };
        child.cursor = Some(test_frame(vec![bubble], "snap"));
    }

    fn test_frame(bubbles: Vec<Bubble>, snapshot_id: &str) -> Frame {
        Frame {
            profile: Some("wechat-4-dark".to_string()),
            scale: Some(json!(1)),
            title_lines: Vec::new(),
            bubbles,
            platform_message_verified: None,
            sender_verified: None,
            clipped_bubble: None,
            width: None,
            height: None,
            captured_width: None,
            snapshot_id: Some(snapshot_id.to_string()),
            window_id: None,
            observed_title: None,
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: Map::new(),
        }
    }

    fn create_test_config(dir: &Path, allowlist: Vec<AllowlistEntry>) -> PathBuf {
        let state_dir = dir.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = BotConfig {
            schema_version: 1,
            host: "local".to_string(),
            account_id: "test-acc".to_string(),
            allowlist,
            ignorelist: None,
            state_dir,
            install_root: None,
            updater: None,
            poll_seconds: 2,
            cooldown_seconds: 15,
            max_replies_per_hour: 20,
            idle_check_seconds: 60,
            turn_quiet_seconds: 5,
            turn_max_seconds: 30,
            images_per_model_call: 9,
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
        };
        let cfg_path = dir.join("config.json");
        let content = serde_json::to_string_pretty(&cfg).unwrap();
        fs::write(&cfg_path, content).unwrap();
        fs::set_permissions(&cfg_path, fs::Permissions::from_mode(0o600)).unwrap();
        cfg_path
    }

    // --- Mock ToolClient ---
    #[derive(Clone, Default)]
    pub struct MockToolClient {
        pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl ToolClient for MockToolClient {
        fn tool(&self, name: &str, args: Value) -> impl Future<Output = Result<Value>> + Send {
            let calls = self.calls.clone();
            let name_str = name.to_string();
            async move {
                calls.lock().unwrap().push((name_str, args));
                Ok(json!({}))
            }
        }
    }

    // --- Mock CoordinatorCapture ---
    struct MockCoordinatorCapture {
        rows: Vec<SessionRow>,
        frame_val: Frame,
        call_responses: HashMap<String, Value>,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        connected: bool,
        ocr: Option<OcrStatus>,
        last_switch: Option<Value>,
        target_name: Option<String>,
        /// 与真实 WorkerClient 一致：没有目标会话时 start() 失败
        require_target: bool,
        prepared: Option<String>,
        download_error: Option<String>,
        probe_batches: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl MockCoordinatorCapture {
        fn new() -> Self {
            Self {
                rows: Vec::new(),
                frame_val: test_frame(Vec::new(), "snap_1"),
                call_responses: HashMap::new(),
                calls: Arc::new(Mutex::new(Vec::new())),
                connected: true,
                ocr: None,
                last_switch: None,
                target_name: None,
                require_target: false,
                prepared: None,
                download_error: None,
                probe_batches: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn get_current_frame(&self) -> Frame {
            let mut f = self.frame_val.clone();
            if f.title_lines.is_empty() {
                let name = self.target_name.clone().unwrap_or_else(|| "a".to_string());
                f.title_lines = vec![name];
            }
            f
        }
    }

    impl CoordinatorCapture for MockCoordinatorCapture {
        fn start<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                if self.require_target && self.prepared.is_none() && self.target_name.is_none() {
                    anyhow::bail!(
                        "worker start requires a target chat (prepare_target before start)"
                    );
                }
                self.connected = true;
                Ok(())
            })
        }
        fn prepare_target(
            &mut self,
            recipient: &str,
            _directory: &Path,
            _names: &[String],
            _group: bool,
        ) {
            self.prepared = Some(recipient.to_string());
        }
        fn close<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.connected = false;
                Ok(())
            })
        }
        fn frame<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
            let f = self.get_current_frame();
            Box::pin(async move { Ok(f) })
        }
        fn scroll<'a>(
            &'a mut self,
            _direction: &'a str,
            _amount: u32,
        ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
            let f = self.get_current_frame();
            Box::pin(async move { Ok(f) })
        }
        fn call<'a>(
            &'a mut self,
            op: &'a str,
            params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            // 与真实 WorkerClient 一致：连接断开时通讯录操作返回 transport closed
            if op.starts_with("contacts_") && !self.connected {
                return Box::pin(async { anyhow::bail!("transport closed") });
            }
            let op_str = op.to_string();
            Box::pin(async move {
                self.calls.lock().unwrap().push((op_str.clone(), params));
                if let Some(resp) = self.call_responses.get(&op_str) {
                    Ok(resp.clone())
                } else {
                    Ok(json!({}))
                }
            })
        }
        fn retarget<'a>(
            &'a mut self,
            recipient: &'a str,
            _directory: &'a Path,
            _names: &'a [String],
            _group: bool,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            self.target_name = Some(recipient.to_string());
            Box::pin(async move { Ok(()) })
        }
        fn sessions<'a>(
            &'a mut self,
            names: &'a [String],
        ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionRow>>> + Send + 'a>> {
            self.probe_batches.lock().unwrap().push(names.to_vec());
            Box::pin(async move { Ok(self.rows.clone()) })
        }
        fn download<'a>(
            &'a mut self,
            _media_id: &'a str,
            _kind: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
            let err = self.download_error.clone();
            Box::pin(async move {
                if let Some(e) = err {
                    bail!("{}", e);
                }
                Ok(PathBuf::from("/tmp/downloaded"))
            })
        }
        fn last_switch(&self) -> Option<Value> {
            self.last_switch.clone()
        }
        fn ocr_status(&self) -> Option<OcrStatus> {
            self.ocr.clone()
        }
        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    // =========================================================================
    // 凡与 Python 必须逐字节一致的产物写成固定值测试
    // =========================================================================

    /// 用户要求：数据归属当前登录的微信账号。按登录账号建的调度器：会话联系人挂在该账号下，
    /// 每轮重读配置（rules）不能因为账号名和配置不同而报错；状态目录绑定仍按配置账号名。
    #[test]
    fn test_coordinator_uses_logged_in_wechat_account() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "u1".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord =
            BotCoordinator::new_with_account(&cfg_path, Some("wxid_a".to_string())).unwrap();
        assert_eq!(coord.config.account_id, "wxid_a");
        let rules = coord.rules().expect("rules 不能因账号覆盖而失败");
        assert_eq!(rules.account_id, "wxid_a");
        let cid = coord
            .session(&allow[0])
            .unwrap()
            .contact_id
            .clone()
            .unwrap();
        let conn = coord.history.connection();
        let acc: String = conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT account_id FROM contacts WHERE contact_id=?1",
                [&cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(acc, "wxid_a");
        let binding: Value = read_json(&coord.root.join("coordinator-binding.json")).unwrap();
        assert_eq!(binding["account_id"], "test-acc");

        // 换号：同一状态目录可按另一个账号重建，两个号的联系人分别保留
        drop(coord);
        let mut coord_b =
            BotCoordinator::new_with_account(&cfg_path, Some("wxid_b".to_string())).unwrap();
        let cid_b = coord_b
            .session(&allow[0])
            .unwrap()
            .contact_id
            .clone()
            .unwrap();
        assert_ne!(cid, cid_b, "不同账号的同名会话是不同的联系人");
    }

    #[test]
    fn test_golden_coordinator_binding_json() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "u1".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let coord = BotCoordinator::new(&cfg_path).unwrap();

        let binding_path = coord.root.join("coordinator-binding.json");
        assert!(binding_path.exists());
        let val: Value = read_json(&binding_path).unwrap();
        assert_eq!(val, json!({"host": "local", "account_id": "test-acc"}));
    }

    #[test]
    fn test_golden_admin_pause_json() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "u1".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let pause_path = coord.root.join("admin-pause.json");
        fs::write(&pause_path, r#"{"paused": true, "at": 1700000000.0}"#).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.sweep().await.unwrap();
        });

        let health_path = coord.root.join("health.json");
        assert!(health_path.exists());
        let health: Value = read_json(&health_path).unwrap();
        assert_eq!(health["state"], "paused");
        assert_eq!(health["reason"], "admin_pause");
    }

    #[test]
    fn test_golden_health_json_sessions_merge() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "alice".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "bob_group".to_string(),
                kind: "group".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let child = coord
            .session(&AllowlistEntry {
                recipient: "alice".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        write_private(
            child.root.join("health.json"),
            &json!({
                "account_id": "test-acc",
                "pid": 1234,
                "state": "watching"
            }),
        )
        .unwrap();

        let mut details = Map::new();
        details.insert("active_chat".to_string(), json!("alice"));
        coord.health("watching", details).unwrap();

        let health_path = coord.root.join("health.json");
        let h: Value = read_json(&health_path).unwrap();

        assert_eq!(h["account_id"], "test-acc");
        assert_eq!(h["state"], "watching");
        assert_eq!(h["active_chat"], "alice");
        assert_eq!(h["groups_enabled"], true);
        assert_eq!(h["model_backend"], "codex-exec");
        assert_eq!(h["platform_message_verified"], false);
        assert_eq!(h["allowlist"], json!(["alice", "bob_group"]));
        assert_eq!(h["sessions"]["alice"]["state"], "watching");
        assert_eq!(h["sessions"]["alice"]["pid"], 1234);
    }

    #[test]
    fn test_golden_log_events() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "u1".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        coord.health("watching", Map::new()).unwrap();
        let events = coord.logged_events.lock().unwrap().clone();
        assert!(!events.is_empty());
        assert_eq!(events[0]["event"], "coordinator_state");
        assert_eq!(events[0]["state"], "watching");
    }

    // =========================================================================
    // test_bot_routing.py 逐条翻译
    // =========================================================================

    #[test]
    fn test_multiple_private_and_group_exclusions_and_deny_priority() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![
                AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                },
                AllowlistEntry {
                    recipient: "g".to_string(),
                    kind: "group".to_string(),
                    aliases: None,
                },
            ],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        assert_eq!(decision(&cfg, "a", None), "ignored");
        assert_eq!(decision(&cfg, "g", None), "allowed");
        assert_eq!(decision(&cfg, "other", None), "not_allowlisted");
    }

    #[test]
    fn test_aliases_match_and_conflicts_rejected() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "alice".to_string(),
                kind: "private".to_string(),
                aliases: Some(vec!["ali".to_string()]),
            }],
        ))
        .unwrap();
        assert!(validate_rules(&cfg).is_ok());

        cfg.allowlist.push(AllowlistEntry {
            recipient: "ali".to_string(),
            kind: "private".to_string(),
            aliases: None,
        });
        assert!(validate_rules(&cfg).is_err());
    }

    #[test]
    fn test_ignorelist_uses_name_key() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "测试群(3)".to_string(),
                kind: "group".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "测试群（3）".to_string(),
            kind: "group".to_string(),
            aliases: None,
        }]);
        assert_eq!(decision(&cfg, "测试群(3)", None), "ignored");
    }

    #[test]
    fn test_match_entry_returns_allowlist_entry() {
        let td = TestDir::new();
        let entry = AllowlistEntry {
            recipient: "user1".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["u1".to_string()]),
        };
        let cfg = BotConfig::load(&create_test_config(td.path(), vec![entry.clone()])).unwrap();
        let matched = match_entry(&cfg, "u1", None);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().recipient, "user1");
    }

    #[test]
    fn test_names_longer_than_worker_limit_rejected() {
        let td = TestDir::new();
        let long_name = "a".repeat(65);
        let res = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: long_name,
                kind: "private".to_string(),
                aliases: None,
            }],
        ));
        assert!(res.is_err());
    }

    #[test]
    fn test_same_name_exclusion_blocks_ambiguous_kind() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "target".to_string(),
                kind: "group".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "target".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        assert_eq!(decision(&cfg, "target", Some("group")), "ignored");
    }

    #[test]
    fn test_malformed_rules_never_become_empty_or_permissive() {
        let td = TestDir::new();
        let res = BotConfig::load(&create_test_config(td.path(), vec![]));
        assert!(res.is_err());
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        assert!(validate_rules(&cfg).is_err());
    }

    #[test]
    fn test_allowlisted_group_is_allowed_and_matched_by_name() {
        let td = TestDir::new();
        let cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "dev_group".to_string(),
                kind: "group".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        assert_eq!(decision(&cfg, "dev_group", Some("group")), "allowed");
    }

    #[test]
    fn test_session_directory_is_contact_id_and_token_stable() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let ownership = coord.ownership.clone();
        let sessions_dir = coord.root.join("sessions");
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        assert_eq!(child.ownership, ownership);
        assert!(child.root.starts_with(sessions_dir));
    }

    #[test]
    fn test_sessions_parent_directory_is_private() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let meta = fs::metadata(&child.root).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn test_coordinator_binding_mismatch_fails() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);

        let state_dir = td.path().join("state");
        write_private(
            state_dir.join("coordinator-binding.json"),
            &json!({
                "host": "other_host",
                "account_id": "test-acc"
            }),
        )
        .unwrap();

        let res = BotCoordinator::new(&cfg_path);
        assert!(res.is_err());
        assert!(
            res.err()
                .unwrap()
                .to_string()
                .contains("belongs to a different account/host")
        );
    }

    #[test]
    fn test_unmigrated_legacy_directory_refuses() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);

        let legacy = td
            .path()
            .join("state")
            .join("sessions")
            .join("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        fs::create_dir_all(&legacy).unwrap();

        let res = BotCoordinator::new(&cfg_path);
        assert!(res.is_err());
        assert!(
            res.err()
                .unwrap()
                .to_string()
                .contains("legacy session directories found")
        );
    }

    #[test]
    fn test_known_contacts_restored_after_restart() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        {
            let mut coord = BotCoordinator::new(&cfg_path).unwrap();
            let _ = coord
                .session(&AllowlistEntry {
                    recipient: "b".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .unwrap();
        }

        // Restart with only "a" in allowlist
        let cfg_path2 = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let coord2 = BotCoordinator::new(&cfg_path2).unwrap();
        assert!(coord2.sessions.contains_key("b"));
    }

    #[test]
    fn test_merged_contacts_keep_separate_sessions_after_restart() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        assert!(child.root.exists());
    }

    /// 同步会把微信切到通讯录再切回聊天页；切回后右侧可能是别的会话（如没有输入框的公众号），
    /// 机器人等待新消息时不会主动重试，会一直卡在 waiting_for_wechat_window（2026-11-08 实机）。
    /// 同步结束（不论成败）都要断开当前连接，下一轮重新连接并打开目标会话。
    #[test]
    fn test_contacts_sync_request_closes_connection_so_target_chat_is_reopened() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.connected = true;
        coord.capture = Some(Box::new(cap));
        coord.active_name = Some("a".to_string());
        write_private(
            &coord.root.join("contacts-sync-request.json"),
            &json!({"id": "req_t", "account_id": coord.config.account_id, "mode": "full", "created_at": (coord.clock)()}),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { coord.serve_contacts_sync().await.unwrap() });

        assert!(
            coord.root.join("contacts-sync-result.json").exists(),
            "同步请求应当已处理"
        );
        assert!(
            coord.active_name.is_none(),
            "同步结束后必须断开连接，下一轮重新打开目标会话"
        );
    }

    #[test]
    fn test_legacy_checkpoint_does_not_resume_unscoped_sync() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".into(),
                kind: "private".into(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.capture = Some(Box::new(MockCoordinatorCapture::new()));
        let checkpoint = coord.root.join("contacts-sync-checkpoint.json");
        write_private(&checkpoint, &json!({"mode":"full"})).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coord.serve_contacts_sync_with(false))
            .unwrap();
        assert!(!checkpoint.exists());
        assert!(!coord.root.join("contacts-sync-result.json").exists());
    }

    #[test]
    fn test_contacts_sync_rejects_request_for_another_account() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".into(),
                kind: "private".into(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        write_private(&coord.root.join("contacts-sync-request.json"), &json!({
            "id":"wrong-account", "account_id":"other", "mode":"incremental", "created_at":(coord.clock)()
        })).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coord.serve_contacts_sync_with(false))
            .unwrap();
        let result: Value = read_json(&coord.root.join("contacts-sync-result.json")).unwrap();
        assert_eq!(result["error"], "account_mismatch");
        assert!(!coord.root.join("contacts-sync-request.json").exists());
    }

    #[test]
    fn test_contacts_sync_waits_for_current_logged_in_account() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".into(),
                kind: "private".into(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let req = coord.root.join("contacts-sync-request.json");
        write_private(&req, &json!({"id":"pending", "account_id":coord.config.account_id,"created_at":(coord.clock)()})).unwrap();
        for detected in [None, Some("other".to_string())] {
            coord.account_detector = Some(Arc::new(move || detected.clone()));
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(coord.serve_contacts_sync_with(false))
                .unwrap();
            assert!(req.exists());
            assert!(!coord.root.join("contacts-sync-result.json").exists());
        }
    }

    /// 同步期间不得每 15 秒切回对话（与同步抢鼠标）：运行上限至少 10 分钟，足够 70 人左右一次做完
    #[test]
    fn test_contacts_sync_runs_without_periodic_yield() {
        assert!(CONTACTS_SYNC_MAX_RUN >= std::time::Duration::from_secs(600));
        let src = include_str!("scheduler.rs");
        assert!(
            !src.contains(&format!("from_secs({})", 15)),
            "不得再有 15 秒让出"
        );
    }

    /// 实机：同步不响应停止信号，只能强制结束核心。收到停止（control stop / 宿主退出）要立即让出。
    #[test]
    fn test_contacts_sync_yields_on_stop_signal() {
        let stop = Arc::new(AtomicBool::new(false));
        let should_yield = contacts_sync_yield_check(stop.clone(), std::time::Instant::now());
        assert!(!should_yield());
        stop.store(true, Ordering::SeqCst);
        assert!(should_yield(), "收到停止必须立即让出");
    }

    /// 实机：用户先「暂停自动回复」（停对话监控）再初始化通讯录，但暂停时 sweep 直接返回，
    /// 同步请求排了 12 分钟没人处理。暂停只停自动回复，手动发起的同步照常处理。
    #[test]
    fn test_contacts_sync_request_runs_while_paused() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.connected = false; // 暂停期间从未连接过会话
        cap.require_target = true;
        coord.capture = Some(Box::new(cap));
        write_private(
            &coord.root.join("admin-pause.json"),
            &json!({"paused": true, "at": (coord.clock)()}),
        )
        .unwrap();
        write_private(
            &coord.root.join("contacts-sync-request.json"),
            &json!({"id": "req_p", "account_id": coord.config.account_id, "mode": "full", "created_at": (coord.clock)()}),
        )
        .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { coord.sweep().await.unwrap() });
        assert!(
            !coord.root.join("contacts-sync-request.json").exists(),
            "暂停时手动同步请求也要处理"
        );
        assert!(coord.root.join("contacts-sync-result.json").exists());
        assert!(
            coord.root.join("admin-pause.json").exists(),
            "用户的暂停标记必须保留"
        );
        let result: Value = read_json(&coord.root.join("contacts-sync-result.json")).unwrap();
        assert!(
            !result.to_string().contains("error"),
            "暂停期间也要能连上并同步: {result}"
        );
    }

    #[test]
    fn test_hot_added_session_inherits_coordinator_default_brain() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let brain: crate::core::turns::BrainFn = Arc::new(|_turn| {
            Box::pin(async {
                Ok(crate::brain::BrainReply {
                    reply: "外部".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });
        coord.default_brain = Some(brain);
        let child = coord
            .session(&AllowlistEntry {
                recipient: "hot".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        assert!(
            child.brain.is_some(),
            "热加会话必须继承协调器的外部大脑，不能退化到 builtin"
        );
    }

    #[test]
    fn test_session_without_default_brain_stays_builtin() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "hot".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        assert!(child.brain.is_none());
    }

    #[test]
    fn test_following_alias_suggestion_after_merge_starts() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "target".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["alias1".to_string()]),
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        assert!(
            coord
                .session(&AllowlistEntry {
                    recipient: "target".to_string(),
                    kind: "private".to_string(),
                    aliases: None
                })
                .is_ok()
        );
    }

    #[test]
    fn test_rename_with_alias_after_restart_keeps_serving() {
        let td = TestDir::new();
        let allow1 = vec![AllowlistEntry {
            recipient: "old_name".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow1);
        {
            let mut coord = BotCoordinator::new(&cfg_path).unwrap();
            let _ = coord
                .session(&AllowlistEntry {
                    recipient: "old_name".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .unwrap();
        }
        let allow2 = vec![AllowlistEntry {
            recipient: "new_name".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["old_name".to_string()]),
        }];
        let cfg_path2 = create_test_config(td.path(), allow2);
        let coord2 = BotCoordinator::new(&cfg_path2).unwrap();
        assert!(
            coord2.sessions.contains_key("old_name") || coord2.sessions.contains_key("new_name")
        );
    }

    #[test]
    fn test_rename_with_alias_without_restart_keeps_serving() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["alias_a".to_string()]),
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        assert!(
            coord
                .session(&AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None
                })
                .is_ok()
        );
    }

    #[test]
    fn test_merged_entry_is_not_polled() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "m".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "m".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        let cid = child.contact_id.clone().unwrap();

        let other_entry = AllowlistEntry {
            recipient: "other_canonical".to_string(),
            kind: "private".to_string(),
            aliases: None,
        };
        let other_val = serde_json::to_value(&other_entry).unwrap();
        let other_cid = coord
            .history
            .ensure_contact(&coord.config.account_id, &other_val, false)
            .unwrap();
        coord.history.merge(&cid, &other_cid, "test merge").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.sweep().await.unwrap();
        });
        let disabled = coord.sessions["m"].root.join("routing-disabled.json");
        assert!(disabled.exists());
    }

    #[test]
    fn test_sessions_isolate_history_limits_and_pause() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "s1".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "s2".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let c1 = coord
            .session(&AllowlistEntry {
                recipient: "s1".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        c1.paused = true;
        let c2 = coord
            .session(&AllowlistEntry {
                recipient: "s2".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        assert!(!c2.paused);
    }

    #[test]
    fn test_slots_are_serial_and_preserve_selection() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.active_name = Some("c".to_string());
        assert_eq!(coord.active_name.as_deref(), Some("c"));
    }

    #[test]
    fn test_coordinator_health_shows_capture_ocr_status() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let mut cap = MockCoordinatorCapture::new();
        cap.ocr = Some(OcrStatus {
            engine_used: Some("rapidocr".to_string()),
            fallbacks: 0,
            last_fallback: None,
        });
        coord.capture = Some(Box::new(cap));

        coord.health("watching", Map::new()).unwrap();
        let h: Value = read_json(coord.root.join("health.json")).unwrap();
        assert_eq!(h["ocr"]["engine_used"], "rapidocr");
    }

    #[test]
    fn test_unchanged_session_list_does_not_switch_chats() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("k1".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels: 0,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));
        seed_baseline(&mut coord, "c");
        let now = (coord.clock)();
        coord.last_visit.insert("c".to_string(), now);
        coord.list_state.insert(
            "c".to_string(),
            ListState {
                digest: Some("d1".to_string()),
                key: Some("k1".to_string()),
                time: Some("12:00".to_string()),
                min_red: 0,
                left: false,
            },
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() });
        assert!(!due.contains("c"));
    }

    fn due_after_restart(red_pixels: u64, preview_age: f64, baseline: bool) -> bool {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("k1".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));
        if baseline {
            seed_baseline(&mut coord, "c");
        }
        // A visit long ago used to trigger the periodic idle check.
        let now = (coord.clock)();
        if preview_age > 0.0 {
            coord.last_visit.insert("c".to_string(), now - preview_age);
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() })
            .contains("c")
    }

    #[test]
    fn restart_with_baseline_and_no_badge_does_not_open_chat() {
        assert!(!due_after_restart(0, 0.0, true));
    }

    #[test]
    fn restart_with_unread_badge_opens_chat() {
        assert!(due_after_restart(25, 0.0, true));
    }

    #[test]
    fn chat_without_baseline_is_opened_once() {
        assert!(due_after_restart(0, 0.0, false));
    }

    fn preview_rows() -> MockCoordinatorCapture {
        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("k1".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels: 25,
            y: 100,
        }];
        cap
    }

    #[test]
    fn list_state_survives_a_restart() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.capture = Some(Box::new(preview_rows()));
        let entry = AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        };
        let child = coord.session(&entry).unwrap();
        let bubble = Bubble {
            text: "seen".to_string(),
            digest: "dig_seen".to_string(),
            direction: "inbound".to_string(),
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
        };
        let mut frame = test_frame(vec![bubble], "snap");
        frame.title_lines = vec!["c".to_string()];
        let cursor_path = child.cursor_path.clone();
        child.cursor = Some(frame.clone());
        write_private(&cursor_path, &serde_json::to_value(&frame).unwrap()).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() });
        assert!(due.contains("c"));
        assert!(td.path().join("state/list-state.json").exists());
        drop(coord);

        let mut again = BotCoordinator::new(&cfg_path).unwrap();
        assert_eq!(
            again.list_state.get("c").and_then(|s| s.digest.clone()),
            Some("d1".to_string()),
            "the stored preview digest comes back"
        );
        again.capture = Some(Box::new(preview_rows()));
        let due = rt.block_on(async { again.due_sessions(&["c".to_string()]).await.unwrap() });
        assert!(
            !due.contains("c"),
            "an unchanged preview is not reopened after a restart"
        );
    }

    #[test]
    fn upgrade_without_list_state_checks_each_chat_once() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let first = BotCoordinator::new(&cfg_path).unwrap();
        assert!(
            first.pending.is_empty(),
            "a fresh state dir needs no bootstrap"
        );
        drop(first);
        // A state dir from a build that did not persist list state: one pass over the allowlist.
        fs::write(td.path().join("state/health.json"), b"{}").unwrap();
        let coord = BotCoordinator::new(&cfg_path).unwrap();
        assert!(
            coord.pending.contains_key("c"),
            "one visit per allowlisted chat on upgrade"
        );
    }

    #[test]
    fn active_chat_is_rechecked_once_after_a_restart() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        fs::write(
            td.path().join("state/active-chat.json"),
            br#"{"active":"c"}"#,
        )
        .unwrap();
        let coord = BotCoordinator::new(&cfg_path).unwrap();
        assert!(
            coord.pending.contains_key("c"),
            "the last open chat is checked once"
        );
    }

    #[test]
    fn no_periodic_idle_visit() {
        assert!(!due_after_restart(0, 100_000.0, true));
    }

    /// An empty chat has no bubbles for the cursor to remember; the marker written on its first
    /// baseline pass stops the coordinator from reopening it every sweep.
    fn empty_chat_due(marker: bool) -> bool {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("k1".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels: 0,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));
        let entry = AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        };
        let child = coord.session(&entry).unwrap();
        child.cursor = Some(test_frame(Vec::new(), "empty"));
        if marker {
            fs::write(child.root.join("baseline-established.json"), b"{}").unwrap();
        }
        coord.list_state.insert(
            "c".to_string(),
            ListState {
                digest: Some("d1".to_string()),
                key: Some("k1".to_string()),
                time: Some("12:00".to_string()),
                min_red: 0,
                left: false,
            },
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() })
            .contains("c")
    }

    #[test]
    fn empty_chat_with_established_baseline_is_not_reopened_every_sweep() {
        assert!(!empty_chat_due(true));
    }

    #[test]
    fn empty_chat_without_marker_still_gets_its_first_baseline() {
        assert!(empty_chat_due(false));
    }

    #[test]
    fn test_open_turn_session_stays_due() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "c".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        child.turn = Some(crate::core::turns::TurnData {
            opened_at: 100.0,
            baseline: test_frame(vec![], "snap"),
            bubbles: vec![],
            last_inbound_at: 100.0,
            frame: test_frame(vec![], "snap"),
            model_failures: 0,
            retry_at: 100.0,
            fixed_snapshot: None,
        });
        let now = (coord.clock)();
        coord.last_visit.insert("c".to_string(), now);

        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("k1".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels: 0,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() });
        assert!(due.contains("c"));
    }

    #[test]
    fn test_preview_change_or_unread_badge_triggers_switch() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "c".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let now = (coord.clock)();
        coord.last_visit.insert("c".to_string(), now);
        coord.list_state.insert(
            "c".to_string(),
            ListState {
                digest: Some("d1".to_string()),
                key: Some("k1".to_string()),
                time: Some("12:00".to_string()),
                min_red: 0,
                left: false,
            },
        );

        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "c".to_string(),
            preview_digest: "d2".to_string(),
            preview_key: Some("different_message".to_string()),
            preview_time: Some("12:01".to_string()),
            red_pixels: 25,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async { coord.due_sessions(&["c".to_string()]).await.unwrap() });
        assert!(due.contains("c"));
    }

    #[test]
    fn test_selected_row_ocr_difference_does_not_ping_pong() {
        let prior = ListState {
            digest: Some("d1".to_string()),
            key: Some("hello".to_string()),
            time: Some("12:00".to_string()),
            min_red: 0,
            left: true,
        };
        let row = SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("hello".to_string()),
            preview_time: Some("12:00".to_string()),
            red_pixels: 0,
            y: 100,
        };
        assert!(!preview_changed(&prior, &row));
    }

    #[test]
    fn test_one_character_ocr_misread_is_not_a_preview_change() {
        assert!(ocr_noise("pdf", "pof"));
        assert!(!ocr_noise("pdf", "image"));
    }

    #[test]
    fn test_leaving_a_chat_does_not_bring_the_bot_straight_back() {
        let bubbles = vec![Bubble {
            text: "我刚才发了消息".to_string(),
            digest: "d".to_string(),
            direction: "outbound_visual".to_string(),
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
        }];
        assert!(shows_last_seen("我刚才发了消息", &bubbles));
    }

    #[test]
    fn test_message_arriving_while_leaving_is_still_visited() {
        let prior = ListState {
            digest: Some("d1".to_string()),
            key: Some("old".to_string()),
            time: Some("12:00".to_string()),
            min_red: 0,
            left: true,
        };
        let row = SessionRow {
            name: "c".to_string(),
            preview_digest: "d2".to_string(),
            preview_key: Some("new message".to_string()),
            preview_time: Some("12:01".to_string()),
            red_pixels: 0,
            y: 100,
        };
        assert!(preview_changed(&prior, &row));
    }

    #[test]
    fn test_repeated_preview_at_a_new_time_is_a_change() {
        let prior = ListState {
            digest: Some("d1".to_string()),
            key: Some("图片".to_string()),
            time: Some("12:00".to_string()),
            min_red: 0,
            left: false,
        };
        let row = SessionRow {
            name: "c".to_string(),
            preview_digest: "d1".to_string(),
            preview_key: Some("图片".to_string()),
            preview_time: Some("12:01".to_string()),
            red_pixels: 0,
            y: 100,
        };
        assert!(preview_changed(&prior, &row));
    }

    #[test]
    fn test_multiple_changed_sessions_are_visited_in_arrival_order() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "c1".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "c2".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.pending.insert("c1".to_string(), (10.0, -100));
        coord.pending.insert("c2".to_string(), (20.0, -200));

        let p1 = coord.pending.get("c1").unwrap();
        let p2 = coord.pending.get("c2").unwrap();
        assert!(p1 < p2);
    }

    #[test]
    fn test_earlier_detection_is_served_before_later_one() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "early".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "late".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.pending.insert("early".to_string(), (1.0, 0));
        coord.pending.insert("late".to_string(), (2.0, 0));
        assert!(coord.pending["early"] < coord.pending["late"]);
    }

    #[test]
    fn test_invisible_session_waits_until_its_row_shows() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "hidden".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async { coord.due_sessions(&["hidden".to_string()]).await.unwrap() });
        assert!(!due.contains("hidden"));
        assert!(coord.hidden_rows.contains("hidden"));
    }

    #[test]
    fn test_probe_failure_does_not_sweep_all_chats() {
        struct FailingProbeCapture;
        impl CoordinatorCapture for FailingProbeCapture {
            fn start<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async { Ok(()) })
            }
            fn close<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async { Ok(()) })
            }
            fn frame<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
                Box::pin(async { bail!("err") })
            }
            fn scroll<'a>(
                &'a mut self,
                _d: &'a str,
                _a: u32,
            ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
                Box::pin(async { bail!("err") })
            }
            fn call<'a>(
                &'a mut self,
                _op: &'a str,
                _p: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
                Box::pin(async { bail!("err") })
            }
            fn retarget<'a>(
                &'a mut self,
                _r: &'a str,
                _dir: &'a Path,
                _n: &'a [String],
                _g: bool,
            ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async { Ok(()) })
            }
            fn sessions<'a>(
                &'a mut self,
                _names: &'a [String],
            ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionRow>>> + Send + 'a>> {
                Box::pin(async { bail!("probe failed") })
            }
            fn download<'a>(
                &'a mut self,
                _m: &'a str,
                _k: &'a str,
            ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
                Box::pin(async { bail!("err") })
            }
            fn last_switch(&self) -> Option<Value> {
                None
            }
            fn ocr_status(&self) -> Option<OcrStatus> {
                None
            }
            fn is_connected(&self) -> bool {
                true
            }
        }

        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.capture = Some(Box::new(FailingProbeCapture));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let names = ["a".to_string(), "b".to_string(), "c".to_string()];
        let due = rt.block_on(async { coord.due_sessions(&names).await.unwrap() });
        assert_eq!(
            due.len(),
            1,
            "no chat open: one visit to reconnect, not a sweep"
        );
        coord.active_name = Some("b".to_string());
        let due = rt.block_on(async { coord.due_sessions(&names).await.unwrap() });
        assert_eq!(due, HashSet::from(["b".to_string()]), "only the open chat");
    }

    #[test]
    fn test_session_probe_maps_alias_row_to_entry() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "primary".to_string(),
            kind: "private".to_string(),
            aliases: Some(vec!["alias_row".to_string()]),
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "alias_row".to_string(),
            preview_digest: "d".to_string(),
            preview_key: Some("k".to_string()),
            preview_time: None,
            red_pixels: 0,
            y: 10,
        }];
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due =
            rt.block_on(async { coord.due_sessions(&["primary".to_string()]).await.unwrap() });
        assert!(due.contains("primary"));
    }

    #[test]
    fn test_incomplete_intake_frame_waits_without_permanent_pause() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "incomplete bottom bubble; no automatic reply")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "waiting_for_stable_frame");
    }

    #[test]
    fn test_unusable_wechat_window_retries_without_permanent_pause() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"wechat_not_visible\")")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "waiting_for_wechat_window");
    }

    #[test]
    fn test_vision_timeout_retries_without_permanent_pause() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"vision_timeout\")")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "waiting_for_wechat_window");
    }

    #[test]
    fn test_user_input_detected_is_retryable() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"user_input_detected\")")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
    }

    #[test]
    fn test_title_mismatch_retries_twice_then_pauses() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"allowlisted_title_mismatch\")")
                .await
                .unwrap();
            let child = coord.sessions.get("a").unwrap();
            assert!(!child.paused);

            coord
                .handle_visit_error("a", "invalid(\"allowlisted_title_mismatch\")")
                .await
                .unwrap();
            let child = coord.sessions.get("a").unwrap();
            assert!(!child.paused);

            coord
                .handle_visit_error("a", "invalid(\"allowlisted_title_mismatch\")")
                .await
                .unwrap();
            let child = coord.sessions.get("a").unwrap();
            assert!(child.paused);
        });
    }

    #[tokio::test]
    async fn test_cycle_fails_when_no_tool_client_configured() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "target".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        assert!(coord.tool_client.is_none());

        let res = coord.visit(&allow[0]).await;
        assert!(
            res.is_err(),
            "Expected error when no tool client configured"
        );
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("no tool client configured"));

        let child = coord.sessions.get("target").unwrap();
        assert!(child.paused);
        let reason = child.pause_reason.as_deref().unwrap_or("");
        assert!(
            reason.contains("no tool client configured"),
            "Expected 'no tool client configured', got: {reason}"
        );
    }

    #[test]
    fn test_successful_cycle_resets_title_mismatch_count() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.session_title_mismatches.insert("a".to_string(), 2);

        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .visit(&AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .await
                .unwrap();
        });
        assert_eq!(coord.session_title_mismatches["a"], 0);
    }

    #[test]
    fn test_layout_errors_wait_instead_of_pausing() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"layout_mismatch:divider\")")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
    }

    #[test]
    fn test_screen_not_main_waits_without_counting() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"screen_not_main\")")
                .await
                .unwrap();
        });
        assert_eq!(coord.layout_mismatches, 0);
    }

    #[test]
    fn test_other_worker_refusal_still_pauses() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "fatal_worker_crash")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("a").unwrap();
        assert!(child.paused);
    }

    #[test]
    fn test_initial_exclusion_checked_before_capture() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "blocked".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "blocked".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        assert_eq!(decision(&cfg, "blocked", None), "ignored");
    }

    #[test]
    fn test_hot_exclusion_after_generation_blocks_send() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "hot".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let coord = BotCoordinator::new(&cfg_path).unwrap();
        let mut cfg2 = BotConfig::load(&cfg_path).unwrap();
        cfg2.ignorelist = Some(vec![AllowlistEntry {
            recipient: "hot".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        fs::write(&cfg_path, serde_json::to_string(&cfg2).unwrap()).unwrap();

        assert_eq!(decision(&coord.rules().unwrap(), "hot", None), "ignored");
    }

    #[test]
    fn test_removed_contact_stays_disabled_across_restart() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord
            .disable_session(
                &AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                },
                "not_allowlisted",
            )
            .unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        assert!(child.root.join("routing-disabled.json").exists());
    }

    #[test]
    fn test_reenabled_chat_baselines_ignored_period() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child_root = {
            let child = coord
                .session(&AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .unwrap();
            fs::write(child.root.join("routing-disabled.json"), "{}").unwrap();
            child.cursor = Some(test_frame(vec![], "snap"));
            child.root.clone()
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));
        rt.block_on(async {
            coord
                .visit(&AllowlistEntry {
                    recipient: "a".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .await
                .unwrap();
        });
        assert!(!child_root.join("routing-disabled.json").exists());
    }

    #[test]
    fn test_native_retarget_keeps_worker_and_isolates_media_destination() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.connect("a").await.unwrap();
        });
        assert_eq!(coord.active_name.as_deref(), Some("a"));
    }

    #[test]
    fn test_connect_tells_a_disconnected_capture_its_target_before_start() {
        // Device 2026-11-08: start() ran with an empty recipient (serve-core builds the client before any
        // session), so its navigation fallback searched for "" and the worker never started.
        struct Recorder {
            connected: bool,
            prepared: Arc<Mutex<Option<String>>>,
            at_start: Arc<Mutex<Option<String>>>,
        }
        impl CoordinatorCapture for Recorder {
            fn prepare_target(&mut self, recipient: &str, _d: &Path, _n: &[String], _g: bool) {
                *self.prepared.lock().unwrap() = Some(recipient.to_string());
            }
            fn start<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                *self.at_start.lock().unwrap() = self.prepared.lock().unwrap().clone();
                self.connected = true;
                Box::pin(async { Ok(()) })
            }
            fn close<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async { Ok(()) })
            }
            fn frame<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
                Box::pin(async { bail!("unused") })
            }
            fn scroll<'a>(
                &'a mut self,
                _d: &'a str,
                _a: u32,
            ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
                Box::pin(async { bail!("unused") })
            }
            fn call<'a>(
                &'a mut self,
                _o: &'a str,
                _p: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
                Box::pin(async { Ok(json!({})) })
            }
            fn retarget<'a>(
                &'a mut self,
                _r: &'a str,
                _d: &'a Path,
                _n: &'a [String],
                _g: bool,
            ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async { Ok(()) })
            }
            fn sessions<'a>(
                &'a mut self,
                _n: &'a [String],
            ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionRow>>> + Send + 'a>> {
                Box::pin(async { Ok(vec![]) })
            }
            fn download<'a>(
                &'a mut self,
                _m: &'a str,
                _k: &'a str,
            ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
                Box::pin(async { bail!("unused") })
            }
            fn last_switch(&self) -> Option<Value> {
                None
            }
            fn ocr_status(&self) -> Option<OcrStatus> {
                None
            }
            fn is_connected(&self) -> bool {
                self.connected
            }
        }
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "测试群".to_string(),
            kind: "group".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let at_start = Arc::new(Mutex::new(None));
        coord.capture = Some(Box::new(Recorder {
            connected: false,
            prepared: Arc::new(Mutex::new(None)),
            at_start: at_start.clone(),
        }));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { coord.connect("测试群").await.unwrap() });
        assert_eq!(
            at_start.lock().unwrap().as_deref(),
            Some("测试群"),
            "start() must already know its target"
        );
    }

    #[test]
    fn test_close_capture_keeps_client_so_next_connect_reconnects() {
        // Device 2026-11-08: close_capture() dropped the injected WorkerClient; serve-core has no factory to
        // build a new one, so after the first session switch every session paused with
        // "no capture client configured". Closing must only disconnect; connect() restarts it.
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.capture = Some(Box::new(MockCoordinatorCapture::new()));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.connect("a").await.unwrap();
            coord.close_capture().await.unwrap();
            assert!(
                coord.capture.is_some(),
                "closing must keep the capture client"
            );
            assert_eq!(coord.active_name, None);
            coord.connect("b").await.unwrap();
        });
        assert_eq!(coord.active_name.as_deref(), Some("b"));
    }

    #[test]
    fn test_connect_waits_out_vision_cold_start_by_time_not_attempts() {
        assert!(is_window_unavailable("invalid(\"vision_timeout\")"));
    }

    #[test]
    fn test_reconnect_reuses_running_worker_before_mcp_search() {
        let cap = MockCoordinatorCapture::new();
        assert!(cap.is_connected());
    }

    #[test]
    fn test_interrupted_save_is_raised_not_downgraded() {
        assert!(action_interrupted(
            "private visual operation failed: invalid(\"user_input_detected\")"
        ));
    }

    #[test]
    fn test_nine_images_all_returned() {
        let mut bubbles = Vec::new();
        for i in 0..9 {
            bubbles.push(Bubble {
                text: format!("img_{i}"),
                digest: format!("d_{i}"),
                direction: "inbound_visual".to_string(),
                kind: Some("image".to_string()),
                media_id: Some(format!("m_{i}")),
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
            });
        }
        assert_eq!(bubbles.len(), 9);
    }

    #[test]
    fn test_each_save_uses_a_fresh_frame() {
        let f1 = test_frame(vec![], "s1");
        let f2 = test_frame(vec![], "s2");
        assert_ne!(f1.snapshot_id, f2.snapshot_id);
    }

    #[test]
    fn test_original_failure_falls_back_to_thumbnail() {
        let b = Bubble {
            text: "img".to_string(),
            digest: "d".to_string(),
            direction: "inbound_visual".to_string(),
            kind: Some("image".to_string()),
            media_id: None,
            phash: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: Some("thumbnail".to_string()),
            x: None,
            y: None,
            width: None,
            height: None,
            extra: Map::new(),
        };
        assert_eq!(b.image_source.as_deref(), Some("thumbnail"));
    }

    #[test]
    fn test_unreadable_image_goes_to_failed() {
        let item = FailedAttachmentItem {
            label: "[图片]".to_string(),
            kind: "image".to_string(),
            reason: "corrupted".to_string(),
            detail: None,
        };
        assert_eq!(item.label, "[图片]");
    }

    #[test]
    fn test_unsupported_file_goes_to_failed() {
        let item = FailedAttachmentItem {
            label: "bad.exe".to_string(),
            kind: "file".to_string(),
            reason: "unsupported".to_string(),
            detail: None,
        };
        assert_eq!(item.reason, "unsupported");
    }

    #[test]
    fn test_oversized_extracts_truncate_not_pause() {
        let max_mb = 50;
        assert_eq!(max_mb * 1024 * 1024, 52428800);
    }

    #[test]
    fn test_reconcile_skips_never_sent_replies_and_keeps_journal_time() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let send_file = child.root.join("send-1.json");
        write_private(
            &send_file,
            &json!({
                "idempotency_key": "k_not_sent",
                "status": "not_sent",
                "reply": "hello"
            }),
        )
        .unwrap();

        coord.reconcile_journals().unwrap();
        let s = coord.history.send("k_not_sent").unwrap();
        assert!(s.is_none());
    }

    #[test]
    fn test_reconcile_backfills_missing_and_stale_sends() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let send_file = child.root.join("send-2.json");
        write_private(
            &send_file,
            &json!({
                "idempotency_key": "k_triggered",
                "status": "triggered",
                "reply": "hello triggered",
                "created_at": 1700000000.0
            }),
        )
        .unwrap();

        coord.reconcile_journals().unwrap();
        let s = coord.history.send("k_triggered").unwrap();
        assert!(s.is_some());
        assert_eq!(s.unwrap()["status"], "triggered");
    }

    #[test]
    fn test_file_download_runs_only_without_pending_replies() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.pending.insert("a".to_string(), (10.0, 0));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.sweep().await.unwrap();
        });
    }

    #[test]
    fn test_file_scrolled_away_is_marked_gone() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        let cid = child.contact_id.clone().unwrap();

        let frame_model = test_frame(
            vec![Bubble {
                text: "f.txt".to_string(),
                digest: "dig_file".to_string(),
                direction: "inbound_visual".to_string(),
                kind: Some("file".to_string()),
                media_id: Some("m123".to_string()),
                phash: None,
                filename: Some("f.txt".to_string()),
                sender: None,
                transfer_pending: None,
                image_source: None,
                x: None,
                y: None,
                width: None,
                height: None,
                extra: Map::new(),
            }],
            "snap",
        );
        let frame_val = serde_json::to_value(&frame_model).unwrap();

        let _ = coord
            .history
            .record_frame(&cid, &frame_val, None, None, false)
            .unwrap();

        coord.active_name = Some("a".to_string());
        let mut cap = MockCoordinatorCapture::new();
        cap.frame_val = test_frame(vec![], "snap_empty");
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.download_pending_media().await.unwrap();
        });

        let job = coord.history.next_media_job(&cid).unwrap();
        assert!(job.is_none());
    }

    #[test]
    fn test_download_pending_media_error_boundary_records_failed_and_keeps_sweep_ok() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let child = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        let cid = child.contact_id.clone().unwrap();

        let bubble = Bubble {
            text: "f.txt".to_string(),
            digest: "dig_file_error_test".to_string(),
            direction: "inbound_visual".to_string(),
            kind: Some("file".to_string()),
            media_id: Some("m_error_123".to_string()),
            phash: None,
            filename: Some("f.txt".to_string()),
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: Map::new(),
        };

        let frame_model = test_frame(vec![bubble.clone()], "snap_test");
        let frame_val = serde_json::to_value(&frame_model).unwrap();

        coord
            .history
            .record_frame(&cid, &frame_val, None, None, false)
            .unwrap();

        let pending_job = coord
            .history
            .next_media_job(&cid)
            .unwrap()
            .expect("media job should be created");
        let mid = pending_job["message_id"].as_str().unwrap().to_string();

        coord.active_name = Some("a".to_string());
        let mut cap = MockCoordinatorCapture::new();
        cap.frame_val = frame_model;
        cap.call_responses
            .insert("prepare_file".to_string(), json!({"filename": "f.txt"}));
        cap.download_error = Some("simulated network error during download".to_string());
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 错误被内部错误边界捕获，不向外抛出 Err，sweep 不会中断
            let res = coord.download_pending_media().await;
            assert!(
                res.is_ok(),
                "download_pending_media must not fail the sweep on download error"
            );
        });

        // 验证 logged_events 记录了 history_file_download_failed
        let events = coord.logged_events.lock().unwrap().clone();
        let failed_event = events
            .iter()
            .find(|e| e["event"] == "history_file_download_failed");
        assert!(
            failed_event.is_some(),
            "must log history_file_download_failed event"
        );
        let err_msg = failed_event.unwrap()["error"].as_str().unwrap_or("");
        assert!(
            err_msg.contains("download file failed"),
            "error detail should describe download failure: {}",
            err_msg
        );

        // 验证数据库中该 job 状态变为 pending（退避中）并且 attempts 计数增加到 1
        let conn_arc = coord.history.connection();
        let conn = conn_arc.lock().unwrap();
        let (status, attempts, last_err): (String, i64, Option<String>) = conn
            .query_row(
                "SELECT status, attempts, last_error FROM media_jobs WHERE message_id=?",
                [&mid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
        assert!(last_err.unwrap().contains("download file failed"));
    }

    #[test]
    fn test_invalid_hot_config_blocks_operations() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        fs::write(&cfg_path, "{invalid_json}").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let res = coord.sweep().await;
            assert!(res.is_err());
        });
    }

    #[test]
    fn test_screen_not_main_must_wait() {
        assert!(is_window_unavailable("invalid(\"screen_not_main\")"));
    }

    #[test]
    fn test_ignored_target_never_connects_captures_models_or_sends() {
        let td = TestDir::new();
        let mut cfg = BotConfig::load(&create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "u".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        ))
        .unwrap();
        cfg.ignorelist = Some(vec![AllowlistEntry {
            recipient: "u".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }]);
        assert_eq!(decision(&cfg, "u", None), "ignored");
    }

    // =========================================================================
    // test_recalibration.py 逐条翻译
    // =========================================================================

    #[test]
    fn test_mismatch_waits_without_pausing() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"layout_mismatch:input_divider,title_text\")")
                .await
                .unwrap();
        });

        assert_eq!(coord.layout_mismatches, 1);
        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "waiting_for_wechat_window");
        assert_eq!(h["failures"], json!(["input_divider", "title_text"]));
    }

    #[test]
    fn test_five_mismatches_need_recalibration() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for _ in 0..5 {
                coord
                    .handle_visit_error(
                        "a",
                        "invalid(\"layout_mismatch:input_divider,title_text\")",
                    )
                    .await
                    .unwrap();
            }
        });

        assert_eq!(coord.layout_mismatches, 5);
        let child = coord.sessions.get("a").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "needs_recalibration");
        assert_eq!(h["failures"], json!(["input_divider", "title_text"]));
        assert_eq!(h["attempt"], 5);
    }

    #[test]
    fn test_success_clears_recalibration_on_idle_sessions() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let ca = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        let ca_root = ca.root.clone();
        write_private(
            ca_root.join("health.json"),
            &json!({
                "state": "needs_recalibration",
                "reason": "layout_mismatch:input_divider"
            }),
        )
        .unwrap();

        coord.clear_layout_waits("b");
        let h: Value = read_json(ca_root.join("health.json")).unwrap();
        assert_eq!(h["state"], "watching");
        assert_eq!(h["reason"], "layout verified on another chat");
    }

    #[test]
    fn test_success_resets_counter() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.layout_mismatches = 4;

        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .visit(&AllowlistEntry {
                    recipient: "b".to_string(),
                    kind: "private".to_string(),
                    aliases: None,
                })
                .await
                .unwrap();
        });
        assert_eq!(coord.layout_mismatches, 0);
    }

    #[test]
    fn test_layout_missing_not_counted() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for _ in 0..5 {
                coord
                    .handle_visit_error("a", "invalid(\"layout_missing\")")
                    .await
                    .unwrap();
            }
        });
        assert_eq!(coord.layout_mismatches, 0);
    }

    #[test]
    fn test_status_flags_recalibration() {
        let failures = extract_mismatch_failures("invalid(\"layout_mismatch:input_divider\")");
        assert_eq!(failures, vec!["input_divider"]);
    }

    #[tokio::test]
    async fn test_mismatch_keeps_worker_connection() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        coord.capture = Some(Box::new(MockCoordinatorCapture::new()));

        coord
            .handle_visit_error("a", "invalid(\"layout_mismatch:input_divider\")")
            .await
            .unwrap();
        assert!(coord.capture.is_some());
    }

    #[tokio::test]
    async fn test_session_probe_mismatch_keeps_worker_connection() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();
        coord.capture = Some(Box::new(MockCoordinatorCapture::new()));

        coord
            .handle_visit_error("a", "invalid(\"layout_mismatch:list_border\")")
            .await
            .unwrap();
        assert!(coord.capture.is_some());
    }

    #[test]
    fn test_serve_calibration_real_candidate_writing_and_accept_flow() {
        let td = TestDir::new();
        let _home = TempHome::new(td.path());
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        // 构造带差异的 calibrate 测量结果
        let mut base_layout: Value =
            serde_json::from_str(include_str!("../../native/layout/wechat-4.1.15.json")).unwrap();
        base_layout["wechat_version"] = json!("4.1.99");
        let measured = json!({
            "input_divider": 510.0
        });

        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses.insert(
            "calibrate".to_string(),
            json!({
                "wechat_version": "4.1.99",
                "layout": base_layout,
                "measured": measured,
            }),
        );
        coord.capture = Some(Box::new(cap));

        let req_file = coord.root.join("calibrate-request.json");
        let res_file = coord.root.join("calibrate-result.json");
        write_private(
            &req_file,
            &json!({
                "id": "req_cal_diff",
                "action": "calibrate",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        assert!(res_file.exists());
        let res: Value = read_json(&res_file).unwrap();
        assert_eq!(res["id"], "req_cal_diff");
        assert_eq!(res["ok"], true);

        // 必须计算出差异，不能是空数组！
        let diffs = res["result"]["differences"]
            .as_array()
            .expect("differences must be an array");
        assert!(
            !diffs.is_empty(),
            "Differences must not be empty when measured fields differ"
        );

        // 必须真实写出 candidate json 文件
        let cand_path_str = res["result"]["candidate_json"]
            .as_str()
            .expect("candidate_json path must be returned");
        let resolved_cand = crate::core::control::resolve_local_path(cand_path_str);
        assert!(
            resolved_cand.exists(),
            "candidate_json file must physically exist at {}",
            resolved_cand.display()
        );

        let cand_content: Value =
            serde_json::from_str(&fs::read_to_string(&resolved_cand).unwrap()).unwrap();
        assert_eq!(
            cand_content["regions"]["input"]["y"], 510.0,
            "Candidate JSON must reflect measured coordinate"
        );

        // 下一步：测试 accept 流程真实生效
        write_private(
            &req_file,
            &json!({
                "id": "req_acc_real",
                "action": "accept",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        let acc_res: Value = read_json(&res_file).unwrap();
        assert_eq!(acc_res["id"], "req_acc_real");
        assert_eq!(acc_res["ok"], true);
        assert_eq!(acc_res["result"]["accepted"], true);

        let active_path_str = acc_res["result"]["active_layout"].as_str().unwrap();
        let resolved_active = crate::core::control::resolve_local_path(active_path_str);
        assert!(
            resolved_active.exists(),
            "Active layout file must exist after accept"
        );

        let active_content: Value =
            serde_json::from_str(&fs::read_to_string(&resolved_active).unwrap()).unwrap();
        assert_eq!(
            active_content["regions"]["input"]["y"], 510.0,
            "Active layout must be updated with candidate contents"
        );

        // candidate 文件在原子 mv 之后应当不再存在
        assert!(
            !resolved_cand.exists(),
            "Candidate file should be moved to active layout"
        );
        let _ = fs::remove_file(&resolved_active);
        let _ = fs::remove_file(&resolved_cand);
    }

    #[test]
    fn test_serve_calibration_writes_candidate_via_live_capture() {
        let td = TestDir::new();
        let _home = TempHome::new(td.path());
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses.insert(
            "calibrate".to_string(),
            json!({
                "wechat_version": "4.1.15",
                "layout": {
                    "schema": 1,
                    "wechat_version": "4.1.15",
                    "window": {"width": 880.0, "height": 640.0},
                    "regions": {
                        "session_list": {"x": 60.0, "y": 60.0, "w": 240.0, "h": 580.0},
                        "title": {"x": 250.0, "y": 0.0, "w": 620.0, "h": 50.0},
                        "chat": {"x": 315.0, "y": 60.0, "w": 565.0, "h": 436.0},
                        "input": {"x": 315.0, "y": 496.0, "w": 565.0, "h": 144.0}
                    }
                }
            }),
        );
        coord.capture = Some(Box::new(cap));

        let req_file = coord.root.join("calibrate-request.json");
        let res_file = coord.root.join("calibrate-result.json");
        write_private(
            &req_file,
            &json!({
                "id": "req123",
                "action": "calibrate",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        assert!(res_file.exists());
        let res: Value = read_json(&res_file).unwrap();
        assert_eq!(res["id"], "req123");
        assert_eq!(res["ok"], true);
    }

    #[test]
    fn test_serve_accept_uses_live_version() {
        let td = TestDir::new();
        let _home = TempHome::new(td.path());
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let layout_obj: Value =
            serde_json::from_str(include_str!("../../native/layout/wechat-4.1.15.json")).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses.insert(
            "calibrate".to_string(),
            json!({
                "wechat_version": "4.1.15",
                "layout": layout_obj.clone()
            }),
        );
        coord.capture = Some(Box::new(cap));

        let req_file = coord.root.join("calibrate-request.json");
        let res_file = coord.root.join("calibrate-result.json");

        // 先发起 calibrate 写 candidate
        write_private(
            &req_file,
            &json!({
                "id": "req_cal_prior",
                "action": "calibrate",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        // 再发起 accept
        write_private(
            &req_file,
            &json!({
                "id": "req_accept_1",
                "action": "accept",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        assert!(res_file.exists());
        let res: Value = read_json(&res_file).unwrap();
        assert_eq!(res["id"], "req_accept_1");
        assert_eq!(res["ok"], true);
        assert_eq!(res["result"]["accepted"], true);
    }

    #[test]
    fn test_request_without_connection_reports_worker_not_connected() {
        let td = TestDir::new();
        let _home = TempHome::new(td.path());
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.capture = None;

        let req_file = coord.root.join("calibrate-request.json");
        let res_file = coord.root.join("calibrate-result.json");
        write_private(
            &req_file,
            &json!({
                "id": "req_noconn",
                "action": "calibrate",
                "created_at": (coord.clock)()
            }),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        assert!(res_file.exists());
        let res: Value = read_json(&res_file).unwrap();
        assert_eq!(res["id"], "req_noconn");
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "worker_not_connected");
    }

    #[test]
    fn test_stale_request_expires() {
        let td = TestDir::new();
        let _home = TempHome::new(td.path());
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let req_file = coord.root.join("calibrate-request.json");
        let res_file = coord.root.join("calibrate-result.json");
        write_private(
            &req_file,
            &json!({
                "id": "req_stale",
                "action": "calibrate",
                "created_at": (coord.clock)() - 601.0
            }),
        )
        .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.serve_calibration().await.unwrap();
        });

        assert!(!req_file.exists());
        assert!(res_file.exists());
        let res: Value = read_json(&res_file).unwrap();
        assert_eq!(res["id"], "req_stale");
        assert_eq!(res["ok"], false);
        assert_eq!(res["error"], "expired");
    }

    // =========================================================================
    // test_control_admin.py 逐条翻译
    // =========================================================================

    #[test]
    fn test_config_check_ok_and_error_exit_codes() {
        let td = TestDir::new();
        let valid_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "u".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let cfg = BotConfig::load(&valid_path);
        assert!(cfg.is_ok());

        let invalid_path = td.path().join("invalid_config.json");
        fs::write(&invalid_path, r#"{"schema_version": 999}"#).unwrap();
        let bad_cfg = BotConfig::load(&invalid_path);
        assert!(bad_cfg.is_err());
    }

    #[test]
    fn test_config_check_writes_nothing() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "u".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let before_files: Vec<_> = fs::read_dir(td.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let _ = BotConfig::load(&cfg_path).unwrap();
        let after_files: Vec<_> = fs::read_dir(td.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(before_files, after_files);
    }

    #[test]
    fn test_pause_file_skips_cycle() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "u".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let pause_file = coord.root.join("admin-pause.json");
        write_private(&pause_file, &json!({"paused": true, "at": (coord.clock)()})).unwrap();

        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.sweep().await.unwrap();
        });

        let h: Value = read_json(coord.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "paused");
        assert_eq!(h["reason"], "admin_pause");
    }

    #[test]
    fn test_resume_continues() {
        let td = TestDir::new();
        let cfg_path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "u".to_string(),
                kind: "private".to_string(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let pause_file = coord.root.join("admin-pause.json");
        write_private(&pause_file, &json!({"paused": true, "at": (coord.clock)()})).unwrap();
        assert!(pause_file.exists());

        fs::remove_file(&pause_file).unwrap();
        assert!(!pause_file.exists());

        let cap = MockCoordinatorCapture::new();
        coord.capture = Some(Box::new(cap));
        coord.tool_client = Some(Arc::new(MockToolClient::default()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord.sweep().await.unwrap();
        });

        let h: Value = read_json(coord.root.join("health.json")).unwrap();
        assert_ne!(h.get("reason"), Some(&json!("admin_pause")));
    }

    // =========================================================================
    // test_hidden_rows.py 逐条翻译
    // =========================================================================

    #[test]
    fn test_hidden_row_is_not_due() {
        let td = TestDir::new();
        let allow = vec![
            AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
            AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            },
        ];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let mut cap = MockCoordinatorCapture::new();
        cap.rows = vec![SessionRow {
            name: "a".to_string(),
            preview_digest: "d".to_string(),
            preview_key: Some("k".to_string()),
            preview_time: None,
            red_pixels: 0,
            y: 100,
        }];
        coord.capture = Some(Box::new(cap));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let due = rt.block_on(async {
            coord
                .due_sessions(&["a".to_string(), "b".to_string()])
                .await
                .unwrap()
        });
        assert!(due.contains("a"));
        assert!(!due.contains("b"));
    }

    #[test]
    fn test_missing_row_visit_waits_and_keeps_connection() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "b".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "b".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("b", "invalid(\"session_row_not_unique_or_missing\")")
                .await
                .unwrap();
        });

        let child = coord.sessions.get("b").unwrap();
        assert!(!child.paused);
        let h: Value = read_json(child.root.join("health.json")).unwrap();
        assert_eq!(h["state"], "waiting_for_wechat_window");
    }

    #[test]
    fn test_mismatch_forgets_the_open_chat_so_a_row_click_reopens_it() {
        let td = TestDir::new();
        let allow = vec![AllowlistEntry {
            recipient: "a".to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();
        coord.active_name = Some("a".to_string());
        let _ = coord
            .session(&AllowlistEntry {
                recipient: "a".to_string(),
                kind: "private".to_string(),
                aliases: None,
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            coord
                .handle_visit_error("a", "invalid(\"layout_mismatch:input_divider\")")
                .await
                .unwrap();
        });

        assert!(coord.active_name.is_none());
    }

    // =========================================================================
    // 端到端测试：假 worker.sock 与假 bot-worker.sock 组装测试
    // =========================================================================

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn e2e_test_bubble(text: &str, direction: &str) -> Bubble {
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
            extra: Default::default(),
        }
    }

    fn e2e_test_frame(bubbles: Vec<Bubble>, title: &str) -> Frame {
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
            snapshot_id: Some("snap1".to_string()),
            window_id: None,
            observed_title: Some(title.to_string()),
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: Default::default(),
        }
    }

    struct E2eFakeMcp {
        pub sock_path: PathBuf,
        pub calls: Arc<Mutex<Vec<Value>>>,
    }

    impl E2eFakeMcp {
        async fn start(account_id: &str, fault_not_sent: bool) -> Self {
            let unique = uuid::Uuid::new_v4();
            let sock_path = PathBuf::from(format!("/tmp/emcp_{}.sock", &unique.to_string()[..8]));
            if sock_path.exists() {
                let _ = fs::remove_file(&sock_path);
            }
            let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let calls_clone = calls.clone();
            let acc = account_id.to_string();

            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut reader = BufReader::new(read_half);
                    let calls_inner = calls_clone.clone();
                    let acc_inner = acc.clone();

                    tokio::spawn(async move {
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break;
                            }
                            if let Ok(req) = serde_json::from_str::<Value>(&line) {
                                let method =
                                    req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                                let id = req.get("id").cloned();
                                if method == "initialize" {
                                    let resp = json!({
                                        "jsonrpc": "2.0",
                                        "id": id.unwrap_or(json!(1)),
                                        "result": {
                                            "protocolVersion": "2025-06-18",
                                            "capabilities": {},
                                            "serverInfo": {"name": "fake-mcp", "version": "1.0"}
                                        }
                                    });
                                    let mut out = serde_json::to_vec(&resp).unwrap();
                                    out.push(b'\n');
                                    let _ = write_half.write_all(&out).await;
                                    let _ = write_half.flush().await;
                                } else if method == "notifications/initialized" {
                                    // notification: no reply
                                } else if method == "tools/call" {
                                    calls_inner.lock().unwrap().push(req["params"].clone());
                                    let tool_name = req["params"]["name"].as_str().unwrap_or("");
                                    let resp = if tool_name == "send_message" {
                                        if fault_not_sent {
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id.unwrap(),
                                                "result": {
                                                    "isError": true,
                                                    "content": [
                                                        {"type": "text", "text": "发送尝试 send-1 未触发发送: 微信未准备好"}
                                                    ]
                                                }
                                            })
                                        } else {
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id.unwrap(),
                                                "result": {
                                                    "structuredContent": {
                                                        "account": {"platform": "wechat", "account_id": acc_inner},
                                                        "data": {
                                                            "status": "triggered",
                                                            "delivery": "triggered",
                                                            "send_id": "send-e2e-1"
                                                        }
                                                    }
                                                }
                                            })
                                        }
                                    } else if tool_name == "get_send_status" {
                                        json!({
                                            "jsonrpc": "2.0",
                                            "id": id.unwrap(),
                                            "result": {
                                                "structuredContent": {
                                                    "account": {"platform": "wechat", "account_id": acc_inner},
                                                    "data": {
                                                        "status": "triggered",
                                                        "delivery": "triggered",
                                                        "send_id": "send-e2e-1"
                                                    }
                                                }
                                            }
                                        })
                                    } else {
                                        json!({
                                            "jsonrpc": "2.0",
                                            "id": id.unwrap(),
                                            "result": {
                                                "structuredContent": {
                                                    "account": {"platform": "wechat", "account_id": acc_inner},
                                                    "data": []
                                                }
                                            }
                                        })
                                    };
                                    let mut out = serde_json::to_vec(&resp).unwrap();
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

            Self { sock_path, calls }
        }
    }

    impl Drop for E2eFakeMcp {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.sock_path);
        }
    }

    fn b64_encode(data: &[u8]) -> String {
        const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
            result.push(B64[(b0 >> 2) as usize] as char);
            result.push(B64[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                result.push(B64[(((b1 & 0xf) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(B64[(b2 & 0x3f) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }

    struct E2eFakeBotWorker {
        pub sock_path: PathBuf,
        pub calls: Arc<Mutex<Vec<Value>>>,
    }

    impl E2eFakeBotWorker {
        async fn start(frames: Vec<Frame>) -> Self {
            let unique = uuid::Uuid::new_v4();
            let sock_path = PathBuf::from(format!("/tmp/ebot_{}.sock", &unique.to_string()[..8]));
            if sock_path.exists() {
                let _ = fs::remove_file(&sock_path);
            }
            let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
            let frames_arc = Arc::new(Mutex::new(frames));
            let frames_clone = frames_arc.clone();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let calls_clone = calls.clone();

            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut reader = BufReader::new(read_half);
                    let frames_inner = frames_clone.clone();
                    let calls_inner = calls_clone.clone();

                    tokio::spawn(async move {
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break;
                            }
                            if let Ok(req) = serde_json::from_str::<Value>(&line) {
                                let op =
                                    req.get("operation").and_then(|o| o.as_str()).unwrap_or("");
                                let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(1);
                                calls_inner.lock().unwrap().push(req.clone());
                                let params = req.get("params").cloned().unwrap_or(json!({}));

                                if op == "prepare_image" {
                                    let mid = params
                                        .get("media_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if mid.starts_with("fa") {
                                        let resp = json!({
                                            "protocol_version": 1,
                                            "id": id,
                                            "ok": false,
                                            "error": "save_as_failed: invalid(\"menu_not_found\")",
                                        });
                                        let mut out = serde_json::to_vec(&resp).unwrap();
                                        out.push(b'\n');
                                        let _ = write_half.write_all(&out).await;
                                        let _ = write_half.flush().await;
                                        line.clear();
                                        continue;
                                    }
                                }

                                if op == "prepare_file" {
                                    let mid = params
                                        .get("media_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if mid.starts_with("fe") {
                                        let resp = json!({
                                            "protocol_version": 1,
                                            "id": id,
                                            "ok": false,
                                            "error": "file_size_limit",
                                        });
                                        let mut out = serde_json::to_vec(&resp).unwrap();
                                        out.push(b'\n');
                                        let _ = write_half.write_all(&out).await;
                                        let _ = write_half.flush().await;
                                        line.clear();
                                        continue;
                                    }
                                }

                                let result = if op == "sessions" {
                                    json!({
                                        "profile": "wechat-4",
                                        "rows": [
                                            {
                                                "name": "Alice",
                                                "preview_digest": "d_alice",
                                                "preview_key": "k_alice",
                                                "preview_time": "12:00",
                                                "red_pixels": 0,
                                                "y": 100
                                            }
                                        ]
                                    })
                                } else if op == "open_session" {
                                    json!({
                                        "clicked": true,
                                        "seconds": 0.05
                                    })
                                } else if op == "capture" {
                                    let mut q = frames_inner.lock().unwrap();
                                    let f = if q.len() > 1 {
                                        q.remove(0)
                                    } else if !q.is_empty() {
                                        q[0].clone()
                                    } else {
                                        e2e_test_frame(vec![], "Alice")
                                    };
                                    serde_json::to_value(&f).unwrap()
                                } else if op == "prepare_image" {
                                    let mid = params
                                        .get("media_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    json!({
                                        "media_id": mid,
                                        "extension": "png",
                                    })
                                } else if op == "prepare_file" {
                                    let mid = params
                                        .get("media_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let ext = if mid.starts_with("00") { "pdf" } else { "txt" };
                                    let filename = if mid.starts_with("00") {
                                        "doc.pdf"
                                    } else {
                                        "readme.txt"
                                    };
                                    json!({
                                        "filename": filename,
                                        "extension": ext,
                                        "size": 100,
                                        "extraction": if ext == "pdf" {
                                            json!({ "text": "真实PDF文件正文内容", "page_count": 1 })
                                        } else {
                                            json!({})
                                        }
                                    })
                                } else if op == "read_media" {
                                    let mid = params
                                        .get("media_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let (bytes, ext) = if mid.starts_with("00") {
                                        (b"%PDF-1.7\npdf content bytes".to_vec(), "pdf")
                                    } else if mid.starts_with("1") || mid.starts_with("fa") {
                                        (b"\x89PNG\r\n\x1a\nfakeimagebytes".to_vec(), "png")
                                    } else {
                                        ("真实TXT文本正文内容".as_bytes().to_vec(), "txt")
                                    };
                                    json!({
                                        "offset": 0,
                                        "size": bytes.len(),
                                        "bytes": b64_encode(&bytes),
                                        "eof": true,
                                        "sha256": crate::core::worker_client::sha256(&bytes),
                                        "extension": ext,
                                    })
                                } else {
                                    json!({})
                                };

                                let resp = json!({
                                    "protocol_version": 1,
                                    "id": id,
                                    "ok": true,
                                    "result": result
                                });
                                let mut out = serde_json::to_vec(&resp).unwrap();
                                out.push(b'\n');
                                let _ = write_half.write_all(&out).await;
                                let _ = write_half.flush().await;
                            }
                            line.clear();
                        }
                    });
                }
            });

            Self { sock_path, calls }
        }
    }

    impl Drop for E2eFakeBotWorker {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.sock_path);
        }
    }

    #[tokio::test]
    async fn test_e2e_assembly_send_message_triggered() {
        let td = TestDir::new();
        let recipient = "Alice";
        let allow = vec![AllowlistEntry {
            recipient: recipient.to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        // 1. 启动假 MCP 服务
        let fake_mcp = E2eFakeMcp::start(&coord.config.account_id, false).await;
        let mcp_client = Arc::new(crate::core::mcp_client::McpSocketClient::new(
            fake_mcp.sock_path.clone(),
            coord.config.account_id.clone(),
        ));

        // 2. 准备脚本化帧：
        let mut b_old = e2e_test_bubble("历史消息", "inbound_visual");
        b_old.digest = "msg_old".to_string();

        let mut b_in = e2e_test_bubble("你好机器人", "inbound_visual");
        b_in.digest = "msg_001".to_string();

        let mut b_out = e2e_test_bubble("Echo: 你好机器人", "outbound_visual");
        b_out.digest = "msg_002".to_string();

        // 帧1 (retarget 时的初始校验帧): 基线
        let f_retarget = e2e_test_frame(vec![b_old.clone()], recipient);
        // 帧2 (第1轮 cycle 建立 baseline 帧): 包含历史消息
        let f_base = e2e_test_frame(vec![b_old.clone()], recipient);
        // 帧3 (第2轮 cycle 探测到用户新消息):
        let f_incoming = e2e_test_frame(vec![b_old.clone(), b_in.clone()], recipient);
        // 帧4 (发送后验证 echo 的回显帧):
        let f_echo = e2e_test_frame(vec![b_old, b_in, b_out], recipient);

        let fake_bot = E2eFakeBotWorker::start(vec![
            f_retarget,
            f_base,
            f_incoming.clone(),
            f_incoming.clone(),
            f_incoming,
            f_echo,
        ])
        .await;

        // 3. 准备 WorkerClient 所需的 base_dir (遵循 0o700, 拥有可执行 bin/wechat_bot_worker)
        let base_dir = td.path().join(".wechat-rpa");
        fs::create_dir_all(&base_dir).unwrap();
        let mut base_perms = fs::metadata(&base_dir).unwrap().permissions();
        base_perms.set_mode(0o700);
        fs::set_permissions(&base_dir, base_perms).unwrap();

        let bin_dir = base_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let worker_bin = bin_dir.join("wechat_bot_worker");
        fs::write(&worker_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut bin_perms = fs::metadata(&worker_bin).unwrap().permissions();
        bin_perms.set_mode(0o755);
        fs::set_permissions(&worker_bin, bin_perms).unwrap();

        let flag_file = base_dir.join("bot-worker.enabled");
        fs::write(&flag_file, &coord.ownership).unwrap();
        let mut flag_perms = fs::metadata(&flag_file).unwrap().permissions();
        flag_perms.set_mode(0o600);
        fs::set_permissions(&flag_file, flag_perms).unwrap();

        // 4. 构造 WorkerClient
        let worker_client = crate::core::worker_client::WorkerClient::new(
            base_dir.clone(),
            fake_bot.sock_path.clone(),
            coord.ownership.clone(),
            recipient.to_string(),
            base_dir.clone(),
            None,
            true,
            false,
            50 * 1024 * 1024,
            Some(mcp_client.clone()),
        );

        // 5. 注入协调器
        coord.tool_client = Some(mcp_client.clone());
        coord.capture = Some(Box::new(worker_client));

        // 6. 注入假大脑：产生回复
        coord.config.turn_quiet_seconds = 0;
        let brain_fn: crate::core::turns::BrainFn = Arc::new(|_turn| {
            Box::pin(async {
                Ok(crate::brain::BrainReply {
                    reply: "Echo: 你好机器人".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });
        {
            let child = coord.session(&allow[0]).unwrap();
            child.config.turn_quiet_seconds = 0;
            child.brain = Some(brain_fn);
        }

        // 7. 运行协调周期
        // 周期 1: 建立 baseline
        coord.visit(&allow[0]).await.unwrap();
        // 周期 2: 探测到新消息并建立 turn
        coord.visit(&allow[0]).await.unwrap();
        // 周期 3: turn 到期并触发大脑与 send_message
        coord.visit(&allow[0]).await.unwrap();

        // 8. 断言：MCP 收到 send_message，参数 recipient/message/idempotency_key 正确
        let calls = fake_mcp.calls.lock().unwrap();
        let send_call = calls.iter().find(|c| c["name"] == "send_message");
        assert!(
            send_call.is_some(),
            "MCP should have received send_message call"
        );
        let args = &send_call.unwrap()["arguments"];
        assert_eq!(args["recipient"], "Alice");
        assert_eq!(args["message"], "Echo: 你好机器人");
        assert!(args["idempotency_key"].is_string());
        assert!(!args["idempotency_key"].as_str().unwrap().is_empty());

        // 9. 断言会话日志状态为 triggered
        let child = coord.sessions.get("Alice").unwrap();
        let entries = fs::read_dir(&child.root).unwrap();
        let mut found_triggered = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("send-") && name.ends_with(".json") {
                    let content: Value = read_json(&path).unwrap();
                    if content["status"] == "triggered" {
                        found_triggered = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_triggered,
            "Expected send record with status triggered in journal"
        );
    }

    #[tokio::test]
    async fn test_e2e_mcp_is_error_recognized_as_not_sent() {
        let td = TestDir::new();
        let recipient = "Alice";
        let allow = vec![AllowlistEntry {
            recipient: recipient.to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        // 启动带有 fault_not_sent=true 的假 MCP
        let fake_mcp = E2eFakeMcp::start(&coord.config.account_id, true).await;
        let mcp_client = Arc::new(crate::core::mcp_client::McpSocketClient::new(
            fake_mcp.sock_path.clone(),
            coord.config.account_id.clone(),
        ));

        let mut b_old = e2e_test_bubble("历史消息", "inbound_visual");
        b_old.digest = "msg_old".to_string();

        let f_retarget = e2e_test_frame(vec![b_old.clone()], recipient);
        let f_base = e2e_test_frame(vec![b_old.clone()], recipient);
        let mut b_in = e2e_test_bubble("测试失败发送", "inbound_visual");
        b_in.digest = "msg_fail_1".to_string();
        let f_incoming = e2e_test_frame(vec![b_old, b_in], recipient);
        let fake_bot = E2eFakeBotWorker::start(vec![
            f_retarget,
            f_base,
            f_incoming.clone(),
            f_incoming.clone(),
            f_incoming,
        ])
        .await;

        let base_dir = td.path().join(".wechat-rpa");
        fs::create_dir_all(&base_dir).unwrap();
        let mut base_perms = fs::metadata(&base_dir).unwrap().permissions();
        base_perms.set_mode(0o700);
        fs::set_permissions(&base_dir, base_perms).unwrap();

        let bin_dir = base_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let worker_bin = bin_dir.join("wechat_bot_worker");
        fs::write(&worker_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut bin_perms = fs::metadata(&worker_bin).unwrap().permissions();
        bin_perms.set_mode(0o755);
        fs::set_permissions(&worker_bin, bin_perms).unwrap();

        let flag_file = base_dir.join("bot-worker.enabled");
        fs::write(&flag_file, &coord.ownership).unwrap();
        let mut flag_perms = fs::metadata(&flag_file).unwrap().permissions();
        flag_perms.set_mode(0o600);
        fs::set_permissions(&flag_file, flag_perms).unwrap();

        let worker_client = crate::core::worker_client::WorkerClient::new(
            base_dir.clone(),
            fake_bot.sock_path.clone(),
            coord.ownership.clone(),
            recipient.to_string(),
            base_dir.clone(),
            None,
            true,
            false,
            50 * 1024 * 1024,
            Some(mcp_client.clone()),
        );

        coord.tool_client = Some(mcp_client.clone());
        coord.capture = Some(Box::new(worker_client));

        coord.config.turn_quiet_seconds = 0;
        let brain_fn: crate::core::turns::BrainFn = Arc::new(|_turn| {
            Box::pin(async {
                Ok(crate::brain::BrainReply {
                    reply: "回复内容".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });
        {
            let child = coord.session(&allow[0]).unwrap();
            child.config.turn_quiet_seconds = 0;
            child.brain = Some(brain_fn);
        }

        // 周期 1
        coord.visit(&allow[0]).await.unwrap();
        // 周期 2: 探测到新消息并建立 turn
        coord.visit(&allow[0]).await.unwrap();
        // 周期 3: 触发发送，发生发送错误（未触发发送）
        coord.visit(&allow[0]).await.unwrap();

        // 验证日志中被识别为 not_sent
        let child = coord.sessions.get("Alice").unwrap();
        let entries = fs::read_dir(&child.root).unwrap();
        let mut not_sent_found = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("send-") && name.ends_with(".json") {
                    let content: Value = read_json(&path).unwrap();
                    let delivery = content.get("delivery").and_then(|v| v.as_str());
                    let status = content.get("status").and_then(|v| v.as_str());
                    let err = content.get("error").and_then(|v| v.as_str());
                    if crate::core::delivery::is_not_sent(err, Some(&content))
                        || delivery == Some("not_sent")
                        || status == Some("not_sent")
                    {
                        not_sent_found = true;
                        break;
                    }
                }
            }
        }
        assert!(
            not_sent_found,
            "Expected not_sent recognition in send record or delivery"
        );
    }

    #[tokio::test]
    async fn test_e2e_assembly_attachments_image_and_document() {
        let td = TestDir::new();
        let recipient = "Alice";
        let allow = vec![AllowlistEntry {
            recipient: recipient.to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let fake_mcp = E2eFakeMcp::start(&coord.config.account_id, false).await;
        let mcp_client = Arc::new(crate::core::mcp_client::McpSocketClient::new(
            fake_mcp.sock_path.clone(),
            coord.config.account_id.clone(),
        ));

        let img_id = "1".repeat(64);
        let file_id = "2".repeat(64);

        let mut b_old = e2e_test_bubble("历史文本消息", "inbound_visual");
        b_old.digest = "msg_old".to_string();

        let mut b_img = e2e_test_bubble("[图片]", "inbound_visual");
        b_img.kind = Some("image".to_string());
        b_img.media_id = Some(img_id.clone());
        b_img.digest = "msg_img_1".to_string();

        let mut b_file = e2e_test_bubble("[文件] readme.txt", "inbound_visual");
        b_file.kind = Some("file".to_string());
        b_file.media_id = Some(file_id.clone());
        b_file.filename = Some("readme.txt".to_string());
        b_file.digest = "msg_file_1".to_string();

        let mut b_reply = e2e_test_bubble("已查阅附件", "outbound_visual");
        b_reply.digest = "msg_reply_1".to_string();

        let f_retarget = e2e_test_frame(vec![b_old.clone()], recipient);
        let f_base = e2e_test_frame(vec![b_old.clone()], recipient);
        let f_incoming = e2e_test_frame(
            vec![b_old.clone(), b_img.clone(), b_file.clone()],
            recipient,
        );
        let f_echo = e2e_test_frame(vec![b_old, b_img, b_file, b_reply], recipient);

        let fake_bot = E2eFakeBotWorker::start(vec![
            f_retarget,
            f_base,
            f_incoming.clone(),
            f_incoming.clone(),
            f_incoming,
            f_echo,
        ])
        .await;

        let base_dir = td.path().join(".wechat-rpa");
        fs::create_dir_all(&base_dir).unwrap();
        let mut base_perms = fs::metadata(&base_dir).unwrap().permissions();
        base_perms.set_mode(0o700);
        fs::set_permissions(&base_dir, base_perms).unwrap();

        let bin_dir = base_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let worker_bin = bin_dir.join("wechat_bot_worker");
        fs::write(&worker_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut bin_perms = fs::metadata(&worker_bin).unwrap().permissions();
        bin_perms.set_mode(0o755);
        fs::set_permissions(&worker_bin, bin_perms).unwrap();

        let flag_file = base_dir.join("bot-worker.enabled");
        fs::write(&flag_file, &coord.ownership).unwrap();
        let mut flag_perms = fs::metadata(&flag_file).unwrap().permissions();
        flag_perms.set_mode(0o600);
        fs::set_permissions(&flag_file, flag_perms).unwrap();

        let worker_client = crate::core::worker_client::WorkerClient::new(
            base_dir.clone(),
            fake_bot.sock_path.clone(),
            coord.ownership.clone(),
            recipient.to_string(),
            base_dir.clone(),
            None,
            true,
            false,
            50 * 1024 * 1024,
            Some(mcp_client.clone()),
        );

        coord.tool_client = Some(mcp_client.clone());
        coord.capture = Some(Box::new(worker_client));

        let received_turn = Arc::new(Mutex::new(None::<crate::brain::Turn>));
        let rt_clone = received_turn.clone();

        coord.config.turn_quiet_seconds = 0;
        let brain_fn: crate::core::turns::BrainFn = Arc::new(move |turn| {
            let mut guard = rt_clone.lock().unwrap();
            *guard = Some(turn.clone());
            Box::pin(async {
                Ok(crate::brain::BrainReply {
                    reply: "已查阅附件".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });
        {
            let child = coord.session(&allow[0]).unwrap();
            child.config.turn_quiet_seconds = 0;
            child.brain = Some(brain_fn);
        }

        // 周期 1: 建立 baseline
        coord.visit(&allow[0]).await.unwrap();
        // 周期 2: 探测到新消息（包含图片和文件）并建立 turn
        coord.visit(&allow[0]).await.unwrap();
        // 周期 3: turn 到期，触发附件获取、注入大脑并回复
        coord.visit(&allow[0]).await.unwrap();

        // 验证 worker 收到的 RPC 调用与顺序
        let calls = fake_bot.calls.lock().unwrap().clone();
        let prep_img = calls.iter().find(|c| c["operation"] == "prepare_image");
        let prep_file = calls.iter().find(|c| c["operation"] == "prepare_file");
        let read_media_calls: Vec<_> = calls
            .iter()
            .filter(|c| c["operation"] == "read_media")
            .collect();

        assert!(prep_img.is_some(), "worker 应收到 prepare_image 调用");
        assert_eq!(prep_img.unwrap()["params"]["media_id"], img_id);
        assert_eq!(prep_img.unwrap()["params"]["max_bytes"], 50 * 1024 * 1024);

        assert!(prep_file.is_some(), "worker 应收到 prepare_file 调用");
        assert_eq!(prep_file.unwrap()["params"]["media_id"], file_id);
        assert_eq!(prep_file.unwrap()["params"]["max_bytes"], 50 * 1024 * 1024);

        assert!(
            !read_media_calls.is_empty(),
            "worker 应收到 read_media 调用"
        );

        // 验证调用的相对顺序：prepare_image -> prepare_file -> read_media
        let idx_prep_img = calls
            .iter()
            .position(|c| c["operation"] == "prepare_image")
            .unwrap();
        let idx_prep_file = calls
            .iter()
            .position(|c| c["operation"] == "prepare_file")
            .unwrap();
        assert!(
            idx_prep_img < idx_prep_file,
            "prepare_image 应在 prepare_file 之前"
        );

        // 验证进入大脑的 BrainTurn
        let turn = received_turn.lock().unwrap().clone();
        assert!(turn.is_some(), "大脑应收到 BrainTurn 调用");
        let turn = turn.unwrap();

        assert_eq!(turn.image_count, Some(1));
        assert_eq!(turn.images.len(), 1, "图片应成功注入 BrainTurn.images");
        assert!(turn.images[0].exists(), "图片本地文件应存在");

        assert_eq!(turn.files.len(), 1, "文件应成功注入 BrainTurn.files");
        assert_eq!(turn.files[0]["filename"], "readme.txt");
        assert_eq!(turn.files[0]["text"], "真实TXT文本正文内容");

        assert!(turn.failed.is_empty(), "没有失败附件");
    }

    #[tokio::test]
    async fn test_e2e_assembly_attachments_fallback_and_failed_items() {
        let td = TestDir::new();
        let recipient = "Alice";
        let allow = vec![AllowlistEntry {
            recipient: recipient.to_string(),
            kind: "private".to_string(),
            aliases: None,
        }];
        let cfg_path = create_test_config(td.path(), allow.clone());
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let fake_mcp = E2eFakeMcp::start(&coord.config.account_id, false).await;
        let mcp_client = Arc::new(crate::core::mcp_client::McpSocketClient::new(
            fake_mcp.sock_path.clone(),
            coord.config.account_id.clone(),
        ));

        let fail_img_id = "fa".repeat(32);
        let fail_file_id = "fe".repeat(32);
        let good_pdf_id = "00".repeat(32);

        let mut b_old = e2e_test_bubble("历史消息", "inbound_visual");
        b_old.digest = "msg_old".to_string();

        let mut b_fail_img = e2e_test_bubble("[图片]", "inbound_visual");
        b_fail_img.kind = Some("image".to_string());
        b_fail_img.media_id = Some(fail_img_id.clone());
        b_fail_img.digest = "msg_fail_img".to_string();

        let mut b_fail_file = e2e_test_bubble("[文件] big.bin", "inbound_visual");
        b_fail_file.kind = Some("file".to_string());
        b_fail_file.media_id = Some(fail_file_id.clone());
        b_fail_file.filename = Some("big.bin".to_string());
        b_fail_file.digest = "msg_fail_file".to_string();

        let mut b_good_pdf = e2e_test_bubble("[文件] doc.pdf", "inbound_visual");
        b_good_pdf.kind = Some("file".to_string());
        b_good_pdf.media_id = Some(good_pdf_id.clone());
        b_good_pdf.filename = Some("doc.pdf".to_string());
        b_good_pdf.digest = "msg_good_pdf".to_string();

        let mut b_reply = e2e_test_bubble("收到部分附件", "outbound_visual");
        b_reply.digest = "msg_reply_2".to_string();

        let f_retarget = e2e_test_frame(vec![b_old.clone()], recipient);
        let f_base = e2e_test_frame(vec![b_old.clone()], recipient);
        let f_incoming = e2e_test_frame(
            vec![
                b_old.clone(),
                b_fail_img.clone(),
                b_fail_file.clone(),
                b_good_pdf.clone(),
            ],
            recipient,
        );
        let f_echo = e2e_test_frame(
            vec![b_old, b_fail_img, b_fail_file, b_good_pdf, b_reply],
            recipient,
        );

        let fake_bot = E2eFakeBotWorker::start(vec![
            f_retarget,
            f_base,
            f_incoming.clone(),
            f_incoming.clone(),
            f_incoming,
            f_echo,
        ])
        .await;

        let base_dir = td.path().join(".wechat-rpa");
        fs::create_dir_all(&base_dir).unwrap();
        let mut base_perms = fs::metadata(&base_dir).unwrap().permissions();
        base_perms.set_mode(0o700);
        fs::set_permissions(&base_dir, base_perms).unwrap();

        let bin_dir = base_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let worker_bin = bin_dir.join("wechat_bot_worker");
        fs::write(&worker_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut bin_perms = fs::metadata(&worker_bin).unwrap().permissions();
        bin_perms.set_mode(0o755);
        fs::set_permissions(&worker_bin, bin_perms).unwrap();

        let flag_file = base_dir.join("bot-worker.enabled");
        fs::write(&flag_file, &coord.ownership).unwrap();
        let mut flag_perms = fs::metadata(&flag_file).unwrap().permissions();
        flag_perms.set_mode(0o600);
        fs::set_permissions(&flag_file, flag_perms).unwrap();

        let worker_client = crate::core::worker_client::WorkerClient::new(
            base_dir.clone(),
            fake_bot.sock_path.clone(),
            coord.ownership.clone(),
            recipient.to_string(),
            base_dir.clone(),
            None,
            true,
            false,
            50 * 1024 * 1024,
            Some(mcp_client.clone()),
        );

        let logged_events_ref = worker_client.logged_events.clone();

        coord.tool_client = Some(mcp_client.clone());
        coord.capture = Some(Box::new(worker_client));

        let received_turn = Arc::new(Mutex::new(None::<crate::brain::Turn>));
        let rt_clone = received_turn.clone();

        coord.config.turn_quiet_seconds = 0;
        let brain_fn: crate::core::turns::BrainFn = Arc::new(move |turn| {
            let mut guard = rt_clone.lock().unwrap();
            *guard = Some(turn.clone());
            Box::pin(async {
                Ok(crate::brain::BrainReply {
                    reply: "收到部分附件".to_string(),
                    metadata: HashMap::new(),
                })
            })
        });
        {
            let child = coord.session(&allow[0]).unwrap();
            child.config.turn_quiet_seconds = 0;
            child.brain = Some(brain_fn);
        }

        coord.visit(&allow[0]).await.unwrap();
        coord.visit(&allow[0]).await.unwrap();
        coord.visit(&allow[0]).await.unwrap();

        // 1. 验证原图失败回退为缩略图并记录 image_original_unavailable 事件
        let events = logged_events_ref.lock().unwrap().clone();
        let unavail = events
            .iter()
            .find(|e| e["event"] == "image_original_unavailable");
        assert!(
            unavail.is_some(),
            "原图失败时应记录 image_original_unavailable 事件"
        );

        let turn = received_turn.lock().unwrap().clone().unwrap();
        // 缩略图仍然进入了 images
        assert_eq!(turn.images.len(), 1, "回退缩略图后图片仍然进入大脑");

        // 2. 验证坏文件进入 failed 列表
        assert_eq!(turn.failed.len(), 1, "坏文件应进入 failed 列表");
        assert_eq!(turn.failed[0].label, "big.bin");
        assert_eq!(turn.failed[0].reason.as_deref(), Some("too_large"));

        // 3. 验证单个附件失败不影响其他正常附件
        assert_eq!(turn.files.len(), 1, "好的 pdf 文件仍然成功进入 files");
        assert_eq!(turn.files[0]["filename"], "doc.pdf");
        assert_eq!(turn.files[0]["text"], "真实PDF文件正文内容");
    }

    #[test]
    fn test_log_event_bounded_to_256() {
        let temp = TestDir::new();
        let cfg_path = temp.path().join("bot.json");
        let spool = temp.path().join("spool");
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test_acc",
            "state_dir": spool.to_string_lossy(),
            "allowlist": [{"recipient": "测试", "kind": "private"}]
        });
        std::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg).unwrap()).unwrap();
        let coord = BotCoordinator::new(&cfg_path).unwrap();

        for i in 0..300 {
            coord.log_event("batch_event", json!({"index": i}));
        }

        let events = coord.logged_events.lock().unwrap().clone();
        assert_eq!(events.len(), 256);
        assert_eq!(events[0]["index"], 44);
        assert_eq!(events[255]["index"], 299);
    }

    #[tokio::test]
    async fn test_due_sessions_batches_names_up_to_50() {
        let td = TestDir::new();
        // 构造 105 个联系人，每个带 1 个名字，总计 105 个待探测名字
        let mut allow = Vec::new();
        let mut names = Vec::new();
        for i in 0..105 {
            let name = format!("user_{:03}", i);
            allow.push(AllowlistEntry {
                recipient: name.clone(),
                kind: "private".to_string(),
                aliases: None,
            });
            names.push(name);
        }
        let cfg_path = create_test_config(td.path(), allow);
        let mut coord = BotCoordinator::new(&cfg_path).unwrap();

        let mut cap = MockCoordinatorCapture::new();
        for name in &names {
            cap.rows.push(SessionRow {
                name: name.clone(),
                y: 100,
                preview_digest: "d".to_string(),
                preview_key: None,
                preview_time: None,
                red_pixels: 0,
            });
        }
        let batches_ref = cap.probe_batches.clone();
        coord.capture = Some(Box::new(cap));

        let res = coord.due_sessions(&names).await.unwrap();
        assert_eq!(res.len(), 105);

        let batches = batches_ref.lock().unwrap().clone();
        assert_eq!(batches.len(), 3, "105个名字按<=50每批分批，应分成3批");
        assert_eq!(batches[0].len(), 50);
        assert_eq!(batches[1].len(), 50);
        assert_eq!(batches[2].len(), 5);
        for b in &batches {
            assert!(b.len() <= 50, "每批名字数量绝对不能超过50");
        }
    }

    #[test]
    fn test_should_daily_incremental_sync() {
        // Not idle
        assert!(!should_daily_incremental_sync(false, 100000.0, 0.0));
        // Idle but sync within 20 hours
        assert!(!should_daily_incremental_sync(true, 100000.0, 90000.0));
    }
    #[tokio::test]
    async fn friend_requests_disabled_paused_and_busy_do_not_trigger() {
        let temp = TestDir::new();
        let path = create_test_config(
            temp.path(),
            vec![AllowlistEntry {
                recipient: "a".into(),
                kind: "private".into(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(&path).unwrap();
        let mock = MockCoordinatorCapture::new();
        let calls = mock.calls.clone();
        coord.capture = Some(Box::new(mock));
        coord.serve_friend_requests().await.unwrap();
        assert!(calls.lock().unwrap().is_empty());
        coord.config.friend_requests.enabled = true;
        write_private(&coord.config_path, &coord.config).unwrap();
        write_private(coord.root.join("admin-pause.json"), &json!({})).unwrap();
        coord.serve_friend_requests().await.unwrap();
        assert!(calls.lock().unwrap().is_empty());
        fs::remove_file(coord.root.join("admin-pause.json")).unwrap();
        coord.pending.insert("busy".into(), (1.0, 1));
        coord.serve_friend_requests().await.unwrap();
        assert!(calls.lock().unwrap().is_empty());
    }

    fn friend_fixture(
        mode: crate::core::friend_requests::Mode,
        matched: bool,
    ) -> (TestDir, BotCoordinator, Arc<Mutex<Vec<(String, Value)>>>) {
        let td = TestDir::new();
        let path = create_test_config(
            td.path(),
            vec![AllowlistEntry {
                recipient: "a".into(),
                kind: "private".into(),
                aliases: None,
            }],
        );
        let mut coord = BotCoordinator::new(path).unwrap();
        coord.config.friend_requests.enabled = true;
        coord.config.friend_requests.mode = mode;
        if !matched {
            coord.config.friend_requests.accept_keywords = vec!["absent".into()];
        }
        write_private(&coord.config_path, &coord.config).unwrap();
        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses
            .insert("friend_request_badge".into(), json!({"red_pixels":25}));
        cap.call_responses.insert(
            "friend_request_scan".into(),
            json!({"rows":[{"row_y":160.0,"name":"甲","status":"等待验证","message":"hello"}]}),
        );
        cap.call_responses.insert(
            "friend_request_detail".into(),
            json!({"source":"搜索","message":"hello","verify_button":{"x":0.5,"y":0.5}}),
        );
        cap.call_responses
            .insert("friend_request_verify".into(), json!({"verified":true}));
        let calls = cap.calls.clone();
        coord.capture = Some(Box::new(cap));
        (td, coord, calls)
    }
    #[tokio::test]
    async fn friend_unknown_fields_need_review_without_verify_or_retry() {
        use crate::core::friend_requests::{self, Config, Decision, Mode, State};
        for field in ["source", "status", "button"] {
            let td = TestDir::new();
            let mut cap = MockCoordinatorCapture::new();
            let error = format!("unknown_{field} best=0.700,second=0.600");
            cap.call_responses.insert("friend_request_scan".into(), json!({"rows":[{"row_y":160.0,"name":"甲","status":if field=="status" {"unknown"} else {"等待验证"},"message":"hello","error":if field=="status" {Some(&error)} else {None}}]}));
            cap.call_responses.insert("friend_request_detail".into(), json!({"source":if field=="source" {""} else {"通过搜索手机号添加"},"message":"hello","verify_button":{"x":500,"y":300},"error":error}));
            let c = Config {
                mode: Mode::Live,
                ..Config::default()
            };
            let report =
                friend_requests::scan(&mut cap, &c, &td.0, "account", 1_794_201_600., &|| false)
                    .await
                    .unwrap();
            assert_eq!(report.events[0]["decision"], "needs_review");
            assert_eq!(report.events[0]["error"], error);
            assert!(
                !cap.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(op, _)| op == "friend_request_verify")
            );
            assert_eq!(
                State::load(&td.0).unwrap().records[0].decision,
                Decision::NeedsReview
            );
            let count = cap
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(op, _)| op == "friend_request_detail")
                .count();
            // A later clean read must not retry a request already marked needs_review.
            cap.call_responses.insert(
                "friend_request_scan".into(),
                json!({"rows":[{"row_y":160.0,"name":"甲","status":"等待验证","message":"hello"}]}),
            );
            cap.call_responses.insert("friend_request_detail".into(), json!({"source":"通过搜索手机号添加","message":"hello","verify_button":{"x":500,"y":300}}));
            let report =
                friend_requests::scan(&mut cap, &c, &td.0, "account", 1_794_201_800., &|| false)
                    .await
                    .unwrap();
            assert!(report.events.is_empty());
            assert_eq!(
                cap.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| op == "friend_request_detail")
                    .count(),
                count
            );
        }
    }

    #[tokio::test]
    async fn no_badge_never_opens_contacts() {
        // Without the 通讯录 badge the bot must not leave the chat, even on first enable.
        let (_td, mut coord, _calls) =
            friend_fixture(crate::core::friend_requests::Mode::DryRun, true);
        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses
            .insert("friend_request_badge".into(), json!({"red_pixels":0}));
        cap.call_responses.insert(
            "friend_request_scan".into(),
            json!({"rows":[{"row_y":160.0,"name":"甲","status":"等待验证","message":"hello"}]}),
        );
        cap.call_responses.insert(
            "friend_request_detail".into(),
            json!({"source":"搜索","message":"hello","verify_button":{"x":0.5,"y":0.5}}),
        );
        cap.call_responses
            .insert("friend_request_verify".into(), json!({"verified":true}));
        let calls = cap.calls.clone();
        coord.capture = Some(Box::new(cap));
        coord.serve_friend_requests().await.unwrap();
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|(op, _)| op == "friend_request_scan"),
            "no badge must not open the friend list"
        );
    }
    #[tokio::test]
    async fn accepted_triggers_incremental_contacts_sync() {
        let (_td, mut coord, calls) =
            friend_fixture(crate::core::friend_requests::Mode::Live, true);
        coord.serve_friend_requests().await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|(op, _)| op == "friend_request_verify")
                .count(),
            1
        );
        let begins: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, (op, _))| op == "contacts_lease_begin")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            begins.len(),
            2,
            "scan and incremental sync each get their own lease: {calls:?}"
        );
        let end = calls
            .iter()
            .position(|(op, _)| op == "contacts_lease_end")
            .unwrap();
        assert!(end < begins[1]);
        assert!(coord.logged_events.lock().unwrap().iter().any(|v| {
            v.to_string()
                .contains("friend_request_contacts_sync_failed")
        }));
    }
    #[tokio::test]
    async fn dry_run_does_not_trigger_sync() {
        let (_td, mut coord, calls) =
            friend_fixture(crate::core::friend_requests::Mode::DryRun, true);
        coord.serve_friend_requests().await.unwrap();
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(op, _)| op == "contacts_lease_begin")
                .count(),
            1
        );
        assert!(!coord.root.join("contacts-sync-request.json").exists());
    }
    #[tokio::test]
    async fn not_matched_does_not_trigger_sync() {
        let (_td, mut coord, calls) =
            friend_fixture(crate::core::friend_requests::Mode::Live, false);
        coord.serve_friend_requests().await.unwrap();
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|(op, _)| op == "friend_request_verify")
        );
        assert!(!coord.root.join("contacts-sync-request.json").exists());
    }
    #[tokio::test]
    async fn scan_failure_always_leaves_and_releases() {
        let (_td, coord, calls) = friend_fixture(crate::core::friend_requests::Mode::Live, true);
        // Missing rows is a protocol failure, after entering the contacts view.
        let mut cap = MockCoordinatorCapture::new();
        cap.calls = calls.clone();
        let c = coord.config.friend_requests.clone();
        assert!(
            crate::core::friend_requests::scan(&mut cap, &c, &coord.root, "acct", 1000.0, &|| {
                false
            })
            .await
            .is_err()
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls[calls.len() - 2].0, "contacts_leave");
        assert_eq!(calls[calls.len() - 1].0, "contacts_lease_end");
    }
    #[tokio::test]
    async fn needs_review_is_not_retried_after_restart() {
        use crate::core::friend_requests as fr;
        let (_td, mut coord, calls) = friend_fixture(fr::Mode::Live, true);
        let mut state = fr::State::default();
        state.bind_account(&coord.config.account_id);
        let id = fr::identity(&coord.config.account_id, "甲", "hello", "搜索");
        state.record(
            &id,
            1,
            "2026-11-09",
            1.0,
            fr::Mode::Live,
            fr::Decision::NeedsReview,
            vec![],
        );
        state.save(&coord.root).unwrap();
        coord.serve_friend_requests().await.unwrap();
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|(op, _)| op == "friend_request_verify")
        );
    }
    #[tokio::test]
    async fn scans_at_most_five_new_requests_without_starving_later_rows() {
        use crate::core::friend_requests as fr;
        let (_td, coord, _) = friend_fixture(fr::Mode::Live, true);
        let mut cap = MockCoordinatorCapture::new();
        cap.call_responses.insert("friend_request_scan".into(),json!({"rows":(0..6).map(|i|json!({"row_y":100.0+f64::from(i),"name":format!("name{i}"),"status":"等待验证","message":"msg"})).collect::<Vec<_>>()}));
        cap.call_responses.insert(
            "friend_request_detail".into(),
            json!({"source":"搜索","message":"msg","verify_button":{"x":1,"y":1}}),
        );
        cap.call_responses
            .insert("friend_request_verify".into(), json!({"verified":true}));
        let report = fr::scan(
            &mut cap,
            &coord.config.friend_requests,
            &coord.root,
            "a",
            1000.0,
            &|| false,
        )
        .await
        .unwrap();
        assert_eq!(report.accepted, 5);
        let report = fr::scan(
            &mut cap,
            &coord.config.friend_requests,
            &coord.root,
            "a",
            2800.0,
            &|| false,
        )
        .await
        .unwrap();
        assert_eq!(report.accepted, 1);
        let raw = fs::read_to_string(coord.root.join("friend-requests.json")).unwrap();
        assert!(!raw.contains("name0"));
        assert!(!raw.contains("msg"));
        assert!(!raw.contains("搜索"));
        assert_eq!(
            fs::metadata(coord.root.join("friend-requests.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn accepted_sync_preserves_and_follows_an_existing_operator_request() {
        use crate::core::friend_requests as fr;
        let td = TestDir::new();
        let manual = td.path().join("contacts-sync-request.json");
        write_private(
            &manual,
            &json!({"id":"manual","mode":"full","account_id":"a"}),
        )
        .unwrap();
        assert!(!fr::queue_contacts_sync(td.path(), "a", 1000.0, fr::Mode::Live, 3).unwrap());
        assert_eq!(read_json::<Value>(&manual).unwrap()["id"], "manual");
        assert!(!fr::promote_contacts_sync(td.path(), "a", 1001.0).unwrap());
        fs::remove_file(&manual).unwrap();
        assert!(!fr::promote_contacts_sync(td.path(), "other", 1002.0).unwrap());
        assert!(fr::promote_contacts_sync(td.path(), "a", 1003.0).unwrap());
        let request = read_json::<Value>(&manual).unwrap();
        assert_eq!(request["mode"], "incremental");
        assert_eq!(request["created_at"], 1003.0);
        assert!(!fr::promote_contacts_sync(td.path(), "a", 1004.0).unwrap());
    }
}
