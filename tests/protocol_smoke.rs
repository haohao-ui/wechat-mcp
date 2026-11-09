//! Protocol smoke integration tests.
//! Migrated from `tests/protocol_smoke.py`.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

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

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
}

impl McpProcess {
    fn spawn(binary: &Path, cwd: &Path, env: &HashMap<String, String>) -> Self {
        let mut cmd = Command::new(binary);
        cmd.current_dir(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

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
        self.send_raw(&format!("{}\n", s));
    }

    fn send_raw(&mut self, raw: &str) {
        let stdin = self.stdin.as_mut().expect("stdin closed");
        stdin.write_all(raw.as_bytes()).expect("write to stdin");
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

impl Drop for McpProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn check_peer_lock<F>(binary: &Path, directory: &Path, env: &HashMap<String, String>, release: F)
where
    F: FnOnce(),
{
    let mut peer = McpProcess::spawn(binary, directory, env);

    peer.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2024-11-05"}
    }));
    let resp1 = peer.receive(Duration::from_secs(2)).expect("peer resp 1");
    assert_eq!(resp1["id"], 1);

    peer.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "focus_wechat", "arguments": {}}
    }));
    peer.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "ping"
    }));

    let resp3 = peer.receive(Duration::from_secs(2)).expect("peer resp 3");
    assert_eq!(
        resp3["id"], 3,
        "Peer executed UI while another process owned the desktop"
    );

    // Ensure id 2 did not complete yet (lock is exclusive)
    let early_resp = peer.receive(Duration::from_millis(150));
    assert!(
        early_resp.is_none(),
        "UI lock was not exclusive: {:?}",
        early_resp
    );

    release();

    let resp2 = peer
        .receive(Duration::from_secs(3))
        .expect("peer resp 2 after release");
    assert_eq!(resp2["id"], 2);
    assert_eq!(resp2["result"]["isError"], false, "{:?}", resp2);

    peer.close_stdin();
    let status = peer.wait(Duration::from_secs(3));
    assert!(status.success(), "peer did not exit cleanly: {:?}", status);
}

fn setup_test_directory(directory: &Path) {
    let bin_dir = directory.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let osascript = directory.join("osascript");
    fs::write(
        &osascript,
        "#!/bin/sh\n/bin/cat >/dev/null\nprintf \"true\\n\"\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&osascript).unwrap().permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&osascript, perms).unwrap();

    let screencapture = directory.join("screencapture");
    fs::write(&screencapture, "#!/bin/sh\nexit 1\n").unwrap();
    let mut perms = fs::metadata(&screencapture).unwrap().permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&screencapture, perms).unwrap();

    let wechat_click = bin_dir.join("wechat_click");
    fs::write(&wechat_click, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&wechat_click).unwrap().permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&wechat_click, perms).unwrap();

    let wechat_window = bin_dir.join("wechat_window");
    fs::write(
        &wechat_window,
        r#"#!/bin/sh
if [ ! -f "$MESSAGE_TEST_HOLD" ]; then
    : > "$MESSAGE_TEST_HOLD"
    exec /bin/sleep 60
fi
if [ "$1" = "--screens" ]; then
    printf '%s\n' '[{"x":0,"y":0,"width":1920,"height":1080}]'
else
    printf '%s\n' '{"window_id":1,"x":0,"y":0,"width":950,"height":680,"on_screen":true}'
fi
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&wechat_window).unwrap().permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&wechat_window, perms).unwrap();

    let accounts_json = directory.join("accounts.json");
    let accounts_content = serde_json::json!({
        "accounts": [{
            "platform": "wechat",
            "account_id": "local",
            "adapter": "wechat_desktop"
        }]
    });
    fs::write(
        &accounts_json,
        serde_json::to_string(&accounts_content).unwrap(),
    )
    .unwrap();
}

fn run_protocol_smoke_case(version: &str) {
    let binary = wechat_mcp_binary();
    let temp = TempDir::new("protocol_smoke");
    let directory = &temp.path;
    setup_test_directory(directory);

    let mut env: HashMap<String, String> = std::env::vars().collect();
    let hold_path = directory.join(format!("hold-{}", version));
    env.insert(
        "MESSAGE_TEST_HOLD".into(),
        hold_path.to_str().unwrap().into(),
    );
    let old_path = env.get("PATH").cloned().unwrap_or_default();
    env.insert(
        "PATH".into(),
        format!("{}:{}", directory.display(), old_path),
    );
    env.insert(
        "MESSAGE_GATEWAY_DB".into(),
        directory.join("service.sqlite3").to_str().unwrap().into(),
    );
    env.insert(
        "MESSAGE_GATEWAY_CONFIG".into(),
        directory.join("accounts.json").to_str().unwrap().into(),
    );

    let mut process = McpProcess::spawn(&binary, directory, &env);

    // 1. initialize
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "isolated-audit", "version": "1"}
        }
    }));
    let resp1 = process.receive(Duration::from_secs(2)).expect("receive 1");
    assert_eq!(resp1["result"]["protocolVersion"], version);

    // notifications/initialized
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // 2. list_accounts
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "list_accounts", "arguments": {}}
    }));
    let resp2 = process.receive(Duration::from_secs(2)).expect("receive 2");
    let res2 = &resp2["result"];
    assert_eq!(
        res2.get("structuredContent").is_some(),
        version == "2025-06-18"
    );
    let data_str = res2["content"][0]["text"].as_str().expect("text");
    let data: Value = serde_json::from_str(data_str).expect("parse accounts data");
    assert_eq!(
        data["accounts"][0]["key"],
        serde_json::json!({"platform": "wechat", "account_id": "local"})
    );

    // 3. wait_for_message
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "wait_for_message",
            "arguments": {
                "platform": "wechat",
                "account_id": "local",
                "timeout_seconds": 60
            }
        }
    }));

    let deadline = Instant::now() + Duration::from_secs(3);
    while !hold_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        hold_path.exists(),
        "Isolated UI operation did not acquire the desktop lock"
    );

    // 4. ping while UI operation holds lock
    let start_ping = Instant::now();
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "ping"
    }));
    let resp4 = process.receive(Duration::from_secs(2)).expect("receive 4");
    assert_eq!(resp4["result"], serde_json::json!({}));
    assert!(
        start_ping.elapsed() < Duration::from_secs(1),
        "ping blocked by UI polling"
    );

    // check peer lock and cancel request 3
    check_peer_lock(&binary, directory, &env, || {
        process.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": 3}
        }));
    });

    // 5. list_conversations
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "list_conversations",
            "arguments": {"platform": "wechat", "account_id": "local"}
        }
    }));
    let resp5 = process.receive(Duration::from_secs(3)).expect("receive 5");
    let res5 = &resp5["result"];
    assert_eq!(res5["isError"], true, "{:?}", res5);
    let text5 = res5["content"][0]["text"].as_str().unwrap();
    assert!(text5.contains("截图微信窗口失败"), "{:?}", res5);

    // 6. unknown method
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "unknown_method"
    }));
    let resp6 = process.receive(Duration::from_secs(2)).expect("receive 6");
    assert_eq!(resp6["error"]["code"], -32601);

    // 7. invalid jsonrpc version
    process.send_raw("{\"jsonrpc\":\"9.9\",\"id\":7,\"method\":\"ping\"}\n");
    let resp7 = process.receive(Duration::from_secs(2)).expect("receive 7");
    assert_eq!(resp7["error"]["code"], -32600);

    // parse error
    process.send_raw("{\n");
    let resp_parse = process
        .receive(Duration::from_secs(2))
        .expect("receive parse error");
    assert_eq!(resp_parse["error"]["code"], -32700);

    // 8. wait_for_events, then close stdin
    process.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "wait_for_events",
            "arguments": {
                "platform": "wechat",
                "account_id": "local",
                "timeout_seconds": 60
            }
        }
    }));
    process.close_stdin();
    let status = process.wait(Duration::from_secs(3));
    assert!(status.success(), "process exited with code: {:?}", status);

    let unexpected = process.receive(Duration::from_millis(200));
    assert!(
        unexpected.is_none(),
        "cancelled request unexpectedly produced a response: {:?}",
        unexpected
    );
}

#[test]
fn test_protocol_smoke_2024_11_05() {
    run_protocol_smoke_case("2024-11-05");
}

#[test]
fn test_protocol_smoke_2025_06_18() {
    run_protocol_smoke_case("2025-06-18");
}
