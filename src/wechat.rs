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

fn get_click_binary_path() -> std::path::PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let candidate = parent.join("wechat_click");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    let local = std::path::PathBuf::from("bin/wechat_click");
    if local.exists() {
        return local;
    }
    std::path::PathBuf::from("/path/to/wechat-mcp/bin/wechat_click")
}

/// 调用原生工具或 macOS 原生 CoreGraphics 发送鼠标单击事件
pub fn mouse_click(x: f64, y: f64) {
    let click_bin = get_click_binary_path();
    info!("执行鼠标点击: ({}, {}) [工具: {:?}, 是否存在: {}]", x, y, click_bin, click_bin.exists());
    if click_bin.exists() {
        let status = std::process::Command::new(&click_bin)
            .args([&x.to_string(), &y.to_string()])
            .status();
        info!("wechat_click 执行结果: {:?}", status);
        return;
    }

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

/// 获取微信当前主窗口屏幕位置与尺寸 (x, y, width, height)
pub async fn get_wechat_window_rect() -> Result<(f64, f64, f64, f64)> {
    let script = r#"
        tell application "System Events"
            tell process "WeChat"
                set targetWin to missing value
                repeat with w in (every window)
                    set sz to size of w
                    if (item 1 of sz > 400) and (item 2 of sz > 300) then
                        set targetWin to w
                        exit repeat
                    end if
                end repeat
                if targetWin is missing value then
                    set targetWin to window 1
                end if
                set pos to position of targetWin
                set sz to size of targetWin
                return (item 1 of pos as string) & "," & (item 2 of pos as string) & "," & (item 1 of sz as string) & "," & (item 2 of sz as string)
            end tell
        end tell
    "#;
    let res = run_applescript(script).await?;
    let parts: Vec<&str> = res.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 4 {
        let x: f64 = parts[0].parse().unwrap_or(100.0);
        let y: f64 = parts[1].parse().unwrap_or(100.0);
        let w: f64 = parts[2].parse().unwrap_or(950.0);
        let h: f64 = parts[3].parse().unwrap_or(680.0);
        Ok((x, y, w, h))
    } else {
        bail!("无法解析微信窗口坐标与大小: {}", res);
    }
}

/// 获取微信当前窗口屏幕坐标原点 (x, y)
#[allow(dead_code)]
pub async fn get_wechat_window_pos() -> Result<(f64, f64)> {
    let (x, y, _, _) = get_wechat_window_rect().await?;
    Ok((x, y))
}

/// 唤起微信窗口并确保处于安全的标准主屏坐标与尺寸 (避免多屏负坐标导致点击失效)
pub async fn focus_wechat() -> Result<(f64, f64, f64, f64)> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    let script = r#"
        tell application "WeChat" to activate
        delay 0.15
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                keystroke "1" using {command down}
                delay 0.1
                repeat with w in (every window)
                    set sz to size of w
                    if (item 1 of sz > 400) and (item 2 of sz > 300) then
                        set position of w to {100, 100}
                        set size of w to {950, 680}
                        exit repeat
                    end if
                end repeat
            end tell
        end tell
    "#;

    run_applescript(script).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    Ok((100.0, 100.0, 950.0, 680.0))
}

/// 切换并打开指定联系人/群聊的会话窗口 (优先左侧最近会话列表 OCR 直达，fallback 到搜索栏)
pub async fn open_chat_conversation(recipient: &str) -> Result<()> {
    if recipient.trim().is_empty() {
        bail!("接收人不能为空");
    }

    let (win_x, win_y, _win_w, win_h) = focus_wechat().await?;
    info!("正在定位联系人/群聊 [{}]...", recipient);

    // 方案一：优先通过 Vision OCR 扫描左侧会话列表，实现无感精准直达
    let list_crop_x = (win_x + 65.0).round() as i64;
    let list_crop_y = (win_y + 60.0).round() as i64;
    let list_crop_w = 250i64;
    let list_crop_h = (win_h - 60.0).round() as i64;

    let tmp_list_img = format!("/tmp/wechat_list_{}.png", std::process::id());
    let cap_res = Command::new("screencapture")
        .args([
            "-R",
            &format!("{},{},{},{}", list_crop_x, list_crop_y, list_crop_w, list_crop_h),
            "-x",
            &tmp_list_img,
        ])
        .status()
        .await;

    let mut found_in_list = false;
    if let Ok(st) = cap_res {
        if st.success() {
            let ocr_bin = get_ocr_binary_path();
            if let Ok(ocr_out) = Command::new(&ocr_bin).arg(&tmp_list_img).output().await {
                let _ = std::fs::remove_file(&tmp_list_img);
                if ocr_out.status.success() {
                    let json_str = String::from_utf8_lossy(&ocr_out.stdout);
                    let items: Vec<OcrItem> = serde_json::from_str(json_str.trim()).unwrap_or_default();
                    for item in items {
                        if item.text.contains(recipient) || recipient.contains(&item.text) {
                            // 计算该项在屏幕上的物理 Y 坐标并点击 (稍微向下微调 4px 命中中心)
                            let target_y = list_crop_y as f64 + (item.y * list_crop_h as f64) + 4.0;
                            let target_x = win_x + 150.0;
                            info!("在会话列表中找到 [{}]，直接点击坐标: ({}, {})", recipient, target_x, target_y);
                            mouse_click(target_x, target_y);
                            found_in_list = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    if found_in_list {
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
        return Ok(());
    }

    // 方案二：如果左侧列表未直接包含，通过顶部搜索栏输入
    info!("[{}] 不在最近会话前排，启动全局搜索定位...", recipient);
    let saved_clipboard = clipboard::get_clipboard().await.ok().flatten();
    clipboard::set_clipboard(recipient).await?;

    let search_script = r#"
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                try
                    click menu item "搜索" of menu "编辑" of menu bar item "编辑" of menu bar 1
                on error
                    keystroke "f" using {command down}
                end try
                delay 0.3
                keystroke "a" using {command down}
                keystroke "v" using {command down}
            end tell
        end tell
    "#;
    run_applescript(search_script).await?;

    // 等待搜索结果弹窗（0.7s）
    tokio::time::sleep(tokio::time::Duration::from_millis(700)).await;

    // 模拟点击搜索结果第一条联系人（避开网络搜一搜）
    let click_x = win_x + 230.0;
    let click_y = win_y + 150.0;
    info!("点击搜索结果第一项联系人: ({}, {})", click_x, click_y);
    mouse_click(click_x, click_y);

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    if let Some(old_content) = saved_clipboard {
        let _ = clipboard::set_clipboard(&old_content).await;
    }

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

    // 1. 定位并打开目标会话
    open_chat_conversation(recipient).await?;

    // 2. 获取当前窗口坐标并精准点击聊天输入框中心，确保光标聚焦
    let (win_x, win_y, _win_w, win_h) = get_wechat_window_rect().await?;
    let input_x = win_x + 450.0;
    let input_y = win_y + win_h - 80.0;
    info!("点击聊天输入框激活光标焦点: ({}, {})", input_x, input_y);
    mouse_click(input_x, input_y);
    tokio::time::sleep(tokio::time::Duration::from_millis(350)).await;

    // 3. 暂存剪贴板并将正文粘贴发送
    let saved_clipboard = clipboard::get_clipboard().await.ok().flatten();
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

    // 4. 恢复原剪贴板
    if let Some(old_content) = saved_clipboard {
        if let Err(e) = clipboard::set_clipboard(&old_content).await {
            warn!("恢复原剪贴板内容失败: {:?}", e);
        }
    }

    info!("已成功完成向 [{}] 发送消息", recipient);
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct OcrItem {
    pub text: String,
    pub x: f64,
    pub y: f64,
}

pub fn get_ocr_binary_path() -> std::path::PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let candidate = parent.join("wechat_ocr");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    let local = std::path::PathBuf::from("bin/wechat_ocr");
    if local.exists() {
        return local;
    }
    std::path::PathBuf::from("/path/to/wechat-mcp/bin/wechat_ocr")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatSession {
    pub name: String,
    pub time: String,
    pub preview: String,
    pub click_x: f64,
    pub click_y: f64,
}

/// 获取当前微信左侧最近会话列表及各会话的最新消息预览
pub async fn get_recent_sessions() -> Result<Vec<ChatSession>> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    let (win_x, win_y, _win_w, win_h) = focus_wechat().await?;

    let list_crop_x = (win_x + 65.0).round() as i64;
    let list_crop_y = (win_y + 60.0).round() as i64;
    let list_crop_w = 250i64;
    let list_crop_h = (win_h - 60.0).round() as i64;

    let tmp_file = format!("/tmp/wechat_sessions_{}.png", std::process::id());
    let cap_status = Command::new("screencapture")
        .args([
            "-R",
            &format!("{},{},{},{}", list_crop_x, list_crop_y, list_crop_w, list_crop_h),
            "-x",
            &tmp_file,
        ])
        .status()
        .await
        .context("截图会话列表失败")?;

    if !cap_status.success() {
        bail!("截图左侧会话列表失败");
    }

    let ocr_bin = get_ocr_binary_path();
    let ocr_output = Command::new(&ocr_bin)
        .arg(&tmp_file)
        .output()
        .await
        .context("执行 OCR 解析会话列表失败")?;

    let _ = std::fs::remove_file(&tmp_file);

    if !ocr_output.status.success() {
        let err = String::from_utf8_lossy(&ocr_output.stderr);
        bail!("OCR 会话列表识别失败: {}", err);
    }

    let raw_json = String::from_utf8_lossy(&ocr_output.stdout);
    let mut items: Vec<OcrItem> = serde_json::from_str(raw_json.trim()).unwrap_or_default();
    items.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

    struct RawSessionGroup {
        base_y: f64,
        name: String,
        time: String,
        preview_parts: Vec<String>,
    }

    let mut groups: Vec<RawSessionGroup> = Vec::new();

    for item in items {
        let mut placed = false;
        for g in &mut groups {
            if (item.y - g.base_y).abs() < 0.08 {
                if item.x > 0.55 && g.time.is_empty() {
                    g.time = item.text.clone();
                } else if g.name.is_empty() && item.x <= 0.55 {
                    g.name = item.text.clone();
                } else {
                    g.preview_parts.push(item.text.clone());
                }
                placed = true;
                break;
            }
        }

        if !placed {
            let mut new_g = RawSessionGroup {
                base_y: item.y,
                name: String::new(),
                time: String::new(),
                preview_parts: Vec::new(),
            };
            if item.x > 0.55 {
                new_g.time = item.text;
            } else {
                new_g.name = item.text;
            }
            groups.push(new_g);
        }
    }

    let mut result = Vec::new();
    for g in groups {
        if g.name.trim().is_empty() {
            continue;
        }
        let target_y = list_crop_y as f64 + (g.base_y * list_crop_h as f64) + 6.0;
        let target_x = win_x + 150.0;
        result.push(ChatSession {
            name: g.name.trim().to_string(),
            time: g.time.trim().to_string(),
            preview: g.preview_parts.join(" ").trim().to_string(),
            click_x: target_x,
            click_y: target_y,
        });
    }

    Ok(result)
}

/// 读取指定联系人/群聊（或当前打开窗口）的聊天记录文字（纯净右侧气泡区域）
pub async fn read_chat_messages(recipient: Option<&str>) -> Result<Vec<String>> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    if let Some(target) = recipient {
        if !target.trim().is_empty() {
            open_chat_conversation(target).await?;
        }
    } else {
        focus_wechat().await?;
    }

    let (win_x, win_y, win_w, win_h) = get_wechat_window_rect().await?;

    // 计算右侧真实聊天气泡区域：
    // 微信左侧导航栏 (~65px) + 会话列表栏 (~250px) 共约 315px
    // 避开顶部标题栏 (~60px) 与底部输入框发送区 (~120px)
    let chat_x = (win_x + 315.0).round() as i64;
    let chat_y = (win_y + 60.0).round() as i64;
    let chat_w = (win_w - 325.0).max(300.0).round() as i64;
    let chat_h = (win_h - 180.0).max(200.0).round() as i64;

    info!("读取聊天区域几何坐标: ({}, {}, {}, {})", chat_x, chat_y, chat_w, chat_h);

    let tmp_file = format!("/tmp/wechat_chat_{}.png", std::process::id());

    // 调用系统 screencapture
    let cap_status = Command::new("screencapture")
        .args([
            "-R",
            &format!("{},{},{},{}", chat_x, chat_y, chat_w, chat_h),
            "-x",
            &tmp_file,
        ])
        .status()
        .await
        .context("执行 screencapture 失败")?;

    if !cap_status.success() {
        bail!("截取聊天区域失败");
    }

    let ocr_bin = get_ocr_binary_path();
    let ocr_output = Command::new(&ocr_bin)
        .arg(&tmp_file)
        .output()
        .await
        .context("执行 wechat_ocr 失败")?;

    let _ = std::fs::remove_file(&tmp_file);

    if !ocr_output.status.success() {
        let err = String::from_utf8_lossy(&ocr_output.stderr);
        bail!("Apple Vision OCR 识别失败: {}", err);
    }

    let raw_json = String::from_utf8_lossy(&ocr_output.stdout);
    info!("OCR 识别原始返回: {}", raw_json.trim());
    let items: Vec<OcrItem> = serde_json::from_str(raw_json.trim()).unwrap_or_default();

    let text_lines: Vec<String> = items
        .into_iter()
        .map(|item| item.text.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    Ok(text_lines)
}

/// 长轮询等待新消息到达：
/// - 若 recipient 填入指定好友/群聊：专门监听该会话的新增消息
/// - 若 recipient 为空或 None：全局监听左侧会话列表，任何好友/群聊发来新消息时自动捕获，并点击打开该会话提取完整聊天内容
pub async fn wait_for_new_message(recipient: Option<&str>, timeout_seconds: u64) -> Result<Option<String>> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    let timeout = std::time::Duration::from_secs(timeout_seconds.max(5));
    let start = std::time::Instant::now();

    if let Some(target) = recipient.filter(|r| !r.trim().is_empty()) {
        // === 模式 A：单目标会话长轮询 ===
        info!("开始长轮询监听目标 [{}] 的新消息，超时设定: {} 秒", target, timeout_seconds);
        open_chat_conversation(target).await?;

        let initial_lines = read_chat_messages(None).await.unwrap_or_default();
        let mut known_lines: std::collections::HashSet<String> = initial_lines.into_iter().collect();

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

            let current_lines = read_chat_messages(None).await.unwrap_or_default();
            let mut new_lines = Vec::new();

            for line in &current_lines {
                if !known_lines.contains(line) {
                    new_lines.push(line.clone());
                }
            }

            if !new_lines.is_empty() {
                for line in &new_lines {
                    known_lines.insert(line.clone());
                }
                let combined = new_lines.join("\n");
                info!("监听到 [{}] 的新消息:\n{}", target, combined);
                return Ok(Some(combined));
            }
        }

        Ok(None)
    } else {
        // === 模式 B：全局会话收件箱长轮询（支持任意好友/群聊新消息自动感知） ===
        info!("开始全局长轮询监听微信新消息（收件箱模式），超时设定: {} 秒", timeout_seconds);

        // 获取初始会话列表基线
        let initial_sessions = get_recent_sessions().await.unwrap_or_default();
        // 保存 key: name -> (time, preview)
        let mut session_baseline: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
        for s in initial_sessions {
            session_baseline.insert(s.name, (s.time, s.preview));
        }

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1800)).await;

            let current_sessions = get_recent_sessions().await.unwrap_or_default();
            for s in current_sessions {
                let is_changed = match session_baseline.get(&s.name) {
                    Some((old_time, old_preview)) => {
                        // 时间变动或最新消息预览变动且非空
                        (!s.preview.is_empty() && s.preview != *old_preview) || (!s.time.is_empty() && s.time != *old_time)
                    }
                    None => {
                        // 新冒出的置顶会话且有消息
                        !s.preview.is_empty()
                    }
                };

                if is_changed {
                    info!("检测到联系人 [{}] 发来新消息 (时间: {}, 预览: {})，自动点击查看...", s.name, s.time, s.preview);
                    // 点击该会话打开其聊天窗口
                    mouse_click(s.click_x, s.click_y);
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // 读取该会话的右侧真实气泡记录
                    let full_chat = read_chat_messages(None).await.unwrap_or_default();
                    let recent_bubbles = if full_chat.len() > 5 {
                        full_chat[full_chat.len() - 5..].join("\n")
                    } else {
                        full_chat.join("\n")
                    };

                    let result_msg = format!(
                        "收到来自 [{}] 的新消息：\n【时间】: {}\n【消息摘要】: {}\n【最新聊天气泡】:\n{}",
                        s.name, s.time, s.preview, recent_bubbles
                    );
                    return Ok(Some(result_msg));
                }
            }
        }

        Ok(None)
    }
}

