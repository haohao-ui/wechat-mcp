use anyhow::{bail, Context, Result};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{info, warn};

use crate::clipboard;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateMouseEvent(
        source: *const std::ffi::c_void,
        mouse_type: u32,
        position: CGPoint,
        mouse_button: u32,
    ) -> *mut std::ffi::c_void;
    fn CGEventPost(tap: u32, event: *mut std::ffi::c_void);
    fn CFRelease(cf: *mut std::ffi::c_void);
}

/// 调用 macOS 原生 CoreGraphics 发送鼠标单击事件
pub fn mouse_click(x: f64, y: f64) {
    unsafe {
        let pt = CGPoint { x, y };
        // 1 = kCGEventLeftMouseDown, 2 = kCGEventLeftMouseUp, 0 = kCGMouseButtonLeft
        let down = CGEventCreateMouseEvent(std::ptr::null(), 1, pt, 0);
        let up = CGEventCreateMouseEvent(std::ptr::null(), 2, pt, 0);
        CGEventPost(0, down); // 0 = kCGHIDEventTap
        std::thread::sleep(std::time::Duration::from_millis(60));
        CGEventPost(0, up);
        if !down.is_null() {
            CFRelease(down);
        }
        if !up.is_null() {
            CFRelease(up);
        }
    }
}

/// 执行一段 AppleScript 代码，通过 stdin 输入以避免 shell 转义与参数长度问题
async fn run_applescript(script: &str) -> Result<String> {
    let mut child = Command::new("osascript")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("启动 osascript 进程失败")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .await
            .context("向 osascript stdin 写入脚本失败")?;
        drop(stdin);
    }

    let output = child.wait_with_output().await.context("等待 osascript 执行失败")?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        bail!("AppleScript 执行错误: {}", err_msg.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// 检查微信客户端是否正在运行
pub async fn is_wechat_running() -> Result<bool> {
    let script = r#"application "WeChat" is running"#;
    let res = run_applescript(script).await?;
    Ok(res == "true")
}

/// 获取微信当前窗口屏幕坐标原点 (x, y)
pub async fn get_wechat_window_pos() -> Result<(f64, f64)> {
    let script = r#"
        tell application "System Events"
            tell process "WeChat"
                tell window 1
                    return position
                end tell
            end tell
        end tell
    "#;
    let res = run_applescript(script).await?;
    let parts: Vec<&str> = res.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        let x: f64 = parts[0].parse().unwrap_or(200.0);
        let y: f64 = parts[1].parse().unwrap_or(100.0);
        Ok((x, y))
    } else {
        bail!("无法解析微信窗口坐标: {}", res);
    }
}

/// 唤起并聚焦微信窗口
pub async fn focus_wechat() -> Result<()> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    let script = r#"
        tell application "WeChat" to activate
        delay 0.3
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
            end tell
        end tell
    "#;
    run_applescript(script).await?;
    Ok(())
}

/// 发送微信消息给指定联系人或群组
pub async fn send_message(recipient: &str, message: &str) -> Result<()> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    if recipient.trim().is_empty() {
        bail!("接收人不能为空");
    }

    if message.trim().is_empty() {
        bail!("消息内容不能为空");
    }

    info!("准备向 [{}] 发送微信消息...", recipient);

    // 1. 暂存当前系统剪贴板
    let saved_clipboard = clipboard::get_clipboard().await.ok().flatten();

    // 2. 将联系人写入剪贴板
    clipboard::set_clipboard(recipient).await?;

    // 3. 激活微信并执行搜索
    let search_script = r#"
        tell application "WeChat" to activate
        delay 0.4
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                try
                    click menu item "搜索" of menu "编辑" of menu bar item "编辑" of menu bar 1
                on error
                    keystroke "f" using {command down}
                end try
                delay 0.4
                keystroke "a" using {command down}
                keystroke "v" using {command down}
            end tell
        end tell
    "#;
    run_applescript(search_script).await?;

    // 4. 等待搜索结果渲染完成（0.8s）
    tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

    // 5. 动态获取窗口坐标并模拟点击第一条搜索结果
    let (win_x, win_y) = get_wechat_window_pos().await.unwrap_or((200.0, 100.0));
    let click_x = win_x + 160.0;
    let click_y = win_y + 110.0;
    info!("模拟点击第一条搜索结果，坐标: ({}, {})", click_x, click_y);
    mouse_click(click_x, click_y);

    // 6. 等待聊天会话面板完全打开并获取焦点（0.6s）
    tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;

    // 7. 将消息正文写入剪贴板并粘贴回车发送
    clipboard::set_clipboard(message).await?;

    let send_script = r#"
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                keystroke "v" using {command down}
                delay 0.3
                key code 36
            end tell
        end tell
    "#;
    run_applescript(send_script).await?;

    // 8. 恢复先前备份的剪贴板内容
    if let Some(old_content) = saved_clipboard {
        if let Err(e) = clipboard::set_clipboard(&old_content).await {
            warn!("恢复原剪贴板内容失败: {:?}", e);
        }
    }

    info!("已成功触发向 [{}] 发送消息", recipient);
    Ok(())
}
