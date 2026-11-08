use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::core::model::{Bubble, Frame};
use crate::core::scheduler::{CoordinatorCapture, SessionRow};
use crate::core::turns::{AttachmentFile, AttachmentImage, FailedAttachmentItem};

#[repr(C)]
struct Tm {
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
    fn getuid() -> u32;
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

pub const FRAME_DUMP_MAX_BYTES: u64 = 50 * 1024 * 1024;
pub const FRAME_DUMP_DAYS: u64 = 3;

fn format_local_date(timestamp_secs: i64) -> String {
    unsafe {
        let mut tm: Tm = std::mem::zeroed();
        localtime_r(&timestamp_secs, &mut tm);
        format!(
            "{:04}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        )
    }
}

pub fn find_frame_dump_root(directory: &Path) -> &Path {
    if let Some(parent) = directory.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("sessions") {
            if let Some(grandparent) = parent.parent() {
                return grandparent;
            }
        }
    }
    directory
}

pub fn dump_frame_impl(
    directory: &Path,
    recipient: &str,
    op: &str,
    frame_val: &Value,
    max_bytes: u64,
) -> Result<()> {
    let root = find_frame_dump_root(directory);
    if !root.join("frame-dump.enabled").exists() {
        return Ok(());
    }

    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now_secs = dur.as_secs() as i64;
    let now_f64 = dur.as_secs_f64();

    let today = format_local_date(now_secs);
    let cutoff = format_local_date(now_secs - (FRAME_DUMP_DAYS as i64) * 86400);

    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("frames-") && name.ends_with(".jsonl") && name.len() >= 15 {
                    let date_part = &name[7..15];
                    if date_part < cutoff.as_str() {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
    }

    let dump_path = directory.join(format!("frames-{}.jsonl", today));
    if dump_path.exists() {
        if let Ok(meta) = fs::metadata(&dump_path) {
            if meta.len() >= max_bytes {
                return Ok(());
            }
        }
    }

    let row = json!({
        "t": now_f64,
        "op": op,
        "recipient": recipient,
        "frame": frame_val,
    });
    let mut line = serde_json::to_string(&row)?;
    line.push('\n');

    // macOS O_NOFOLLOW = 0x0100
    const O_NOFOLLOW: i32 = 0x0100;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW)
        .open(&dump_path)?;
    file.write_all(line.as_bytes())?;

    Ok(())
}

pub const ROW_NOT_VISIBLE: &str = "session_row_not_unique_or_missing";
pub const ACTION_INTERRUPTED: &[&str] = &["user_input_detected", "target_not_reached"];
pub const MAX_EXTRACTED: usize = 24000;

pub fn action_interrupted(error: &str) -> bool {
    ACTION_INTERRUPTED
        .iter()
        .any(|code| error.contains(&format!("invalid(\"{}\")", code)))
}

pub fn layout_unavailable(error: &str) -> bool {
    error.contains("invalid(\"layout_")
}

pub fn must_wait(error: &str) -> bool {
    action_interrupted(error)
        || layout_unavailable(error)
        || error.contains("invalid(\"screen_not_main")
        || error.contains(ROW_NOT_VISIBLE)
}

pub fn worker_code(error: &str) -> String {
    if let Some(start) = error.find("invalid(\"") {
        let rest = &error[start + 9..];
        if let Some(end) = rest.find("\")") {
            let code = &rest[..end];
            return code.chars().take(120).collect();
        }
    }
    if let Some(start) = error.find("folder_access_blocked:") {
        return error[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"')
            .take(120)
            .collect();
    }
    "error".to_string()
}

pub fn media_reason(error: &str) -> String {
    // 附件落在受保护文件夹（桌面/文稿/下载）而本机未授权：要人去系统设置里允许，不是下载偶发失败
    if error.contains("folder_access_blocked:") || error.contains(":denied=") {
        "folder_permission".to_string()
    } else if error.contains("unsupported_file_type") {
        "unsupported_type".to_string()
    } else if error.contains("file_size_limit") || error.contains("media_too_large") {
        "too_large".to_string()
    } else if error.contains("file_snapshot_changed") {
        "snapshot_changed".to_string()
    } else {
        "download_failed".to_string()
    }
}

pub fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OcrStatus {
    pub engine_used: Option<String>,
    pub fallbacks: usize,
    pub last_fallback: Option<String>,
}

pub trait Navigator: Send + Sync {
    fn get_messages<'a>(
        &'a self,
        recipient: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>;
}

pub struct WorkerClient {
    pub base_dir: PathBuf,
    pub socket_path: PathBuf,
    pub owner_token: String,
    pub recipient: String,
    pub names: Vec<String>,
    pub directory: PathBuf,
    pub manage_window: bool,
    pub group: bool,
    pub max_bytes: usize,
    pub ocr_status: Option<OcrStatus>,
    pub counter: u64,
    pub last_switch: Option<Value>,
    pub active: bool,
    pub navigator: Option<Arc<dyn Navigator>>,
    pub reader: Option<BufReader<tokio::net::unix::OwnedReadHalf>>,
    pub writer: Option<tokio::net::unix::OwnedWriteHalf>,
    pub rpc_lock: Mutex<()>,
    pub logged_events: Arc<StdMutex<Vec<Value>>>,
    /// 协调器的停止标志：等待 worker 的长时间循环要随时响应停止（control stop / 宿主关闭）
    pub stop: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl WorkerClient {
    pub fn new(
        base_dir: PathBuf,
        socket_path: PathBuf,
        owner_token: String,
        recipient: String,
        directory: PathBuf,
        names: Option<Vec<String>>,
        manage_window: bool,
        group: bool,
        max_bytes: usize,
        navigator: Option<Arc<dyn Navigator>>,
    ) -> Self {
        let resolved_names = names.unwrap_or_else(|| vec![recipient.clone()]);
        Self {
            base_dir,
            socket_path,
            owner_token,
            recipient,
            names: resolved_names,
            directory,
            manage_window,
            group,
            max_bytes,
            ocr_status: None,
            counter: 0,
            last_switch: None,
            active: false,
            navigator,
            reader: None,
            writer: None,
            rpc_lock: Mutex::new(()),
            logged_events: Arc::new(StdMutex::new(Vec::new())),
            stop: None,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.owner_token.is_empty() {
            bail!("persistent worker requires stable ownership token");
        }
        if self.recipient.trim().is_empty() {
            bail!("worker start requires a target chat (prepare_target before start)");
        }

        if !self.base_dir.exists() {
            bail!("unsafe_private_path: base directory does not exist");
        }
        let meta = fs::metadata(&self.base_dir)?;
        let mode = meta.permissions().mode() & 0o777;
        let uid = meta.uid();
        unsafe {
            let current_uid = getuid();
            if uid != current_uid {
                bail!("unsafe_private_path: owner mismatch");
            }
        }
        if mode != 0o700 {
            bail!("unsafe_private_path: mode {:o} != 0700", mode);
        }

        let worker_bin = self.base_dir.join("bin").join("wechat_bot_worker");
        if !worker_bin.exists() {
            bail!("worker_missing: bin/wechat_bot_worker not found");
        }
        let perms = fs::metadata(&worker_bin)?.permissions();
        if (perms.mode() & 0o111) == 0 {
            bail!("worker_missing: bin/wechat_bot_worker is not executable");
        }

        let flag_path = self.base_dir.join("bot-worker.enabled");
        if flag_path.exists() {
            let existing = fs::read_to_string(&flag_path)?;
            if existing != self.owner_token {
                bail!("ownership token mismatch");
            }
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&flag_path)?;
            file.write_all(self.owner_token.as_bytes())?;
        }
        self.active = true;

        match self.connect(3).await {
            Ok(()) => {
                let rec = self.recipient.clone();
                let names = self.names.clone();
                let res = self.open_session(&rec, &names).await;
                match res {
                    Ok(val) => {
                        self.last_switch = Some(val);
                        return Ok(());
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        if must_wait(&err_str) {
                            return Err(err);
                        }
                        self.close_transport().await;
                    }
                }
            }
            Err(err) => {
                let err_str = err.to_string();
                if must_wait(&err_str) {
                    return Err(err);
                }
                self.close_transport().await;
            }
        }

        if let Some(nav) = &self.navigator {
            // Python logs this failure (quiet_relaunch_failed); swallowing it hid why the worker never started.
            let relaunch = tokio::select! {
                res = nav.get_messages(None) => Some(res),
                _ = Self::until_stop_requested(self.stop.clone()) => None,
            };
            match relaunch {
                None => bail!("stop_requested: quiet relaunch abandoned"),
                Some(Err(error)) => tracing::warn!("quiet_relaunch_get_messages_failed: {:#}", error),
                Some(Ok(_)) => {}
            }
            match self.connect(60).await {
                Ok(()) => {
                    let rec = self.recipient.clone();
                    let names = self.names.clone();
                    let res = self.open_session(&rec, &names).await;
                    match res {
                        Ok(val) => {
                            self.last_switch = Some(val);
                            return Ok(());
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            if must_wait(&err_str) {
                                return Err(err);
                            }
                            tracing::warn!("quiet_relaunch_failed: {}", err_str);
                            self.close_transport().await;
                        }
                    }
                }
                Err(err) => {
                    let err_str = err.to_string();
                    if must_wait(&err_str) {
                        return Err(err);
                    }
                    tracing::warn!("quiet_relaunch_failed: {}", err_str);
                    self.close_transport().await;
                }
            }
        }

        let nav_names = self.names.clone();
        self.navigate(&nav_names).await?;
        self.connect(60).await?;
        Ok(())
    }

    fn stop_requested(&self) -> bool {
        self.stop
            .as_ref()
            .map_or(false, |s| s.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// 停止请求到来时结束（每 200ms 检查一次）；没有停止标志则永不结束。
    async fn until_stop_requested(stop: Option<Arc<std::sync::atomic::AtomicBool>>) {
        match stop {
            Some(flag) => {
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
            None => std::future::pending::<()>().await,
        }
    }

    pub async fn connect(&mut self, timeout_seconds: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
            if self.stop_requested() {
                self.close_transport().await;
                bail!("stop_requested: worker connect abandoned");
            }
            match UnixStream::connect(&self.socket_path).await {
                Ok(stream) => {
                    let (read_half, write_half) = stream.into_split();
                    self.reader = Some(BufReader::new(read_half));
                    self.writer = Some(write_half);
                    let probe_names = if self.names.is_empty() {
                        vec![self.recipient.clone()]
                    } else {
                        self.names.clone()
                    };
                    match self.call("sessions", json!({"names": probe_names})).await {
                        Ok(_) => return Ok(()),
                        Err(_) => {
                            self.close_transport().await;
                        }
                    }
                }
                Err(_) => {
                    self.close_transport().await;
                }
            }
            if Instant::now() >= deadline {
                bail!("private GUI bot worker did not become ready");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub async fn call(&mut self, operation: &str, mut params: Value) -> Result<Value> {
        let _guard = self.rpc_lock.lock().await;
        self.counter += 1;

        if let Value::Object(ref mut map) = params {
            map.entry("recipient")
                .or_insert_with(|| json!(self.recipient));
            map.entry("accepted_names").or_insert_with(|| {
                json!(if self.names.is_empty() {
                    vec![self.recipient.clone()]
                } else {
                    self.names.clone()
                })
            });
            map.entry("manage_window")
                .or_insert_with(|| json!(self.manage_window));
            map.entry("group_title")
                .or_insert_with(|| json!(self.group));
        }

        let request = json!({
            "protocol_version": 1,
            "id": self.counter,
            "operation": operation,
            "params": params,
        });

        let writer = self.writer.as_mut().context("transport closed")?;
        let reader = self.reader.as_mut().context("transport closed")?;

        let mut data = serde_json::to_vec(&request)?;
        data.push(b'\n');

        if let Err(e) = writer.write_all(&data).await {
            self.writer = None;
            self.reader = None;
            return Err(e.into());
        }

        let mut line = String::new();
        let read_result =
            tokio::time::timeout(Duration::from_secs(35), reader.read_line(&mut line)).await;
        let n = match read_result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                self.writer = None;
                self.reader = None;
                return Err(e.into());
            }
            Err(_) => {
                self.writer = None;
                self.reader = None;
                bail!("private bot worker timeout after 35s");
            }
        };

        if n == 0 {
            self.writer = None;
            self.reader = None;
            bail!("private bot worker disconnected");
        }

        let response: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                self.writer = None;
                self.reader = None;
                return Err(e.into());
            }
        };

        if response.get("id").and_then(|v| v.as_u64()) != Some(self.counter)
            || response.get("protocol_version").and_then(|v| v.as_u64()) != Some(1)
        {
            self.writer = None;
            self.reader = None;
            bail!("private bot worker protocol mismatch");
        }

        drop(_guard);

        if let Some(ocr) = response.get("ocr") {
            self.record_ocr(ocr);
        }

        if response.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err_msg = response
                .get("error")
                .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            bail!("private visual operation failed: {}", err_msg);
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn record_ocr(&mut self, report: &Value) {
        if let Some(engine_used) = report.get("engine_used").and_then(|v| v.as_str()) {
            let mut status = self.ocr_status.take().unwrap_or_default();
            status.engine_used = Some(engine_used.to_string());
            if let Some(fb) = report.get("fallback").and_then(|v| v.as_str()) {
                status.fallbacks += 1;
                status.last_fallback = Some(fb.chars().take(200).collect());
            }
            self.ocr_status = Some(status);
        }
    }

    pub fn dump_frame(&self, op: &str, frame_val: &Value) {
        if let Err(err) = dump_frame_impl(
            &self.directory,
            &self.recipient,
            op,
            frame_val,
            FRAME_DUMP_MAX_BYTES,
        ) {
            tracing::warn!("frame_dump_failed: {:#}", err);
        }
    }

    pub async fn frame(&mut self) -> Result<crate::core::model::Frame> {
        let res = self.call("capture", json!({})).await?;
        if res.get("window_resized").and_then(|v| v.as_bool()) == Some(true) {
            tracing::info!(
                "window_resized: profile={:?}, scale={:?}",
                res.get("profile"),
                res.get("scale")
            );
        }
        self.dump_frame("capture", &res);
        serde_json::from_value(res).context("failed to deserialize Frame from operation 'capture'")
    }

    pub async fn scroll(&mut self, direction: &str, pages: u32) -> Result<crate::core::model::Frame> {
        if direction != "up" && direction != "down" {
            bail!("invalid scroll request: direction must be up or down");
        }
        if !(1..=40).contains(&pages) {
            bail!("invalid scroll request: pages must be in 1..=40");
        }
        let res = self
            .call(
                "scroll_chat",
                json!({"direction": direction, "pages": pages}),
            )
            .await?;
        self.dump_frame("scroll_chat", &res);
        serde_json::from_value(res).context("failed to deserialize Frame from operation 'scroll_chat'")
    }

    pub async fn sessions(&mut self, names: &[String], debug: bool) -> Result<Value> {
        self.call("sessions", json!({"names": names, "debug": debug}))
            .await
    }

    pub async fn calibrate(&mut self) -> Result<Value> {
        self.call("calibrate", json!({})).await
    }

    pub async fn open_session(&mut self, target: &str, target_names: &[String]) -> Result<Value> {
        self.call(
            "open_session",
            json!({"target": target, "target_names": target_names}),
        )
        .await
    }

    pub async fn retarget(
        &mut self,
        recipient: &str,
        directory: impl AsRef<Path>,
        names: Option<Vec<String>>,
        group: bool,
    ) -> Result<()> {
        let target_names = names.unwrap_or_else(|| vec![recipient.to_string()]);
        self.names = target_names.clone();
        self.group = group;

        let switch_res = self.open_session(recipient, &target_names).await;
        match switch_res {
            Ok(val) => {
                self.last_switch = Some(val);
            }
            Err(err) => {
                let err_str = err.to_string();
                if must_wait(&err_str) {
                    return Err(err);
                }
                self.last_switch = Some(json!({
                    "opened": false,
                    "fallback": err_str.chars().take(200).collect::<String>()
                }));
                self.navigate(&self.names).await?;
            }
        }
        self.recipient = recipient.to_string();
        self.directory = directory.as_ref().to_path_buf();
        self.frame().await?;
        Ok(())
    }

    pub async fn download(&mut self, media_id: &str, _kind: &str) -> Result<PathBuf> {
        if media_id.len() != 64 || !media_id.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid media ID");
        }

        let mut data = Vec::new();
        let mut final_ext = "png".to_string();

        loop {
            let res = self
                .call(
                    "read_media",
                    json!({
                        "media_id": media_id,
                        "offset": data.len(),
                        "max_bytes": self.max_bytes
                    }),
                )
                .await?;

            let offset = res
                .get("offset")
                .and_then(|v| v.as_u64())
                .context("missing offset")? as usize;
            let size = res
                .get("size")
                .and_then(|v| v.as_u64())
                .context("missing size")? as usize;

            if offset != data.len() || size > self.max_bytes {
                bail!("invalid media size/offset");
            }

            let b64_str = res
                .get("bytes")
                .and_then(|v| v.as_str())
                .context("missing bytes")?;
            let chunk = if b64_str.is_empty() {
                Vec::new()
            } else {
                crate::wechat::files::decode_base64(b64_str)?
            };
            let eof = res.get("eof").and_then(|v| v.as_bool()).unwrap_or(false);

            if chunk.is_empty() && !eof {
                bail!("empty nonterminal media chunk");
            }

            data.extend_from_slice(&chunk);
            if data.len() > size {
                bail!("oversized media response");
            }

            if eof {
                if data.len() != size {
                    bail!("media digest mismatch: size mismatch");
                }
                let sha256_expected = res
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .context("missing sha256")?;
                if sha256(&data) != sha256_expected {
                    bail!("media digest mismatch");
                }
                if let Some(ext) = res.get("extension").and_then(|v| v.as_str()) {
                    final_ext = ext.to_string();
                }
                break;
            }
        }

        let allowed_exts = [
            "png", "jpg", "jpeg", "pdf", "txt", "md", "csv", "json", "docx", "xlsx",
        ];
        if !allowed_exts.contains(&final_ext.as_str()) {
            bail!("unexpected media extension");
        }

        let filename = format!("{}.{}", media_id, final_ext);
        let dest = self.directory.join(filename);

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&dest)?;
        file.write_all(&data)?;

        Ok(dest)
    }

    pub async fn prepare_file(
        &mut self,
        media_id: &str,
        snapshot_id: &str,
        max_bytes: Option<usize>,
    ) -> Result<Value> {
        self.call(
            "prepare_file",
            json!({
                "media_id": media_id,
                "snapshot_id": snapshot_id,
                "max_bytes": max_bytes.unwrap_or(self.max_bytes),
            }),
        )
        .await
    }

    pub async fn prepare_image(
        &mut self,
        media_id: &str,
        snapshot_id: &str,
        max_bytes: Option<usize>,
    ) -> Result<Value> {
        self.call(
            "prepare_image",
            json!({
                "media_id": media_id,
                "snapshot_id": snapshot_id,
                "max_bytes": max_bytes.unwrap_or(self.max_bytes),
            }),
        )
        .await
    }

    pub async fn read_media(
        &mut self,
        media_id: &str,
        offset: usize,
        max_bytes: Option<usize>,
    ) -> Result<Value> {
        self.call(
            "read_media",
            json!({
                "media_id": media_id,
                "offset": offset,
                "max_bytes": max_bytes.unwrap_or(self.max_bytes),
            }),
        )
        .await
    }

    pub async fn navigate(&self, names: &[String]) -> Result<Value> {
        let nav = self.navigator.as_ref().context("no navigator configured")?;
        let mut last_err = None;
        let candidates = if names.is_empty() {
            vec![self.recipient.clone()]
        } else {
            names.to_vec()
        };
        for name in candidates {
            match nav.get_messages(Some(&name)).await {
                Ok(val) => return Ok(val),
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("navigation failed")))
    }

    pub async fn close_transport(&mut self) {
        self.writer = None;
        self.reader = None;
    }

    pub async fn close(&mut self) -> Result<()> {
        self.close_transport().await;
        if self.active {
            let flag = self.base_dir.join("bot-worker.enabled");
            if flag.exists() {
                let token = fs::read_to_string(&flag)?;
                if token == self.owner_token {
                    let _ = fs::remove_file(&flag);
                }
            }
            self.active = false;
        }
        Ok(())
    }

    fn folder_permission_marker(&self) -> PathBuf {
        self.base_dir.join("folder-permission-needed.json")
    }

    /// 附件因文件夹权限失败：记警告并写标记文件，宿主状态面板据此提示去「文件和文件夹」里允许。
    fn note_folder_permission(&self, err_str: &str) {
        if media_reason(err_str) != "folder_permission" {
            return;
        }
        let detail = worker_code(err_str);
        tracing::warn!(detail = %detail, "attachment_folder_permission: 附件保存所在文件夹未授权，请在系统设置 > 隐私与安全性 > 文件和文件夹中允许");
        self.log_event("attachment_folder_permission", json!({"detail": detail}));
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = crate::core::state::write_private(&self.folder_permission_marker(), &json!({"detail": detail, "at": at}));
    }

    /// 附件原件保存成功即说明权限已恢复，清掉提示。
    fn clear_folder_permission(&self) {
        let _ = fs::remove_file(self.folder_permission_marker());
    }

    pub fn log_event(&self, event: &str, fields: Value) {
        let mut item = match fields {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        item.insert("event".to_string(), json!(event));
        tracing::info!(event = %event, fields = %serde_json::to_string(&item).unwrap_or_default());
        const MAX_LOGGED_EVENTS: usize = 256;
        let mut events = self.logged_events.lock().unwrap();
        if events.len() >= MAX_LOGGED_EVENTS {
            events.remove(0);
        }
        events.push(Value::Object(item));
    }

    pub async fn attachments(
        &mut self,
        items: &[Bubble],
        frame: &Frame,
    ) -> Result<(
        Vec<AttachmentImage>,
        Vec<AttachmentFile>,
        Vec<FailedAttachmentItem>,
    )> {
        let mut images = Vec::new();
        let mut files = Vec::new();
        let mut failed = Vec::new();
        let mut used = 0usize;
        let mut number = 0usize;
        let mut items = items.to_vec();
        let mut current_frame = frame.clone();

        for index in 0..items.len() {
            let item = items[index].clone();
            let kind = item.kind.as_deref().unwrap_or("text");
            if kind == "image" {
                number += 1;
                let label = format!("第{}张图片", number);
                let mut source = "original".to_string();
                let media_id = item.media_id.clone().unwrap_or_default();

                let prep_res = self
                    .call(
                        "prepare_image",
                        json!({
                            "media_id": media_id,
                            "snapshot_id": current_frame.snapshot_id,
                            "max_bytes": self.max_bytes,
                        }),
                    )
                    .await;

                match prep_res {
                    Ok(_) => {
                        self.clear_folder_permission();
                        let f = self.frame().await?;
                        current_frame = f;
                        if index + 1 < items.len() {
                            let relocated = crate::core::policy::relocate(&items[index + 1..], &current_frame.bubbles);
                            items.splice(index + 1.., relocated);
                        }
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        if action_interrupted(&err_str) {
                            return Err(err);
                        }
                        source = "chat_thumbnail".to_string();
                        self.note_folder_permission(&err_str);
                        self.log_event(
                            "image_original_unavailable",
                            json!({
                                "reason": media_reason(&err_str),
                                "detail": worker_code(&err_str),
                            }),
                        );
                    }
                }

                match self.download(&media_id, "image").await {
                    Ok(path) => {
                        images.push(AttachmentImage {
                            path,
                            label,
                            source,
                            bubble: item.clone(),
                            media_id: Some(media_id),
                        });
                    }
                    Err(_) => {
                        failed.push(FailedAttachmentItem {
                            label,
                            kind: "image".to_string(),
                            reason: "download_failed".to_string(),
                            detail: None,
                        });
                        continue;
                    }
                }
            } else if kind == "file" {
                let mut label = item.filename.clone().unwrap_or_else(|| "文件".to_string());
                let media_id = item.media_id.clone().unwrap_or_default();

                let prep_step = async {
                    let info = self
                        .call(
                            "prepare_file",
                            json!({
                                "media_id": media_id,
                                "snapshot_id": current_frame.snapshot_id,
                                "max_bytes": self.max_bytes,
                            }),
                        )
                        .await?;
                    let path = self.download(&media_id, "file").await?;
                    let f = self.frame().await?;
                    Ok::<_, anyhow::Error>((info, path, f))
                }
                .await;

                let (info, path, new_frame) = match prep_step {
                    Ok(res) => {
                        self.clear_folder_permission();
                        res
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        if action_interrupted(&err_str) {
                            return Err(err);
                        }
                        self.note_folder_permission(&err_str);
                        failed.push(FailedAttachmentItem {
                            label,
                            kind: "file".to_string(),
                            reason: media_reason(&err_str),
                            detail: Some(worker_code(&err_str)),
                        });
                        continue;
                    }
                };

                current_frame = new_frame;
                if index + 1 < items.len() {
                    let relocated = crate::core::policy::relocate(&items[index + 1..], &current_frame.bubbles);
                    items.splice(index + 1.., relocated);
                }

                if let Some(fn_str) = info.get("filename").and_then(|v| v.as_str()) {
                    label = fn_str.to_string();
                }

                let ext = info.get("extension").and_then(|v| v.as_str()).unwrap_or("");
                if ext == "png" || ext == "jpg" || ext == "jpeg" {
                    images.push(AttachmentImage {
                        path,
                        label,
                        source: "original".to_string(),
                        bubble: item.clone(),
                        media_id: Some(media_id),
                    });
                    continue;
                }

                let extraction_val = info.get("extraction");
                let content = match crate::brain::documents::extract(
                    &path,
                    ext,
                    extraction_val,
                    Some(self.max_bytes as u64),
                ) {
                    Ok(val) => val,
                    Err(_) => {
                        failed.push(FailedAttachmentItem {
                            label,
                            kind: "file".to_string(),
                            reason: "extract_failed".to_string(),
                            detail: None,
                        });
                        continue;
                    }
                };

                let previews = extraction_val
                    .and_then(|e| e.get("previews"))
                    .and_then(|p| p.as_array());

                let has_text = content
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_rows = content
                    .get("rows")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                let has_sheets = content
                    .get("sheets")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                let has_previews = previews.map(|p| !p.is_empty()).unwrap_or(false);

                if !has_text && !has_rows && !has_sheets && !has_previews {
                    failed.push(FailedAttachmentItem {
                        label,
                        kind: "file".to_string(),
                        reason: "empty".to_string(),
                        detail: None,
                    });
                    continue;
                }

                let size = serde_json::to_string(&content)?.chars().count();
                if used + size > MAX_EXTRACTED {
                    failed.push(FailedAttachmentItem {
                        label,
                        kind: "file".to_string(),
                        reason: "too_large".to_string(),
                        detail: None,
                    });
                    continue;
                }
                used += size;

                if let Some(prevs) = previews {
                    for (page_idx, prev) in prevs.iter().enumerate() {
                        if let Some(prev_id) = prev.as_str() {
                            let prev_path = self.download(prev_id, "image").await?;
                            images.push(AttachmentImage {
                                path: prev_path,
                                label: format!("{}第{}页", label, page_idx + 1),
                                source: "preview".to_string(),
                                bubble: item.clone(),
                                media_id: Some(prev_id.to_string()),
                            });
                        }
                    }
                }

                let content_map = match content {
                    Value::Object(m) => m,
                    _ => serde_json::Map::new(),
                };

                files.push(AttachmentFile {
                    filename: info
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&label)
                        .to_string(),
                    label,
                    original_bytes: true,
                    path,
                    bubble: item.clone(),
                    content: content_map,
                });
            }
        }

        Ok((images, files, failed))
    }
}

impl CoordinatorCapture for WorkerClient {
    fn start<'a>(&'a mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.start().await })
    }

    fn close<'a>(&'a mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.close().await })
    }

    fn frame<'a>(&'a mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<crate::core::model::Frame>> + Send + 'a>> {
        Box::pin(async move { self.frame().await })
    }

    fn scroll<'a>(&'a mut self, direction: &'a str, amount: u32) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<crate::core::model::Frame>> + Send + 'a>> {
        Box::pin(async move { self.scroll(direction, amount).await })
    }

    fn call<'a>(&'a mut self, op: &'a str, params: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move { self.call(op, params).await })
    }

    fn prepare_target(&mut self, recipient: &str, directory: &Path, names: &[String], group: bool) {
        self.recipient = recipient.to_string();
        self.names = names.to_vec();
        self.group = group;
        self.directory = directory.to_path_buf();
    }

    fn retarget<'a>(&'a mut self, recipient: &'a str, directory: &'a Path, names: &'a [String], group: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        let names_vec = names.to_vec();
        Box::pin(async move {
            self.recipient = recipient.to_string();
            self.names = names_vec.clone();
            self.group = group;
            self.directory = directory.to_path_buf();
            if !self.is_connected() {
                self.start().await?;
                return Ok(());
            }
            self.retarget(recipient, directory, Some(names_vec), group).await
        })
    }

    fn sessions<'a>(&'a mut self, names: &'a [String]) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SessionRow>>> + Send + 'a>> {
        Box::pin(async move {
            let res = self.sessions(names, false).await?;
            let rows_val = res.get("rows").cloned().unwrap_or(res);
            serde_json::from_value(rows_val).context("failed to deserialize SessionRow array from sessions")
        })
    }

    fn download<'a>(&'a mut self, media_id: &'a str, kind: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send + 'a>> {
        Box::pin(async move { self.download(media_id, kind).await })
    }

    fn attachments<'a>(
        &'a mut self,
        items: &'a [Bubble],
        frame: &'a Frame,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<(
                        Vec<AttachmentImage>,
                        Vec<AttachmentFile>,
                        Vec<FailedAttachmentItem>,
                    )>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.attachments(items, frame).await })
    }

    fn last_switch(&self) -> Option<Value> {
        self.last_switch.clone()
    }

    fn ocr_status(&self) -> Option<OcrStatus> {
        self.ocr_status.clone()
    }

    fn is_connected(&self) -> bool {
        self.writer.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;
    use tokio::sync::mpsc;

    const B64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn test_b64_encode(data: &[u8]) -> String {
        let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

            result.push(B64_CHARS[(b0 >> 2) as usize] as char);
            result.push(B64_CHARS[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                result.push(B64_CHARS[(((b1 & 0xf) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(B64_CHARS[(b2 & 0x3f) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let path = PathBuf::from(format!("/tmp/wc_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
            fs::create_dir_all(&path).unwrap();
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o700);
            fs::set_permissions(&path, perms).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct MockWorkerServer {
        socket_path: PathBuf,
        handle: tokio::task::JoinHandle<()>,
        _tx_req: mpsc::Sender<Value>,
        _rx_resp: Arc<Mutex<mpsc::Receiver<Value>>>,
    }

    impl MockWorkerServer {
        async fn spawn(
            socket_path: PathBuf,
            handler: impl Fn(Value) -> Value + Send + Sync + 'static,
        ) -> Self {
            let _ = fs::remove_file(&socket_path);
            let listener = UnixListener::bind(&socket_path).unwrap();
            let (tx_req, _rx_req_inner) = mpsc::channel::<Value>(100);
            let (_tx_resp, rx_resp) = mpsc::channel::<Value>(100);
            let handler = Arc::new(handler);
            let tx_req_clone = tx_req.clone();

            let handle = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let tx = tx_req_clone.clone();
                    let h = handler.clone();
                    tokio::spawn(async move {
                        let (read_half, mut write_half) = stream.into_split();
                        let mut reader = BufReader::new(read_half);
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break;
                            }
                            if let Ok(req) = serde_json::from_str::<Value>(&line) {
                                let _ = tx.send(req.clone()).await;
                                let resp = h(req);
                                let mut data = serde_json::to_vec(&resp).unwrap();
                                data.push(b'\n');
                                if write_half.write_all(&data).await.is_err() {
                                    break;
                                }
                            }
                            line.clear();
                        }
                    });
                }
            });

            Self {
                socket_path,
                handle,
                _tx_req: tx_req,
                _rx_resp: Arc::new(Mutex::new(rx_resp)),
            }
        }
    }

    impl Drop for MockWorkerServer {
        fn drop(&mut self) {
            self.handle.abort();
            let _ = fs::remove_file(&self.socket_path);
        }
    }

    struct MockNavigator {
        calls: Arc<Mutex<Vec<Option<String>>>>,
        fail_times: AtomicUsize,
    }

    impl MockNavigator {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_times: AtomicUsize::new(0),
            }
        }

        fn with_failures(fail_times: usize) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_times: AtomicUsize::new(fail_times),
            }
        }
    }

    impl Navigator for MockNavigator {
        fn get_messages<'a>(
            &'a self,
            recipient: Option<&'a str>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>
        {
            let rec = recipient.map(|s| s.to_string());
            Box::pin(async move {
                self.calls.lock().await.push(rec);
                let remaining = self.fail_times.load(Ordering::SeqCst);
                if remaining > 0 {
                    self.fail_times.fetch_sub(1, Ordering::SeqCst);
                    bail!("title mismatch");
                }
                Ok(json!({"ok": true}))
            })
        }
    }

    fn setup_test_client(
        temp: &TestDir,
        socket_path: PathBuf,
        navigator: Option<Arc<dyn Navigator>>,
    ) -> WorkerClient {
        let base_dir = temp.path().join(".wechat-rpa");
        fs::create_dir_all(&base_dir).unwrap();
        let mut base_perms = fs::metadata(&base_dir).unwrap().permissions();
        base_perms.set_mode(0o700);
        fs::set_permissions(&base_dir, base_perms).unwrap();

        let bin_dir = base_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let worker_bin = bin_dir.join("wechat_bot_worker");
        fs::write(&worker_bin, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = fs::metadata(&worker_bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&worker_bin, perms).unwrap();

        let root = temp.path().join("private");
        fs::create_dir_all(&root).unwrap();

        WorkerClient::new(
            base_dir,
            socket_path,
            "0".repeat(64),
            "a".to_string(),
            root,
            Some(vec!["a".to_string()]),
            true,
            false,
            50 * 1024 * 1024,
            navigator,
        )
    }

    // --- test_hidden_rows.py ---
    #[tokio::test]
    async fn test_missing_row_waits_without_search() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "invalid(\"session_row_not_unique_or_missing\")"
                })
            } else {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {}
                })
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::new());
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        client.active = true;
        client.connect(5).await.unwrap();

        let err = client
            .retarget(
                "b",
                temp.path().join("b"),
                Some(vec!["b".to_string()]),
                false,
            )
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("session_row_not_unique_or_missing"));
        assert_eq!(nav.calls.lock().await.len(), 0);
    }

    // --- test_calibrate.py ---
    #[tokio::test]
    async fn test_worker_probe_names_a_recipient() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "calibrate" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "wechat_version": "4.1.15",
                        "layout_path": "/fake/layout.json",
                        "layout": {}
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "test_user".to_string();
        client.names = vec!["test_user".to_string()];
        client.connect(5).await.unwrap();

        let res = client.calibrate().await.unwrap();
        assert_eq!(res["wechat_version"], "4.1.15");
    }

    // --- test_bot_documents.py ---
    #[tokio::test]
    async fn test_chunked_bytes_digest_and_extension() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let test_data = b"actual original file bytes".repeat(1000);
        let data_clone = test_data.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "read_media" {
                let offset = req["params"]["offset"].as_u64().unwrap() as usize;
                let chunk_size = 65536.min(data_clone.len() - offset);
                let chunk = &data_clone[offset..offset + chunk_size];
                let eof = offset + chunk_size == data_clone.len();
                let hex_digest = sha256(&data_clone);
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "offset": offset,
                        "size": data_clone.len(),
                        "bytes": test_b64_encode(chunk),
                        "eof": eof,
                        "sha256": hex_digest,
                        "extension": "txt"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let media_id = "b".repeat(64);
        let path = client.download(&media_id, "file").await.unwrap();
        let saved_bytes = fs::read(&path).unwrap();
        assert_eq!(saved_bytes, test_data);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn test_corrupt_transfer_never_produces_attachment() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "read_media" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "offset": 0,
                        "size": 3,
                        "bytes": "YmFk",
                        "eof": true,
                        "sha256": "0".repeat(64),
                        "extension": "txt"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let media_id = "c".repeat(64);
        let err = client.download(&media_id, "file").await.unwrap_err();
        assert!(err.to_string().contains("digest mismatch"));
    }

    #[tokio::test]
    async fn test_media_path_cannot_escape_private_directory() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let mut client = setup_test_client(&temp, sock, None);

        let err = client.download("../escape", "file").await.unwrap_err();
        assert!(err.to_string().contains("invalid media ID"));
    }

    // --- test_attachment_limit.py ---
    #[tokio::test]
    async fn test_download_accepts_30mb_and_rejects_above_limit() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let size_30mb = 30 * 1024 * 1024;
        let chunk_size = 65536;

        let server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "read_media" {
                let offset = req["params"]["offset"].as_u64().unwrap() as usize;
                let chunk_len = chunk_size.min(size_30mb - offset);
                let dummy_chunk = vec![b'x'; chunk_len];
                let eof = offset + chunk_len == size_30mb;
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "offset": offset,
                        "size": size_30mb,
                        "bytes": test_b64_encode(&dummy_chunk),
                        "eof": eof,
                        "sha256": "726ec6766f96dcdc1001a5dbf49603b18d59a023c7a62785dd3684e551ceabf3",
                        "extension": "txt"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock.clone(), None);
        client.connect(5).await.unwrap();
        let path = client.download(&"a".repeat(64), "file").await.unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), size_30mb as u64);

        drop(server);
        let _oversized_server = MockWorkerServer::spawn(sock, move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "read_media" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "offset": 0,
                        "size": 50 * 1024 * 1024 + 1,
                        "bytes": test_b64_encode(b"x"),
                        "eof": true,
                        "sha256": "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881",
                        "extension": "txt"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client2 = setup_test_client(&temp, client.socket_path.clone(), None);
        client2.connect(5).await.unwrap();
        let err = client2.download(&"b".repeat(64), "file").await.unwrap_err();
        assert!(err.to_string().contains("invalid media size/offset"));
    }

    #[tokio::test]
    async fn test_download_rejects_corrupted_digest() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let payload = b"correct payload";

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "read_media" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "offset": 0,
                        "size": payload.len(),
                        "bytes": test_b64_encode(payload),
                        "eof": true,
                        "sha256": "bad0000000000000000000000000000000000000000000000000000000000000",
                        "extension": "txt"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        let err = client.download(&"c".repeat(64), "file").await.unwrap_err();
        assert!(err.to_string().contains("media digest mismatch"));
    }

    #[tokio::test]
    async fn test_prepare_and_read_calls_carry_configured_max_bytes() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let last_max_bytes = Arc::new(AtomicUsize::new(0));
        let lmb = last_max_bytes.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let mb = req["params"]["max_bytes"].as_u64().unwrap_or(0) as usize;
            lmb.store(mb, Ordering::SeqCst);
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.max_bytes = 42 * 1024 * 1024;
        client.connect(5).await.unwrap();

        client.prepare_file("a", "s0", None).await.unwrap();
        assert_eq!(last_max_bytes.load(Ordering::SeqCst), 42 * 1024 * 1024);

        client.prepare_image("a", "s0", None).await.unwrap();
        assert_eq!(last_max_bytes.load(Ordering::SeqCst), 42 * 1024 * 1024);

        client.read_media(&"a".repeat(64), 0, None).await.unwrap();
        assert_eq!(last_max_bytes.load(Ordering::SeqCst), 42 * 1024 * 1024);
    }

    // --- test_bot_routing.py ---
    #[tokio::test]
    async fn test_worker_calls_carry_manage_window_switch() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let recorded_manage = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rm = recorded_manage.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let m = req["params"]["manage_window"].as_bool().unwrap();
            let mut list = rm.lock().unwrap();
            list.push(m);
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
        })
        .await;

        let mut client_true = setup_test_client(&temp, sock.clone(), None);
        client_true.manage_window = true;
        client_true.connect(5).await.unwrap();
        client_true
            .sessions(&["a".to_string()], false)
            .await
            .unwrap();
        client_true.close().await.unwrap();

        let mut client_false = setup_test_client(&temp, sock, None);
        client_false.manage_window = false;
        client_false.connect(5).await.unwrap();
        client_false
            .sessions(&["a".to_string()], false)
            .await
            .unwrap();
        client_false.close().await.unwrap();

        let list = recorded_manage.lock().unwrap();
        assert_eq!(*list, vec![true, true, false, false]);
    }

    #[tokio::test]
    async fn test_worker_receives_alias_candidates() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let last_names = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ln = last_names.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            if let Some(names) = req["params"]["accepted_names"].as_array() {
                let arr: Vec<String> = names
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                *ln.lock().unwrap() = arr;
            }
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "🍓a".to_string();
        client.names = vec!["🍓a".to_string(), "a".to_string()];
        client.connect(5).await.unwrap();
        client.frame().await.unwrap();

        let recorded = last_names.lock().unwrap();
        assert_eq!(*recorded, vec!["🍓a".to_string(), "a".to_string()]);
    }

    #[tokio::test]
    async fn test_group_capture_accepts_member_count_title() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let last_group = Arc::new(std::sync::Mutex::new(false));
        let lg = last_group.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            *lg.lock().unwrap() = req["params"]["group_title"].as_bool().unwrap();
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.group = true;
        client.connect(5).await.unwrap();
        client.frame().await.unwrap();

        let g = *last_group.lock().unwrap();
        assert!(g);
    }

    #[tokio::test]
    async fn test_scroll_call_carries_manage_window() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {"bubbles": []}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let res = client.scroll("up", 2).await;
        assert!(res.is_ok());

        let bad_dir = client.scroll("left", 1).await;
        assert!(bad_dir.is_err());
        let bad_page = client.scroll("up", 41).await;
        assert!(bad_page.is_err());
    }

    #[tokio::test]
    async fn test_worker_refusal_keeps_transport_open() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "refused"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let err = client
            .open_session("b", &["b".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refused"));
        assert!(client.writer.is_some());
    }

    #[tokio::test]
    async fn test_worker_calls_record_ocr_engine_and_fallbacks() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let c = cc.fetch_add(1, Ordering::SeqCst);
            let ocr = match c {
                0 => None,
                1 => Some(
                    json!({"engine": "rapidocr", "engine_used": "vision", "fallback": "rapidocr_timeout"}),
                ),
                2 => Some(json!({"engine": "rapidocr", "engine_used": "rapidocr"})),
                _ => None,
            };
            let mut resp = json!({"id": id, "protocol_version": 1, "ok": true, "result": {}});
            if let Some(o) = ocr {
                resp["ocr"] = o;
            }
            resp
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        assert_eq!(client.ocr_status, None);

        client.sessions(&["a".to_string()], false).await.unwrap();
        let status1 = client.ocr_status.clone().unwrap();
        assert_eq!(status1.engine_used.as_deref(), Some("vision"));
        assert_eq!(status1.fallbacks, 1);
        assert_eq!(status1.last_fallback.as_deref(), Some("rapidocr_timeout"));

        client.sessions(&["a".to_string()], false).await.unwrap();
        client.sessions(&["a".to_string()], false).await.unwrap();
        let status2 = client.ocr_status.clone().unwrap();
        assert_eq!(status2.engine_used.as_deref(), Some("rapidocr"));
        assert_eq!(status2.fallbacks, 1);
        assert_eq!(status2.last_fallback.as_deref(), Some("rapidocr_timeout"));
    }

    #[tokio::test]
    async fn test_layout_error_does_not_fall_back_to_search() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "invalid(\"layout_missing\")"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::new());
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        client.active = true;
        client.connect(5).await.unwrap();

        let err = client
            .retarget(
                "b",
                temp.path().join("b"),
                Some(vec!["b".to_string()]),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("layout_missing"));
        assert_eq!(nav.calls.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_must_wait_keeps_the_worker_connection() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "invalid(\"screen_not_main:context_menu\")"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        let err = client.start().await.unwrap_err();
        assert!(err.to_string().contains("screen_not_main"));
        assert!(client.writer.is_some());
    }

    #[tokio::test]
    async fn test_interrupted_switch_does_not_fall_back_to_search() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "invalid(\"user_input_detected\")"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::new());
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        let err = client.start().await.unwrap_err();
        assert!(err.to_string().contains("user_input_detected"));
        assert_eq!(nav.calls.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_dead_worker_relaunches_quietly_before_search() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");

        struct RelaunchNavigator {
            calls: Arc<Mutex<Vec<Option<String>>>>,
            sock: PathBuf,
            server_handle: Arc<Mutex<Option<MockWorkerServer>>>,
        }

        impl Navigator for RelaunchNavigator {
            fn get_messages<'a>(
                &'a self,
                recipient: Option<&'a str>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>
            {
                let rec = recipient.map(|s| s.to_string());
                Box::pin(async move {
                    self.calls.lock().await.push(rec);
                    let mut guard = self.server_handle.lock().await;
                    if guard.is_none() {
                        let s = MockWorkerServer::spawn(self.sock.clone(), |req| {
                            let id = req["id"].as_u64().unwrap_or(1);
                            json!({"id": id, "protocol_version": 1, "ok": true, "result": {"opened": true}})
                        })
                        .await;
                        *guard = Some(s);
                    }
                    Ok(json!({"ok": true}))
                })
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let nav = Arc::new(RelaunchNavigator {
            calls: calls.clone(),
            sock: sock.clone(),
            server_handle: Arc::new(Mutex::new(None)),
        });

        let mut client = setup_test_client(&temp, sock, Some(nav));
        client.start().await.unwrap();
        let recorded = calls.lock().await;
        assert_eq!(*recorded, vec![None]);
    }

    #[tokio::test]
    async fn test_relaunch_with_bad_map_waits_without_search() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");

        struct BadMapNavigator {
            calls: Arc<Mutex<Vec<Option<String>>>>,
            sock: PathBuf,
            server_handle: Arc<Mutex<Option<MockWorkerServer>>>,
        }

        impl Navigator for BadMapNavigator {
            fn get_messages<'a>(
                &'a self,
                recipient: Option<&'a str>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>
            {
                let rec = recipient.map(|s| s.to_string());
                Box::pin(async move {
                    self.calls.lock().await.push(rec);
                    let mut guard = self.server_handle.lock().await;
                    if guard.is_none() {
                        let s = MockWorkerServer::spawn(self.sock.clone(), |req| {
                            let id = req["id"].as_u64().unwrap_or(1);
                            let op = req["operation"].as_str().unwrap_or("");
                            if op == "open_session" {
                                json!({
                                    "id": id,
                                    "protocol_version": 1,
                                    "ok": false,
                                    "error": "invalid(\"layout_mismatch:input_divider\")"
                                })
                            } else {
                                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
                            }
                        })
                        .await;
                        *guard = Some(s);
                    }
                    Ok(json!({"ok": true}))
                })
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let nav = Arc::new(BadMapNavigator {
            calls: calls.clone(),
            sock: sock.clone(),
            server_handle: Arc::new(Mutex::new(None)),
        });

        let mut client = setup_test_client(&temp, sock, Some(nav));
        let err = client.start().await.unwrap_err();
        assert!(err.to_string().contains("layout_mismatch"));
        let recorded = calls.lock().await;
        assert_eq!(*recorded, vec![None]);
    }

    #[tokio::test]
    async fn test_native_retarget_switches_recipient_and_directory() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "capture" {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {"bubbles": []}})
            } else if op == "open_session" {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {"opened": true, "clicked": true}})
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::new());
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        client.connect(5).await.unwrap();

        let new_dir = temp.path().join("b");
        client.retarget("b", &new_dir, None, false).await.unwrap();
        assert_eq!(client.recipient, "b");
        assert_eq!(client.directory, new_dir);
        assert_eq!(nav.calls.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_native_retarget_falls_back_when_click_is_refused() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "wechat_row_not_topmost"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {"bubbles": []}})
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::new());
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        client.connect(5).await.unwrap();

        let new_dir = temp.path().join("b");
        client.retarget("b", &new_dir, None, false).await.unwrap();
        assert_eq!(client.recipient, "b");
        let calls = nav.calls.lock().await;
        assert_eq!(*calls, vec![Some("b".to_string())]);
        assert_eq!(client.last_switch.as_ref().unwrap()["opened"], false);
    }

    #[tokio::test]
    async fn test_fallback_navigation_tries_alias_names() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "open_session" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": false,
                    "error": "wechat_row_not_topmost"
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {"bubbles": []}})
            }
        })
        .await;

        let nav = Arc::new(MockNavigator::with_failures(1));
        let mut client = setup_test_client(&temp, sock, Some(nav.clone()));
        client.connect(5).await.unwrap();

        let names = vec!["🍓a".to_string(), "a".to_string()];
        client
            .retarget("🍓a", temp.path().join("a"), Some(names), false)
            .await
            .unwrap();
        let calls = nav.calls.lock().await;
        assert_eq!(
            *calls,
            vec![Some("🍓a".to_string()), Some("a".to_string())]
        );
    }

    #[tokio::test]
    async fn test_window_resize_by_worker_is_logged() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "capture" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "profile": "wechat-4-light",
                        "scale": 2,
                        "window_resized": true,
                        "bubbles": []
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        let frame = client.frame().await.unwrap();
        assert_eq!(frame.profile.as_deref(), Some("wechat-4-light"));
        assert_eq!(frame.extra.get("window_resized"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn test_ownership_token_verification_and_creation() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {"opened": true}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        let flag_file = client.base_dir.join("bot-worker.enabled");
        assert!(!flag_file.exists());

        client.start().await.unwrap();
        assert!(flag_file.exists());
        let token = fs::read_to_string(&flag_file).unwrap();
        assert_eq!(token, client.owner_token);
        let mode = fs::metadata(&flag_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn test_ownership_token_close_cleans_flag() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({"id": id, "protocol_version": 1, "ok": true, "result": {"opened": true}})
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.start().await.unwrap();
        let flag_file = client.base_dir.join("bot-worker.enabled");
        assert!(flag_file.exists());

        client.close().await.unwrap();
        assert!(!flag_file.exists());
        assert!(!client.active);
    }

    #[tokio::test]
    async fn test_ownership_token_mismatch_fails() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let mut client = setup_test_client(&temp, sock, None);
        let flag_file = client.base_dir.join("bot-worker.enabled");
        fs::write(&flag_file, b"other_token").unwrap();

        let err = client.start().await.unwrap_err();
        assert!(err.to_string().contains("ownership token mismatch"));
    }

    #[test]
    fn test_must_wait_detection() {
        assert!(must_wait("invalid(\"user_input_detected\")"));
        assert!(must_wait("invalid(\"target_not_reached\")"));
        assert!(must_wait("invalid(\"layout_missing\")"));
        assert!(must_wait("invalid(\"layout_mismatch:input_divider\")"));
        assert!(must_wait("invalid(\"screen_not_main:context_menu\")"));
        assert!(must_wait("session_row_not_unique_or_missing"));
        assert!(!must_wait("wechat_row_not_topmost"));
        assert!(!must_wait("unsupported_operation"));
    }

    #[test]
    fn test_media_reason_and_worker_code() {
        assert_eq!(media_reason("unsupported_file_type"), "unsupported_type");
        assert_eq!(media_reason("file_size_limit"), "too_large");
        assert_eq!(media_reason("media_too_large"), "too_large");
        assert_eq!(media_reason("file_snapshot_changed"), "snapshot_changed");
        assert_eq!(media_reason("something_else"), "download_failed");

        assert_eq!(
            worker_code("private visual operation failed: invalid(\"layout_missing\")"),
            "layout_missing"
        );
    }

    #[test]
    fn test_sha256_known_vector() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn test_start_fails_and_does_not_create_nonexistent_base_dir() {
        let temp = TestDir::new();
        let nonexistent_base = temp.path().join("does_not_exist");
        let sock = temp.path().join("worker.sock");
        let root = temp.path().join("private");
        fs::create_dir_all(&root).unwrap();

        let mut client = WorkerClient::new(
            nonexistent_base.clone(),
            sock,
            "0".repeat(64),
            "a".to_string(),
            root,
            None,
            true,
            false,
            50 * 1024 * 1024,
            None,
        );

        let err = client.start().await.unwrap_err();
        assert!(err.to_string().contains("unsafe_private_path"));
        assert!(!nonexistent_base.exists());
    }

    #[tokio::test]
    async fn test_frame_deserialization_failure_reports_operation_name() {
        let temp = TestDir::new();
        let sock = temp.path().join("worker.sock");
        let _server = MockWorkerServer::spawn(sock.clone(), |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            let op = req["operation"].as_str().unwrap_or("");
            if op == "capture" {
                json!({
                    "id": id,
                    "protocol_version": 1,
                    "ok": true,
                    "result": {
                        "bubbles": "not_an_array"
                    }
                })
            } else {
                json!({"id": id, "protocol_version": 1, "ok": true, "result": {}})
            }
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        let err = client.frame().await.unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("capture"),
            "Error should report operation name: {}",
            msg
        );
    }

    fn test_sample_frame() -> Value {
        json!({
            "bubbles": [
                {
                    "text": "你好",
                    "digest": "d1",
                    "direction": "inbound_visual"
                }
            ],
            "title_lines": ["测试群 (3)"]
        })
    }

    #[tokio::test]
    async fn test_frame_dump_no_flag_writes_nothing() {
        let temp = TestDir::new();
        let state_dir = temp.path().join("state");
        let session_dir = state_dir.join("sessions").join("c_0123456789abcdef");
        fs::create_dir_all(&session_dir).unwrap();

        let sock = temp.path().join("worker.sock");
        let sample = test_sample_frame();
        let sample_clone = sample.clone();
        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({
                "id": id,
                "protocol_version": 1,
                "ok": true,
                "result": sample_clone
            })
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "测试群".to_string();
        client.directory = session_dir.clone();
        client.connect(5).await.unwrap();

        let res = client.frame().await.unwrap();
        assert_eq!(res.title_lines, vec!["测试群 (3)"]);

        let dumped_files: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("frames-"))
            .collect();
        assert!(dumped_files.is_empty(), "未开启 frame-dump 时不写入任何文件");
    }

    #[tokio::test]
    async fn test_frame_dump_flag_appends_capture_and_scroll_privately() {
        let temp = TestDir::new();
        let state_dir = temp.path().join("state");
        let session_dir = state_dir.join("sessions").join("c_0123456789abcdef");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(state_dir.join("frame-dump.enabled"), "").unwrap();

        let sock = temp.path().join("worker.sock");
        let sample = test_sample_frame();
        let sample_clone = sample.clone();
        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({
                "id": id,
                "protocol_version": 1,
                "ok": true,
                "result": sample_clone
            })
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "测试群".to_string();
        client.directory = session_dir.clone();
        client.connect(5).await.unwrap();

        let _ = client.frame().await.unwrap();
        let _ = client.scroll("up", 2).await.unwrap();

        let dumped_files: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("frames-"))
            .map(|e| e.path())
            .collect();
        assert_eq!(dumped_files.len(), 1, "应该生成一个 frames 日志文件");

        let file_path = &dumped_files[0];
        let meta = fs::metadata(file_path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "文件权限必须为 0600");

        let content = fs::read_to_string(file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "应记录两行 (capture 与 scroll_chat)");

        let row0: Value = serde_json::from_str(lines[0]).unwrap();
        let row1: Value = serde_json::from_str(lines[1]).unwrap();

        assert_eq!(row0["op"], "capture");
        assert_eq!(row0["recipient"], "测试群");
        assert_eq!(row0["frame"]["title_lines"][0], "测试群 (3)");
        assert!(row0["t"].as_f64().is_some());

        assert_eq!(row1["op"], "scroll_chat");
        assert_eq!(row1["recipient"], "测试群");
        assert_eq!(row1["frame"]["title_lines"][0], "测试群 (3)");
        assert!(row1["t"].as_f64().is_some());
    }

    #[tokio::test]
    async fn test_frame_dump_failure_never_breaks_capture() {
        let temp = TestDir::new();
        let state_dir = temp.path().join("state");
        let session_dir = state_dir.join("sessions").join("c_0123456789abcdef");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(state_dir.join("frame-dump.enabled"), "").unwrap();

        let sock = temp.path().join("worker.sock");
        let sample = test_sample_frame();
        let sample_clone = sample.clone();
        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({
                "id": id,
                "protocol_version": 1,
                "ok": true,
                "result": sample_clone
            })
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "测试群".to_string();
        let unwritable = session_dir.join("readonly_dir").join("subdir");
        client.directory = unwritable;
        client.connect(5).await.unwrap();

        let res = client.frame().await.unwrap();
        assert_eq!(res.title_lines, vec!["测试群 (3)"]);
    }

    #[tokio::test]
    async fn test_frame_dump_old_files_pruned_and_size_capped() {
        let temp = TestDir::new();
        let state_dir = temp.path().join("state");
        let session_dir = state_dir.join("sessions").join("c_0123456789abcdef");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(state_dir.join("frame-dump.enabled"), "").unwrap();

        let old_file = session_dir.join("frames-20000101.jsonl");
        fs::write(&old_file, "{}\n").unwrap();
        assert!(old_file.exists());

        let sock = temp.path().join("worker.sock");
        let sample = test_sample_frame();
        let sample_clone = sample.clone();
        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let id = req["id"].as_u64().unwrap_or(1);
            json!({
                "id": id,
                "protocol_version": 1,
                "ok": true,
                "result": sample_clone
            })
        })
        .await;

        let mut client = setup_test_client(&temp, sock, None);
        client.recipient = "测试群".to_string();
        client.directory = session_dir.clone();
        client.connect(5).await.unwrap();

        let _ = client.frame().await.unwrap();
        assert!(!old_file.exists(), "超过 3 天的历史 frames 文件应被清理");

        let dumped_files: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("frames-"))
            .map(|e| e.path())
            .collect();
        assert_eq!(dumped_files.len(), 1);
        let today_file = &dumped_files[0];
        let today_size = fs::metadata(today_file).unwrap().len();

        let dummy_frame = json!({"bubbles": []});
        dump_frame_impl(&session_dir, "测试群", "capture", &dummy_frame, today_size).unwrap();

        let content = fs::read_to_string(today_file).unwrap();
        assert_eq!(content.lines().count(), 1, "达到大小上限后不再追加写入");
    }

    fn test_bubble_helper(kind: &str, media_id: &str, filename: Option<&str>) -> Bubble {
        serde_json::from_value(json!({
            "kind": kind,
            "media_id": media_id,
            "filename": filename,
            "direction": "inbound_visual",
            "digest": format!("{}_{}", kind, media_id),
            "text": if kind == "image" { "[图片]" } else { "[文件]" }
        })).unwrap()
    }

    fn test_empty_frame() -> Frame {
        serde_json::from_value(json!({
            "snapshot_id": "snap_1",
            "bubbles": [],
            "title_lines": ["测试"]
        })).unwrap()
    }

    #[tokio::test]
    async fn test_attachments_full_flow_image_and_document() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let calls_clone = calls.clone();

        let img_id = "1".repeat(64);
        let file_id = "2".repeat(64);

        let img_bytes = b"\x89PNG\r\n\x1a\nfakeimagebytes";
        let file_bytes = "口令：竹叶-47".as_bytes();

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            let params = &req["params"];
            calls_clone.lock().unwrap().push(req.clone());

            let result = if op == "prepare_image" {
                json!({"media_id": params["media_id"], "extension": "png"})
            } else if op == "prepare_file" {
                json!({
                    "filename": "sample.txt",
                    "extension": "txt",
                    "size": file_bytes.len(),
                })
            } else if op == "capture" {
                serde_json::to_value(test_sample_frame()).unwrap()
            } else if op == "read_media" {
                let mid = params["media_id"].as_str().unwrap_or("");
                let (bytes, ext) = if mid == "1".repeat(64) {
                    (img_bytes.to_vec(), "png")
                } else {
                    (file_bytes.to_vec(), "txt")
                };
                json!({
                    "offset": 0,
                    "size": bytes.len(),
                    "bytes": test_b64_encode(&bytes),
                    "eof": true,
                    "sha256": sha256(&bytes),
                    "extension": ext,
                })
            } else {
                json!({})
            };

            json!({
                "protocol_version": 1,
                "id": id,
                "ok": true,
                "result": result,
            })
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        calls.lock().unwrap().clear();

        let bubble_img = test_bubble_helper("image", &img_id, None);
        let bubble_file = test_bubble_helper("file", &file_id, Some("sample.txt"));

        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[bubble_img, bubble_file], &frame).await.unwrap();

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].label, "第1张图片");
        assert_eq!(images[0].source, "original");
        assert_eq!(images[0].media_id.as_deref(), Some(img_id.as_str()));
        assert!(images[0].path.exists());

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "sample.txt");
        assert_eq!(files[0].label, "sample.txt");
        assert!(files[0].original_bytes);
        assert_eq!(files[0].content.get("text").and_then(|v| v.as_str()), Some("口令：竹叶-47"));
        assert!(files[0].path.exists());

        assert!(failed.is_empty());

        let recorded: Vec<String> = calls.lock().unwrap().iter()
            .map(|c| c["operation"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(recorded, vec!["prepare_image", "capture", "read_media", "prepare_file", "read_media", "capture"]);
    }

    #[tokio::test]
    async fn test_attachments_image_original_unavailable_fallback_thumbnail() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let img_id = "1".repeat(64);
        let img_bytes = b"thumbnailbytes";

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            if op == "prepare_image" {
                json!({
                    "protocol_version": 1,
                    "id": id,
                    "ok": false,
                    "error": "save_as_failed: invalid(\"menu_not_found\")",
                })
            } else if op == "read_media" {
                json!({
                    "protocol_version": 1,
                    "id": id,
                    "ok": true,
                    "result": {
                        "offset": 0,
                        "size": img_bytes.len(),
                        "bytes": test_b64_encode(img_bytes),
                        "eof": true,
                        "sha256": sha256(img_bytes),
                        "extension": "png",
                    }
                })
            } else {
                json!({"protocol_version": 1, "id": id, "ok": true, "result": {}})
            }
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let bubble_img = test_bubble_helper("image", &img_id, None);
        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[bubble_img], &frame).await.unwrap();

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].label, "第1张图片");
        assert_eq!(images[0].source, "chat_thumbnail");
        assert!(files.is_empty());
        assert!(failed.is_empty());

        let events = client.logged_events.lock().unwrap().clone();
        let unavail_event = events.iter().find(|e| e["event"] == "image_original_unavailable");
        assert!(unavail_event.is_some(), "原图不可用时应记录 image_original_unavailable 事件");
        let ev = unavail_event.unwrap();
        assert_eq!(ev["reason"], "download_failed");
        assert_eq!(ev["detail"], "menu_not_found");
    }

    #[tokio::test]
    async fn test_attachments_action_interrupted_bubble_propagates() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            if op == "prepare_image" {
                json!({
                    "protocol_version": 1,
                    "id": id,
                    "ok": false,
                    "error": "private visual operation failed: invalid(\"user_input_detected\")",
                })
            } else {
                json!({
                    "protocol_version": 1,
                    "id": id,
                    "ok": true,
                    "result": {}
                })
            }
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let bubble_img = test_bubble_helper("image", &"1".repeat(64), None);
        let frame = test_empty_frame();
        let err = client.attachments(&[bubble_img], &frame).await.unwrap_err();
        assert!(action_interrupted(&err.to_string()), "action_interrupted 必须向上抛出");
    }

    /// 停止请求必须打断等待 worker 的 connect（原来最长 60 秒不看停止标志）
    #[tokio::test]
    async fn test_connect_returns_promptly_when_stop_requested() {
        let temp = TestDir::new();
        let sock = temp.path().join("absent.sock");
        let mut client = setup_test_client(&temp, sock, None);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        client.stop = Some(stop.clone());
        let s2 = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            s2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let began = Instant::now();
        let err = client.connect(60).await.unwrap_err();
        assert!(began.elapsed() < Duration::from_secs(3), "{:?}", began.elapsed());
        assert!(err.to_string().contains("stop_requested"), "{err}");
    }

    struct HangingNavigator;
    impl Navigator for HangingNavigator {
        fn get_messages<'a>(
            &'a self,
            _recipient: Option<&'a str>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(json!({}))
            })
        }
    }

    /// 静默重启里 get_messages 可能等桌面锁 30 秒：停止请求同样要能打断
    #[tokio::test]
    async fn test_quiet_relaunch_aborts_promptly_when_stop_requested() {
        let temp = TestDir::new();
        let sock = temp.path().join("absent.sock");
        let mut client = setup_test_client(&temp, sock, Some(Arc::new(HangingNavigator)));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        client.stop = Some(stop.clone());
        let s2 = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            s2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let began = Instant::now();
        let res = client.start().await;
        assert!(res.is_err());
        assert!(began.elapsed() < Duration::from_secs(5), "start 没有响应停止: {:?}", began.elapsed());
    }

    #[test]
    fn test_folder_permission_errors_are_classified_with_folder() {
        assert_eq!(media_reason("MCP tool failed: folder_access_blocked:Desktop"), "folder_permission");
        assert_eq!(media_reason("invalid(\"download_completion_unknown:denied=Desktop,Documents\")"), "folder_permission");
        assert_eq!(worker_code("folder_access_blocked:Desktop"), "folder_access_blocked:Desktop");
        assert_eq!(
            worker_code("invalid(\"download_completion_unknown:denied=Desktop\")"),
            "download_completion_unknown:denied=Desktop"
        );
    }

    async fn folder_blocked_server(sock: PathBuf, op_fails: &'static str) -> MockWorkerServer {
        let ok_txt = b"ok";
        MockWorkerServer::spawn(sock, move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            if op == op_fails {
                return json!({"protocol_version": 1, "id": id, "ok": false, "error": "folder_access_blocked:Desktop"});
            }
            let result = match op {
                "capture" => serde_json::to_value(test_sample_frame()).unwrap(),
                "prepare_file" => json!({"filename": "a.txt", "extension": "txt"}),
                "read_media" => json!({"offset": 0, "size": ok_txt.len(), "bytes": test_b64_encode(ok_txt),
                                       "eof": true, "sha256": sha256(ok_txt), "extension": "png"}),
                _ => json!({}),
            };
            json!({"protocol_version": 1, "id": id, "ok": true, "result": result})
        })
        .await
    }

    #[tokio::test]
    async fn test_file_folder_permission_recorded_logged_and_marked() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let _server = folder_blocked_server(sock.clone(), "prepare_file").await;
        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b = test_bubble_helper("file", &"5".repeat(64), Some("a.txt"));
        let (_, files, failed) = client.attachments(&[b], &test_empty_frame()).await.unwrap();

        assert!(files.is_empty());
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].reason, "folder_permission");
        assert_eq!(failed[0].detail.as_deref(), Some("folder_access_blocked:Desktop"));
        let events = client.logged_events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e["event"] == "attachment_folder_permission"
            && e["detail"] == "folder_access_blocked:Desktop"), "{events:?}");
        let marker: Value = serde_json::from_str(
            &fs::read_to_string(client.base_dir.join("folder-permission-needed.json")).unwrap()).unwrap();
        assert_eq!(marker["detail"], "folder_access_blocked:Desktop");
    }

    #[tokio::test]
    async fn test_image_original_folder_permission_logged_and_marked() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let _server = folder_blocked_server(sock.clone(), "prepare_image").await;
        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b = test_bubble_helper("image", &"6".repeat(64), None);
        let (images, _, _) = client.attachments(&[b], &test_empty_frame()).await.unwrap();

        assert_eq!(images.len(), 1, "原图拿不到时仍用聊天缩略图");
        assert_eq!(images[0].source, "chat_thumbnail");
        let events = client.logged_events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e["event"] == "attachment_folder_permission"), "{events:?}");
        assert!(client.base_dir.join("folder-permission-needed.json").exists());
    }

    #[tokio::test]
    async fn test_successful_original_clears_folder_permission_marker() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let _server = folder_blocked_server(sock.clone(), "none").await;
        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();
        let marker = client.base_dir.join("folder-permission-needed.json");
        fs::write(&marker, "{}").unwrap();

        let b = test_bubble_helper("image", &"7".repeat(64), None);
        let (images, _, _) = client.attachments(&[b], &test_empty_frame()).await.unwrap();

        assert_eq!(images[0].source, "original");
        assert!(!marker.exists(), "权限恢复后提示要自动消失");
    }

    #[tokio::test]
    async fn test_attachments_failed_items_recorded_and_others_continue() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let bad_img_id = "1".repeat(64);
        let good_img_id = "2".repeat(64);
        let empty_file_id = "3".repeat(64);
        let good_file_id = "4".repeat(64);

        let good_txt = b"valid file content";

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            let params = &req["params"];
            let mid = params["media_id"].as_str().unwrap_or("");

            if op == "read_media" && mid == "1".repeat(64) {
                return json!({
                    "protocol_version": 1,
                    "id": id,
                    "ok": false,
                    "error": "media missing",
                });
            }

            let result = if op == "prepare_image" {
                json!({})
            } else if op == "prepare_file" {
                if mid == "3".repeat(64) {
                    json!({ "filename": "empty.txt", "extension": "txt" })
                } else {
                    json!({ "filename": "good.txt", "extension": "txt" })
                }
            } else if op == "capture" {
                serde_json::to_value(test_sample_frame()).unwrap()
            } else if op == "read_media" {
                if mid == "3".repeat(64) {
                    json!({
                        "offset": 0,
                        "size": 0,
                        "bytes": "",
                        "eof": true,
                        "sha256": sha256(b""),
                        "extension": "txt",
                    })
                } else {
                    json!({
                        "offset": 0,
                        "size": good_txt.len(),
                        "bytes": test_b64_encode(good_txt),
                        "eof": true,
                        "sha256": sha256(good_txt),
                        "extension": "txt",
                    })
                }
            } else {
                json!({})
            };

            json!({
                "protocol_version": 1,
                "id": id,
                "ok": true,
                "result": result,
            })
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b_bad_img = test_bubble_helper("image", &bad_img_id, None);
        let b_empty_file = test_bubble_helper("file", &empty_file_id, Some("empty.txt"));
        let b_good_file = test_bubble_helper("file", &good_file_id, Some("good.txt"));
        let b_good_img = test_bubble_helper("image", &good_img_id, None);

        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[b_bad_img, b_empty_file, b_good_file, b_good_img], &frame).await.unwrap();

        assert_eq!(images.len(), 1, "好的图片应该成功");
        assert_eq!(images[0].label, "第2张图片");

        assert_eq!(files.len(), 1, "好的文件应该成功");
        assert_eq!(files[0].filename, "good.txt");

        assert_eq!(failed.len(), 2, "坏图片和空文件应记录在 failed 列表");
        assert_eq!(failed[0].label, "第1张图片");
        assert_eq!(failed[0].reason, "download_failed");
        assert_eq!(failed[1].label, "empty.txt");
        assert_eq!(failed[1].reason, "empty");
    }

    #[tokio::test]
    async fn test_attachments_png_jpg_file_treated_as_image() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let png_bytes = b"\x89PNG\r\n\x1a\nfakeimagebytes";

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            let result = if op == "prepare_file" {
                json!({ "filename": "photo.png", "extension": "png" })
            } else if op == "capture" {
                serde_json::to_value(test_sample_frame()).unwrap()
            } else if op == "read_media" {
                json!({
                    "offset": 0,
                    "size": png_bytes.len(),
                    "bytes": test_b64_encode(png_bytes),
                    "eof": true,
                    "sha256": sha256(png_bytes),
                    "extension": "png",
                })
            } else {
                json!({})
            };

            json!({
                "protocol_version": 1,
                "id": id,
                "ok": true,
                "result": result,
            })
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b_file = test_bubble_helper("file", &"1".repeat(64), Some("photo.png"));
        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[b_file], &frame).await.unwrap();

        assert_eq!(images.len(), 1, "png 扩展名的文件应作为图片处理进入 images");
        assert_eq!(images[0].label, "photo.png");
        assert_eq!(images[0].source, "original");
        assert!(files.is_empty(), "png 文件不应进入 files");
        assert!(failed.is_empty());
    }

    #[tokio::test]
    async fn test_attachments_max_extracted_limit() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let big_str_1 = "a".repeat(20000);
        let big_str_2 = "b".repeat(10000);

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            let params = &req["params"];
            let mid = params["media_id"].as_str().unwrap_or("");

            let result = if op == "prepare_file" {
                let name = if mid == "1".repeat(64) { "f1.txt" } else { "f2.txt" };
                json!({ "filename": name, "extension": "txt" })
            } else if op == "capture" {
                serde_json::to_value(test_sample_frame()).unwrap()
            } else if op == "read_media" {
                let content = if mid == "1".repeat(64) { big_str_1.as_bytes() } else { big_str_2.as_bytes() };
                json!({
                    "offset": 0,
                    "size": content.len(),
                    "bytes": test_b64_encode(content),
                    "eof": true,
                    "sha256": sha256(content),
                    "extension": "txt",
                })
            } else {
                json!({})
            };

            json!({
                "protocol_version": 1,
                "id": id,
                "ok": true,
                "result": result,
            })
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b1 = test_bubble_helper("file", &"1".repeat(64), Some("f1.txt"));
        let b2 = test_bubble_helper("file", &"2".repeat(64), Some("f2.txt"));

        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[b1, b2], &frame).await.unwrap();

        assert!(images.is_empty());
        assert_eq!(files.len(), 1, "第 1 个文件未超限成功提取");
        assert_eq!(files[0].filename, "f1.txt");

        assert_eq!(failed.len(), 1, "第 2 个文件因总字数超 24000 标记 too_large");
        assert_eq!(failed[0].label, "f2.txt");
        assert_eq!(failed[0].reason, "too_large");
    }

    #[tokio::test]
    async fn test_attachments_pdf_with_previews() {
        let temp = TestDir::new();
        let sock = temp.path().join("bot-worker.sock");
        let pdf_bytes = b"%PDF-1.7\nfake pdf content";
        let prev_bytes = b"\x89PNG\r\n\x1a\npreviewimage";

        let _server = MockWorkerServer::spawn(sock.clone(), move |req| {
            let op = req["operation"].as_str().unwrap_or("");
            let id = req["id"].as_u64().unwrap_or(1);
            let params = &req["params"];
            let mid = params["media_id"].as_str().unwrap_or("");

            let result = if op == "prepare_file" {
                json!({
                    "filename": "doc.pdf",
                    "extension": "pdf",
                    "extraction": {
                        "text": "PDF测试提取文字",
                        "page_count": 2,
                        "previews": ["a".repeat(64), "b".repeat(64)],
                    }
                })
            } else if op == "capture" {
                serde_json::to_value(test_sample_frame()).unwrap()
            } else if op == "read_media" {
                let (bytes, ext) = if mid == "a".repeat(64) || mid == "b".repeat(64) {
                    (prev_bytes.to_vec(), "png")
                } else {
                    (pdf_bytes.to_vec(), "pdf")
                };
                json!({
                    "offset": 0,
                    "size": bytes.len(),
                    "bytes": test_b64_encode(&bytes),
                    "eof": true,
                    "sha256": sha256(&bytes),
                    "extension": ext,
                })
            } else {
                json!({})
            };

            json!({
                "protocol_version": 1,
                "id": id,
                "ok": true,
                "result": result,
            })
        }).await;

        let mut client = setup_test_client(&temp, sock, None);
        client.connect(5).await.unwrap();

        let b_pdf = test_bubble_helper("file", &"1".repeat(64), Some("doc.pdf"));
        let frame = test_empty_frame();
        let (images, files, failed) = client.attachments(&[b_pdf], &frame).await.unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "doc.pdf");
        assert_eq!(files[0].content.get("text").and_then(|v| v.as_str()), Some("PDF测试提取文字"));

        assert_eq!(images.len(), 2, "PDF 的两页预览应加入 images");
        assert_eq!(images[0].label, "doc.pdf第1页");
        assert_eq!(images[0].source, "preview");
        assert_eq!(images[1].label, "doc.pdf第2页");
        assert_eq!(images[1].source, "preview");

        assert!(failed.is_empty());
    }
}
