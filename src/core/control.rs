use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::config::BotConfig;
use crate::core::state::write_private;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(f))
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create tokio runtime")
            .block_on(f)
    }
}

pub const BOT_LABEL: &str = "local.wechat-rpa.bot";
pub const HISTORY_LABEL: &str = "local.wechat-rpa.history";
pub const REMOTE_LAYOUT_DIR: &str = ".wechat-rpa/layout";
pub const CALIBRATE_PROBE: &str = "calibrate-probe";

pub fn core_enabled_file() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    home.join(".wechat-rpa/core.enabled")
}

/// 与 serve-core 使用同一路径解析（含 WECHAT_CORE_SOCK 覆盖）
pub fn core_socket_file() -> PathBuf {
    crate::core::external_brain::default_socket_path()
}

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

pub fn format_timestamp_compact(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    unsafe {
        let mut tm: Tm = std::mem::zeroed();
        localtime_r(&secs, &mut tm);
        format!(
            "{:04}{:02}{:02}{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

pub fn shlex_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn resolve_local_path(p: &str) -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    if p == "~" {
        home
    } else if let Some(stripped) = p.strip_prefix("~/") {
        home.join(stripped)
    } else if p.starts_with('/') {
        PathBuf::from(p)
    } else {
        home.join(p)
    }
}

pub fn validate_version_string(version: &str) -> bool {
    let s = version.trim();
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }
    for part in parts {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

pub fn validate_layout_candidate(candidate: &Value) -> Result<()> {
    let obj = candidate.as_object().context("layout_unreadable")?;
    if obj.get("schema").and_then(Value::as_i64) != Some(1) {
        anyhow::bail!("layout_schema_unsupported");
    }
    let window = obj
        .get("window")
        .and_then(Value::as_object)
        .context("layout_invalid_value:window")?;
    let win_w = window
        .get("width")
        .and_then(Value::as_f64)
        .context("layout_invalid_value:window")?;
    let win_h = window
        .get("height")
        .and_then(Value::as_f64)
        .context("layout_invalid_value:window")?;

    let regions = obj
        .get("regions")
        .and_then(Value::as_object)
        .context("layout_unreadable")?;
    let required_regions = ["session_list", "title", "chat", "input"];
    for req in required_regions {
        if !regions.contains_key(req) {
            anyhow::bail!("layout_missing_region:{}", req);
        }
    }

    for (name, rect_val) in regions {
        let rect = rect_val
            .as_object()
            .with_context(|| format!("layout_invalid_value:{}", name))?;
        let x = rect.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = rect.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        let w = match rect.get("w").and_then(Value::as_f64) {
            Some(w) => w,
            None => anyhow::bail!("layout_invalid_value:{}", name),
        };
        let h = match rect.get("h").and_then(Value::as_f64) {
            Some(h) => h,
            None => anyhow::bail!("layout_invalid_value:{}", name),
        };
        if w <= 0.0 {
            anyhow::bail!("layout_invalid_value:{}.w", name);
        }
        if h <= 0.0 {
            anyhow::bail!("layout_invalid_value:{}.h", name);
        }
        if x < 0.0 || y < 0.0 || (x + w) > win_w || (y + h) > win_h {
            anyhow::bail!("layout_region_out_of_window:{}", name);
        }
    }

    let scroll = obj
        .get("scroll")
        .and_then(Value::as_object)
        .context("layout_invalid_value:scroll")?;
    let ppt = scroll
        .get("points_per_tick")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if ppt <= 0.0 {
        anyhow::bail!("layout_invalid_value:scroll.points_per_tick");
    }
    let lpt = scroll
        .get("lines_per_tick")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if lpt <= 0 {
        anyhow::bail!("layout_invalid_value:scroll.lines_per_tick");
    }

    Ok(())
}

pub fn derive_candidate_fields(
    base_layout: &Value,
    measured: &Value,
) -> (Vec<Value>, Vec<String>, Value) {
    let mut candidate = base_layout.clone();
    let window_h = candidate
        .get("window")
        .and_then(|w| w.get("height"))
        .and_then(Value::as_f64)
        .unwrap_or(640.0);

    let mut diffs = Vec::new();
    let mut missing = Vec::new();

    if let Some(anchors) = candidate.get("anchors").and_then(Value::as_array) {
        for a in anchors {
            let kind = a.get("kind").and_then(Value::as_str).unwrap_or("");
            if kind.ends_with("_line") {
                if let Some(name) = a.get("name").and_then(Value::as_str) {
                    let has_measured = measured
                        .as_object()
                        .and_then(|m| m.get(name))
                        .map(|v| !v.is_null())
                        .unwrap_or(false);
                    if !has_measured {
                        missing.push(name.to_string());
                    }
                }
            }
        }
    }

    if let Some(divider_val) = measured
        .as_object()
        .and_then(|m| m.get("input_divider"))
        .filter(|v| !v.is_null())
    {
        if let Some(divider) = divider_val.as_f64() {
            if let Some(regions) = candidate.get_mut("regions").and_then(Value::as_object_mut) {
                if let Some(chat) = regions.get_mut("chat").and_then(Value::as_object_mut) {
                    let chat_y = chat.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    let new_chat_h = divider - chat_y;
                    let old_chat_h = chat.get("h").cloned().unwrap_or(Value::Null);
                    if chat.get("h").and_then(Value::as_f64) != Some(new_chat_h) {
                        diffs
                            .push(json!({"field": "chat.h", "old": old_chat_h, "new": new_chat_h}));
                        chat.insert("h".to_string(), json!(new_chat_h));
                    }
                }
                if let Some(chat_content) = regions
                    .get_mut("chat_content")
                    .and_then(Value::as_object_mut)
                {
                    let cc_y = chat_content.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    let new_cc_h = divider - cc_y;
                    let old_cc_h = chat_content.get("h").cloned().unwrap_or(Value::Null);
                    if chat_content.get("h").and_then(Value::as_f64) != Some(new_cc_h) {
                        diffs.push(
                            json!({"field": "chat_content.h", "old": old_cc_h, "new": new_cc_h}),
                        );
                        chat_content.insert("h".to_string(), json!(new_cc_h));
                    }
                }
                if let Some(inp) = regions.get_mut("input").and_then(Value::as_object_mut) {
                    let old_inp_y = inp.get("y").cloned().unwrap_or(Value::Null);
                    if inp.get("y").and_then(Value::as_f64) != Some(divider) {
                        diffs.push(json!({"field": "input.y", "old": old_inp_y, "new": divider}));
                        inp.insert("y".to_string(), json!(divider));
                    }
                    let new_inp_h = window_h - divider;
                    let old_inp_h = inp.get("h").cloned().unwrap_or(Value::Null);
                    if inp.get("h").and_then(Value::as_f64) != Some(new_inp_h) {
                        diffs.push(json!({"field": "input.h", "old": old_inp_h, "new": new_inp_h}));
                        inp.insert("h".to_string(), json!(new_inp_h));
                    }
                }
            }
        }
    }

    if let Some(border_val) = measured
        .as_object()
        .and_then(|m| m.get("list_border"))
        .filter(|v| !v.is_null())
    {
        if let Some(border) = border_val.as_f64() {
            if let Some(regions) = candidate.get_mut("regions").and_then(Value::as_object_mut) {
                if let Some(s_list) = regions
                    .get_mut("session_list")
                    .and_then(Value::as_object_mut)
                {
                    let s_x = s_list.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                    let new_w = border - s_x;
                    let old_w = s_list.get("w").cloned().unwrap_or(Value::Null);
                    if s_list.get("w").and_then(Value::as_f64) != Some(new_w) {
                        diffs.push(json!({"field": "session_list.w", "old": old_w, "new": new_w}));
                        s_list.insert("w".to_string(), json!(new_w));
                    }
                }
            }
        }
    }

    (diffs, missing, candidate)
}

pub fn require_host(config: Option<&Value>, host: Option<&str>) -> Result<String> {
    if let Some(h) = host {
        if !h.trim().is_empty() {
            return Ok(h.trim().to_string());
        }
    }
    if let Some(cfg) = config {
        if let Some(h) = cfg.get("host").and_then(Value::as_str) {
            if !h.trim().is_empty() {
                return Ok(h.trim().to_string());
            }
        }
    }
    anyhow::bail!("config has no WeChat host; calibrate never guesses one");
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub returncode: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub type RemoteRunner<'a> = &'a mut dyn FnMut(&str, &str, Option<&[u8]>) -> Result<CommandOutput>;

pub fn run_calibrate(
    config: Option<&Value>,
    worker_result: Option<&Value>,
    host: Option<&str>,
    layout_dir: Option<&str>,
    mut remote_runner: Option<RemoteRunner>,
) -> Result<(Vec<Value>, Vec<String>, Value)> {
    let host_str = require_host(config, host)?;
    let layout_dir_str = layout_dir.unwrap_or(REMOTE_LAYOUT_DIR);

    let Some(res) = worker_result else {
        anyhow::bail!("layout_missing");
    };
    if !res.is_object() {
        anyhow::bail!("layout_missing");
    }

    let version = res
        .get("wechat_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !validate_version_string(version) {
        anyhow::bail!("layout_missing");
    }

    let base_layout = res
        .get("layout")
        .and_then(Value::as_object)
        .context("layout_missing")?;

    let measured = res.get("measured").cloned().unwrap_or_else(|| json!({}));
    let (diffs, missing, mut candidate) =
        derive_candidate_fields(&Value::Object(base_layout.clone()), &measured);
    candidate["wechat_version"] = json!(version);

    let cand_json_path = format!("{}/candidate-wechat-{}.json", layout_dir_str, version);
    let cand_png_path = format!("{}/candidate-wechat-{}.png", layout_dir_str, version);

    let mut json_bytes = serde_json::to_vec_pretty(&candidate)?;
    json_bytes.push(b'\n');

    let cmd_json = format!(
        "umask 077 && mkdir -p {} && cat > {} && chmod 600 {}",
        shlex_quote(layout_dir_str),
        shlex_quote(&cand_json_path),
        shlex_quote(&cand_json_path)
    );

    let has_png = res.get("annotated_png_base64").and_then(Value::as_str);
    let png_bytes = if let Some(b64) = has_png {
        crate::wechat::files::decode_base64(b64).ok()
    } else {
        None
    };

    if let Some(ref mut runner) = remote_runner {
        let out = runner(&host_str, &cmd_json, Some(&json_bytes))?;
        if out.returncode != 0 {
            let err_msg = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("remote write failed (candidate json): {}", err_msg);
        }
        if let Some(png) = &png_bytes {
            let cmd_png = format!(
                "umask 077 && mkdir -p {} && cat > {} && chmod 600 {}",
                shlex_quote(layout_dir_str),
                shlex_quote(&cand_png_path),
                shlex_quote(&cand_png_path)
            );
            let out_png = runner(&host_str, &cmd_png, Some(png))?;
            if out_png.returncode != 0 {
                let err_msg = String::from_utf8_lossy(&out_png.stderr);
                anyhow::bail!("remote write failed (candidate png): {}", err_msg);
            }
        }
    } else {
        let resolved_dir = resolve_local_path(layout_dir_str);
        fs::create_dir_all(&resolved_dir)?;
        let _ = fs::set_permissions(&resolved_dir, fs::Permissions::from_mode(0o700));

        let resolved_json = resolved_dir.join(format!("candidate-wechat-{}.json", version));
        fs::write(&resolved_json, &json_bytes)?;
        let _ = fs::set_permissions(&resolved_json, fs::Permissions::from_mode(0o600));

        if let Some(png) = &png_bytes {
            let resolved_png = resolved_dir.join(format!("candidate-wechat-{}.png", version));
            fs::write(&resolved_png, png)?;
            let _ = fs::set_permissions(&resolved_png, fs::Permissions::from_mode(0o600));
        }
    }

    Ok((diffs, missing, candidate))
}

pub fn calibrate_accept(
    config: Option<&Value>,
    host: Option<&str>,
    layout_dir: Option<&str>,
    worker_result: Option<&Value>,
    mut remote_runner: Option<RemoteRunner>,
) -> Result<Value> {
    let host_str = require_host(config, host)?;
    let layout_dir_str = layout_dir.unwrap_or(REMOTE_LAYOUT_DIR);

    let Some(res) = worker_result else {
        anyhow::bail!("layout_missing");
    };
    let running_version = res
        .get("wechat_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !validate_version_string(running_version) {
        anyhow::bail!("layout_missing");
    }

    let cand_path_str = format!(
        "{}/candidate-wechat-{}.json",
        layout_dir_str, running_version
    );

    let cand_bytes = if let Some(ref mut runner) = remote_runner {
        let cat_cmd = format!("cat {}", shlex_quote(&cand_path_str));
        let out = runner(&host_str, &cat_cmd, None)?;
        if out.returncode != 0 {
            anyhow::bail!("candidate_not_found: {}", cand_path_str);
        }
        out.stdout
    } else {
        let resolved_cand = resolve_local_path(&cand_path_str);
        if !resolved_cand.exists() {
            anyhow::bail!("candidate_not_found: {}", cand_path_str);
        }
        fs::read(&resolved_cand)?
    };

    if cand_bytes.len() > 256 * 1024 {
        anyhow::bail!("layout_too_large");
    }

    let candidate_val: Value =
        serde_json::from_slice(&cand_bytes).map_err(|_| anyhow::anyhow!("layout_unreadable"))?;
    validate_layout_candidate(&candidate_val)?;

    let cand_version = candidate_val
        .get("wechat_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if cand_version != running_version {
        anyhow::bail!(
            "version_mismatch: candidate is {}, running is {}",
            cand_version,
            running_version
        );
    }

    let active_path_str = format!("{}/wechat-{}.json", layout_dir_str, running_version);
    let timestamp = format_timestamp_compact(SystemTime::now());
    let backup_path_str = format!(
        "{}/wechat-{}.json.bak-{}",
        layout_dir_str, running_version, timestamp
    );

    let cmd_atomic = format!(
        "if [ -f {} ]; then cp {} {} && chmod 600 {}; fi && mv {} {} && chmod 600 {}",
        shlex_quote(&active_path_str),
        shlex_quote(&active_path_str),
        shlex_quote(&backup_path_str),
        shlex_quote(&backup_path_str),
        shlex_quote(&cand_path_str),
        shlex_quote(&active_path_str),
        shlex_quote(&active_path_str)
    );

    if let Some(ref mut runner) = remote_runner {
        let out = runner(&host_str, &cmd_atomic, None)?;
        if out.returncode != 0 {
            anyhow::bail!("remote_accept_failed");
        }
    } else {
        let resolved_active = resolve_local_path(&active_path_str);
        let resolved_backup = resolve_local_path(&backup_path_str);
        let resolved_cand = resolve_local_path(&cand_path_str);

        if resolved_active.exists() {
            fs::copy(&resolved_active, &resolved_backup)?;
            let _ = fs::set_permissions(&resolved_backup, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&resolved_cand, &resolved_active)?;
        let _ = fs::set_permissions(&resolved_active, fs::Permissions::from_mode(0o600));
    }

    Ok(json!({
        "accepted": true,
        "version": running_version,
        "active_layout": active_path_str,
        "backup": backup_path_str,
    }))
}

pub fn locations() -> (PathBuf, PathBuf, PathBuf) {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    let base = home.join(".WeChatRPABot");
    let config = base.join("config.json");
    let plist = home.join("Library/LaunchAgents/local.wechat-rpa.bot.plist");
    (base, config, plist)
}

/// Same rule as control.py: an existing config must load (errors are reported, never replaced by a default
/// directory, so a broken config cannot act on the real state); only a missing config means `base/state`.
pub fn resolve_state_dir(config_path: &Path, base: &Path) -> Result<PathBuf> {
    if config_path.exists() {
        return BotConfig::load(config_path)
            .map(|cfg| cfg.state_dir)
            .with_context(|| format!("invalid config {}", config_path.display()));
    }
    Ok(base.join("state"))
}

pub fn loaded() -> bool {
    let uid = unsafe { getuid() };
    let status = Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/local.wechat-rpa.bot")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

pub fn check_needs_recalibration(health: Option<&Value>, state_dir: Option<&Path>) -> bool {
    if let Some(h) = health {
        if let Some(obj) = h.as_object() {
            if obj.get("state").and_then(|v| v.as_str()) == Some("needs_recalibration") {
                return true;
            }
            if let Some(sessions) = obj.get("sessions").and_then(|v| v.as_object()) {
                for s in sessions.values() {
                    if let Some(sobj) = s.as_object() {
                        if sobj.get("state").and_then(|v| v.as_str()) == Some("needs_recalibration")
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }

    if let Some(dir) = state_dir {
        let sessions_dir = dir.join("sessions");
        if sessions_dir.exists() && sessions_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&sessions_dir) {
                for entry in entries.flatten() {
                    let sub_health = entry.path().join("health.json");
                    if sub_health.exists() {
                        if let Ok(content) = fs::read_to_string(&sub_health) {
                            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                                if val.get("state").and_then(|v| v.as_str())
                                    == Some("needs_recalibration")
                                {
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

fn collect_json_files(dir: &Path, prefix: &str, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_json_files(&p, prefix, out);
            } else if p.is_file() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with(prefix) && name.ends_with(".json") {
                        out.push(p);
                    }
                }
            }
        }
    }
}

pub fn gather_stats(journal_root: &Path) -> Value {
    let mut files = Vec::new();
    if journal_root.exists() {
        collect_json_files(journal_root, "send-", &mut files);
    }

    let mut triggered = 0;
    let mut client_echo_observed = 0;
    let mut needs_review = 0;
    let total = files.len();

    for f in &files {
        if let Ok(content) = fs::read_to_string(f) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if status == "triggered" {
                    triggered += 1;
                }
                if val.get("client_echo_observed").and_then(|v| v.as_bool()) == Some(true) {
                    client_echo_observed += 1;
                }
                if matches!(
                    status,
                    "generating" | "generated" | "attempting" | "unknown"
                ) {
                    needs_review += 1;
                }
            }
        }
    }

    json!({
        "recorded_batches": total,
        "triggered": triggered,
        "client_echo_observed": client_echo_observed,
        "needs_review": needs_review,
    })
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn execute_control<W1: std::io::Write, W2: std::io::Write>(
    args: &[String],
    stdout: &mut W1,
    stderr: &mut W2,
    loaded_override: Option<bool>,
) -> i32 {
    let mut default_loaded = move || loaded_override.unwrap_or_else(loaded);
    execute_control_full(args, stdout, stderr, Some(&mut default_loaded), None)
}

pub fn execute_control_full<W1: std::io::Write, W2: std::io::Write>(
    args: &[String],
    stdout: &mut W1,
    stderr: &mut W2,
    mut loaded_fn: Option<&mut dyn FnMut() -> bool>,
    mut launchctl_runner: Option<&mut dyn FnMut(&str, &[&str]) -> bool>,
) -> i32 {
    if args.is_empty() {
        let _ = writeln!(stderr, "control: missing action");
        return 2;
    }

    let action = &args[0];
    if action == "-h" || action == "--help" {
        let _ = writeln!(
            stdout,
            "Usage: wechat-mcp control <status|start|stop|pause|resume|resume-session|config-check|calibrate|migrate-history|history-status|merge-contact|unmerge-contact|history-backup|contacts-list|mcp-info|contacts-sync|open-api-check|open-api-token> [--config <path>]"
        );
        return 0;
    }
    let (base, default_config_path, _) = locations();

    let is_loaded = if let Some(ref mut lfn) = loaded_fn {
        lfn()
    } else {
        loaded()
    };

    match action.as_str() {
        "start" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);

            // 配置错误（缺失或无效）是退出码 2，且不打开开关
            if let Err(e) = BotConfig::load(&config_path) {
                let _ = writeln!(stderr, "config error ({}): {e:#}", config_path.display());
                return 2;
            }

            // 旧 Python 任务仍加载时，宿主 CoreSupervisor 会拒绝拉起 Rust 核心：先卸载并禁用它。
            // 绝不 enable/bootstrap 旧任务。
            let uid = unsafe { getuid() };
            let domain = format!("gui/{uid}");
            let label = format!("{domain}/{BOT_LABEL}");
            if is_loaded {
                if let Some(ref mut runner) = launchctl_runner {
                    runner("launchctl", &["disable", &label]);
                    runner("launchctl", &["bootout", &label]);
                } else {
                    let _ = Command::new("launchctl").args(["disable", &label]).status();
                    let _ = Command::new("launchctl").args(["bootout", &label]).status();
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                loop {
                    let still = if let Some(ref mut lfn) = loaded_fn {
                        lfn()
                    } else {
                        loaded()
                    };
                    if !still {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = writeln!(
                            stderr,
                            "legacy Python service {BOT_LABEL} is still loaded; core not enabled"
                        );
                        return 1;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }

            let core_enabled_path = core_enabled_file();
            if let Some(parent) = core_enabled_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    let _ = writeln!(stderr, "cannot create {}: {e}", parent.display());
                    return 1;
                }
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
            if let Err(e) = fs::write(&core_enabled_path, b"") {
                let _ = writeln!(stderr, "cannot write {}: {e}", core_enabled_path.display());
                return 1;
            }
            let _ = fs::set_permissions(&core_enabled_path, fs::Permissions::from_mode(0o600));

            let core_running = std::os::unix::net::UnixStream::connect(core_socket_file()).is_ok();
            let _ = writeln!(
                stdout,
                "{}",
                json!({
                    "status": "started",
                    "core_enabled": true,
                    "core_running": core_running,
                    "note": if core_running { "serve-core is running" } else { "微信客服助手 will start serve-core within a few seconds; it must be running" },
                })
            );
            0
        }
        "stop" => {
            let core_enabled_path = core_enabled_file();
            if core_enabled_path.exists() {
                let _ = fs::remove_file(&core_enabled_path);
            }

            let uid = unsafe { getuid() };
            let domain = format!("gui/{uid}");
            let label = format!("{domain}/{BOT_LABEL}");

            if let Some(ref mut runner) = launchctl_runner {
                runner("launchctl", &["disable", &label]);
            } else {
                let _ = Command::new("launchctl").args(["disable", &label]).status();
            }

            let loaded_first = if let Some(ref mut lfn) = loaded_fn {
                lfn()
            } else {
                loaded()
            };

            if loaded_first {
                if let Some(ref mut runner) = launchctl_runner {
                    runner("launchctl", &["bootout", &label]);
                } else {
                    let _ = Command::new("launchctl").args(["bootout", &label]).status();
                }
            }

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let mut is_still_loaded = if let Some(ref mut lfn) = loaded_fn {
                lfn()
            } else {
                loaded()
            };

            while is_still_loaded && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
                is_still_loaded = if let Some(ref mut lfn) = loaded_fn {
                    lfn()
                } else {
                    loaded()
                };
            }

            let final_loaded = if let Some(ref mut lfn) = loaded_fn {
                lfn()
            } else {
                loaded()
            };

            if final_loaded {
                let _ = writeln!(
                    stderr,
                    "service is still unloading; do not start a replacement yet"
                );
                return 1;
            }

            // 宿主看到开关关闭后会优雅停止核心（会先完成正在处理的一轮），等待 core.sock 关闭
            let core_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            let mut core_running =
                std::os::unix::net::UnixStream::connect(core_socket_file()).is_ok();
            while core_running && std::time::Instant::now() < core_deadline {
                std::thread::sleep(std::time::Duration::from_millis(200));
                core_running = std::os::unix::net::UnixStream::connect(core_socket_file()).is_ok();
            }
            if core_running {
                let _ = writeln!(
                    stderr,
                    "serve-core is still finishing its current turn; it will stop shortly"
                );
            }
            let _ = writeln!(
                stdout,
                "{}",
                json!({
                    "status": if core_running { "stopping" } else { "stopped" },
                    "label": BOT_LABEL,
                    "core_enabled": false,
                    "core_running": core_running
                })
            );
            0
        }
        "config-check" => {
            // 支持 config-check <path> 或 config-check --config <path>
            let mut target_path: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    target_path = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else if !args[idx].starts_with('-') && target_path.is_none() {
                    target_path = Some(PathBuf::from(&args[idx]));
                    idx += 1;
                } else {
                    idx += 1;
                }
            }

            let path = target_path.unwrap_or(default_config_path);
            match BotConfig::load(&path) {
                Ok(_) => {
                    let _ = writeln!(stdout, "{}", json!({"ok": true}));
                    0
                }
                Err(e) => {
                    let _ = writeln!(stdout, "{}", json!({"ok": false, "error": e.to_string()}));
                    2
                }
            }
        }
        "pause" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let state_dir = match resolve_state_dir(&config_path, &base) {
                Ok(dir) => dir,
                Err(error) => {
                    let _ = writeln!(stderr, "control: {error:#}");
                    return 2;
                }
            };

            if let Err(e) = fs::create_dir_all(&state_dir) {
                let _ = writeln!(stderr, "failed to create state dir: {}", e);
                return 1;
            }
            let pause_file = state_dir.join("admin-pause.json");
            let pause_data = json!({
                "paused": true,
                "at": now_secs(),
            });
            if let Err(e) = write_private(&pause_file, &pause_data) {
                let _ = writeln!(stderr, "failed to write pause file: {}", e);
                return 1;
            }

            let _ = writeln!(
                stdout,
                "{}",
                json!({
                    "status": "paused",
                    "file": pause_file.to_string_lossy().to_string(),
                })
            );
            0
        }
        "resume" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let state_dir = match resolve_state_dir(&config_path, &base) {
                Ok(dir) => dir,
                Err(error) => {
                    let _ = writeln!(stderr, "control: {error:#}");
                    return 2;
                }
            };

            let pause_file = state_dir.join("admin-pause.json");
            if pause_file.exists() {
                let _ = fs::remove_file(&pause_file);
            }

            let _ = writeln!(stdout, "{}", json!({"status": "resumed"}));
            0
        }
        "resume-session" => {
            let mut target_session: Option<String> = None;
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else if !args[idx].starts_with('-') && target_session.is_none() {
                    target_session = Some(args[idx].clone());
                    idx += 1;
                } else {
                    idx += 1;
                }
            }

            let Some(session_name) = target_session else {
                let _ = writeln!(stderr, "control resume-session: missing session name");
                return 2;
            };

            let config_path = config_file.unwrap_or(default_config_path);
            // Never fall back to a default directory: a wrong config must not touch the real state.
            let state_dir = match BotConfig::load(&config_path) {
                Ok(cfg) => cfg.state_dir,
                Err(_) => match fs::read_to_string(&config_path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| {
                        v.get("state_dir")
                            .and_then(Value::as_str)
                            .map(PathBuf::from)
                    }) {
                    Some(dir) => dir,
                    None => {
                        let _ = writeln!(
                            stderr,
                            "control resume-session: cannot read state_dir from {}",
                            config_path.display()
                        );
                        return 2;
                    }
                },
            };

            let socket = crate::core::brain_tools::default_socket_path();
            match resume_session_with(&socket, &state_dir, &session_name) {
                Ok(ResumeOutcome::Core) => {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        json!({"ok": true, "session": session_name, "status": "resumed", "via": "core", "file_removed": false})
                    );
                    0
                }
                Ok(ResumeOutcome::MarkerRemoved(n)) => {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        json!({"ok": true, "session": session_name, "status": if n > 0 { "resumed" } else { "not_handed_off" }, "via": "file", "file_removed": n > 0})
                    );
                    0
                }
                Err(error) => {
                    let _ = writeln!(stderr, "control resume-session: {error:#}");
                    1
                }
            }
        }
        "status" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let state_dir = match resolve_state_dir(&config_path, &base) {
                Ok(dir) => dir,
                Err(error) => {
                    let _ = writeln!(stderr, "control: {error:#}");
                    return 2;
                }
            };

            let health_path = state_dir.join("health.json");
            let health = if health_path.exists() {
                fs::read_to_string(&health_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            } else {
                None
            };

            let sessions_dir = state_dir.join("sessions");
            let journal_root = if sessions_dir.exists() {
                sessions_dir
            } else {
                state_dir.clone()
            };

            let stats = gather_stats(&journal_root);
            let needs_recalibration = check_needs_recalibration(health.as_ref(), Some(&state_dir));
            if needs_recalibration {
                let _ = writeln!(
                    stderr,
                    "界面核对连续失败，请运行 control calibrate 重新定位"
                );
            }

            let core_enabled_path = core_enabled_file();
            let core_enabled = core_enabled_path.exists();
            let core_sock_path = core_socket_file();
            let core_running = std::os::unix::net::UnixStream::connect(&core_sock_path).is_ok();

            if is_loaded && core_enabled {
                let _ = writeln!(
                    stderr,
                    "警告：检测到旧 Python 服务 (local.wechat-rpa.bot) 与 core.enabled 同时存在冲突"
                );
            }

            let out = json!({
                "launchd_loaded": is_loaded,
                "python_launchd_loaded": is_loaded,
                "core_enabled": core_enabled,
                "core_running": core_running,
                "health": health,
                "stats": stats,
                "config": config_path.to_string_lossy().to_string(),
                "needs_recalibration": needs_recalibration,
            });
            let _ = writeln!(stdout, "{}", out);
            0
        }
        "calibrate" => {
            // 校准请求要由正在运行的机器人处理：只认真实运行的进程，开关文件本身不算
            let core_running = std::os::unix::net::UnixStream::connect(core_socket_file()).is_ok();
            if !(is_loaded || core_running) {
                let _ = writeln!(stderr, "机器人未运行：请先 control start");
                return 1;
            }

            let mut config_file: Option<PathBuf> = None;
            let mut accept = false;
            let mut timeout = 120.0f64;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--accept" => {
                        accept = true;
                        idx += 1;
                    }
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--timeout" if idx + 1 < args.len() => {
                        if let Ok(t) = args[idx + 1].parse::<f64>() {
                            timeout = t;
                        }
                        idx += 2;
                    }
                    "--layout-dir" if idx + 1 < args.len() => {
                        // 保留兼容性
                        idx += 2;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }

            let config_path = config_file.unwrap_or(default_config_path);
            let state_dir = match resolve_state_dir(&config_path, &base) {
                Ok(dir) => dir,
                Err(error) => {
                    let _ = writeln!(stderr, "control: {error:#}");
                    return 2;
                }
            };

            if let Err(e) = fs::create_dir_all(&state_dir) {
                let _ = writeln!(stderr, "failed to create state dir: {}", e);
                return 1;
            }
            let _ = fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700));

            let req_id = uuid::Uuid::new_v4().as_simple().to_string();
            let action = if accept { "accept" } else { "calibrate" };
            let req_file = state_dir.join("calibrate-request.json");
            let res_file = state_dir.join("calibrate-result.json");

            let req_data = json!({
                "id": req_id,
                "action": action,
                "created_at": now_secs(),
            });
            if let Err(e) = write_private(&req_file, &req_data) {
                let _ = writeln!(stderr, "failed to write calibrate request: {}", e);
                return 1;
            }

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout);
            let mut result_data: Option<Value> = None;

            while std::time::Instant::now() < deadline {
                if res_file.exists() {
                    if let Ok(content) = fs::read_to_string(&res_file) {
                        if let Ok(data) = serde_json::from_str::<Value>(&content) {
                            if data.get("id").and_then(|v| v.as_str()) == Some(&req_id) {
                                result_data = Some(data);
                                break;
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            let Some(res_val) = result_data else {
                let _ = writeln!(stderr, "calibrate timed out waiting for bot");
                return 1;
            };

            if res_val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let err = res_val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                let prefix = if accept {
                    "calibrate accept failed"
                } else {
                    "calibrate failed"
                };
                let _ = writeln!(stderr, "{}: {}", prefix, err);
                return 1;
            }

            let empty_res = json!({});
            let res = res_val.get("result").unwrap_or(&empty_res);
            if accept {
                let _ = writeln!(stdout, "{}", res);
                return 0;
            }

            let diffs = res
                .get("differences")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let missing = res
                .get("missing")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if !diffs.is_empty() {
                let _ = writeln!(stdout, "Differences from current layout:");
                for d in &diffs {
                    let field = d.get("field").and_then(|v| v.as_str()).unwrap_or("");
                    let old = d.get("old").unwrap_or(&Value::Null);
                    let new = d.get("new").unwrap_or(&Value::Null);
                    let _ = writeln!(stdout, "  {}: {} -> {}", field, old, new);
                }
            } else {
                let _ = writeln!(stdout, "No layout differences detected.");
            }

            if !missing.is_empty() {
                let _ = writeln!(stdout, "Missing measurements (preserved original values):");
                for m in &missing {
                    let m_str = m.as_str().unwrap_or("");
                    let _ = writeln!(stdout, "  {}", m_str);
                }
            }

            let _ = writeln!(stdout, "{}", res);
            0
        }
        "migrate-history" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let config_val = match BotConfig::load(&config_path) {
                Ok(c) => serde_json::to_value(&c).unwrap_or(Value::Null),
                Err(e) => {
                    let _ = writeln!(stderr, "failed to load config: {}", e);
                    return 2;
                }
            };
            match crate::core::history::migrate::migrate(&config_val) {
                Ok(res) => {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        serde_json::to_string(&res).unwrap_or_default()
                    );
                    0
                }
                Err(e) => {
                    let _ = writeln!(stderr, "migrate error: {}", e);
                    1
                }
            }
        }
        "history-status" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                if args[idx] == "--config" && idx + 1 < args.len() {
                    config_file = Some(PathBuf::from(&args[idx + 1]));
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to load config: {}", e);
                    return 2;
                }
            };
            let store = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to open history store: {}", e);
                    return 1;
                }
            };
            match crate::core::history::migrate::history_status(&store) {
                Ok(val) => {
                    let _ = writeln!(stdout, "{}", val);
                    0
                }
                Err(e) => {
                    let _ = writeln!(stderr, "history status error: {}", e);
                    1
                }
            }
        }
        "merge-contact" => {
            let mut config_file: Option<PathBuf> = None;
            let mut source: Option<String> = None;
            let mut into: Option<String> = None;
            let mut note = String::new();
            let mut idx = 1;
            let mut positional = Vec::new();
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--source" if idx + 1 < args.len() => {
                        source = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    "--into" if idx + 1 < args.len() => {
                        into = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    "--note" if idx + 1 < args.len() => {
                        note = args[idx + 1].clone();
                        idx += 2;
                    }
                    pos if !pos.starts_with('-') => {
                        positional.push(pos.to_string());
                        idx += 1;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            if source.is_none() && !positional.is_empty() {
                source = Some(positional.remove(0));
            }
            if into.is_none() && !positional.is_empty() {
                into = Some(positional.remove(0));
            }
            if note.is_empty() && !positional.is_empty() {
                note = positional.remove(0);
            }
            let (Some(src), Some(dst)) = (source, into) else {
                let _ = writeln!(stderr, "merge-contact requires --source and --into");
                return 2;
            };
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to load config: {}", e);
                    return 2;
                }
            };
            let _maintenance_lock =
                match crate::core::serve::acquire_core_lock_for_maintenance(&cfg.state_dir) {
                    Ok(lock) => lock,
                    Err(e) => {
                        let _ = writeln!(stderr, "{}", e);
                        return 1;
                    }
                };
            let store = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to open history store: {}", e);
                    return 1;
                }
            };
            let src_cid = match crate::core::history::migrate::resolve_contact(
                &store,
                &crate::core::wechat_account::effective_account_id(&cfg.state_dir, &cfg.account_id),
                &src,
            ) {
                Ok(id) => id,
                Err(_) => {
                    let _ = writeln!(stderr, "unknown contact: {}", src);
                    return 1;
                }
            };
            let dst_cid = match crate::core::history::migrate::resolve_contact(
                &store,
                &crate::core::wechat_account::effective_account_id(&cfg.state_dir, &cfg.account_id),
                &dst,
            ) {
                Ok(id) => id,
                Err(_) => {
                    let _ = writeln!(stderr, "unknown contact: {}", dst);
                    return 1;
                }
            };
            match store.merge(&src_cid, &dst_cid, &note) {
                Ok(merge_id) => {
                    let _ = writeln!(stdout, "{}", json!({"merge_id": merge_id}));
                    0
                }
                Err(e) => {
                    let _ = writeln!(stderr, "merge error: {}", e);
                    1
                }
            }
        }
        "unmerge-contact" => {
            let mut config_file: Option<PathBuf> = None;
            let mut merge_id: Option<String> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--merge-id" if idx + 1 < args.len() => {
                        merge_id = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    id if !id.starts_with('-') => {
                        merge_id = Some(id.to_string());
                        idx += 1;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let Some(mid) = merge_id else {
                let _ = writeln!(stderr, "unmerge-contact requires merge_id");
                return 2;
            };
            let Ok(mid_i64) = mid.parse::<i64>() else {
                let _ = writeln!(stderr, "invalid merge_id: {}", mid);
                return 2;
            };
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to load config: {}", e);
                    return 2;
                }
            };
            let _maintenance_lock =
                match crate::core::serve::acquire_core_lock_for_maintenance(&cfg.state_dir) {
                    Ok(lock) => lock,
                    Err(e) => {
                        let _ = writeln!(stderr, "{}", e);
                        return 1;
                    }
                };
            let store = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to open history store: {}", e);
                    return 1;
                }
            };
            match store.unmerge(mid_i64) {
                Ok(_) => {
                    let _ = writeln!(stdout, "{}", json!({"unmerged": mid_i64}));
                    0
                }
                Err(e) => {
                    let _ = writeln!(stderr, "unmerge error: {}", e);
                    1
                }
            }
        }
        "history-backup" => {
            let mut config_file: Option<PathBuf> = None;
            let mut to_path: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--to" if idx + 1 < args.len() => {
                        to_path = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    p if !p.starts_with('-') => {
                        to_path = Some(PathBuf::from(p));
                        idx += 1;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let Some(target) = to_path else {
                let _ = writeln!(stderr, "history-backup requires --to <path>");
                return 2;
            };
            if target.exists() {
                let _ = writeln!(stderr, "backup target already exists: {}", target.display());
                return 1;
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to load config: {}", e);
                    return 2;
                }
            };
            let store = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to open history store: {}", e);
                    return 1;
                }
            };
            match crate::core::history::migrate::history_backup(&store, &target) {
                Ok(bytes) => {
                    let _ = writeln!(
                        stdout,
                        "{}",
                        json!({"backup": target.to_string_lossy(), "bytes": bytes})
                    );
                    0
                }
                Err(e) => {
                    let _ = writeln!(stderr, "history-backup error: {}", e);
                    1
                }
            }
        }
        "contacts-list" => {
            let mut config_file: Option<PathBuf> = None;
            let mut category_filter: Option<String> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--category" if idx + 1 < args.len() => {
                        category_filter = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    "--json" => {
                        idx += 1;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(
                        stderr,
                        "failed to load config ({}): {e:#}",
                        config_path.display()
                    );
                    return 2;
                }
            };
            let store = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to open history store: {e:#}");
                    return 1;
                }
            };

            // 只列当前登录微信账号的联系人（换号后各账号的联系人分别保留）
            let account_id =
                crate::core::wechat_account::effective_account_id(&cfg.state_dir, &cfg.account_id);
            let sync_file = crate::core::contacts::sync::account_state_path(
                &cfg.state_dir,
                &account_id,
                "result",
            );
            let sync_val: Value = if sync_file.exists() {
                match fs::read_to_string(&sync_file)
                    .ok()
                    .and_then(|c| serde_json::from_str(&c).ok())
                {
                    Some(v) => v,
                    None => Value::Null,
                }
            } else {
                Value::Null
            };

            let conn = store.connection();
            let guard = match conn.lock() {
                Ok(g) => g,
                Err(_) => {
                    let _ = writeln!(stderr, "history db lock error");
                    return 1;
                }
            };

            let mut stmt = match guard.prepare(
                r#"SELECT
                    c.contact_id,
                    c.display_name,
                    c.kind,
                    p.category,
                    p.wechat_id,
                    p.nickname,
                    p.remark,
                    p.region,
                    p.source,
                    p.added_on,
                    p.common_groups,
                    p.enterprise,
                    p.real_name,
                    p.tags,
                    p.friend_permission,
                    p.identity,
                    p.removed_at,
                    m.cnt,
                    m.max_obs
                FROM contacts c
                LEFT JOIN contact_profiles p ON c.contact_id = p.contact_id
                LEFT JOIN (
                    SELECT contact_id, COUNT(*) as cnt, MAX(observed_at) as max_obs
                    FROM messages
                    GROUP BY contact_id
                ) m ON c.contact_id = m.contact_id
                WHERE c.merged_into IS NULL AND c.account_id = ?1
                ORDER BY c.rowid"#,
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to prepare query: {e:#}");
                    return 1;
                }
            };

            let mapped = stmt.query_map([&account_id], |row| {
                let cid: String = row.get(0)?;
                let disp_name: String = row.get(1)?;
                let kind: String = row.get(2)?;
                let cat: Option<String> = row.get(3)?;
                let wechat_id: Option<String> = row.get(4)?;
                let nickname: Option<String> = row.get(5)?;
                let remark: Option<String> = row.get(6)?;
                let region: Option<String> = row.get(7)?;
                let source: Option<String> = row.get(8)?;
                let added_on: Option<String> = row.get(9)?;
                let _common_groups: Option<i64> = row.get(10)?;
                let enterprise: Option<String> = row.get(11)?;
                let real_name: Option<String> = row.get(12)?;
                let tags_str: Option<String> = row.get(13)?;
                let _friend_perm: Option<String> = row.get(14)?;
                let identity: Option<String> = row.get(15)?;
                let removed_at: Option<i64> = row.get(16)?;
                let cnt: Option<i64> = row.get(17)?;
                let max_obs: Option<i64> = row.get(18)?;

                let category = cat.unwrap_or_else(|| {
                    if kind == "group" {
                        "group".to_string()
                    } else {
                        "personal".to_string()
                    }
                });
                let tags: Value = tags_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_else(|| json!([]));
                let ident = identity.unwrap_or_else(|| {
                    if wechat_id.is_some() {
                        "strong".to_string()
                    } else {
                        "weak".to_string()
                    }
                });

                Ok((
                    cid, disp_name, category, wechat_id, nickname, remark, region, source,
                    added_on, tags, enterprise, real_name, ident, removed_at, cnt, max_obs,
                ))
            });

            let rows: Vec<_> = match mapped {
                Ok(iter) => iter.filter_map(Result::ok).collect(),
                Err(e) => {
                    let _ = writeln!(stderr, "query error: {e:#}");
                    return 1;
                }
            };

            let mut contacts_out = Vec::new();
            for (
                cid,
                disp_name,
                category,
                wechat_id,
                nickname,
                remark,
                region,
                source,
                added_on,
                tags,
                enterprise,
                real_name,
                identity,
                removed_at,
                cnt,
                max_obs,
            ) in rows
            {
                if let Some(ref cf) = category_filter {
                    if &category != cf {
                        continue;
                    }
                }

                let mut contact_all_names = vec![disp_name.clone()];
                if let Some(ref nk) = nickname {
                    contact_all_names.push(nk.clone());
                }
                if let Some(ref rm) = remark {
                    contact_all_names.push(rm.clone());
                }

                let mut is_ignored = false;
                let mut is_allowed = false;
                for n in &contact_all_names {
                    let d = crate::core::scheduler::decision(&cfg, n, None);
                    if d == "ignored" {
                        is_ignored = true;
                        break;
                    } else if d == "allowed" {
                        is_allowed = true;
                    }
                }
                let handling = if is_ignored {
                    "ignored"
                } else if is_allowed {
                    "auto_reply"
                } else {
                    "record_only"
                };

                contacts_out.push(json!({
                    "contact_id": cid,
                    "category": category,
                    "display_name": disp_name,
                    "nickname": nickname,
                    "remark": remark,
                    "wechat_id": wechat_id,
                    "identity": identity,
                    "region": region,
                    "source": source,
                    "added_on": added_on,
                    "tags": tags,
                    "enterprise": enterprise,
                    "real_name": real_name,
                    "handling": handling,
                    "removed": removed_at.is_some(),
                    "last_message_at_ms": max_obs,
                    "message_count": cnt.unwrap_or(0),
                }));
            }

            let mut rev_stmt = match guard.prepare(
                "SELECT id, reason, candidates, profile FROM contact_reviews WHERE account_id=? AND resolved_at IS NULL ORDER BY id"
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to prepare review query: {e:#}");
                    return 1;
                }
            };
            let rev_mapped = rev_stmt.query_map([&account_id], |r| {
                let id: i64 = r.get(0)?;
                let reason: String = r.get(1)?;
                let cand_str: String = r.get(2)?;
                let prof_str: String = r.get(3)?;
                let candidates: Value =
                    serde_json::from_str(&cand_str).unwrap_or_else(|_| json!([]));
                let prof: Value = serde_json::from_str(&prof_str).unwrap_or_else(|_| json!({}));
                let name = prof
                    .get("remark")
                    .and_then(|v| v.as_str())
                    .or_else(|| prof.get("nickname").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_string();
                Ok(json!({
                    "id": id,
                    "reason": reason,
                    "candidates": candidates,
                    "name": name,
                }))
            });
            let reviews: Vec<Value> = match rev_mapped {
                Ok(iter) => iter.filter_map(Result::ok).collect(),
                Err(_) => Vec::new(),
            };

            let out_obj = json!({
                "account_id": account_id,
                "sync": sync_val,
                "contacts": contacts_out,
                "reviews": reviews,
            });

            let _ = writeln!(
                stdout,
                "{}",
                serde_json::to_string_pretty(&out_obj).unwrap_or_default()
            );
            0
        }
        "mcp-info" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(
                        stderr,
                        "failed to load config ({}): {e:#}",
                        config_path.display()
                    );
                    return 2;
                }
            };

            let brain_val = json!({
                "mode": cfg.brain.mode,
                "external": {
                    "timeout_seconds": cfg.brain.external.timeout_seconds,
                    "timeout_action": cfg.brain.external.timeout_action,
                    "queue_minutes": cfg.brain.external.queue_minutes,
                }
            });

            let home = std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"));
            let tools_path = home.join(".wechat-rpa/bin/wechat-mcp");
            let tools_ok = tools_path.exists() && tools_path.is_file();
            let tools_val_str = if tools_path.exists() {
                tools_path.to_string_lossy().to_string()
            } else {
                "~/.wechat-rpa/bin/wechat-mcp".to_string()
            };

            let sock_path = core_socket_file();
            let sock_ok = std::os::unix::net::UnixStream::connect(&sock_path).is_ok();

            let http_bind = "127.0.0.1";
            let http_port = 8765;
            let http_addr = format!("{}:{}", http_bind, http_port);
            let http_ok = std::net::TcpStream::connect(&http_addr).is_ok();
            let http_val_str = format!("http://{}", http_addr);

            let open_api_val_str =
                format!("http://{}:{}/mcp", cfg.open_api.bind, cfg.open_api.port);
            let (open_api_ok, open_api_detail) = if !cfg.open_api.enabled {
                (false, "已关闭".to_string())
            } else {
                let check = block_on(crate::core::open_api::http::self_check(
                    &cfg.open_api,
                    &cfg.state_dir,
                ));
                let ok = check["ok"].as_bool().unwrap_or(false);
                let detail = if check["busy"].as_bool() == Some(true) {
                    "已有大脑连接中".to_string()
                } else if ok {
                    "可连接".to_string()
                } else {
                    check["error"].as_str().unwrap_or("连接失败").to_string()
                };
                (ok, detail)
            };

            let endpoints = json!([
                {
                    "id": "tools",
                    "label": "MCP 工具服务",
                    "value": tools_val_str,
                    "ok": tools_ok,
                    "detail": ""
                },
                {
                    "id": "core_sock",
                    "label": "外部大脑接口",
                    "value": sock_path.to_string_lossy().to_string(),
                    "ok": sock_ok,
                    "detail": ""
                },
                {
                    "id": "history_http",
                    "label": "聊天历史查询",
                    "value": http_val_str,
                    "ok": http_ok,
                    "detail": ""
                },
                {
                    "id": "open_api",
                    "label": "对外接口",
                    "value": open_api_val_str,
                    "ok": open_api_ok,
                    "detail": open_api_detail
                }
            ]);

            let tools_list = crate::tools::list_tools_with_legacy(true);
            let mut all_tool_names: Vec<String> = tools_list["tools"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t["name"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            for h in crate::core::history::mcp_tools::ALL_HISTORY_TOOLS {
                if !all_tool_names.contains(&h.to_string()) {
                    all_tool_names.push(h.to_string());
                }
            }

            let open_api_tools: Vec<String> = crate::core::open_api::http::open_tool_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            let desktop_tools = vec![
                "desktop_apps".to_string(),
                "desktop_windows".to_string(),
                "desktop_observe".to_string(),
                "desktop_act".to_string(),
                "desktop_stop".to_string(),
            ];
            let mut ops_tools = Vec::new();
            for t in all_tool_names {
                if !open_api_tools.contains(&t) && !desktop_tools.contains(&t) {
                    ops_tools.push(t);
                }
            }

            let tool_groups = json!([
                {"title": "对外接口", "tools": open_api_tools},
                {"title": "运维", "tools": ops_tools},
                {"title": "通用桌面", "tools": desktop_tools}
            ]);

            let out_obj = json!({
                "brain": brain_val,
                "endpoints": endpoints,
                "tool_groups": tool_groups,
            });

            let _ = writeln!(
                stdout,
                "{}",
                serde_json::to_string_pretty(&out_obj).unwrap_or_default()
            );
            0
        }
        "open-api-check" => {
            let mut config_file: Option<PathBuf> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(
                        stderr,
                        "failed to load config ({}): {e:#}",
                        config_path.display()
                    );
                    return 2;
                }
            };

            let check = if !cfg.open_api.enabled {
                json!({
                    "ok": false,
                    "busy": false,
                    "tools": [],
                    "error": "disabled"
                })
            } else {
                block_on(crate::core::open_api::http::self_check(
                    &cfg.open_api,
                    &cfg.state_dir,
                ))
            };
            let _ = writeln!(
                stdout,
                "{}",
                serde_json::to_string_pretty(&check).unwrap_or_default()
            );
            0
        }
        "open-api-token" => {
            let mut config_file: Option<PathBuf> = None;
            let mut rotate = false;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--rotate" => {
                        rotate = true;
                        idx += 1;
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(
                        stderr,
                        "failed to load config ({}): {e:#}",
                        config_path.display()
                    );
                    return 2;
                }
            };

            let token_res = if rotate {
                crate::core::open_api::http::rotate_token(&cfg.state_dir)
            } else {
                crate::core::open_api::http::ensure_token(&cfg.state_dir)
            };
            let token = match token_res {
                Ok(t) => t,
                Err(e) => {
                    let _ = writeln!(stderr, "failed to get token: {e:#}");
                    return 1;
                }
            };
            let out_obj = json!({
                "token": token
            });
            let _ = writeln!(
                stdout,
                "{}",
                serde_json::to_string_pretty(&out_obj).unwrap_or_default()
            );
            0
        }
        "contacts-sync" => {
            let mut config_file: Option<PathBuf> = None;
            let mut full = false;
            let mut wait_seconds: Option<u64> = None;
            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--config" if idx + 1 < args.len() => {
                        config_file = Some(PathBuf::from(&args[idx + 1]));
                        idx += 2;
                    }
                    "--full" => {
                        full = true;
                        idx += 1;
                    }
                    "--wait" if idx + 1 < args.len() => {
                        wait_seconds = args[idx + 1].parse().ok();
                        idx += 2;
                    }
                    other => {
                        // 未知参数（含 --help）只打印用法：同步会在微信里逐个点开资料卡，不能被误触发
                        let _ = writeln!(
                            stderr,
                            "Usage: wechat-mcp control contacts-sync [--full] [--wait SECONDS] [--config PATH]\nunknown argument: {other}"
                        );
                        return 2;
                    }
                }
            }
            let config_path = config_file.unwrap_or(default_config_path);
            let cfg = match BotConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(
                        stderr,
                        "failed to load config ({}): {e:#}",
                        config_path.display()
                    );
                    return 2;
                }
            };
            let state_dir = cfg.state_dir.clone();
            if let Err(e) = fs::create_dir_all(&state_dir) {
                let _ = writeln!(stderr, "failed to create state dir: {e:#}");
                return 1;
            }

            let req_id = format!("req_{}", uuid::Uuid::new_v4().simple());
            let mode = if full { "full" } else { "incremental" };
            let request_account =
                crate::core::wechat_account::effective_account_id(&state_dir, &cfg.account_id);
            let req_val = json!({
                "id": req_id,
                "mode": mode,
                "account_id": request_account,
                "created_at": now_secs(),
            });
            let req_path = state_dir.join("contacts-sync-request.json");
            let res_path = state_dir.join("contacts-sync-result.json");

            if let Err(e) = write_private(&req_path, &req_val) {
                let _ = writeln!(stderr, "failed to write request file: {e:#}");
                return 1;
            }

            if let Some(wait_sec) = wait_seconds {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_sec);
                loop {
                    if res_path.exists() {
                        if let Ok(content) = fs::read_to_string(&res_path) {
                            if let Ok(res_obj) = serde_json::from_str::<Value>(&content) {
                                if res_obj.get("id").and_then(|v| v.as_str()) == Some(&req_id)
                                    && res_obj.get("account_id").and_then(Value::as_str)
                                        == Some(request_account.as_str())
                                {
                                    let _ = writeln!(
                                        stdout,
                                        "{}",
                                        serde_json::to_string_pretty(&res_obj).unwrap_or_default()
                                    );
                                    return 0;
                                }
                            }
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = writeln!(stderr, "contacts-sync wait timed out after {wait_sec}s");
                        return 1;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            } else {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"status": "queued", "id": req_id, "mode": mode})
                );
                0
            }
        }
        action if action.starts_with("brain-") => {
            execute_brain_control(action, args, stdout, stderr, default_config_path)
        }
        other => {
            let _ = writeln!(stderr, "unknown control action: {}", other);
            2
        }
    }
}

fn emit_brain_error<W: std::io::Write>(
    stdout: &mut W,
    code: &str,
    message: &str,
    retryable: bool,
    details: Option<Value>,
    exit_code: i32,
) -> i32 {
    let err = json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message,
            "retryable": retryable,
            "details": details,
        }
    });
    let _ = writeln!(stdout, "{err}");
    exit_code
}

pub(crate) struct ControlContactAccess {
    pub(crate) account_id: String,
    pub(crate) brain_access: Option<crate::core::open_api::access::BrainAccess>,
    pub(crate) conn: Option<rusqlite::Connection>,
}

impl crate::core::brain_profiles::ContactAccess for ControlContactAccess {
    fn permits(
        &self,
        account: &str,
        contact: &str,
        _kind: &crate::core::brain_profiles::ChatKind,
    ) -> bool {
        if account != self.account_id {
            return false;
        }
        if let Some(ba) = &self.brain_access {
            ba.permit(contact)
        } else {
            true
        }
    }
    fn family(&self, account: &str, contact: &str) -> Vec<String> {
        if account != self.account_id {
            return vec![];
        }
        if let Some(conn) = &self.conn {
            if let Ok(mut stmt) = conn.prepare("SELECT contact_id FROM contacts WHERE account_id=? AND (contact_id=? OR merged_into=?)") {
                if let Ok(rows) = stmt.query_map(rusqlite::params![account, contact, contact], |r| r.get::<_, String>(0)) {
                    let list: Vec<String> = rows.filter_map(Result::ok).collect();
                    if !list.is_empty() {
                        return list;
                    }
                }
            }
        }
        vec![contact.to_string()]
    }
}

fn handle_store_error<W: std::io::Write>(
    stdout: &mut W,
    err: crate::core::brain_profiles::Error,
    store: &crate::core::brain_profiles::Store,
    ctx: &crate::core::brain_profiles::AccountContext,
) -> i32 {
    let code = err.0;
    let (retryable, exit_code) = if code == "store_unavailable" {
        (true, 1)
    } else {
        (false, 2)
    };
    let details = if code == "revision_conflict" {
        let cur_gen = store.generation().unwrap_or(0);
        let (_, _, acc) = store.list(ctx).unwrap_or((0, vec![], None));
        Some(json!({
            "current_generation": cur_gen,
            "active": acc.as_ref().and_then(|a| a.active.default.as_ref()).map(|d| &d.role_id),
            "draft": acc.as_ref().and_then(|a| a.draft.default.as_ref()).map(|d| &d.role_id),
        }))
    } else {
        None
    };
    emit_brain_error(stdout, code, code, retryable, details, exit_code)
}

fn execute_write<W: std::io::Write>(
    stdout: &mut W,
    store: &crate::core::brain_profiles::Store,
    ctx: &crate::core::brain_profiles::AccountContext,
    access: &impl crate::core::brain_profiles::ContactAccess,
    expected_generation: u64,
    request_id: String,
    mutation: crate::core::brain_profiles::Mutation,
) -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let write_req = crate::core::brain_profiles::WriteRequest {
        expected_generation,
        request_id,
        mutation,
    };
    match store.write(ctx, &write_req, access, now) {
        Ok(res) => {
            let _ = writeln!(stdout, "{}", json!({ "ok": true, "data": res }));
            0
        }
        Err(e) => handle_store_error(stdout, e, store, ctx),
    }
}

fn execute_brain_control<W1: std::io::Write, W2: std::io::Write>(
    action: &str,
    args: &[String],
    stdout: &mut W1,
    _stderr: &mut W2,
    default_config_path: PathBuf,
) -> i32 {
    use crate::core::brain_profiles::*;
    let mut config_file: Option<PathBuf> = None;
    let mut request_json: Option<String> = None;
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "--config" && idx + 1 < args.len() {
            config_file = Some(PathBuf::from(&args[idx + 1]));
            idx += 2;
        } else if args[idx] == "--request-json" && idx + 1 < args.len() {
            request_json = Some(args[idx + 1].clone());
            idx += 2;
        } else {
            idx += 1;
        }
    }
    let config_path = config_file.unwrap_or(default_config_path);
    let cfg = match BotConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return emit_brain_error(
                stdout,
                "invalid_arguments",
                &format!("config error: {e:#}"),
                false,
                None,
                2,
            );
        }
    };

    let req_val: Value = if let Some(s) = request_json {
        match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => {
                return emit_brain_error(
                    stdout,
                    "invalid_arguments",
                    &format!("invalid request json: {e:#}"),
                    false,
                    None,
                    2,
                );
            }
        }
    } else {
        Value::Null
    };

    if let Some(req_acc) = req_val.get("account_id").and_then(Value::as_str) {
        if req_acc != cfg.account_id {
            return emit_brain_error(stdout, "not_found", "account mismatch", false, None, 2);
        }
    }

    let store = match Store::open(&cfg.state_dir) {
        Ok(s) => s,
        Err(e) => {
            return emit_brain_error(stdout, "store_unavailable", &e.to_string(), true, None, 1);
        }
    };

    let ctx = match AccountContext::new(cfg.account_id.clone()) {
        Ok(c) => c,
        Err(e) => {
            return emit_brain_error(stdout, "invalid_arguments", &e.to_string(), false, None, 2);
        }
    };

    let db_path = cfg.state_dir.join("history/history.sqlite3");
    let (conn, brain_access) = if db_path.exists() {
        if let Ok(c) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        ) {
            let ba = crate::core::open_api::access::BrainAccess::load(&cfg, &c).ok();
            (Some(c), ba)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    let access = ControlContactAccess {
        account_id: cfg.account_id.clone(),
        brain_access,
        conn,
    };

    match action {
        "brain-list" => {
            let (generation, roles, account_entry) = match store.list(&ctx) {
                Ok(res) => res,
                Err(e) => return handle_store_error(stdout, e, &store, &ctx),
            };
            let data = json!({
                "generation": generation,
                "roles": roles,
                "active": account_entry.as_ref().map(|a| &a.active),
                "draft": account_entry.as_ref().map(|a| &a.draft),
                "model_summary": {
                    "mode": format!("{:?}", cfg.brain.mode).to_lowercase(),
                    "timeout_seconds": cfg.brain.external.timeout_seconds,
                }
            });
            let _ = writeln!(stdout, "{}", json!({ "ok": true, "data": data }));
            0
        }
        "brain-get" => {
            let role_id = match req_val.get("role_id").and_then(Value::as_str) {
                Some(id) => id,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing role_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let revision_id = match req_val.get("revision_id").and_then(Value::as_str) {
                Some(r) => r.to_string(),
                None => {
                    let (_, roles, _) = match store.list(&ctx) {
                        Ok(l) => l,
                        Err(e) => return handle_store_error(stdout, e, &store, &ctx),
                    };
                    match roles.into_iter().find(|r| r.role_id == role_id) {
                        Some(e) => e.draft_revision_id,
                        None => {
                            return emit_brain_error(
                                stdout,
                                "not_found",
                                "role not found",
                                false,
                                None,
                                2,
                            );
                        }
                    }
                }
            };
            let role = match store.role(
                &ctx,
                &RoleRef {
                    role_id: role_id.into(),
                    revision_id,
                },
            ) {
                Ok(r) => r,
                Err(e) => return handle_store_error(stdout, e, &store, &ctx),
            };
            let _ = writeln!(stdout, "{}", json!({ "ok": true, "data": role }));
            0
        }
        "brain-save-role" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let mut role: RoleRevision =
                match serde_json::from_value(req_val.get("role").cloned().unwrap_or(Value::Null)) {
                    Ok(r) => r,
                    Err(e) => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            &e.to_string(),
                            false,
                            None,
                            2,
                        );
                    }
                };
            if role.owner_account_id.is_empty() {
                role.owner_account_id = ctx.account_id().into();
            } else if role.owner_account_id != ctx.account_id() {
                return emit_brain_error(stdout, "not_found", "owner mismatch", false, None, 2);
            }
            if role.role_id.is_empty() {
                role.role_id = format!("role_{}", uuid::Uuid::new_v4().simple());
            }
            if role.revision_id.is_empty() {
                role.revision_id = format!("rev_{}", uuid::Uuid::new_v4().simple());
            }
            let sealed = match role.seal() {
                Ok(s) => s,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        &e.to_string(),
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::SaveRole(sealed);
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-save-knowledge" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let mut knowledge: KnowledgeRevision = match serde_json::from_value(
                req_val.get("knowledge").cloned().unwrap_or(Value::Null),
            ) {
                Ok(k) => k,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        &e.to_string(),
                        false,
                        None,
                        2,
                    );
                }
            };
            if knowledge.owner_account_id.is_empty() {
                knowledge.owner_account_id = ctx.account_id().into();
            } else if knowledge.owner_account_id != ctx.account_id() {
                return emit_brain_error(stdout, "not_found", "owner mismatch", false, None, 2);
            }
            if knowledge.knowledge_id.is_empty() {
                knowledge.knowledge_id = format!("kn_{}", uuid::Uuid::new_v4().simple());
            }
            if knowledge.revision_id.is_empty() {
                knowledge.revision_id = format!("rev_{}", uuid::Uuid::new_v4().simple());
            }
            let sealed = match knowledge.seal() {
                Ok(s) => s,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        &e.to_string(),
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::SaveKnowledge(sealed);
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-copy-role" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let role_id = match req_val.get("role_id").and_then(Value::as_str) {
                Some(id) => id,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing role_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let revision_id = match req_val.get("revision_id").and_then(Value::as_str) {
                Some(r) => r,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing revision_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let new_role_id = req_val
                .get("new_role_id")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("role_{}", uuid::Uuid::new_v4().simple()));
            let mutation = Mutation::CopyRole {
                source: RoleRef {
                    role_id: role_id.into(),
                    revision_id: revision_id.into(),
                },
                new_role_id,
            };
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-archive-role" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let role_id = match req_val.get("role_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing role_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::ArchiveRole { role_id };
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-save-bindings" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let draft: BindingSet = match serde_json::from_value(
                req_val.get("draft").cloned().unwrap_or(Value::Null),
            ) {
                Ok(b) => b,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        &e.to_string(),
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::SaveBindings(draft);
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-activate" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::Activate;
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-rollback" => {
            let expected_generation =
                match req_val.get("expected_generation").and_then(Value::as_u64) {
                    Some(g) => g,
                    None => {
                        return emit_brain_error(
                            stdout,
                            "invalid_arguments",
                            "missing expected_generation",
                            false,
                            None,
                            2,
                        );
                    }
                };
            let request_id = match req_val.get("request_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing request_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let target_generation = match req_val.get("target_generation").and_then(Value::as_u64) {
                Some(tg) => tg,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing target_generation",
                        false,
                        None,
                        2,
                    );
                }
            };
            let mutation = Mutation::Rollback {
                generation: target_generation,
            };
            execute_write(
                stdout,
                &store,
                &ctx,
                &access,
                expected_generation,
                request_id,
                mutation,
            )
        }
        "brain-preview" => {
            let role_id = match req_val.get("role_id").and_then(Value::as_str) {
                Some(id) => id,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing role_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let revision_id = match req_val.get("revision_id").and_then(Value::as_str) {
                Some(r) => r.to_string(),
                None => {
                    let (_, roles, _) = match store.list(&ctx) {
                        Ok(l) => l,
                        Err(e) => return handle_store_error(stdout, e, &store, &ctx),
                    };
                    match roles.into_iter().find(|r| r.role_id == role_id) {
                        Some(e) => e.draft_revision_id,
                        None => {
                            return emit_brain_error(
                                stdout,
                                "not_found",
                                "role not found",
                                false,
                                None,
                                2,
                            );
                        }
                    }
                }
            };
            let execution = req_val
                .get("execution")
                .and_then(Value::as_str)
                .unwrap_or("assemble_only");
            let cost_consent = req_val
                .get("cost_consent")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let input: crate::brain::snapshot::SyntheticInput = match serde_json::from_value(
                req_val.get("input").cloned().unwrap_or(Value::Null),
            ) {
                Ok(i) => i,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        &e.to_string(),
                        false,
                        None,
                        2,
                    );
                }
            };
            let role = match store.role(
                &ctx,
                &RoleRef {
                    role_id: role_id.into(),
                    revision_id,
                },
            ) {
                Ok(r) => r,
                Err(e) => return handle_store_error(stdout, e, &store, &ctx),
            };
            let mut knowledge = Vec::new();
            for kref in &role.knowledge_refs {
                match store.knowledge(&ctx, kref) {
                    Ok(k) => knowledge.push(k),
                    Err(e) => return handle_store_error(stdout, e, &store, &ctx),
                }
            }
            match crate::brain::snapshot::preview(
                &role,
                &knowledge,
                ctx.account_id(),
                input,
                execution,
                cost_consent,
            ) {
                Ok(p) => {
                    let _ = writeln!(stdout, "{}", json!({ "ok": true, "data": p }));
                    0
                }
                Err(e) => {
                    let msg = e.to_string();
                    let code = if msg == "cost_consent_required" {
                        "cost_consent_required"
                    } else if msg == "preview_execution_disabled" {
                        "preview_execution_disabled"
                    } else if msg == "context_budget_exceeded" {
                        "context_budget_exceeded"
                    } else if msg == "not_found" {
                        "not_found"
                    } else {
                        "invalid_arguments"
                    };
                    emit_brain_error(stdout, code, &msg, false, None, 2)
                }
            }
        }
        "brain-turn-detail" => {
            let turn_id = match req_val.get("turn_id").and_then(Value::as_str) {
                Some(id) => id,
                None => {
                    return emit_brain_error(
                        stdout,
                        "invalid_arguments",
                        "missing turn_id",
                        false,
                        None,
                        2,
                    );
                }
            };
            let hstore = match crate::core::history::store::HistoryStore::open(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    return emit_brain_error(
                        stdout,
                        "store_unavailable",
                        &e.to_string(),
                        true,
                        None,
                        1,
                    );
                }
            };
            match hstore.get_brain_turn_detail(&cfg.account_id, turn_id) {
                Ok(Some(detail)) => {
                    let _ = writeln!(stdout, "{}", json!({ "ok": true, "data": detail }));
                    0
                }
                Ok(None) => emit_brain_error(stdout, "not_found", "turn not found", false, None, 2),
                Err(e) => {
                    emit_brain_error(stdout, "store_unavailable", &e.to_string(), true, None, 1)
                }
            }
        }
        other => emit_brain_error(
            stdout,
            "invalid_arguments",
            &format!("unknown brain action: {other}"),
            false,
            None,
            2,
        ),
    }
}

#[derive(Debug, PartialEq)]
pub enum ResumeOutcome {
    /// The running core received resume_session over core.sock.
    Core,
    /// No core: this many handoff.json markers naming the recipient were removed.
    MarkerRemoved(usize),
}

/// Ends a person's handoff. A running core owns the handoff state, so it is told over its socket;
/// only when no core is listening are the marker files (sessions/*/handoff.json) removed directly.
pub fn resume_session_with(
    socket: &Path,
    state_dir: &Path,
    recipient: &str,
) -> Result<ResumeOutcome> {
    use std::io::{BufRead, BufReader, Write as _};
    match std::os::unix::net::UnixStream::connect(socket) {
        Ok(stream) => {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            let mut writer = stream.try_clone()?;
            writeln!(
                writer,
                "{}",
                json!({"id": "control-resume", "op": "resume_session", "recipient": recipient})
            )?;
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line)?;
            let response: Value =
                serde_json::from_str(line.trim()).context("core.sock 返回了无效 JSON")?;
            if response.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok(ResumeOutcome::Core)
            } else {
                anyhow::bail!(
                    "core 拒绝 resume_session: {}",
                    response.get("error").cloned().unwrap_or(Value::Null)
                )
            }
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            let mut removed = 0;
            if let Ok(entries) = fs::read_dir(state_dir.join("sessions")) {
                for entry in entries.flatten() {
                    let marker = entry.path().join("handoff.json");
                    let Ok(text) = fs::read_to_string(&marker) else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if value.get("recipient").and_then(Value::as_str) == Some(recipient)
                        && fs::remove_file(&marker).is_ok()
                    {
                        removed += 1;
                    }
                }
            }
            Ok(ResumeOutcome::MarkerRemoved(removed))
        }
        Err(error) => Err(error).context("连接 core.sock 失败"),
    }
}

pub fn run_control(args: &[String]) -> Result<i32> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    Ok(execute_control(args, &mut stdout, &mut stderr, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("test-control-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
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

    fn write_test_config(dir: &Path) -> (PathBuf, PathBuf) {
        let state_dir = dir.join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "test_user", "kind": "private"}],
            "state_dir": state_dir.to_string_lossy().to_string(),
            "poll_seconds": 2,
            "cooldown_seconds": 15,
            "max_replies_per_hour": 20,
        });
        let cfg_path = dir.join("config.json");
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        (cfg_path, state_dir)
    }

    #[test]
    fn test_config_check_valid_and_invalid() {
        let dir = TempDir::new();
        let (cfg_path, _) = write_test_config(dir.path());

        // 1. Valid config
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "config-check".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out, json!({"ok": true}));

        // 2. Invalid config
        let bad_path = dir.path().join("bad.json");
        fs::write(&bad_path, r#"{"schema_version": 999}"#).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "config-check".into(),
                bad_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 2);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["ok"], false);
        assert!(
            out["error"]
                .as_str()
                .unwrap()
                .contains("unsupported bot config schema or field")
        );
    }

    #[test]
    fn test_pause_and_resume() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        let pause_file = state_dir.join("admin-pause.json");
        assert!(!pause_file.exists());

        // Pause
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "pause".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 0);
        assert!(pause_file.exists());
        let mode = fs::metadata(&pause_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["status"], "paused");
        assert_eq!(out["file"], pause_file.to_string_lossy().to_string());

        // Resume
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "resume".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 0);
        assert!(!pause_file.exists());
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out, json!({"status": "resumed"}));
    }

    #[test]
    fn test_status_empty_and_with_batches() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // No health.json, no send batches
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "status".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["launchd_loaded"], false);
        assert_eq!(out["health"], Value::Null);
        assert_eq!(out["needs_recalibration"], false);
        assert_eq!(out["stats"]["recorded_batches"], 0);

        // Add health and batch files
        fs::write(
            state_dir.join("health.json"),
            json!({"state": "running", "pid": 1234}).to_string(),
        )
        .unwrap();

        let batch1 = json!({
            "status": "triggered",
            "client_echo_observed": true,
        });
        let batch2 = json!({
            "status": "generating",
            "client_echo_observed": false,
        });
        fs::write(state_dir.join("send-1.json"), batch1.to_string()).unwrap();
        fs::write(state_dir.join("send-2.json"), batch2.to_string()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "status".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["launchd_loaded"], true);
        assert_eq!(out["health"]["state"], "running");
        assert_eq!(out["stats"]["recorded_batches"], 2);
        assert_eq!(out["stats"]["triggered"], 1);
        assert_eq!(out["stats"]["client_echo_observed"], 1);
        assert_eq!(out["stats"]["needs_review"], 1);
    }

    #[test]
    fn test_status_needs_recalibration_triggers_stderr_warning() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        fs::write(
            state_dir.join("health.json"),
            json!({"state": "needs_recalibration"}).to_string(),
        )
        .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "status".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        assert_eq!(code, 0);
        let stderr_str = String::from_utf8(stderr).unwrap();
        assert!(stderr_str.contains("界面核对连续失败，请运行 control calibrate 重新定位"));
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["needs_recalibration"], true);
    }

    #[test]
    fn test_calibrate_not_running_fails() {
        let dir = TempDir::new();
        let (cfg_path, _) = write_test_config(dir.path());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 1);
        let stderr_str = String::from_utf8(stderr).unwrap();
        assert_eq!(stderr_str, "机器人未运行：请先 control start\n");
    }

    #[test]
    fn test_calibrate_diffs_and_accept_flow() {
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 模拟后台线程响应 calibrate 请求
        let state_dir_clone = state_dir.clone();
        let thread_handle = std::thread::spawn(move || {
            let req_file = state_dir_clone.join("calibrate-request.json");
            let res_file = state_dir_clone.join("calibrate-result.json");
            for _ in 0..100 {
                if req_file.exists() {
                    if let Ok(content) = fs::read_to_string(&req_file) {
                        if let Ok(req) = serde_json::from_str::<Value>(&content) {
                            let id = req["id"].as_str().unwrap();
                            let action = req["action"].as_str().unwrap();
                            if action == "calibrate" {
                                let res = json!({
                                    "id": id,
                                    "ok": true,
                                    "result": {
                                        "differences": [{"field": "input.y", "old": 400.0, "new": 420.0}],
                                        "missing": ["right_line"]
                                    }
                                });
                                fs::write(&res_file, res.to_string()).unwrap();
                                return;
                            } else if action == "accept" {
                                let res = json!({
                                    "id": id,
                                    "ok": true,
                                    "result": {
                                        "accepted": true,
                                        "version": "4.1.15"
                                    }
                                });
                                fs::write(&res_file, res.to_string()).unwrap();
                                return;
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        thread_handle.join().unwrap();
        assert_eq!(code, 0);
        let stdout_str = String::from_utf8(stdout).unwrap();
        assert!(stdout_str.contains("Differences from current layout:\n  input.y: 400.0 -> 420.0"));
        assert!(
            stdout_str.contains("Missing measurements (preserved original values):\n  right_line")
        );

        // 再测试 accept
        let state_dir_clone = state_dir.clone();
        let thread_handle2 = std::thread::spawn(move || {
            let req_file = state_dir_clone.join("calibrate-request.json");
            let res_file = state_dir_clone.join("calibrate-result.json");
            for _ in 0..100 {
                if req_file.exists() {
                    if let Ok(content) = fs::read_to_string(&req_file) {
                        if let Ok(req) = serde_json::from_str::<Value>(&content) {
                            let id = req["id"].as_str().unwrap();
                            let action = req["action"].as_str().unwrap();
                            if action == "accept" {
                                let res = json!({
                                    "id": id,
                                    "ok": true,
                                    "result": {
                                        "accepted": true,
                                        "version": "4.1.15"
                                    }
                                });
                                fs::write(&res_file, res.to_string()).unwrap();
                                return;
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--accept".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        thread_handle2.join().unwrap();
        assert_eq!(code, 0);
        let stdout_str = String::from_utf8(stdout).unwrap();
        assert!(stdout_str.contains("\"accepted\":true"));
    }

    #[test]
    fn test_python_parity_golden_values() {
        let _lock = crate::core::env_lock();
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 1. config-check 对比
        // Captured from the Python implementation on 2026-11-09.
        let mut rust_stdout = Vec::new();
        let mut rust_stderr = Vec::new();
        let rust_code = execute_control(
            &[
                "config-check".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut rust_stdout,
            &mut rust_stderr,
            Some(false),
        );
        assert_eq!(rust_code, 0);
        let py_json = json!({"ok": true});
        let rust_json: Value = serde_json::from_slice(&rust_stdout).unwrap();
        assert_eq!(rust_json, py_json);

        // 2. pause 对比
        let pause_file = state_dir.join("admin-pause.json");
        let mut rust_pause_stdout = Vec::new();
        let mut rust_pause_stderr = Vec::new();
        let rust_pause_code = execute_control(
            &[
                "pause".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut rust_pause_stdout,
            &mut rust_pause_stderr,
            Some(false),
        );
        assert_eq!(rust_pause_code, 0);
        let rust_pause_json: Value = serde_json::from_slice(&rust_pause_stdout).unwrap();
        assert_eq!(rust_pause_json["status"], "paused");
        assert_eq!(
            rust_pause_json["file"],
            pause_file.to_string_lossy().to_string()
        );

        // 3. resume 对比
        let py_resume_json = json!({"status": "resumed"});

        // 重新 pause 后由 Rust resume 并比对
        fs::write(&pause_file, r#"{"paused": true}"#).unwrap();
        let mut rust_resume_stdout = Vec::new();
        let mut rust_resume_stderr = Vec::new();
        let rust_resume_code = execute_control(
            &[
                "resume".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut rust_resume_stdout,
            &mut rust_resume_stderr,
            Some(false),
        );
        assert_eq!(rust_resume_code, 0);
        let rust_resume_json: Value = serde_json::from_slice(&rust_resume_stdout).unwrap();
        assert_eq!(rust_resume_json, py_resume_json);

        // 4. calibrate (not started) 对比
        let mut rust_cal_stdout = Vec::new();
        let mut rust_cal_stderr = Vec::new();
        let rust_cal_code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_string_lossy().to_string(),
            ],
            &mut rust_cal_stdout,
            &mut rust_cal_stderr,
            Some(false),
        );
        assert_eq!(rust_cal_code, 1);
        assert_eq!(
            String::from_utf8(rust_cal_stderr).unwrap(),
            "机器人未运行：请先 control start\n"
        );
    }

    #[test]
    fn test_resume_session_tells_running_core_over_socket() {
        use std::io::{BufRead, BufReader, Write as _};
        let td = TempDir::new();
        // Unix socket paths are limited to ~104 bytes; the temp dir path is too long.
        let sock = PathBuf::from(format!(
            "/tmp/wcc-{}.sock",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let mut out = stream;
            writeln!(
                out,
                "{}",
                json!({"id": request["id"], "ok": true, "result": {"resumed": true}})
            )
            .unwrap();
            request
        });
        let state_dir = td.path().join("state");
        let handoff = state_dir.join("sessions").join("c_1").join("handoff.json");
        fs::create_dir_all(handoff.parent().unwrap()).unwrap();
        fs::write(&handoff, r#"{"recipient":"测试群"}"#).unwrap();

        let outcome = resume_session_with(&sock, &state_dir, "测试群").unwrap();
        let request = server.join().unwrap();
        let _ = fs::remove_file(&sock);
        assert_eq!(outcome, ResumeOutcome::Core);
        assert_eq!(request["op"], "resume_session");
        assert_eq!(request["recipient"], "测试群");
        assert!(
            handoff.exists(),
            "the running core owns the marker; control must not delete it behind its back"
        );
    }

    #[test]
    fn test_resume_session_without_core_removes_only_matching_marker() {
        let td = TempDir::new();
        let state_dir = td.path().join("state");
        let mine = state_dir.join("sessions").join("c_a").join("handoff.json");
        let other = state_dir.join("sessions").join("c_b").join("handoff.json");
        for (path, name) in [(&mine, "测试群"), (&other, "午夜咖啡")] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json!({"recipient": name}).to_string()).unwrap();
        }
        let outcome =
            resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "测试群")
                .unwrap();
        assert_eq!(outcome, ResumeOutcome::MarkerRemoved(1));
        assert!(!mine.exists());
        assert!(other.exists());
        let none =
            resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "测试群")
                .unwrap();
        assert_eq!(none, ResumeOutcome::MarkerRemoved(0));
    }

    #[test]
    fn test_control_resume_session_removes_handoff_and_validates() {
        let _lock = crate::core::env_lock();
        let td = TempDir::new();
        let state_dir = td.path().join("state");
        let session_dir = state_dir.join("sessions").join("test_vip");
        fs::create_dir_all(&session_dir).unwrap();

        let handoff_file = session_dir.join("handoff.json");
        // The marker names its recipient (written by the core on handoff); no core is listening here.
        fs::write(
            &handoff_file,
            json!({"recipient": "test_vip", "turn_id": "t", "at": 1.0}).to_string(),
        )
        .unwrap();
        unsafe { std::env::set_var("WECHAT_CORE_SOCK", "/tmp/wcc-absent-control-test.sock") };
        assert!(handoff_file.exists());

        let cfg_path = td.path().join("config.json");
        fs::write(
            &cfg_path,
            json!({
                "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
                "account_id": "test",
                "state_dir": state_dir.to_str().unwrap()
            })
            .to_string(),
        )
        .unwrap();

        // 1. 缺少 session 参数 -> 退出码 2
        let mut err_out = Vec::new();
        let mut err_err = Vec::new();
        let code_missing = execute_control(
            &[
                "resume-session".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut err_out,
            &mut err_err,
            Some(false),
        );
        assert_eq!(code_missing, 2);

        // 2. 正常指定 session -> 删除 handoff.json 并返回 ok
        let mut succ_out = Vec::new();
        let mut succ_err = Vec::new();
        let code_succ = execute_control(
            &[
                "resume-session".into(),
                "test_vip".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut succ_out,
            &mut succ_err,
            Some(false),
        );
        assert_eq!(code_succ, 0);
        let res: Value = serde_json::from_slice(&succ_out).unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["session"], "test_vip");
        assert_eq!(res["status"], "resumed");
        assert_eq!(res["file_removed"], true);
        assert!(!handoff_file.exists());
    }

    // -------------------------------------------------------------
    // tests/test_calibrate.py 20 项缺口迁移
    // -------------------------------------------------------------

    use std::time::Duration;

    fn load_default_layout() -> Value {
        let layout_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("native")
            .join("layout")
            .join("wechat-4.1.15.json");
        let content = fs::read_to_string(&layout_path).expect("failed to read default layout");
        serde_json::from_str(&content).expect("invalid layout json")
    }

    /// 1. test_no_4_1_15_fallback_in_source
    /// 检查生产源码中不包含硬编码的 4.1.15 fallback，且 Worker.swift 中也没有
    #[test]
    fn test_no_4_1_15_fallback_in_source() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let control_source = fs::read_to_string(manifest_dir.join("src/core/control.rs")).unwrap();
        let worker_source =
            fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let prod_control = control_source.split("#[cfg(test)]").next().unwrap();
        assert!(
            !prod_control.contains("\"4.1.15\""),
            "control.rs 生产代码不得硬编码 '4.1.15' fallback"
        );
        assert!(
            !worker_source.contains("\"4.1.15\""),
            "Worker.swift 不得硬编码 '4.1.15' fallback"
        );
    }

    /// 2. test_version_regex_validation
    /// 版本必须符合正则 ^[0-9]+(\\.[0-9]+){1,3}$
    #[test]
    fn test_version_regex_validation() {
        let valid_versions = ["4.1.15", "4.0.0", "1.0", "1.2.3.4"];
        let invalid_versions = [
            "4.1.15;rm -rf /",
            "v4.1",
            "beta",
            "",
            "4.",
            ".1",
            "4.1.15.1.2",
        ];

        for v in valid_versions {
            assert!(validate_version_string(v), "Expected {} to be valid", v);
        }
        for inv in invalid_versions {
            assert!(
                !validate_version_string(inv),
                "Expected {} to be invalid",
                inv
            );
        }
    }

    /// 3. test_worker_missing_version_aborts_without_writing
    /// worker 响应缺少版本或 layout 时，run_calibrate 拒绝且不写任何文件
    #[test]
    fn test_worker_missing_version_aborts_without_writing() {
        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let bad_worker = json!({
            "measured": {"input_divider": 490.0}
        });

        let res = run_calibrate(
            None,
            Some(&bad_worker),
            Some("wechat-host"),
            Some("~/.wechat-rpa/layout"),
            Some(&mut fake_runner),
        );

        assert!(res.is_err(), "缺少版本或 layout 时应报错中止");
        let write_cmds: Vec<_> = commands.iter().filter(|c| c.contains("cat > ")).collect();
        assert!(write_cmds.is_empty(), "未写入任何文件");
    }

    /// 4. test_worker_calibrate_response_fields_in_swift
    /// Worker.swift calibrate 操作必须返回 wechat_version, layout_path, layout 等字段
    #[test]
    fn test_worker_calibrate_response_fields_in_swift() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let worker_swift =
            fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift
            .find("operation == \"calibrate\"")
            .expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch
            .find("guard let recipient")
            .expect("must bound calibrate branch");
        let cal_block = &branch[..end_idx];

        assert!(cal_block.contains("\"wechat_version\""));
        assert!(cal_block.contains("\"layout_path\""));
        assert!(cal_block.contains("\"layout\""));
        assert!(cal_block.contains("layout_missing"));
    }

    /// 5. test_candidate_written_to_remote_wechat_host
    /// 候选布局和 PNG 写入对应布局目录，设置 umask 077 和权限 0600
    /// 注：Rust 在本机运行，断言对本地文件系统路径等价生效，同时对 runner 命令进行相同校验
    #[test]
    fn test_candidate_written_to_remote_wechat_host() {
        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let mut written_files = std::collections::HashMap::new();
        let mut recorded_commands = Vec::new();

        let mut fake_runner = |host: &str, cmd: &str, input: Option<&[u8]>| {
            recorded_commands.push((host.to_string(), cmd.to_string()));
            if cmd.contains("cat > ") {
                if let Some(pos) = cmd.find("cat > ") {
                    let rest = &cmd[pos + 6..];
                    let path = rest
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .trim_matches('\'')
                        .trim_matches('"');
                    if let Some(data) = input {
                        written_files.insert(path.to_string(), data.to_vec());
                    }
                }
            }
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let worker_res = json!({
            "wechat_version": version,
            "layout_path": format!("/path/to/.wechat-rpa/layout/wechat-{}.json", version),
            "layout": base_layout,
            "measured": {"input_divider": 490.0},
            "annotated_png_base64": "YW5ub3RhdGVkLWJ5dGVz" // base64 of b"annotated-bytes"
        });

        let (diffs, _missing, candidate) = run_calibrate(
            None,
            Some(&worker_res),
            Some("wechat-host"),
            Some("~/.wechat-rpa/layout"),
            Some(&mut fake_runner),
        )
        .unwrap();

        // 1. 派生字段验证
        assert_eq!(candidate["regions"]["input"]["y"], 490.0);
        assert!(
            diffs
                .iter()
                .any(|d| d["field"] == "input.y" && d["new"] == 490.0)
        );

        // 2. 写入文件路径验证
        let json_cand = format!("~/.wechat-rpa/layout/candidate-wechat-{}.json", version);
        let png_cand = format!("~/.wechat-rpa/layout/candidate-wechat-{}.png", version);
        assert!(
            written_files.contains_key(&json_cand),
            "应写入 candidate json"
        );
        assert!(
            written_files.contains_key(&png_cand),
            "应写入 candidate png"
        );
        assert_eq!(written_files[&png_cand], b"annotated-bytes");

        // 3. 校验权限与 umask
        for (host, cmd) in &recorded_commands {
            assert_eq!(host, "wechat-host");
            assert!(cmd.contains("umask 077"));
            assert!(cmd.contains("chmod 600"));
        }
    }

    /// 6. test_accept_refuses_version_mismatch_with_zero_remote_writes
    /// 版本不匹配时拒绝 accept，写操作次数为 0
    #[test]
    fn test_accept_refuses_version_mismatch_with_zero_remote_writes() {
        let base_layout = load_default_layout();
        let _version = base_layout["wechat_version"].as_str().unwrap();

        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            if cmd.starts_with("cat ") {
                let json_bytes = serde_json::to_vec(&base_layout).unwrap();
                return Ok(CommandOutput {
                    returncode: 0,
                    stdout: json_bytes,
                    stderr: vec![],
                });
            }
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let worker_res = json!({
            "wechat_version": "4.1.16",
            "layout": base_layout
        });

        let res = calibrate_accept(
            None,
            Some("wechat-host"),
            Some("~/.wechat-rpa/layout"),
            Some(&worker_res),
            Some(&mut fake_runner),
        );

        assert!(res.is_err(), "版本不匹配必须报错");
        let write_cmds: Vec<_> = commands
            .iter()
            .filter(|c| {
                c.contains("cp ") || c.contains("mv ") || c.contains("cat >") || c.contains("rm ")
            })
            .collect();
        assert!(write_cmds.is_empty(), "版本不匹配时不得执行任何写入命令");
    }

    /// 7. test_accept_refuses_corrupt_candidate_with_zero_remote_writes
    /// 损坏的 candidate JSON 拒绝 accept，写操作次数为 0
    #[test]
    fn test_accept_refuses_corrupt_candidate_with_zero_remote_writes() {
        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            if cmd.starts_with("cat ") {
                return Ok(CommandOutput {
                    returncode: 0,
                    stdout: b"{broken json".to_vec(),
                    stderr: vec![],
                });
            }
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let worker_res = json!({
            "wechat_version": version,
            "layout": base_layout
        });

        let res = calibrate_accept(
            None,
            Some("wechat-host"),
            Some("~/.wechat-rpa/layout"),
            Some(&worker_res),
            Some(&mut fake_runner),
        );

        assert!(res.is_err(), "损坏的 candidate JSON 必须报错");
        let write_cmds: Vec<_> = commands
            .iter()
            .filter(|c| {
                c.contains("cp ") || c.contains("mv ") || c.contains("cat >") || c.contains("rm ")
            })
            .collect();
        assert!(write_cmds.is_empty(), "损坏 JSON 时不得执行任何写入命令");
    }

    /// 8. test_accept_executes_backup_then_mv_atomically
    /// 单条原子命令：先备份当前激活文件，再将候选 mv 到激活文件，cp 先于 mv
    #[test]
    fn test_accept_executes_backup_then_mv_atomically() {
        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let mut updated_layout = base_layout.clone();
        updated_layout["regions"]["input"]["y"] = json!(492.0);
        updated_layout["regions"]["input"]["h"] = json!(148.0);
        updated_layout["regions"]["chat"]["h"] = json!(432.0);

        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            if cmd.starts_with("cat ") {
                let json_bytes = serde_json::to_vec(&updated_layout).unwrap();
                return Ok(CommandOutput {
                    returncode: 0,
                    stdout: json_bytes,
                    stderr: vec![],
                });
            }
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let worker_res = json!({
            "wechat_version": version,
            "layout": base_layout
        });

        let res = calibrate_accept(
            None,
            Some("wechat-host"),
            Some("~/.wechat-rpa/layout"),
            Some(&worker_res),
            Some(&mut fake_runner),
        )
        .unwrap();

        assert_eq!(res["accepted"], true);

        let atomic_cmds: Vec<_> = commands
            .iter()
            .filter(|c| c.contains("cp ") && c.contains("mv "))
            .collect();
        assert_eq!(atomic_cmds.len(), 1, "必须包含恰好一条原子 cp+mv 命令");

        let cmd = &atomic_cmds[0];
        let cp_pos = cmd.find("cp ").unwrap();
        let mv_pos = cmd.find("mv ").unwrap();
        assert!(cp_pos < mv_pos, "cp 必须先于 mv 执行");
        assert!(cmd.contains(".bak-"), "必须包含 .bak- 备份标记");
        assert!(cmd.contains("chmod 600"), "必须包含 chmod 600");
    }

    /// 9. test_cli_calibrate_command
    /// 命令行 control calibrate 正确委托给运行中的机器人并输出差异
    #[test]
    fn test_cli_calibrate_command() {
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg_path = env.path().join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
            "host": "local",
            "account_id": "test",
            "state_dir": state_dir.to_str().unwrap()
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();

        let state_dir_clone = state_dir.clone();
        let handle = std::thread::spawn(move || {
            let req_file = state_dir_clone.join("calibrate-request.json");
            let res_file = state_dir_clone.join("calibrate-result.json");
            for _ in 0..100 {
                if req_file.exists() {
                    if let Ok(c) = fs::read_to_string(&req_file) {
                        if let Ok(req) = serde_json::from_str::<Value>(&c) {
                            let id = req["id"].as_str().unwrap();
                            let res = json!({
                                "id": id,
                                "ok": true,
                                "result": {
                                    "candidate_json": "~/.wechat-rpa/layout/candidate-wechat-4.1.15.json",
                                    "candidate_png": "~/.wechat-rpa/layout/candidate-wechat-4.1.15.png",
                                    "differences": [
                                        {"field": "input.y", "old": 496.0, "new": 500.0},
                                        {"field": "input.h", "old": 144.0, "new": 140.0},
                                        {"field": "chat.h", "old": 436.0, "new": 440.0},
                                        {"field": "session_list.w", "old": 250.0, "new": 255.0}
                                    ],
                                    "missing": []
                                }
                            });
                            fs::write(&res_file, res.to_string()).unwrap();
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        handle.join().unwrap();
        assert_eq!(code, 0);

        let out_str = String::from_utf8(stdout).unwrap();
        let last_line = out_str.lines().last().unwrap();
        let res: Value = serde_json::from_str(last_line).unwrap();
        assert!(
            res["candidate_json"]
                .as_str()
                .unwrap()
                .contains("candidate-wechat-4.1.15.json")
        );
        assert_eq!(res["differences"].as_array().unwrap().len(), 4);
    }

    /// 10. test_cli_calibrate_accept
    /// 命令行 control calibrate --accept 正确返回 accepted
    #[test]
    fn test_cli_calibrate_accept() {
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg_path = env.path().join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
            "host": "local",
            "account_id": "test",
            "state_dir": state_dir.to_str().unwrap()
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();

        let state_dir_clone = state_dir.clone();
        let handle = std::thread::spawn(move || {
            let req_file = state_dir_clone.join("calibrate-request.json");
            let res_file = state_dir_clone.join("calibrate-result.json");
            for _ in 0..100 {
                if req_file.exists() {
                    if let Ok(c) = fs::read_to_string(&req_file) {
                        if let Ok(req) = serde_json::from_str::<Value>(&c) {
                            let id = req["id"].as_str().unwrap();
                            let res = json!({
                                "id": id,
                                "ok": true,
                                "result": {
                                    "accepted": true,
                                    "version": "4.1.15"
                                }
                            });
                            fs::write(&res_file, res.to_string()).unwrap();
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--accept".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        handle.join().unwrap();
        assert_eq!(code, 0);

        let out_str = String::from_utf8(stdout).unwrap();
        let last_line = out_str.lines().last().unwrap();
        let res: Value = serde_json::from_str(last_line).unwrap();
        assert_eq!(res["accepted"], true);
        assert_eq!(res["version"], "4.1.15");
    }

    /// 11. test_calibrate_never_clicks
    /// Worker.swift 中的 calibrate 分支绝不调用任何点击、打字或滚动动作
    #[test]
    fn test_calibrate_never_clicks() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let worker_swift =
            fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift
            .find("operation == \"calibrate\"")
            .expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch
            .find("guard let recipient")
            .expect("must bound calibrate branch");
        let cal_block = &branch[..end_idx];

        let forbidden = [
            "botActions",
            "click(",
            "rightClick(",
            "key(",
            "act(",
            "scroll(",
        ];
        for term in forbidden {
            assert!(!cal_block.contains(term), "calibrate 分支不得调用 {}", term);
        }
    }

    /// 12. test_default_paths_land_in_remote_home
    /// 默认路径写入 HOME 下的 .wechat-rpa/layout，且不会在根下生成字面量 '~' 目录
    /// 注：等价断言在本地环境测试，验证默认路径解析行为与 0600 权限
    #[test]
    fn test_default_paths_land_in_remote_home() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let home = env.path().to_path_buf();
        let layout_dir = home.join(".wechat-rpa").join("layout");
        fs::create_dir_all(&layout_dir).unwrap();

        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let worker_res = json!({
            "wechat_version": version,
            "layout_path": "x",
            "layout": base_layout,
            "measured": {"input_divider": 500.0, "list_border": 300.0},
            "annotated_png_base64": "cG5n"
        });

        let config = json!({"host": "wechat-mac"});

        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", &home) };

        let res = run_calibrate(Some(&config), Some(&worker_res), None, None, None);

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert!(res.is_ok());
        let cand_json = layout_dir.join(format!("candidate-wechat-{}.json", version));
        assert!(cand_json.exists(), "candidate 文件必须存在于 layout 目录");

        let cand_content: Value =
            serde_json::from_str(&fs::read_to_string(&cand_json).unwrap()).unwrap();
        assert_eq!(cand_content["regions"]["input"]["y"], 500.0);

        let mode = fs::metadata(&cand_json).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "candidate 文件模式必须为 0600");

        assert!(!home.join("~").exists(), "绝不能创建名为 ~ 的字面量目录");
    }

    /// 13. test_accept_with_defaults_backs_up_then_replaces
    /// 默认路径下 accept 完整流程：先备份已有文件为 .bak-*，然后更新激活布局
    /// 注：等价断言在本地路径下完整验证
    #[test]
    fn test_accept_with_defaults_backs_up_then_replaces() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let home = env.path().to_path_buf();
        let layout_dir = home.join(".wechat-rpa").join("layout");
        fs::create_dir_all(&layout_dir).unwrap();

        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let active_json = layout_dir.join(format!("wechat-{}.json", version));
        fs::write(
            &active_json,
            serde_json::to_string_pretty(&base_layout).unwrap(),
        )
        .unwrap();

        let worker_res = json!({
            "wechat_version": version,
            "layout_path": "x",
            "layout": base_layout,
            "measured": {"input_divider": 500.0, "list_border": 300.0},
            "annotated_png_base64": "cG5n"
        });
        let config = json!({"host": "wechat-mac"});

        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", &home) };

        let cal_res = run_calibrate(Some(&config), Some(&worker_res), None, None, None);
        assert!(cal_res.is_ok());

        let acc_res = calibrate_accept(Some(&config), None, None, Some(&worker_res), None);

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert!(acc_res.is_ok());
        let acc_val = acc_res.unwrap();
        assert_eq!(acc_val["accepted"], true);

        let new_active: Value =
            serde_json::from_str(&fs::read_to_string(&active_json).unwrap()).unwrap();
        assert_eq!(new_active["regions"]["input"]["y"], 500.0);

        let backups: Vec<_> = fs::read_dir(&layout_dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("wechat-{}.json.bak-", version))
            })
            .collect();
        assert_eq!(backups.len(), 1, "必须恰好产生 1 个备份");
        let backup_content: Value =
            serde_json::from_str(&fs::read_to_string(backups[0].path()).unwrap()).unwrap();
        assert_eq!(backup_content["regions"]["input"]["y"], 496.0);
    }

    /// 14. test_missing_host_is_refused
    /// 缺少 host 时 run_calibrate 必须拒绝并报错
    #[test]
    fn test_missing_host_is_refused() {
        let base_layout = load_default_layout();
        let worker_res = json!({
            "wechat_version": "4.1.15",
            "layout": base_layout
        });
        let config = json!({});

        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            Ok(CommandOutput {
                returncode: 0,
                stdout: vec![],
                stderr: vec![],
            })
        };

        let res = run_calibrate(
            Some(&config),
            Some(&worker_res),
            None,
            None,
            Some(&mut fake_runner),
        );

        assert!(res.is_err(), "缺少 host 时必须报错");
        assert!(commands.is_empty(), "未执行任何命令");
    }

    /// 15. test_failed_remote_write_raises
    /// 远端写失败（runner 返回非零）时 run_calibrate 抛出错误
    #[test]
    fn test_failed_remote_write_raises() {
        let base_layout = load_default_layout();
        let worker_res = json!({
            "wechat_version": "4.1.15",
            "layout": base_layout,
            "measured": {"input_divider": 500.0}
        });
        let config = json!({"host": "wechat-mac"});

        let mut failing_runner = |_host: &str, _cmd: &str, _input: Option<&[u8]>| {
            Ok(CommandOutput {
                returncode: 1,
                stdout: vec![],
                stderr: b"denied".to_vec(),
            })
        };

        let res = run_calibrate(
            Some(&config),
            Some(&worker_res),
            None,
            None,
            Some(&mut failing_runner),
        );

        assert!(res.is_err(), "写失败必须报错");
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("remote write failed") || err_msg.contains("denied"));
    }

    /// 16. test_calibrate_refuses_while_bot_running
    /// 机器人未运行（loaded 为 false）时拒绝校准并退出码 1
    #[test]
    fn test_calibrate_refuses_while_bot_running() {
        let env = TempDir::new();
        let cfg_path = env.path().join("none.json");
        fs::write(&cfg_path, "{}").unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );

        assert_eq!(code, 1);
        let err_str = String::from_utf8(stderr).unwrap();
        assert!(err_str.contains("机器人未运行：请先 control start"));
    }

    /// 17. test_control_calibrate_round_trip
    /// control calibrate 通过写入请求并轮询结果文件实现与机器人的闭环通信
    #[test]
    fn test_control_calibrate_round_trip() {
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg_path = env.path().join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
            "host": "local",
            "account_id": "fixture",
            "state_dir": state_dir.to_str().unwrap()
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();

        let state_dir_clone = state_dir.clone();
        let bot_thread = std::thread::spawn(move || {
            let req_path = state_dir_clone.join("calibrate-request.json");
            let res_path = state_dir_clone.join("calibrate-result.json");
            for _ in 0..50 {
                if req_path.exists() {
                    if let Ok(content) = fs::read_to_string(&req_path) {
                        if let Ok(req) = serde_json::from_str::<Value>(&content) {
                            let id = req["id"].as_str().unwrap();
                            let result_payload = json!({
                                "id": id,
                                "ok": true,
                                "result": {
                                    "candidate_json": ".wechat-rpa/layout/candidate-wechat-4.1.15.json",
                                    "candidate_png": ".wechat-rpa/layout/candidate-wechat-4.1.15.png",
                                    "differences": [{"field": "input.y", "old": 496.0, "new": 500.0}],
                                    "missing": []
                                }
                            });
                            fs::write(&res_path, result_payload.to_string()).unwrap();
                            let _ = fs::remove_file(&req_path);
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        bot_thread.join().unwrap();
        assert_eq!(code, 0);

        let out_str = String::from_utf8(stdout).unwrap();
        let last_line = out_str.lines().last().unwrap();
        let res: Value = serde_json::from_str(last_line).unwrap();
        assert!(
            res["candidate_json"]
                .as_str()
                .unwrap()
                .contains("candidate-wechat-4.1.15.json")
        );
        assert_eq!(res["differences"].as_array().unwrap().len(), 1);
    }

    /// 18. test_control_calibrate_requires_running_bot
    /// 机器人未运行时退出码 1 且 stderr 提示 "机器人未运行：请先 control start"
    #[test]
    fn test_control_calibrate_requires_running_bot() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg_path = env.path().join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
            "host": "wechat-mac",
            "account_id": "fixture",
            "state_dir": state_dir.to_str().unwrap()
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(false),
        );

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 1);
        let err_str = String::from_utf8(stderr).unwrap();
        assert!(err_str.contains("机器人未运行：请先 control start"));
    }

    /// 19. test_anchors_measure_the_raw_capture
    /// Worker.swift 中 calibrate 分支调用 measureAnchors(rawImage, scale:) 与 drawAnnotatedLayout(rawImage, scale:)
    #[test]
    fn test_anchors_measure_the_raw_capture() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let worker_swift =
            fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift
            .find("operation == \"calibrate\"")
            .expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch
            .find("return result")
            .expect("must bound calibrate branch");
        let cal_block = &branch[..end_idx];

        assert!(
            cal_block.contains("measureAnchors(rawImage, scale:"),
            "必须使用 rawImage 测量"
        );
        assert!(
            cal_block.contains("drawAnnotatedLayout(rawImage, scale:"),
            "必须在 rawImage 上绘制标注"
        );
    }

    /// 20. test_text_anchors_are_not_reported_missing
    /// 文本锚点（如 title_text）不属于位置测量线，不能被判定为 missing
    #[test]
    fn test_text_anchors_are_not_reported_missing() {
        let base_layout = load_default_layout();
        let (_diffs, missing, _candidate) = derive_candidate_fields(
            &base_layout,
            &json!({"input_divider": 496.0, "list_border": 300.0}),
        );
        assert_eq!(
            missing,
            Vec::<String>::new(),
            "title_text 等非线锚点不能被算作 missing"
        );
    }

    // -------------------------------------------------------------
    // tests/test_wechat_bot_service.py control 相关 4 项迁移
    // -------------------------------------------------------------

    /// 21. test_default_config_is_in_user_home
    /// 默认配置和 base 目录必须位于用户 HOME 下的 .WeChatRPABot
    #[test]
    fn test_default_config_is_in_user_home() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let home = env.path().to_path_buf();

        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", &home) };

        let (base, config, plist) = locations();

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(config, home.join(".WeChatRPABot/config.json"));
        assert_eq!(base, config.parent().unwrap());
        assert_eq!(
            plist,
            home.join("Library/LaunchAgents/local.wechat-rpa.bot.plist")
        );
    }

    /// 22. test_status_reads_configured_state_after_config_move
    /// status 命令必须读取配置中指定的 state_dir 目录下的 health.json
    #[test]
    fn test_status_reads_configured_state_after_config_move() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let base = env.path().join(".WeChatRPABot");
        fs::create_dir_all(&base).unwrap();

        let custom_state = env.path().join("custom-state");
        fs::create_dir_all(&custom_state).unwrap();

        let config_path = base.join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "allowlist": [{"recipient": "测试群", "kind": "group"}],
            "host": "local",
            "account_id": "test",
            "state_dir": custom_state.to_str().unwrap()
        });
        fs::write(&config_path, cfg.to_string()).unwrap();

        let health = json!({"state": "watching", "pid": 123});
        fs::write(custom_state.join("health.json"), health.to_string()).unwrap();

        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(&["status".into()], &mut stdout, &mut stderr, Some(true));

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["health"], health);
    }

    /// 23. test_stop_waits_until_launchd_task_is_unloaded
    /// stop 命令依次执行 disable 和 bootout，并等待 loaded 变为 false
    #[test]
    fn test_stop_waits_until_launchd_task_is_unloaded() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        let state_sequence = [true, true, false, false];
        let mut state_idx = 0;
        let mut loaded_fn = move || {
            let val = state_sequence[state_idx.min(state_sequence.len() - 1)];
            state_idx += 1;
            val
        };

        let mut run_calls = Vec::new();
        let mut cmd_runner = |_cmd: &str, args: &[&str]| {
            run_calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            true
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["stop".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 0);
        assert_eq!(run_calls.len(), 2);
        assert_eq!(run_calls[0][0], "disable");
        assert_eq!(run_calls[1][0], "bootout");

        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["status"], "stopped");
        assert_eq!(out["label"], BOT_LABEL);
    }

    /// Unix 套接字路径有 104 字节上限，临时目录太长：用短路径并通过 WECHAT_CORE_SOCK 指过去。
    struct ShortSock(PathBuf);
    impl ShortSock {
        fn new() -> Self {
            let p = PathBuf::from(format!(
                "/tmp/wcc-{}.sock",
                &uuid::Uuid::new_v4().simple().to_string()[..12]
            ));
            unsafe { std::env::set_var("WECHAT_CORE_SOCK", &p) };
            ShortSock(p)
        }
    }
    impl Drop for ShortSock {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            unsafe { std::env::remove_var("WECHAT_CORE_SOCK") };
        }
    }

    fn write_valid_home_config(home: &Path) {
        let dir = home.join(".WeChatRPABot");
        fs::create_dir_all(&dir).unwrap();
        let state_dir = home.join("state");
        let cfg = json!({
            "schema_version": 1,
            "account_id": "test_account",
            "host": "local",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试群", "kind": "group"}]
        });
        fs::write(
            dir.join("config.json"),
            serde_json::to_string(&cfg).unwrap(),
        )
        .unwrap();
    }

    /// 24. start 只打开 core.enabled 开关，绝不 enable/bootstrap 旧 Python 任务
    #[test]
    fn test_start_never_bootstraps_python_job() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        write_valid_home_config(env.path());

        let mut loaded_fn = || false;
        let mut run_calls: Vec<Vec<String>> = Vec::new();
        let mut cmd_runner = |_cmd: &str, args: &[&str]| {
            run_calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            true
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["start".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        let enabled = env.path().join(".wechat-rpa/core.enabled").exists();
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
        assert!(enabled);
        assert!(
            !run_calls
                .iter()
                .any(|c| c[0] == "enable" || c[0] == "bootstrap"),
            "start 不得启用或加载旧 Python 任务: {run_calls:?}"
        );
    }

    /// start 发现旧 Python 任务仍加载时先卸载并禁用它，否则宿主不会拉起 Rust 核心
    #[test]
    fn test_start_unloads_legacy_python_job_when_loaded() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        write_valid_home_config(env.path());

        let seq = [true, false, false, false];
        let mut i = 0;
        let mut loaded_fn = move || {
            let v = seq[i.min(seq.len() - 1)];
            i += 1;
            v
        };
        let mut run_calls: Vec<Vec<String>> = Vec::new();
        let mut cmd_runner = |_cmd: &str, args: &[&str]| {
            run_calls.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            true
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["start".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
        let verbs: Vec<&str> = run_calls.iter().map(|c| c[0].as_str()).collect();
        assert!(
            verbs.contains(&"disable") && verbs.contains(&"bootout"),
            "{verbs:?}"
        );
        assert!(
            !verbs.contains(&"enable") && !verbs.contains(&"bootstrap"),
            "{verbs:?}"
        );
    }

    /// 配置无效：退出码 2，且不打开 core.enabled
    #[test]
    fn test_start_rejects_invalid_config_with_exit_2() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        let dir = env.path().join(".WeChatRPABot");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{ not json").unwrap();

        let mut loaded_fn = || false;
        let mut cmd_runner = |_cmd: &str, _args: &[&str]| true;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["start".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        let enabled = env.path().join(".wechat-rpa/core.enabled").exists();
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert_eq!(code, 2);
        assert!(!enabled, "配置无效时不得打开 core.enabled");
    }

    /// 配置文件缺失同样是配置错误
    #[test]
    fn test_start_rejects_missing_config_with_exit_2() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        let mut loaded_fn = || false;
        let mut cmd_runner = |_cmd: &str, _args: &[&str]| true;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["start".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        let enabled = env.path().join(".wechat-rpa/core.enabled").exists();
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        assert_eq!(code, 2);
        assert!(!enabled);
    }

    /// 只有开关文件、核心并未运行时，校准必须拒绝（否则会连不上 core.sock 却报告成功路径）
    #[test]
    fn test_calibrate_requires_running_core_not_just_enabled_flag() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        write_valid_home_config(env.path());
        let flag = env.path().join(".wechat-rpa/core.enabled");
        fs::create_dir_all(flag.parent().unwrap()).unwrap();
        fs::write(&flag, b"").unwrap();

        let mut loaded_fn = || false;
        let mut cmd_runner = |_cmd: &str, _args: &[&str]| true;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &["calibrate".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&stderr).contains("未运行"));
    }

    /// stop 删除开关后要等核心真正退出（core.sock 不再接受连接）才报告 stopped
    #[test]
    fn test_stop_waits_for_core_socket_to_close() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };
        let dir = env.path().join(".wechat-rpa");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("core.enabled"), b"").unwrap();
        let sock = ShortSock::new();
        let listener = std::os::unix::net::UnixListener::bind(&sock.0).unwrap();
        let closer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            drop(listener);
        });

        let mut loaded_fn = || false;
        let mut cmd_runner = |_cmd: &str, _args: &[&str]| true;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let began = std::time::Instant::now();
        let code = execute_control_full(
            &["stop".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        let waited = began.elapsed();
        closer.join().unwrap();
        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
        assert!(
            waited >= std::time::Duration::from_millis(500),
            "stop 没有等待核心退出: {waited:?}"
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["status"], "stopped");
        assert_eq!(out["core_running"], false);
    }

    /// 历史维护命令遇到无效配置：配置错误统一退出码 2（与运行错误 1 区分）
    #[test]
    fn test_history_commands_invalid_config_exit_2() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let cfg_path = env.path().join("config.json");
        fs::write(&cfg_path, "{ broken").unwrap();
        let c = cfg_path.to_str().unwrap().to_string();
        for args in [
            vec!["migrate-history".to_string(), "--config".into(), c.clone()],
            vec!["history-status".to_string(), "--config".into(), c.clone()],
            vec![
                "merge-contact".to_string(),
                "--source".into(),
                "a".into(),
                "--into".into(),
                "b".into(),
                "--config".into(),
                c.clone(),
            ],
            vec![
                "unmerge-contact".to_string(),
                "1".into(),
                "--config".into(),
                c.clone(),
            ],
            vec!["history-backup".to_string(), "--config".into(), c.clone()],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = execute_control(&args, &mut stdout, &mut stderr, Some(false));
            assert_eq!(
                code,
                2,
                "{} 遇到无效配置应退出 2: {}",
                args[0],
                String::from_utf8_lossy(&stderr)
            );
        }
    }

    /// 2026-11-08 实机：`contacts-sync --help` 被当成一次同步请求提交。未知参数只打印用法、退出 2、不提交请求。
    #[test]
    fn test_contacts_sync_unknown_argument_prints_usage_without_queueing() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        let cfg = json!({"schema_version": 1, "account_id": "t", "host": "local",
                         "state_dir": state_dir.to_str().unwrap(),
                         "allowlist": [{"recipient": "测试群", "kind": "group"}]});
        let cfg_path = env.path().join("config.json");
        fs::write(&cfg_path, cfg.to_string()).unwrap();
        for bad in ["--help", "-h", "--ful", "now"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = execute_control(
                &[
                    "contacts-sync".into(),
                    bad.into(),
                    "--config".into(),
                    cfg_path.to_str().unwrap().into(),
                ],
                &mut stdout,
                &mut stderr,
                Some(false),
            );
            assert_eq!(code, 2, "{bad}");
            assert!(
                String::from_utf8_lossy(&stderr).contains("Usage"),
                "{bad}: 应打印用法"
            );
            assert!(
                !state_dir.join("contacts-sync-request.json").exists(),
                "{bad}: 不得提交同步请求"
            );
        }
    }

    #[test]
    fn history_cli_running_core_rejects_writes_allows_reads() {
        let _env_lock = crate::core::env_lock();
        let env = TempDir::new();
        let state = env.path().join("state");
        let cfg_path = env.path().join("config.json");
        fs::write(&cfg_path, json!({"schema_version": 1, "account_id": "acct", "host": "local", "state_dir": state, "allowlist": [{"recipient": "a", "kind": "private"}]}).to_string()).unwrap();
        let store = crate::core::history::store::HistoryStore::open(&state).unwrap();
        let a = store
            .ensure_contact("acct", &json!({"recipient": "a", "kind": "private"}), true)
            .unwrap();
        let b = store
            .ensure_contact("acct", &json!({"recipient": "b", "kind": "private"}), true)
            .unwrap();
        let core = crate::core::serve::acquire_core_lock(&state).unwrap();
        for mut args in [
            vec!["migrate-history".to_string()],
            vec![
                "merge-contact".into(),
                "--source".into(),
                a.clone(),
                "--into".into(),
                b.clone(),
            ],
            vec!["unmerge-contact".into(), "1".into()],
        ] {
            args.extend(["--config".into(), cfg_path.to_str().unwrap().into()]);
            let mut out = Vec::new();
            let mut err = Vec::new();
            assert_eq!(execute_control(&args, &mut out, &mut err, Some(false)), 1);
            assert!(String::from_utf8_lossy(&err).contains("先停止服务"));
        }
        assert_eq!(store.canonical(a.clone()).unwrap(), a);
        let backup = env.path().join("online.sqlite3");
        for mut args in [
            vec!["history-status".to_string()],
            vec![
                "history-backup".into(),
                "--to".into(),
                backup.to_str().unwrap().into(),
            ],
        ] {
            args.extend(["--config".into(), cfg_path.to_str().unwrap().into()]);
            let mut out = Vec::new();
            let mut err = Vec::new();
            assert_eq!(
                execute_control(&args, &mut out, &mut err, Some(false)),
                0,
                "{}",
                String::from_utf8_lossy(&err)
            );
        }
        drop(core);
        assert!(!crate::core::serve::core_is_running(&state));
    }

    #[test]
    fn test_control_history_maintenance_commands_cli() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "account_id": "test_account",
            "host": "local",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试联系人A", "kind": "private"}]
        });
        let cfg_path = env.path().join("config.json");
        fs::write(&cfg_path, serde_json::to_string(&cfg).unwrap()).unwrap();

        // 1. 测试 migrate-history
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "migrate-history".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
        );
        assert_eq!(
            code,
            0,
            "migrate-history should succeed: {}",
            String::from_utf8_lossy(&stderr)
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert!(out.is_object(), "migrate-history output should be json map");

        // 在 history 库中插入两个联系人以供 merge/unmerge 测试
        {
            let store = crate::core::history::store::HistoryStore::open(&state_dir).unwrap();
            let conn_guard = store.connection();
            let conn = conn_guard.lock().unwrap();
            conn.execute(
                "INSERT INTO contacts (contact_id, account_id, kind, display_name, created_at, updated_at) VALUES ('c_user1', 'test_account', 'private', 'User 1', 1, 1), ('c_user2', 'test_account', 'private', 'User 2', 1, 1)",
                [],
            ).unwrap();
        }

        // 2. 测试 history-status
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "history-status".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
        );
        assert_eq!(
            code,
            0,
            "history-status should succeed: {}",
            String::from_utf8_lossy(&stderr)
        );
        let status_out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(status_out["contacts"].as_array().unwrap().len(), 2);

        // 3. 测试 merge-contact
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "merge-contact".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--source".into(),
                "c_user1".into(),
                "--into".into(),
                "c_user2".into(),
                "--note".into(),
                "test merge".into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
        );
        assert_eq!(
            code,
            0,
            "merge-contact should succeed: {}",
            String::from_utf8_lossy(&stderr)
        );
        let merge_out: Value = serde_json::from_slice(&stdout).unwrap();
        let merge_id = merge_out["merge_id"]
            .as_i64()
            .expect("merge_id should be integer");

        // 4. 测试 unmerge-contact
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "unmerge-contact".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                merge_id.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
        );
        assert_eq!(
            code,
            0,
            "unmerge-contact should succeed: {}",
            String::from_utf8_lossy(&stderr)
        );
        let unmerge_out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(unmerge_out["unmerged"], merge_id);

        // 5. 测试 history-backup
        let backup_target = env.path().join("backup_hist.sqlite3");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &[
                "history-backup".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--to".into(),
                backup_target.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
        );
        assert_eq!(
            code,
            0,
            "history-backup should succeed: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert!(backup_target.exists(), "backup destination must exist");
        let backup_out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(backup_out["backup"], backup_target.to_str().unwrap());
    }

    #[test]
    fn test_control_start_and_stop_manage_core_enabled() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        write_valid_home_config(env.path());
        let core_enabled_path = env.path().join(".wechat-rpa/core.enabled");
        assert!(!core_enabled_path.exists());

        // 1. control start 必须创建 core.enabled
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut loaded_fn = || false;
        let mut cmd_runner = |_cmd: &str, _args: &[&str]| true;
        let code = execute_control_full(
            &["start".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        assert_eq!(code, 0);
        assert!(
            core_enabled_path.exists(),
            "control start must create core.enabled for CoreSupervisor"
        );
        let start_out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(start_out["core_enabled"], true);

        // 2. control stop 必须删除 core.enabled
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut loaded_fn = || false;
        let code = execute_control_full(
            &["stop".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            Some(&mut cmd_runner),
        );
        assert_eq!(code, 0);
        assert!(
            !core_enabled_path.exists(),
            "control stop must remove core.enabled"
        );
        let stop_out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(stop_out["core_enabled"], false);

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
    }

    #[test]
    fn test_control_status_distinguishes_python_and_core() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        let core_enabled_path = env.path().join(".wechat-rpa/core.enabled");
        fs::create_dir_all(core_enabled_path.parent().unwrap()).unwrap();
        fs::write(&core_enabled_path, b"").unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut loaded_fn = || false; // Python launchd 未 loaded
        let code = execute_control_full(
            &["status".into()],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            None,
        );
        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["python_launchd_loaded"], false);
        assert_eq!(out["core_enabled"], true);

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
    }

    #[test]
    fn test_control_calibrate_allows_running_core_when_python_not_loaded() {
        let _lock = crate::core::env_lock();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        // 核心真实在运行（core.sock 在监听），python loaded 为 false
        let core_enabled_path = env.path().join(".wechat-rpa/core.enabled");
        fs::create_dir_all(core_enabled_path.parent().unwrap()).unwrap();
        fs::write(&core_enabled_path, b"").unwrap();
        let sock = ShortSock::new();
        let _core_listener = std::os::unix::net::UnixListener::bind(&sock.0).unwrap();

        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg = json!({
            "schema_version": 1,
            "account_id": "test_account",
            "host": "local",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试群", "kind": "group"}]
        });
        let cfg_path = env.path().join("config.json");
        fs::write(&cfg_path, serde_json::to_string(&cfg).unwrap()).unwrap();

        // 后台模拟响应 calibrate-request.json
        let state_dir_clone = state_dir.clone();
        let handle = std::thread::spawn(move || {
            let req_path = state_dir_clone.join("calibrate-request.json");
            let res_path = state_dir_clone.join("calibrate-result.json");
            for _ in 0..50 {
                if req_path.exists() {
                    if let Ok(content) = fs::read_to_string(&req_path) {
                        if let Ok(req) = serde_json::from_str::<Value>(&content) {
                            let id = req["id"].as_str().unwrap();
                            let _ = fs::remove_file(&req_path);
                            let _ = fs::write(
                                &res_path,
                                json!({
                                    "id": id,
                                    "ok": true,
                                    "result": {
                                        "candidate_json": "cand.json",
                                        "candidate_png": "cand.png",
                                        "differences": [],
                                        "missing": []
                                    }
                                })
                                .to_string(),
                            );
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut loaded_fn = || false; // Python 未运行
        let code = execute_control_full(
            &[
                "calibrate".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--timeout".into(),
                "3".into(),
            ],
            &mut stdout,
            &mut stderr,
            Some(&mut loaded_fn),
            None,
        );
        let _ = handle.join();

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        // 不应报错 "机器人未运行：请先 control start"
        assert_eq!(
            code,
            0,
            "Calibrate should succeed while serve-core is running: {}",
            String::from_utf8_lossy(&stderr)
        );
    }

    #[test]
    fn test_control_contacts_list_includes_ignore_and_allow_handling() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-desktop",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "甲备注", "kind": "private"}],
            "ignorelist": [{"recipient": "乙备注", "kind": "private"}]
        });
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let store = crate::core::history::store::HistoryStore::open(&state_dir).unwrap();
        // Insert contacts with profiles
        let p_allow = crate::core::history::store::ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t1".to_string()),
            nickname: "测试甲".to_string(),
            remark: Some("甲备注".to_string()),
            region: None,
            source: None,
            added_on: Some("2023-06-08".to_string()),
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        store
            .upsert_profile("test-desktop", &p_allow, 1000)
            .unwrap();

        let p_ignore = crate::core::history::store::ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t2".to_string()),
            nickname: "测试乙".to_string(),
            remark: Some("乙备注".to_string()),
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        store
            .upsert_profile("test-desktop", &p_ignore, 1000)
            .unwrap();

        // Contact without profile (only in chat)
        store
            .ensure_contact(
                "test-desktop",
                &json!({"recipient": "丙普通", "kind": "private"}),
                true,
            )
            .unwrap();

        // Sync result file
        let sync_res = json!({
            "finished_at_ms": 1794115000000_i64,
            "complete": true,
            "per_category": [{"category": "personal", "expected": 2, "synced": 2}],
            "unreadable_rows": 0,
            "needs_review": 0
        });
        crate::core::contacts::sync::write_sync_result(&state_dir, "test-desktop", &sync_res)
            .unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "contacts-list".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

        let out_val: Value = serde_json::from_slice(&stdout).expect("valid json output");
        assert_eq!(out_val["account_id"], "test-desktop");
        assert_eq!(out_val["sync"]["complete"], true);
        assert_eq!(out_val["sync"]["finished_at_ms"], 1794115000000_i64);

        let contacts = out_val["contacts"].as_array().expect("contacts array");
        assert_eq!(contacts.len(), 3);

        let c_allow = contacts
            .iter()
            .find(|c| c["display_name"] == "甲备注")
            .expect("甲备注");
        assert_eq!(c_allow["handling"], "auto_reply");
        assert_eq!(c_allow["wechat_id"], "wxid_t1");
        assert_eq!(c_allow["identity"], "strong");
        assert_eq!(c_allow["category"], "personal");
        assert_eq!(c_allow["removed"], false);

        let c_ignore = contacts
            .iter()
            .find(|c| c["display_name"] == "乙备注")
            .expect("乙备注");
        assert_eq!(c_ignore["handling"], "ignored");

        let c_record = contacts
            .iter()
            .find(|c| c["display_name"] == "丙普通")
            .expect("丙普通");
        assert_eq!(c_record["handling"], "record_only");
        assert_eq!(c_record["category"], "personal");
        assert!(c_record["wechat_id"].is_null());
    }

    /// 用户要求：联系人归属当前登录的微信账号；换号后各账号的联系人分别保留，列表只显示当前账号的
    #[test]
    fn test_contacts_list_shows_only_current_wechat_account() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-desktop",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试群", "kind": "group"}]
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();
        let store = crate::core::history::store::HistoryStore::open(&state_dir).unwrap();
        store
            .ensure_contact(
                "wxid_a",
                &json!({"recipient": "甲号的客户", "kind": "private"}),
                true,
            )
            .unwrap();
        store
            .ensure_contact(
                "wxid_b",
                &json!({"recipient": "乙号的客户", "kind": "private"}),
                true,
            )
            .unwrap();
        crate::core::contacts::sync::write_sync_result(
            &state_dir,
            "wxid_a",
            &json!({"complete":true,"finished_at_ms":1000}),
        )
        .unwrap();
        crate::core::wechat_account::record_current_account(&state_dir, "wxid_b", 1.0).unwrap();

        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = execute_control_full(
            &[
                "contacts-list".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["account_id"], "wxid_b");
        assert!(out["sync"].is_null(), "乙号不能显示甲号的同步成功状态");
        let names: Vec<&str> = out["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["display_name"].as_str())
            .collect();
        assert_eq!(names, vec!["乙号的客户"]);
    }

    #[test]
    fn test_control_mcp_info_lists_tool_groups() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-desktop",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试联系人", "kind": "private"}]
        });
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "mcp-info".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

        let out_val: Value = serde_json::from_slice(&stdout).expect("valid json output");
        assert!(out_val.get("brain").is_some());
        let endpoints = out_val["endpoints"].as_array().expect("endpoints array");
        assert_eq!(endpoints.len(), 4);
        assert!(endpoints.iter().any(|e| e["id"] == "tools"));
        assert!(endpoints.iter().any(|e| e["id"] == "core_sock"));
        assert!(endpoints.iter().any(|e| e["id"] == "history_http"));
        assert!(endpoints.iter().any(|e| e["id"] == "open_api"));

        let open_ep = endpoints.iter().find(|e| e["id"] == "open_api").unwrap();
        assert_eq!(open_ep["label"], "对外接口");
        assert_eq!(open_ep["value"], "http://127.0.0.1:8770/mcp");
        assert_eq!(open_ep["ok"], false);
        assert_eq!(open_ep["detail"], "已关闭");

        let tool_groups = out_val["tool_groups"]
            .as_array()
            .expect("tool_groups array");
        assert_eq!(tool_groups.len(), 3);
        let titles: Vec<&str> = tool_groups
            .iter()
            .map(|g| g["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["对外接口", "运维", "通用桌面"]);

        let open_group = tool_groups
            .iter()
            .find(|g| g["title"] == "对外接口")
            .unwrap();
        let open_tools: Vec<&str> = open_group["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        assert_eq!(open_tools, crate::core::open_api::http::open_tool_names());

        let desktop_group = tool_groups
            .iter()
            .find(|g| g["title"] == "通用桌面")
            .unwrap();
        let desktop_tools: Vec<&str> = desktop_group["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        assert_eq!(
            desktop_tools,
            vec![
                "desktop_apps",
                "desktop_windows",
                "desktop_observe",
                "desktop_act",
                "desktop_stop"
            ]
        );

        let ops_group = tool_groups.iter().find(|g| g["title"] == "运维").unwrap();
        let ops_tools: Vec<&str> = ops_group["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        for t in &desktop_tools {
            assert!(!ops_tools.contains(t), "通用桌面工具 {t} 不应在运维组中");
        }
        for t in &open_tools {
            assert!(!ops_tools.contains(t), "对外接口工具 {t} 不应在运维组中");
        }
    }

    #[test]
    fn test_open_api_token_rotate_changes_token() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-desktop",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试联系人", "kind": "private"}]
        });
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        // 1. 初次获取 token
        let mut stdout1 = Vec::new();
        let mut stderr1 = Vec::new();
        let code1 = execute_control_full(
            &[
                "open-api-token".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout1,
            &mut stderr1,
            None,
            None,
        );
        assert_eq!(code1, 0, "stderr: {}", String::from_utf8_lossy(&stderr1));
        let val1: Value = serde_json::from_slice(&stdout1).unwrap();
        let token1 = val1["token"].as_str().expect("token string");
        assert_eq!(token1.len(), 64);

        // 2. 再次获取（不带 --rotate），token 应该相同
        let mut stdout2 = Vec::new();
        let mut stderr2 = Vec::new();
        let code2 = execute_control_full(
            &[
                "open-api-token".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout2,
            &mut stderr2,
            None,
            None,
        );
        assert_eq!(code2, 0);
        let val2: Value = serde_json::from_slice(&stdout2).unwrap();
        let token2 = val2["token"].as_str().unwrap();
        assert_eq!(token2, token1);

        // 3. 轮换获取（带 --rotate），token 应该变化
        let mut stdout3 = Vec::new();
        let mut stderr3 = Vec::new();
        let code3 = execute_control_full(
            &[
                "open-api-token".into(),
                "--rotate".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout3,
            &mut stderr3,
            None,
            None,
        );
        assert_eq!(code3, 0);
        let val3: Value = serde_json::from_slice(&stdout3).unwrap();
        let token3 = val3["token"].as_str().unwrap();
        assert_eq!(token3.len(), 64);
        assert_ne!(token3, token1);

        // 4. 轮换后再次获取，应该为新 token
        let mut stdout4 = Vec::new();
        let mut stderr4 = Vec::new();
        let code4 = execute_control_full(
            &[
                "open-api-token".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout4,
            &mut stderr4,
            None,
            None,
        );
        assert_eq!(code4, 0);
        let val4: Value = serde_json::from_slice(&stdout4).unwrap();
        let token4 = val4["token"].as_str().unwrap();
        assert_eq!(token4, token3);
    }

    #[test]
    fn test_open_api_check_disabled_reports_closed() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-desktop",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试联系人", "kind": "private"}],
            "open_api": {
                "enabled": false
            }
        });
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "open-api-check".into(),
                "--json".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));

        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["ok"], false);
        assert_eq!(out["busy"], false);
        assert_eq!(out["tools"].as_array().unwrap().len(), 0);
        assert_eq!(out["error"], "disabled");
    }

    #[test]
    fn test_brain_control_list_and_save() {
        let temp = TempDir::new();
        let cfg_path = temp.path().join("config.json");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let cfg = json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "state_dir": state_dir.to_str().unwrap(),
            "allowlist": [{"recipient": "测试联系人", "kind": "private"}]
        });
        fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-list".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(
            code,
            0,
            "stderr: {}, stdout: {:?}",
            String::from_utf8_lossy(&stderr),
            out
        );
        assert_eq!(out["ok"], true);

        // 2. brain-save-role
        use crate::core::brain_profiles::{AccountContext, legacy_seed};
        let ctx = AccountContext::new("test-account".into()).unwrap();
        let mut r = legacy_seed(&ctx, true, 1).unwrap();
        r.role_id = "role_demo".into();
        r.revision_id = "rev_1".into();
        r.persona = "测试助手".into();
        let r_val = serde_json::to_value(&r).unwrap();

        let req_save = json!({
            "expected_generation": 0,
            "request_id": uuid::Uuid::new_v4().to_string(),
            "role": r_val,
        });

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-save-role".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--request-json".into(),
                req_save.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(
            code,
            0,
            "stderr: {}, stdout: {:?}",
            String::from_utf8_lossy(&stderr),
            out
        );
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"]["role_id"], "role_demo");
        assert_eq!(out["data"]["generation"], 1);

        // 3. brain-get
        let req_get = json!({
            "role_id": "role_demo",
        });
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-get".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--request-json".into(),
                req_get.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(code, 0);
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"]["persona"], "测试助手");

        // 4. brain-preview (assemble_only)
        let req_preview = json!({
            "role_id": "role_demo",
            "execution": "assemble_only",
            "input": {
                "text": "你好",
                "chat_kind": "private",
                "files": [],
                "images": [],
                "failed": [],
                "context": []
            }
        });
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-preview".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--request-json".into(),
                req_preview.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(code, 0);
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"]["cost"]["incurred"], false);
        assert!(
            out["data"]["snapshot"]["compiled"]
                .as_str()
                .unwrap()
                .contains("测试助手")
        );

        // 5. 跨账号拒绝
        let req_cross = json!({
            "account_id": "other-account",
            "role_id": "role_demo",
        });
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-get".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--request-json".into(),
                req_cross.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(code, 2);
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"]["code"], "not_found");

        // 6. 冲突返回 revision_conflict
        let req_conflict = json!({
            "expected_generation": 999,
            "request_id": uuid::Uuid::new_v4().to_string(),
            "role": r_val,
        });
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control_full(
            &[
                "brain-save-role".into(),
                "--config".into(),
                cfg_path.to_str().unwrap().into(),
                "--request-json".into(),
                req_conflict.to_string(),
            ],
            &mut stdout,
            &mut stderr,
            None,
            None,
        );
        let out: Value = serde_json::from_slice(&stdout).unwrap_or(Value::Null);
        assert_eq!(code, 2);
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"]["code"], "revision_conflict");
        assert_eq!(out["error"]["details"]["current_generation"], 1);
    }
}
