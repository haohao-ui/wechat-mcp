use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::core::config::BotConfig;
use crate::core::state::write_private;

pub const BOT_LABEL: &str = "local.wechat-rpa.bot";
pub const HISTORY_LABEL: &str = "local.wechat-rpa.history";
pub const REMOTE_LAYOUT_DIR: &str = ".wechat-rpa/layout";
pub const CALIBRATE_PROBE: &str = "calibrate-probe";

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
    let secs = time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
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
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn resolve_local_path(p: &str) -> PathBuf {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
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
    let window = obj.get("window").and_then(Value::as_object).context("layout_invalid_value:window")?;
    let win_w = window.get("width").and_then(Value::as_f64).context("layout_invalid_value:window")?;
    let win_h = window.get("height").and_then(Value::as_f64).context("layout_invalid_value:window")?;

    let regions = obj.get("regions").and_then(Value::as_object).context("layout_unreadable")?;
    let required_regions = ["session_list", "title", "chat", "input"];
    for req in required_regions {
        if !regions.contains_key(req) {
            anyhow::bail!("layout_missing_region:{}", req);
        }
    }

    for (name, rect_val) in regions {
        let rect = rect_val.as_object().with_context(|| format!("layout_invalid_value:{}", name))?;
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

    let scroll = obj.get("scroll").and_then(Value::as_object).context("layout_invalid_value:scroll")?;
    let ppt = scroll.get("points_per_tick").and_then(Value::as_f64).unwrap_or(0.0);
    if ppt <= 0.0 {
        anyhow::bail!("layout_invalid_value:scroll.points_per_tick");
    }
    let lpt = scroll.get("lines_per_tick").and_then(Value::as_i64).unwrap_or(0);
    if lpt <= 0 {
        anyhow::bail!("layout_invalid_value:scroll.lines_per_tick");
    }

    Ok(())
}

pub fn derive_candidate_fields(base_layout: &Value, measured: &Value) -> (Vec<Value>, Vec<String>, Value) {
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

    if let Some(divider_val) = measured.as_object().and_then(|m| m.get("input_divider")).filter(|v| !v.is_null()) {
        if let Some(divider) = divider_val.as_f64() {
            if let Some(regions) = candidate.get_mut("regions").and_then(Value::as_object_mut) {
                if let Some(chat) = regions.get_mut("chat").and_then(Value::as_object_mut) {
                    let chat_y = chat.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    let new_chat_h = divider - chat_y;
                    let old_chat_h = chat.get("h").cloned().unwrap_or(Value::Null);
                    if chat.get("h").and_then(Value::as_f64) != Some(new_chat_h) {
                        diffs.push(json!({"field": "chat.h", "old": old_chat_h, "new": new_chat_h}));
                        chat.insert("h".to_string(), json!(new_chat_h));
                    }
                }
                if let Some(chat_content) = regions.get_mut("chat_content").and_then(Value::as_object_mut) {
                    let cc_y = chat_content.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    let new_cc_h = divider - cc_y;
                    let old_cc_h = chat_content.get("h").cloned().unwrap_or(Value::Null);
                    if chat_content.get("h").and_then(Value::as_f64) != Some(new_cc_h) {
                        diffs.push(json!({"field": "chat_content.h", "old": old_cc_h, "new": new_cc_h}));
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

    if let Some(border_val) = measured.as_object().and_then(|m| m.get("list_border")).filter(|v| !v.is_null()) {
        if let Some(border) = border_val.as_f64() {
            if let Some(regions) = candidate.get_mut("regions").and_then(Value::as_object_mut) {
                if let Some(s_list) = regions.get_mut("session_list").and_then(Value::as_object_mut) {
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

    let version = res.get("wechat_version").and_then(Value::as_str).unwrap_or("");
    if !validate_version_string(version) {
        anyhow::bail!("layout_missing");
    }

    let base_layout = res.get("layout").and_then(Value::as_object).context("layout_missing")?;

    let measured = res.get("measured").cloned().unwrap_or_else(|| json!({}));
    let (diffs, missing, mut candidate) = derive_candidate_fields(&Value::Object(base_layout.clone()), &measured);
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
    let running_version = res.get("wechat_version").and_then(Value::as_str).unwrap_or("");
    if !validate_version_string(running_version) {
        anyhow::bail!("layout_missing");
    }

    let cand_path_str = format!("{}/candidate-wechat-{}.json", layout_dir_str, running_version);

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

    let candidate_val: Value = serde_json::from_slice(&cand_bytes).map_err(|_| anyhow::anyhow!("layout_unreadable"))?;
    validate_layout_candidate(&candidate_val)?;

    let cand_version = candidate_val.get("wechat_version").and_then(Value::as_str).unwrap_or("");
    if cand_version != running_version {
        anyhow::bail!("version_mismatch: candidate is {}, running is {}", cand_version, running_version);
    }

    let active_path_str = format!("{}/wechat-{}.json", layout_dir_str, running_version);
    let timestamp = format_timestamp_compact(SystemTime::now());
    let backup_path_str = format!("{}/wechat-{}.json.bak-{}", layout_dir_str, running_version, timestamp);

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
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
    let base = home.join(".WeChatRPABot");
    let config = base.join("config.json");
    let plist = home.join("Library/LaunchAgents/local.wechat-rpa.bot.plist");
    (base, config, plist)
}

pub fn resolve_state_dir(config_path: &Path, base: &Path) -> PathBuf {
    if config_path.exists() {
        if let Ok(cfg) = BotConfig::load(config_path) {
            return cfg.state_dir;
        }
        if let Ok(content) = fs::read_to_string(config_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                if let Some(s) = v.get("state_dir").and_then(Value::as_str) {
                    return PathBuf::from(s);
                }
            }
        }
    }
    base.join("state")
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
                        if sobj.get("state").and_then(|v| v.as_str()) == Some("needs_recalibration") {
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
                                if val.get("state").and_then(|v| v.as_str()) == Some("needs_recalibration") {
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
                if matches!(status, "generating" | "generated" | "attempting" | "unknown") {
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
            "Usage: wechat-mcp control <status|start|stop|pause|resume|resume-session|config-check|calibrate> [--config <path>]"
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
            let uid = unsafe { getuid() };
            let domain = format!("gui/{uid}");
            let label = format!("{domain}/{BOT_LABEL}");

            if let Some(ref mut runner) = launchctl_runner {
                runner("launchctl", &["enable", &label]);
            } else {
                let _ = Command::new("launchctl").args(["enable", &label]).status();
            }

            let loaded_now = if let Some(ref mut lfn) = loaded_fn {
                lfn()
            } else {
                loaded()
            };

            if !loaded_now {
                if config_path.exists() {
                    if let Err(e) = BotConfig::load(&config_path) {
                        let _ = writeln!(stderr, "failed to load config: {}", e);
                        return 1;
                    }
                }
                let (_, _, plist_path) = locations();
                let plist_str = plist_path.to_string_lossy();
                if let Some(ref mut runner) = launchctl_runner {
                    runner("launchctl", &["bootstrap", &domain, &plist_str]);
                } else {
                    let _ = Command::new("launchctl")
                        .args(["bootstrap", &domain, &plist_str])
                        .status();
                }
            }

            let _ = writeln!(stdout, "{}", json!({"status": "started", "label": BOT_LABEL}));
            0
        }
        "stop" => {
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
                let _ = writeln!(stderr, "service is still unloading; do not start a replacement yet");
                return 1;
            }

            let _ = writeln!(stdout, "{}", json!({"status": "stopped", "label": BOT_LABEL}));
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
            let state_dir = resolve_state_dir(&config_path, &base);

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
            let state_dir = resolve_state_dir(&config_path, &base);

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
                    .and_then(|v| v.get("state_dir").and_then(Value::as_str).map(PathBuf::from))
                {
                    Some(dir) => dir,
                    None => {
                        let _ = writeln!(stderr, "control resume-session: cannot read state_dir from {}", config_path.display());
                        return 2;
                    }
                },
            };

            let socket = crate::core::brain_tools::default_socket_path();
            match resume_session_with(&socket, &state_dir, &session_name) {
                Ok(ResumeOutcome::Core) => {
                    let _ = writeln!(stdout, "{}", json!({"ok": true, "session": session_name, "status": "resumed", "via": "core", "file_removed": false}));
                    0
                }
                Ok(ResumeOutcome::MarkerRemoved(n)) => {
                    let _ = writeln!(stdout, "{}", json!({"ok": true, "session": session_name, "status": if n > 0 { "resumed" } else { "not_handed_off" }, "via": "file", "file_removed": n > 0}));
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
            let state_dir = resolve_state_dir(&config_path, &base);

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
                let _ = writeln!(stderr, "界面核对连续失败，请运行 control calibrate 重新定位");
            }

            let out = json!({
                "launchd_loaded": is_loaded,
                "health": health,
                "stats": stats,
                "config": config_path.to_string_lossy().to_string(),
                "needs_recalibration": needs_recalibration,
            });
            let _ = writeln!(stdout, "{}", out);
            0
        }
        "calibrate" => {
            if !is_loaded {
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
            let state_dir = resolve_state_dir(&config_path, &base);

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
        other => {
            let _ = writeln!(stderr, "unknown control action: {}", other);
            2
        }
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
pub fn resume_session_with(socket: &Path, state_dir: &Path, recipient: &str) -> Result<ResumeOutcome> {
    use std::io::{BufRead, BufReader, Write as _};
    match std::os::unix::net::UnixStream::connect(socket) {
        Ok(stream) => {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            let mut writer = stream.try_clone()?;
            writeln!(writer, "{}", json!({"id": "control-resume", "op": "resume_session", "recipient": recipient}))?;
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line)?;
            let response: Value = serde_json::from_str(line.trim()).context("core.sock 返回了无效 JSON")?;
            if response.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok(ResumeOutcome::Core)
            } else {
                anyhow::bail!("core 拒绝 resume_session: {}", response.get("error").cloned().unwrap_or(Value::Null))
            }
        }
        Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            let mut removed = 0;
            if let Ok(entries) = fs::read_dir(state_dir.join("sessions")) {
                for entry in entries.flatten() {
                    let marker = entry.path().join("handoff.json");
                    let Ok(text) = fs::read_to_string(&marker) else { continue };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                    if value.get("recipient").and_then(Value::as_str) == Some(recipient) && fs::remove_file(&marker).is_ok() {
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
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
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
            &["config-check".into(), cfg_path.to_string_lossy().to_string()],
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
            &["config-check".into(), bad_path.to_string_lossy().to_string()],
            &mut stdout,
            &mut stderr,
            Some(false),
        );
        assert_eq!(code, 2);
        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("unsupported bot config schema or field"));
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
        assert!(stdout_str.contains("Missing measurements (preserved original values):\n  right_line"));

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
        let dir = TempDir::new();
        let (cfg_path, state_dir) = write_test_config(dir.path());

        // 1. config-check 对比
        let py_check = Command::new("python3")
            .args(["-m", "scripts.wechat_bot.control", "config-check", cfg_path.to_str().unwrap()])
            .output()
            .unwrap();
        let mut rust_stdout = Vec::new();
        let mut rust_stderr = Vec::new();
        let rust_code = execute_control(
            &["config-check".into(), cfg_path.to_string_lossy().to_string()],
            &mut rust_stdout,
            &mut rust_stderr,
            Some(false),
        );
        assert_eq!(rust_code, py_check.status.code().unwrap());
        let py_json: Value = serde_json::from_slice(&py_check.stdout).unwrap();
        let rust_json: Value = serde_json::from_slice(&rust_stdout).unwrap();
        assert_eq!(rust_json, py_json);

        // 2. pause 对比
        let pause_file = state_dir.join("admin-pause.json");
        let mut rust_pause_stdout = Vec::new();
        let mut rust_pause_stderr = Vec::new();
        let rust_pause_code = execute_control(
            &["pause".into(), "--config".into(), cfg_path.to_string_lossy().to_string()],
            &mut rust_pause_stdout,
            &mut rust_pause_stderr,
            Some(false),
        );
        assert_eq!(rust_pause_code, 0);
        let rust_pause_json: Value = serde_json::from_slice(&rust_pause_stdout).unwrap();
        assert_eq!(rust_pause_json["status"], "paused");
        assert_eq!(rust_pause_json["file"], pause_file.to_string_lossy().to_string());

        // 3. resume 对比
        let py_resume = Command::new("python3")
            .args(["-m", "scripts.wechat_bot.control", "resume", "--config", cfg_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(py_resume.status.code().unwrap(), 0);
        let py_resume_json: Value = serde_json::from_slice(&py_resume.stdout).unwrap();

        // 重新 pause 后由 Rust resume 并比对
        fs::write(&pause_file, r#"{"paused": true}"#).unwrap();
        let mut rust_resume_stdout = Vec::new();
        let mut rust_resume_stderr = Vec::new();
        let rust_resume_code = execute_control(
            &["resume".into(), "--config".into(), cfg_path.to_string_lossy().to_string()],
            &mut rust_resume_stdout,
            &mut rust_resume_stderr,
            Some(false),
        );
        assert_eq!(rust_resume_code, 0);
        let rust_resume_json: Value = serde_json::from_slice(&rust_resume_stdout).unwrap();
        assert_eq!(rust_resume_json, py_resume_json);

        // 4. calibrate (not started) 对比
        let py_cal = Command::new("python3")
            .args(["-m", "scripts.wechat_bot.control", "calibrate", "--config", cfg_path.to_str().unwrap()])
            .output()
            .unwrap();
        let mut rust_cal_stdout = Vec::new();
        let mut rust_cal_stderr = Vec::new();
        let rust_cal_code = execute_control(
            &["calibrate".into(), "--config".into(), cfg_path.to_string_lossy().to_string()],
            &mut rust_cal_stdout,
            &mut rust_cal_stderr,
            Some(false),
        );
        assert_eq!(rust_cal_code, py_cal.status.code().unwrap());
        assert_eq!(
            String::from_utf8(rust_cal_stderr).unwrap(),
            String::from_utf8(py_cal.stderr).unwrap()
        );
    }

    #[test]
    fn test_resume_session_tells_running_core_over_socket() {
        use std::io::{BufRead, BufReader, Write as _};
        let td = TempDir::new();
        // Unix socket paths are limited to ~104 bytes; the temp dir path is too long.
        let sock = PathBuf::from(format!("/tmp/wcc-{}.sock", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let mut out = stream;
            writeln!(out, "{}", json!({"id": request["id"], "ok": true, "result": {"resumed": true}})).unwrap();
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
        assert!(handoff.exists(), "the running core owns the marker; control must not delete it behind its back");
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
        let outcome = resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "测试群").unwrap();
        assert_eq!(outcome, ResumeOutcome::MarkerRemoved(1));
        assert!(!mine.exists());
        assert!(other.exists());
        let none = resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "测试群").unwrap();
        assert_eq!(none, ResumeOutcome::MarkerRemoved(0));
    }

    #[test]
    fn test_control_resume_session_removes_handoff_and_validates() {
        let td = TempDir::new();
        let state_dir = td.path().join("state");
        let session_dir = state_dir.join("sessions").join("test_vip");
        fs::create_dir_all(&session_dir).unwrap();

        let handoff_file = session_dir.join("handoff.json");
        // The marker names its recipient (written by the core on handoff); no core is listening here.
        fs::write(&handoff_file, json!({"recipient": "test_vip", "turn_id": "t", "at": 1.0}).to_string()).unwrap();
        unsafe { std::env::set_var("WECHAT_CORE_SOCK", "/tmp/wcc-absent-control-test.sock") };
        assert!(handoff_file.exists());

        let cfg_path = td.path().join("config.json");
        fs::write(
            &cfg_path,
            json!({
                "schema_version": 1,
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
            &["resume-session".into(), "--config".into(), cfg_path.to_str().unwrap().into()],
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let worker_source = fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let prod_control = control_source.split("#[cfg(test)]").next().unwrap();
        assert!(!prod_control.contains("\"4.1.15\""), "control.rs 生产代码不得硬编码 '4.1.15' fallback");
        assert!(!worker_source.contains("\"4.1.15\""), "Worker.swift 不得硬编码 '4.1.15' fallback");
    }

    /// 2. test_version_regex_validation
    /// 版本必须符合正则 ^[0-9]+(\\.[0-9]+){1,3}$
    #[test]
    fn test_version_regex_validation() {
        let valid_versions = ["4.1.15", "4.0.0", "1.0", "1.2.3.4"];
        let invalid_versions = ["4.1.15;rm -rf /", "v4.1", "beta", "", "4.", ".1", "4.1.15.1.2"];

        for v in valid_versions {
            assert!(validate_version_string(v), "Expected {} to be valid", v);
        }
        for inv in invalid_versions {
            assert!(!validate_version_string(inv), "Expected {} to be invalid", inv);
        }
    }

    /// 3. test_worker_missing_version_aborts_without_writing
    /// worker 响应缺少版本或 layout 时，run_calibrate 拒绝且不写任何文件
    #[test]
    fn test_worker_missing_version_aborts_without_writing() {
        let mut commands = Vec::new();
        let mut fake_runner = |_host: &str, cmd: &str, _input: Option<&[u8]>| {
            commands.push(cmd.to_string());
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
        let worker_swift = fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift.find("operation == \"calibrate\"").expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch.find("guard let recipient").expect("must bound calibrate branch");
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
                    let path = rest.split_whitespace().next().unwrap().trim_matches('\'').trim_matches('"');
                    if let Some(data) = input {
                        written_files.insert(path.to_string(), data.to_vec());
                    }
                }
            }
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
        ).unwrap();

        // 1. 派生字段验证
        assert_eq!(candidate["regions"]["input"]["y"], 490.0);
        assert!(diffs.iter().any(|d| d["field"] == "input.y" && d["new"] == 490.0));

        // 2. 写入文件路径验证
        let json_cand = format!("~/.wechat-rpa/layout/candidate-wechat-{}.json", version);
        let png_cand = format!("~/.wechat-rpa/layout/candidate-wechat-{}.png", version);
        assert!(written_files.contains_key(&json_cand), "应写入 candidate json");
        assert!(written_files.contains_key(&png_cand), "应写入 candidate png");
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
                return Ok(CommandOutput { returncode: 0, stdout: json_bytes, stderr: vec![] });
            }
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
        let write_cmds: Vec<_> = commands.iter().filter(|c| {
            c.contains("cp ") || c.contains("mv ") || c.contains("cat >") || c.contains("rm ")
        }).collect();
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
                return Ok(CommandOutput { returncode: 0, stdout: b"{broken json".to_vec(), stderr: vec![] });
            }
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
        let write_cmds: Vec<_> = commands.iter().filter(|c| {
            c.contains("cp ") || c.contains("mv ") || c.contains("cat >") || c.contains("rm ")
        }).collect();
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
                return Ok(CommandOutput { returncode: 0, stdout: json_bytes, stderr: vec![] });
            }
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
        ).unwrap();

        assert_eq!(res["accepted"], true);

        let atomic_cmds: Vec<_> = commands.iter().filter(|c| c.contains("cp ") && c.contains("mv ")).collect();
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
            &["calibrate".into(), "--config".into(), cfg_path.to_str().unwrap().into(), "--timeout".into(), "3".into()],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        handle.join().unwrap();
        assert_eq!(code, 0);

        let out_str = String::from_utf8(stdout).unwrap();
        let last_line = out_str.lines().last().unwrap();
        let res: Value = serde_json::from_str(last_line).unwrap();
        assert!(res["candidate_json"].as_str().unwrap().contains("candidate-wechat-4.1.15.json"));
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
            &["calibrate".into(), "--accept".into(), "--config".into(), cfg_path.to_str().unwrap().into(), "--timeout".into(), "3".into()],
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
        let worker_swift = fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift.find("operation == \"calibrate\"").expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch.find("guard let recipient").expect("must bound calibrate branch");
        let cal_block = &branch[..end_idx];

        let forbidden = ["botActions", "click(", "rightClick(", "key(", "act(", "scroll("];
        for term in forbidden {
            assert!(!cal_block.contains(term), "calibrate 分支不得调用 {}", term);
        }
    }

    /// 12. test_default_paths_land_in_remote_home
    /// 默认路径写入 HOME 下的 .wechat-rpa/layout，且不会在根下生成字面量 '~' 目录
    /// 注：等价断言在本地环境测试，验证默认路径解析行为与 0600 权限
    #[test]
    fn test_default_paths_land_in_remote_home() {
        let _lock = ENV_LOCK.lock().unwrap();
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

        let res = run_calibrate(
            Some(&config),
            Some(&worker_res),
            None,
            None,
            None,
        );

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }

        assert!(res.is_ok());
        let cand_json = layout_dir.join(format!("candidate-wechat-{}.json", version));
        assert!(cand_json.exists(), "candidate 文件必须存在于 layout 目录");

        let cand_content: Value = serde_json::from_str(&fs::read_to_string(&cand_json).unwrap()).unwrap();
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
        let _lock = ENV_LOCK.lock().unwrap();
        let env = TempDir::new();
        let home = env.path().to_path_buf();
        let layout_dir = home.join(".wechat-rpa").join("layout");
        fs::create_dir_all(&layout_dir).unwrap();

        let base_layout = load_default_layout();
        let version = base_layout["wechat_version"].as_str().unwrap();

        let active_json = layout_dir.join(format!("wechat-{}.json", version));
        fs::write(&active_json, serde_json::to_string_pretty(&base_layout).unwrap()).unwrap();

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

        let new_active: Value = serde_json::from_str(&fs::read_to_string(&active_json).unwrap()).unwrap();
        assert_eq!(new_active["regions"]["input"]["y"], 500.0);

        let backups: Vec<_> = fs::read_dir(&layout_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("wechat-{}.json.bak-", version)))
            .collect();
        assert_eq!(backups.len(), 1, "必须恰好产生 1 个备份");
        let backup_content: Value = serde_json::from_str(&fs::read_to_string(backups[0].path()).unwrap()).unwrap();
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
            Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
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
            Ok(CommandOutput { returncode: 1, stdout: vec![], stderr: b"denied".to_vec() })
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
            &["calibrate".into(), "--config".into(), cfg_path.to_str().unwrap().into()],
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
            "host": "wechat-mac",
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
            &["calibrate".into(), "--config".into(), cfg_path.to_str().unwrap().into(), "--timeout".into(), "3".into()],
            &mut stdout,
            &mut stderr,
            Some(true),
        );
        bot_thread.join().unwrap();
        assert_eq!(code, 0);

        let out_str = String::from_utf8(stdout).unwrap();
        let last_line = out_str.lines().last().unwrap();
        let res: Value = serde_json::from_str(last_line).unwrap();
        assert!(res["candidate_json"].as_str().unwrap().contains("candidate-wechat-4.1.15.json"));
        assert_eq!(res["differences"].as_array().unwrap().len(), 1);
    }

    /// 18. test_control_calibrate_requires_running_bot
    /// 机器人未运行时退出码 1 且 stderr 提示 "机器人未运行：请先 control start"
    #[test]
    fn test_control_calibrate_requires_running_bot() {
        let env = TempDir::new();
        let state_dir = env.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let cfg_path = env.path().join("config.json");
        let cfg = json!({
            "schema_version": 1,
            "host": "wechat-mac",
            "account_id": "fixture",
            "state_dir": state_dir.to_str().unwrap()
        });
        fs::write(&cfg_path, cfg.to_string()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = execute_control(
            &["calibrate".into(), "--config".into(), cfg_path.to_str().unwrap().into()],
            &mut stdout,
            &mut stderr,
            Some(false),
        );

        assert_eq!(code, 1);
        let err_str = String::from_utf8(stderr).unwrap();
        assert!(err_str.contains("机器人未运行：请先 control start"));
    }

    /// 19. test_anchors_measure_the_raw_capture
    /// Worker.swift 中 calibrate 分支调用 measureAnchors(rawImage, scale:) 与 drawAnnotatedLayout(rawImage, scale:)
    #[test]
    fn test_anchors_measure_the_raw_capture() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let worker_swift = fs::read_to_string(manifest_dir.join("native/bot/Worker.swift")).unwrap();

        let cal_idx = worker_swift.find("operation == \"calibrate\"").expect("Worker.swift must have calibrate");
        let branch = &worker_swift[cal_idx..];
        let end_idx = branch.find("return result").expect("must bound calibrate branch");
        let cal_block = &branch[..end_idx];

        assert!(cal_block.contains("measureAnchors(rawImage, scale:"), "必须使用 rawImage 测量");
        assert!(cal_block.contains("drawAnnotatedLayout(rawImage, scale:"), "必须在 rawImage 上绘制标注");
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
        assert_eq!(missing, Vec::<String>::new(), "title_text 等非线锚点不能被算作 missing");
    }

    // -------------------------------------------------------------
    // tests/test_wechat_bot_service.py control 相关 4 项迁移
    // -------------------------------------------------------------

    /// 21. test_default_config_is_in_user_home
    /// 默认配置和 base 目录必须位于用户 HOME 下的 .WeChatRPABot
    #[test]
    fn test_default_config_is_in_user_home() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        assert_eq!(plist, home.join("Library/LaunchAgents/local.wechat-rpa.bot.plist"));
    }

    /// 22. test_status_reads_configured_state_after_config_move
    /// status 命令必须读取配置中指定的 state_dir 目录下的 health.json
    #[test]
    fn test_status_reads_configured_state_after_config_move() {
        let _lock = ENV_LOCK.lock().unwrap();
        let env = TempDir::new();
        let base = env.path().join(".WeChatRPABot");
        fs::create_dir_all(&base).unwrap();

        let custom_state = env.path().join("custom-state");
        fs::create_dir_all(&custom_state).unwrap();

        let config_path = base.join("config.json");
        let cfg = json!({
            "schema_version": 1,
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
        let code = execute_control(
            &["status".into()],
            &mut stdout,
            &mut stderr,
            Some(true),
        );

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
        let _lock = ENV_LOCK.lock().unwrap();
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

    /// 24. test_start_reenables_and_loads_unloaded_task
    /// start 命令依次执行 enable 和 bootstrap
    #[test]
    fn test_start_reenables_and_loads_unloaded_task() {
        let _lock = ENV_LOCK.lock().unwrap();
        let env = TempDir::new();
        let orig_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", env.path()) };

        let mut loaded_fn = || false;

        let mut run_calls = Vec::new();
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

        assert_eq!(code, 0);
        assert_eq!(run_calls.len(), 2);
        assert_eq!(run_calls[0][0], "enable");
        assert_eq!(run_calls[1][0], "bootstrap");

        let out: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(out["status"], "started");
        assert_eq!(out["label"], BOT_LABEL);
    }
}
