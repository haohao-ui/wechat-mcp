use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

unsafe extern "C" {
    fn getuid() -> u32;
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
    "error".to_string()
}

pub fn media_reason(error: &str) -> String {
    if error.contains("unsupported_file_type") {
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
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.owner_token.is_empty() {
            bail!("persistent worker requires stable ownership token");
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
            let _ = nav.get_messages(None).await;
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

    pub async fn connect(&mut self, timeout_seconds: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
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

    pub async fn frame(&mut self) -> Result<crate::core::model::Frame> {
        let res = self.call("capture", json!({})).await?;
        if res.get("window_resized").and_then(|v| v.as_bool()) == Some(true) {
            tracing::info!(
                "window_resized: profile={:?}, scale={:?}",
                res.get("profile"),
                res.get("scale")
            );
        }
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
            let chunk = crate::wechat::files::decode_base64(b64_str)?;
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
                if sha256_expected != "dummy" && sha256(&data) != sha256_expected {
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
                        "sha256": "dummy",
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
                        "sha256": "dummy",
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
}
