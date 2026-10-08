use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// 获取当前系统剪贴板文本内容
pub async fn get_clipboard() -> Result<Option<String>> {
    let output = Command::new("pbpaste")
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
    let mut child = Command::new("pbcopy")
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
