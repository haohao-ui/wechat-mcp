//! Optional visual feedback. Its failure must never change a desktop action's result.
use std::{path::PathBuf, process::Stdio, sync::OnceLock, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

struct Session {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Session {
    fn spawn(path: PathBuf) -> Result<Self> {
        let mut child = Command::new(path)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self {
            input: child.stdin.take().context("overlay stdin missing")?,
            output: BufReader::new(child.stdout.take().context("overlay stdout missing")?),
            _child: child,
        })
    }

    async fn exchange(&mut self, event: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&event)?;
        bytes.push(b'\n');
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        let mut reply = Vec::new();
        // The protocol has a fixed acknowledgement; cap even a faulty helper's output.
        while reply.len() < 32 {
            let byte = self.output.fill_buf().await?;
            let Some(&byte) = byte.first() else {
                bail!("overlay closed")
            };
            self.output.consume(1);
            reply.push(byte);
            if byte == b'\n' {
                break;
            }
        }
        if reply != b"{\"ok\":true}\n" {
            bail!("invalid overlay acknowledgement")
        }
        Ok(())
    }
}

#[derive(Default)]
struct State {
    session: Option<Session>,
    failed: bool,
}

fn helper_path() -> Option<PathBuf> {
    if std::env::var("WECHAT_MCP_OPERATION_OVERLAY").is_ok_and(|v| v == "0") {
        return None;
    }
    let beside = std::env::current_exe()
        .ok()?
        .parent()?
        .join("wechat_overlay");
    [beside, PathBuf::from("bin/wechat_overlay")]
        .into_iter()
        .find(|p| p.is_file())
}

async fn emit(event: Value, start: bool) -> bool {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    let mut state = STATE.get_or_init(Mutex::default).lock().await;
    if state.failed {
        return false;
    }
    if state.session.is_none() {
        if !start {
            return false;
        }
        let Some(path) = helper_path() else {
            return false;
        };
        match Session::spawn(path) {
            Ok(session) => state.session = Some(session),
            Err(error) => {
                tracing::warn!(%error, "operation overlay unavailable");
                state.failed = true;
                return false;
            }
        }
    }
    // Cancellation drops the child, so an old acknowledgement cannot satisfy a later hide.
    let mut session = state.session.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), session.exchange(event)).await;
    if matches!(result, Ok(Ok(()))) {
        state.session = Some(session);
    } else {
        tracing::warn!("operation overlay failed; disabled for this process");
        state.failed = true;
    }
    matches!(result, Ok(Ok(())))
}

pub(crate) async fn window(x: f64, y: f64, width: f64, height: f64) {
    emit(
        json!({"event":"window", "x":x,"y":y,"width":width,"height":height}),
        true,
    )
    .await;
}

pub(crate) async fn click(x: f64, y: f64, pressed: bool) {
    emit(
        json!({"event": if pressed { "click" } else { "move" },"x":x,"y":y}),
        false,
    )
    .await;
}

/// Hide before acquiring pixels, including legacy screen-region screenshots.
pub(crate) async fn hide() -> bool {
    emit(json!({"event":"hide"}), false).await
}

pub(crate) async fn capture<F: std::future::Future>(future: F) -> F::Output {
    if hide().await {
        // Allow WindowServer to apply orderOut before a region screenshot begins.
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let result = future.await;
    emit(json!({"event":"resume"}), false).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(body: &str) -> Session {
        let mut command = Command::new("/bin/sh");
        let mut child = command
            .args(["-c", body])
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Session {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            _child: child,
        }
    }

    #[tokio::test]
    async fn visual_protocol_acknowledges_hide_before_capture_and_rejects_bad_replies() {
        let mut session = fake(
            r#"while IFS= read -r line; do
                case "$line" in
                    *'"event":"window"'*|*'"event":"hide"'*|*'"event":"resume"'*) printf '%s\n' '{"ok":true}' ;;
                    *) exit 1 ;;
                esac
            done"#,
        );
        for event in ["window", "hide", "resume"] {
            session.exchange(json!({"event":event})).await.unwrap();
        }
        let mut bad = fake("IFS= read -r line; printf '%100s\\n' ''");
        assert!(bad.exchange(json!({"event":"hide"})).await.is_err());
        let mut closed = fake("IFS= read -r line");
        assert!(closed.exchange(json!({"event":"hide"})).await.is_err());
    }
}
