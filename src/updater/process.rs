//! Bounded system-tool execution for production staging and candidate checks.
use anyhow::{Result, bail};
use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_LIMIT: usize = 256 * 1024;

async fn read_bounded(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok(output);
        }
        if count > limit.saturating_sub(output.len()) {
            bail!("updater_system_tool_output_limit");
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

pub(crate) async fn run(
    program: &PathTool,
    args: &[&OsStr],
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut command = tokio::process::Command::new(program.path());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    let mut child = command.spawn()?;
    let pid = child.id().expect("spawned child has PID");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            read_bounded(stdout, OUTPUT_LIMIT),
            read_bounded(stderr, OUTPUT_LIMIT)
        )
    })
    .await;
    match result {
        Ok(Ok((status, stdout, stderr))) if status.success() => Ok((stdout, stderr)),
        result => {
            // Only this freshly spawned, isolated group is signalled. Reap the
            // direct child as well; no name-based process matching is used.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            let _ = child.wait().await;
            match result {
                Err(_) => bail!("updater_system_tool_timeout: {}", program.path()),
                Ok(Err(error)) => Err(error),
                Ok(Ok((status, _, _))) => bail!(
                    "updater_system_tool_failed: {} exit {:?}",
                    program.path(),
                    status.code()
                ),
            }
        }
    }
}

/// Production callers cannot substitute PATH programs or arbitrary executables.
#[derive(Clone, Copy)]
pub(crate) enum PathTool {
    CodeSign,
    Security,
    Plutil,
    Lipo,
    SwVers,
    #[cfg(test)]
    Shell,
}
impl PathTool {
    fn path(&self) -> &'static str {
        match self {
            Self::CodeSign => "/usr/bin/codesign",
            Self::Security => "/usr/bin/security",
            Self::Plutil => "/usr/bin/plutil",
            Self::Lipo => "/usr/bin/lipo",
            Self::SwVers => "/usr/bin/sw_vers",
            #[cfg(test)]
            Self::Shell => "/bin/sh",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_failure_flood_and_hung_process_group() {
        let tool = PathTool::Shell;
        let run_script = |script: &'static str| async move {
            run(
                &tool,
                &[OsStr::new("-c"), OsStr::new(script)],
                Duration::from_millis(300),
            )
            .await
        };
        assert!(
            run_script("exit 7")
                .await
                .unwrap_err()
                .to_string()
                .contains("failed")
        );
        assert!(
            run_script("yes unbounded-output")
                .await
                .unwrap_err()
                .to_string()
                .contains("output_limit")
        );
        assert!(
            run_script("sleep 30 & wait")
                .await
                .unwrap_err()
                .to_string()
                .contains("timeout")
        );
    }
    #[tokio::test]
    async fn captures_successful_stdout_and_stderr() {
        let output = run(
            &PathTool::Shell,
            &[
                OsStr::new("-c"),
                OsStr::new("printf result; printf diagnostics >&2"),
            ],
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(output, (b"result".to_vec(), b"diagnostics".to_vec()));
    }
}
