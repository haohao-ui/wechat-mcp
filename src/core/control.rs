use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::core::config::BotConfig;
use crate::core::state::write_private;

unsafe extern "C" {
    fn getuid() -> u32;
}

pub fn locations() -> (PathBuf, PathBuf, PathBuf) {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
    let base = home.join(".WeChatRPABot");
    let config = base.join("config.json");
    let plist = home.join("Library/LaunchAgents/local.wechat-rpa.bot.plist");
    (base, config, plist)
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
    if args.is_empty() {
        let _ = writeln!(stderr, "control: missing action");
        return 2;
    }

    let action = &args[0];
    if action == "-h" || action == "--help" {
        let _ = writeln!(
            stdout,
            "Usage: wechat-mcp control <status|pause|resume|resume-session|config-check|calibrate> [--config <path>]"
        );
        return 0;
    }
    let (base, default_config_path, _) = locations();

    let is_loaded = loaded_override.unwrap_or_else(loaded);

    match action.as_str() {
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
            let state_dir = if config_path.exists() {
                if let Ok(cfg) = BotConfig::load(&config_path) {
                    cfg.state_dir
                } else {
                    base.join("state")
                }
            } else {
                base.join("state")
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
            let state_dir = if config_path.exists() {
                if let Ok(cfg) = BotConfig::load(&config_path) {
                    cfg.state_dir
                } else {
                    base.join("state")
                }
            } else {
                base.join("state")
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
            let state_dir = if config_path.exists() {
                if let Ok(cfg) = BotConfig::load(&config_path) {
                    cfg.state_dir
                } else {
                    base.join("state")
                }
            } else {
                base.join("state")
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
            let state_dir = if config_path.exists() {
                if let Ok(cfg) = BotConfig::load(&config_path) {
                    cfg.state_dir
                } else {
                    base.join("state")
                }
            } else {
                base.join("state")
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
        fs::write(&handoff, r#"{"recipient":"示例群"}"#).unwrap();

        let outcome = resume_session_with(&sock, &state_dir, "示例群").unwrap();
        let request = server.join().unwrap();
        let _ = fs::remove_file(&sock);
        assert_eq!(outcome, ResumeOutcome::Core);
        assert_eq!(request["op"], "resume_session");
        assert_eq!(request["recipient"], "示例群");
        assert!(handoff.exists(), "the running core owns the marker; control must not delete it behind its back");
    }

    #[test]
    fn test_resume_session_without_core_removes_only_matching_marker() {
        let td = TempDir::new();
        let state_dir = td.path().join("state");
        let mine = state_dir.join("sessions").join("c_a").join("handoff.json");
        let other = state_dir.join("sessions").join("c_b").join("handoff.json");
        for (path, name) in [(&mine, "示例群"), (&other, "示例联系人甲")] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json!({"recipient": name}).to_string()).unwrap();
        }
        let outcome = resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "示例群").unwrap();
        assert_eq!(outcome, ResumeOutcome::MarkerRemoved(1));
        assert!(!mine.exists());
        assert!(other.exists());
        let none = resume_session_with(Path::new("/tmp/wcc-absent-core.sock"), &state_dir, "示例群").unwrap();
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
}
