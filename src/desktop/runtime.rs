//! Bounded, serialized IPC. A cancelled exchange drops its child before reuse.
use std::{path::PathBuf, sync::OnceLock, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

use super::protocol::{MAX_FRAME_BYTES, Region, Response, VERSION};
use crate::domain::observation::Snapshot;

enum Mode {
    Legacy,
    Stream(PathBuf),
    Invalid(String),
}

pub(crate) struct DesktopRuntime {
    mode: Mode,
    session: Mutex<Option<Session>>,
}

struct Session {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    request_id: u64,
}

impl Session {
    fn spawn(mut command: Command) -> Result<Self> {
        let mut child = command
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .context("cannot start desktop helper; run scripts/build.sh")?;
        Ok(Self {
            input: child.stdin.take().context("helper stdin missing")?,
            output: BufReader::new(child.stdout.take().context("helper stdout missing")?),
            _child: child,
            request_id: 0,
        })
    }

    async fn exchange(&mut self, operation: &str, params: Value) -> Result<Value> {
        self.request_id = self
            .request_id
            .checked_add(1)
            .context("helper request ID exhausted")?;
        let mut request = serde_json::to_vec(&json!({
            "protocol_version":VERSION,"id":self.request_id,
            "operation":operation,"params":params,
        }))?;
        if request.len() > MAX_FRAME_BYTES {
            bail!("desktop request too large");
        }
        request.push(b'\n');
        self.input.write_all(&request).await?;
        self.input.flush().await?;
        let bytes = read_frame(&mut self.output).await?;
        serde_json::from_slice::<Response>(&bytes)
            .context("invalid desktop helper JSON")?
            .into_result(self.request_id)
    }
}

async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            bail!("desktop helper closed before response");
        }
        let end = buffer.iter().position(|&b| b == b'\n');
        let count = end.map_or(buffer.len(), |position| position + 1);
        if frame.len() + count > MAX_FRAME_BYTES {
            bail!("desktop helper response too large");
        }
        frame.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(frame);
        }
    }
}

fn helper_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("WECHAT_MCP_DESKTOP_HELPER") {
        return PathBuf::from(explicit);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let sibling = parent.join("wechat_runtime");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("bin/wechat_runtime")
}

impl DesktopRuntime {
    fn from_environment() -> Self {
        // Keep the proven driver until the operator opts into the native stream.
        let mode = match std::env::var("WECHAT_MCP_DESKTOP_RUNTIME")
            .as_deref()
            .unwrap_or("legacy")
        {
            "legacy" => Mode::Legacy,
            "stream" => Mode::Stream(helper_path()),
            other => Mode::Invalid(format!("invalid WECHAT_MCP_DESKTOP_RUNTIME: {other}")),
        };
        Self {
            mode,
            session: Mutex::new(None),
        }
    }

    async fn request(&self, operation: &str, params: Value) -> Result<Option<Value>> {
        let path = match &self.mode {
            Mode::Legacy => return Ok(None),
            Mode::Stream(path) => path,
            Mode::Invalid(error) => bail!("{error}"),
        };
        let mut slot = self.session.lock().await;
        // Taking ownership ensures an aborted request cannot leave a half-read reply.
        let mut session = match slot.take() {
            Some(session) => session,
            None => Session::spawn(Command::new(path))?,
        };
        let result =
            tokio::time::timeout(Duration::from_secs(20), session.exchange(operation, params))
                .await
                .context("desktop helper timed out; observation unavailable")??;
        *slot = Some(session);
        Ok(Some(result))
    }

    pub async fn snapshot(&self, region: Region, refresh: bool) -> Result<Option<Snapshot>> {
        let params = json!({"region":region,"refresh":refresh});
        let Some(value) = super::overlay::capture(self.request("snapshot", params)).await? else {
            return Ok(None);
        };
        let parsed = serde_json::from_value::<Snapshot>(value)
            .context("invalid desktop snapshot")
            .and_then(|snapshot| {
                snapshot.validate()?;
                Ok(snapshot)
            });
        match parsed {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(error) => {
                self.shutdown().await;
                Err(error)
            }
        }
    }

    pub async fn diagnostics(&self) -> Result<Value> {
        self.request("diagnostics", json!({}))
            .await?
            .context("native diagnostics require WECHAT_MCP_DESKTOP_RUNTIME=stream")
    }

    pub async fn shutdown(&self) {
        self.session.lock().await.take();
    }
}

pub(crate) fn shared() -> &'static DesktopRuntime {
    static RUNTIME: OnceLock<DesktopRuntime> = OnceLock::new();
    RUNTIME.get_or_init(DesktopRuntime::from_environment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_preserves_extra_replies_and_rejects_truncation_and_oversize() {
        let mut reader = BufReader::new(&b"first\nsecond\n"[..]);
        assert_eq!(read_frame(&mut reader).await.unwrap(), b"first\n");
        assert_eq!(read_frame(&mut reader).await.unwrap(), b"second\n");
        assert!(read_frame(&mut reader).await.is_err());
        assert!(
            read_frame(&mut BufReader::new(&b"partial"[..]))
                .await
                .is_err()
        );
        let data = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert!(
            read_frame(&mut BufReader::new(data.as_slice()))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn child_is_reused_for_complete_exchanges() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "read -r a; printf '%s\\n' '{\"protocol_version\":1,\"id\":1,\"ok\":true,\"result\":{\"n\":1}}'; read -r b; printf '%s\\n' '{\"protocol_version\":1,\"id\":2,\"ok\":true,\"result\":{\"n\":2}}'"]);
        let mut session = Session::spawn(command).unwrap();
        assert_eq!(
            session.exchange("diagnostics", json!({})).await.unwrap()["n"],
            1
        );
        assert_eq!(
            session.exchange("diagnostics", json!({})).await.unwrap()["n"],
            2
        );
    }

    #[tokio::test]
    async fn helper_errors_are_not_empty_successes() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "read -r a; printf '%s\\n' '{\"protocol_version\":1,\"id\":1,\"ok\":false,\"error\":\"permission_missing\"}'"]);
        let mut session = Session::spawn(command).unwrap();
        assert!(
            session
                .exchange("snapshot", json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("permission_missing")
        );
    }

    #[tokio::test]
    async fn cancelled_exchange_drops_session_before_the_next_request() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "read -r request; exec /bin/sleep 60"]);
        let runtime = DesktopRuntime {
            mode: Mode::Stream(PathBuf::from("/no-such-desktop-helper")),
            session: Mutex::new(Some(Session::spawn(command).unwrap())),
        };
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                runtime.request("snapshot", json!({}))
            )
            .await
            .is_err()
        );
        assert!(runtime.session.lock().await.is_none());
        assert!(runtime.request("diagnostics", json!({})).await.is_err());
    }
}
