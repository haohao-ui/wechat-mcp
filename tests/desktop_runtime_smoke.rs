//! Desktop runtime native-stream and legacy smoke tests.
//! Migrated from `tests/desktop_runtime_smoke.py`.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn has_swiftc() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    Command::new("which")
        .arg("swiftc")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn desktop_selftest_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if !has_swiftc() {
            return None;
        }
        let root = repo_root();
        let out_dir = root.join("target").join("swift_test_bin");
        fs::create_dir_all(&out_dir).ok()?;
        let bin_path = out_dir.join("desktop_smoke_selftest");

        let main_swift = root.join("tests").join("desktop_smoke_selftest").join("main.swift");

        let mut cmd = Command::new("swiftc");
        cmd.arg("-O");
        cmd.arg(&main_swift);
        cmd.arg("-o").arg(&bin_path);

        let output = cmd.output().ok()?;
        if !output.status.success() {
            eprintln!(
                "swiftc compilation of desktop_smoke_selftest failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Some(bin_path)
    })
    .as_ref()
}

fn wechat_mcp_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_wechat-mcp") {
        PathBuf::from(path)
    } else {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.join("target").join("debug").join("wechat-mcp")
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir_name = format!("{}_{}_{}_{}", prefix, std::process::id(), count, nanos);
        let path = std::env::temp_dir().join(dir_name);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct TestProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
}

impl TestProcess {
    fn spawn(binary: &Path, cwd: &Path, env: &HashMap<String, String>) -> Self {
        let mut cmd = Command::new(binary);
        cmd.current_dir(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().expect("failed to spawn wechat-mcp");
        let stdin = child.stdin.take().expect("failed to open stdin");
        let stdout = child.stdout.take().expect("failed to open stdout");

        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(l) = line {
                    if let Ok(val) = serde_json::from_str::<Value>(&l) {
                        let _ = tx.send(val);
                    }
                }
            }
        });

        Self {
            child,
            stdin: Some(stdin),
            rx,
        }
    }

    fn send(&mut self, request: &Value) {
        let s = serde_json::to_string(request).expect("serialize request");
        let stdin = self.stdin.as_mut().expect("stdin closed");
        stdin.write_all(format!("{}\n", s).as_bytes()).expect("write to stdin");
        stdin.flush().expect("flush stdin");
    }

    fn receive(&self, timeout: Duration) -> Option<Value> {
        self.rx.recv_timeout(timeout).ok()
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn wait(&mut self, timeout: Duration) -> std::process::ExitStatus {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Ok(Some(status)) = self.child.try_wait() {
                return status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        self.child.wait().expect("child wait after kill")
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o700);
    fs::set_permissions(path, perms).unwrap();
}

fn run_case(mode: &str) {
    let selftest_bin = match desktop_selftest_bin() {
        Some(b) => b,
        None => {
            eprintln!("swiftc not available, skipping test");
            return;
        }
    };
    let selftest_str = selftest_bin.to_str().unwrap();

    let temp = TempDir::new("desktop_runtime");
    let case_dir = &temp.path;
    let bin_dir = case_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let binary = wechat_mcp_binary();
    let legacy_case = mode.starts_with("legacy_");

    // helper script
    write_executable(
        &case_dir.join("helper"),
        &format!("#!/bin/sh\nexec \"{}\" helper \"$@\"\n", selftest_str),
    );

    // wechat_overlay script
    write_executable(
        &case_dir.join("wechat_overlay"),
        &format!("#!/bin/sh\nexec \"{}\" overlay \"$@\"\n", selftest_str),
    );

    // osascript
    write_executable(
        &case_dir.join("osascript"),
        "#!/bin/sh\n/bin/cat >/dev/null\nprintf 'true\\n'\n",
    );

    // screencapture
    if legacy_case {
        write_executable(
            &case_dir.join("screencapture"),
            &format!("#!/bin/sh\nexec \"{}\" legacy_capture \"$@\"\n", selftest_str),
        );
    } else {
        write_executable(&case_dir.join("screencapture"), "#!/bin/sh\nexit 0\n");
    }

    // pbpaste & pbcopy
    write_executable(&case_dir.join("pbpaste"), "#!/bin/sh\nexit 0\n");
    write_executable(&case_dir.join("pbcopy"), "#!/bin/sh\n/bin/cat >/dev/null\n");

    // bin/wechat_click
    write_executable(
        &bin_dir.join("wechat_click"),
        "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" >> \"$HELPER_CLICKS\"\n",
    );

    // bin/wechat_ocr
    if legacy_case {
        write_executable(
            &bin_dir.join("wechat_ocr"),
            &format!("#!/bin/sh\nexec \"{}\" legacy_ocr \"$@\"\n", selftest_str),
        );
    } else {
        write_executable(
            &bin_dir.join("wechat_ocr"),
            "#!/bin/sh\nprintf '%s\\n' '[{\"text\":\"Alice\",\"x\":0.1,\"y\":0.1}]'\n",
        );
    }

    // bin/wechat_window
    write_executable(
        &bin_dir.join("wechat_window"),
        r#"#!/bin/sh
if [ "$1" = "--screens" ]; then
    printf '%s\n' '[{"x":0,"y":0,"width":1920,"height":1080}]'
else
    printf '%s\n' '{"window_id":1,"x":0,"y":0,"width":950,"height":680,"on_screen":true}'
fi
"#,
    );

    // accounts.json
    let accounts_json = serde_json::json!({
        "accounts": [{
            "platform": "wechat",
            "account_id": "fixture",
            "adapter": "wechat_desktop"
        }]
    });
    fs::write(case_dir.join("accounts.json"), serde_json::to_string(&accounts_json).unwrap()).unwrap();

    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert(
        "WECHAT_MCP_DESKTOP_RUNTIME".into(),
        if legacy_case { "legacy".into() } else { "stream".into() },
    );
    env.insert("WECHAT_MCP_DESKTOP_HELPER".into(), case_dir.join("helper").to_str().unwrap().into());
    env.insert("HELPER_CASE".into(), mode.into());
    env.insert("HELPER_STARTS".into(), case_dir.join("starts").to_str().unwrap().into());
    env.insert("HELPER_CLICKS".into(), case_dir.join("clicks").to_str().unwrap().into());
    env.insert("WECHAT_MCP_OPERATION_OVERLAY".into(), "1".into());
    env.insert("OVERLAY_STATE".into(), case_dir.join("overlay-state").to_str().unwrap().into());
    env.insert("OVERLAY_EVENTS".into(), case_dir.join("overlay-events").to_str().unwrap().into());
    env.insert("MESSAGE_GATEWAY_DB".into(), case_dir.join("service.sqlite3").to_str().unwrap().into());
    env.insert("MESSAGE_GATEWAY_CONFIG".into(), case_dir.join("accounts.json").to_str().unwrap().into());

    let old_path = env.get("PATH").cloned().unwrap_or_default();
    env.insert("PATH".into(), format!("{}:{}", case_dir.display(), old_path));

    let navigation_case = legacy_case || mode.starts_with("search_") || mode == "duplicate_target" || mode == "blank_window";

    let mut requests = Vec::new();
    if navigation_case {
        requests.push(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_messages",
                "arguments": {
                    "platform": "wechat",
                    "account_id": "fixture",
                    "recipient": "Alice"
                }
            }
        }));
    } else {
        for (num, tool) in [
            "list_conversations",
            "list_conversations",
            "get_messages",
        ]
        .into_iter()
        .enumerate()
        {
            requests.push(serde_json::json!({
                "jsonrpc": "2.0",
                "id": num + 1,
                "method": "tools/call",
                "params": {
                    "name": tool,
                    "arguments": {
                        "platform": "wechat",
                        "account_id": "fixture"
                    }
                }
            }));
        }
    }

    let mut process = TestProcess::spawn(&binary, case_dir, &env);
    let mut responses = Vec::new();
    for req in &requests {
        process.send(req);
        let resp = process.receive(Duration::from_secs(10)).expect("receive response");
        responses.push(resp);
    }

    if mode == "normal" {
        let mut call_tool = |number: i64, name: &str, arguments: serde_json::Map<String, Value>| -> Value {
            let mut args = arguments;
            args.insert("platform".into(), serde_json::json!("wechat"));
            args.insert("account_id".into(), serde_json::json!("fixture"));
            process.send(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": number,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": args
                }
            }));
            let resp = process.receive(Duration::from_secs(10)).expect("receive tool response");
            assert_eq!(resp["id"], number, "{:?}", resp);
            let result = &resp["result"];
            assert_eq!(result["isError"], false, "{:?}", result);
            let text = result["content"][0]["text"].as_str().unwrap();
            let parsed: Value = serde_json::from_str(text).unwrap();
            parsed["data"].clone()
        };

        let mut cust_args = serde_json::Map::new();
        cust_args.insert("idempotency_key".into(), serde_json::json!("fixture-customer"));
        cust_args.insert("display_name".into(), serde_json::json!("Alice"));
        cust_args.insert("recipient".into(), serde_json::json!("Alice"));
        let customer = call_tool(4, "create_customer", cust_args);

        let mut cap1_args = serde_json::Map::new();
        cap1_args.insert("customer_id".into(), customer["customer_id"].clone());
        let first = call_tool(5, "capture_customer_context", cap1_args);

        let mut cap2_args = serde_json::Map::new();
        cap2_args.insert("customer_id".into(), customer["customer_id"].clone());
        let second = call_tool(6, "capture_customer_context", cap2_args);

        assert_eq!(first["payload"]["text_continuity"]["state"], "baseline", "{:?}", first);
        let continuity = &second["payload"]["text_continuity"];
        assert_eq!(continuity["previous_event_id"], first["event_id"], "{:?}", second);
        assert_eq!(continuity["state"], "unchanged", "{:?}", second);
        assert_eq!(continuity["new_lines"], serde_json::json!([]), "{:?}", second);
        assert_eq!(continuity["message_continuity_verified"], false, "{:?}", second);
        assert_eq!(second["state"], "recorded", "{:?}", second);
        assert_eq!(second["verified"], false, "{:?}", second);
    }

    process.close_stdin();
    let status = process.wait(Duration::from_secs(5));
    assert!(status.success(), "process failed: {:?}", status);

    if navigation_case {
        let success = mode == "search_success" || mode == "legacy_success";
        let is_err = responses[0]["result"]["isError"].as_bool().unwrap();
        assert_eq!(is_err, !success, "{:?}", responses);

        let clicks = if case_dir.join("clicks").exists() {
            fs::read_to_string(case_dir.join("clicks"))
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty())
                .count()
        } else {
            0
        };

        let expected_clicks = match mode {
            "search_success" => 2,
            "search_readback_mismatch" => 1,
            "duplicate_target" => 0,
            "blank_window" => 0,
            "legacy_success" => 2,
            "legacy_query_mismatch" => 1,
            "legacy_duplicate" => 1,
            _ => 0,
        };
        assert_eq!(clicks, expected_clicks, "clicks for {}", mode);

        if clicks > 0 {
            let events_str = fs::read_to_string(case_dir.join("overlay-events")).unwrap_or_default();
            let events: Vec<&str> = events_str.lines().filter(|l| !l.is_empty()).collect();
            let move_count = events.iter().filter(|&&e| e == "move").count();
            let click_count = events.iter().filter(|&&e| e == "click").count();
            assert_eq!(move_count, clicks, "{:?}", events);
            assert_eq!(click_count, clicks, "{:?}", events);
        }

        if mode == "blank_window" {
            let txt = responses[0]["result"]["content"][0]["text"].as_str().unwrap();
            assert!(txt.contains("无可读文字"), "{:?}", responses);
        }

        if legacy_case {
            assert!(
                !case_dir.join("starts").exists(),
                "Legacy navigation started the stream helper"
            );
        } else {
            let starts = fs::read_to_string(case_dir.join("starts")).unwrap();
            assert_eq!(starts.trim(), "1", "Native navigation unexpectedly restarted helper");
        }
    } else if mode == "normal" {
        assert!(
            responses.iter().all(|r| r["result"]["isError"].as_bool() == Some(false)),
            "{:?}",
            responses
        );
        let outputs: Vec<Value> = responses
            .iter()
            .map(|r| serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap())
            .collect();

        assert_eq!(outputs[0]["data"]["conversations"][0]["display_name"], "Alice");
        assert_eq!(outputs[0]["data"]["conversations"][0]["preview"], "preview");
        assert_eq!(outputs[1]["data"]["capture"]["sequence"], 2);
        assert_eq!(outputs[2]["data"]["text_lines"], serde_json::json!(["收到", "收到"]));
        assert_eq!(outputs[2]["data"]["observation"]["kind"], "ocr_text_lines");
        assert_eq!(outputs[2]["data"]["direction"], "unknown");

        let starts = fs::read_to_string(case_dir.join("starts")).unwrap();
        assert_eq!(starts.trim(), "1", "Helper process was not reused");
    } else {
        assert!(
            responses.iter().all(|r| r["result"]["isError"].as_bool() == Some(true)),
            "{:?}",
            responses
        );
        if mode != "changed_window" {
            let starts = fs::read_to_string(case_dir.join("starts")).unwrap();
            assert_eq!(starts.trim(), "3", "Failed helper session was reused");
        }
    }
}

#[test]
fn test_desktop_runtime_normal() {
    run_case("normal");
}

#[test]
fn test_desktop_runtime_wrong_id() {
    run_case("wrong_id");
}

#[test]
fn test_desktop_runtime_failed() {
    run_case("failed");
}

#[test]
fn test_desktop_runtime_oversized() {
    run_case("oversized");
}

#[test]
fn test_desktop_runtime_bad_bounds() {
    run_case("bad_bounds");
}

#[test]
fn test_desktop_runtime_changed_window() {
    run_case("changed_window");
}

#[test]
fn test_desktop_runtime_search_success() {
    run_case("search_success");
}

#[test]
fn test_desktop_runtime_search_readback_mismatch() {
    run_case("search_readback_mismatch");
}

#[test]
fn test_desktop_runtime_duplicate_target() {
    run_case("duplicate_target");
}

#[test]
fn test_desktop_runtime_blank_window() {
    run_case("blank_window");
}

#[test]
fn test_desktop_runtime_legacy_success() {
    run_case("legacy_success");
}

#[test]
fn test_desktop_runtime_legacy_query_mismatch() {
    run_case("legacy_query_mismatch");
}

#[test]
fn test_desktop_runtime_legacy_duplicate() {
    run_case("legacy_duplicate");
}
