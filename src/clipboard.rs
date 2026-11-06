use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// pbcopy/pbpaste decode and encode text by the locale. Started from launchd or Finder the process has
/// no LANG, they fall back to MacRoman and Chinese turns into garbage, so UTF-8 is always set here.
fn pasteboard_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.env("LANG", "en_US.UTF-8").env("LC_ALL", "en_US.UTF-8");
    command
}

/// 获取当前系统剪贴板文本内容
pub async fn get_clipboard() -> Result<Option<String>> {
    let output = Command::from(pasteboard_command("pbpaste"))
        .kill_on_drop(true)
        .output()
        .await
        .context("执行 pbpaste 失败")?;

    if output.status.success() {
        let content = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

/// 将文本写入系统剪贴板
pub async fn set_clipboard(content: &str) -> Result<()> {
    let mut child = Command::from(pasteboard_command("pbcopy"))
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("启动 pbcopy 失败")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .await
            .context("写入 pbcopy stdin 失败")?;
        drop(stdin);
    }

    let status = child.wait().await.context("等待 pbcopy 退出失败")?;
    if !status.success() {
        anyhow::bail!("pbcopy 退出状态异常: {:?}", status.code());
    }

    Ok(())
}

/// 保存文本剪贴板；正常路径异步恢复，取消或提前返回时同步兜底。
/// pbpaste/pbcopy 不保存图片等非文本格式。
pub struct ClipboardGuard {
    original: Option<String>,
}

impl ClipboardGuard {
    pub async fn replace(content: &str) -> Result<Self> {
        // 读取失败时不能覆盖剪贴板；空文本也必须保存和恢复。
        let original = get_clipboard()
            .await?
            .context("无法保存原剪贴板，已停止操作")?;
        let guard = Self {
            original: Some(original),
        };
        set_clipboard(content).await?;
        Ok(guard)
    }

    pub async fn restore(&mut self) -> Result<()> {
        if let Some(original) = &self.original {
            set_clipboard(original).await.context("恢复原剪贴板失败")?;
            self.original = None;
        }
        Ok(())
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        let Some(original) = self.original.take() else {
            return;
        };
        // Drop 无法 await；必须在释放桌面锁之前完成尽力恢复。
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut child = pasteboard_command("pbcopy")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("启动剪贴板恢复兜底失败")?;
            let write_result = child
                .stdin
                .take()
                .context("剪贴板恢复兜底缺少 stdin")
                .and_then(|mut stdin| {
                    stdin
                        .write_all(original.as_bytes())
                        .context("恢复剪贴板写入失败")
                });
            let status = child.wait().context("等待剪贴板恢复兜底失败")?;
            write_result?;
            if !status.success() {
                anyhow::bail!("剪贴板恢复兜底退出状态异常: {:?}", status.code());
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!("恢复原剪贴板兜底失败: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasteboard_tools_always_run_with_utf8_locale() {
        // Device 2026-11-07: launched from launchd/Finder, wechat-mcp had no LANG; pbcopy then read the
        // reply as MacRoman and WeChat received garbled text (and the user's clipboard was mangled).
        for program in ["pbcopy", "pbpaste"] {
            let command = pasteboard_command(program);
            let envs: Vec<(String, String)> = command
                .get_envs()
                .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v.and_then(|v| v.to_str())?.to_string())))
                .collect();
            for key in ["LANG", "LC_ALL"] {
                assert!(
                    envs.contains(&(key.to_string(), "en_US.UTF-8".to_string())),
                    "{program} {key}: {envs:?}"
                );
            }
            assert_eq!(command.get_program(), program);
        }
    }
}
