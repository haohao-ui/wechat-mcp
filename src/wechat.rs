use anyhow::{Context, Result, bail};
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
unsafe extern "C" {
    fn CGEventCreateMouseEvent(
        source: *const std::ffi::c_void,
        mouse_type: u32,
        position: CGPoint,
        mouse_button: u32,
    ) -> *mut std::ffi::c_void;
    fn CGEventPost(tap: u32, event: *mut std::ffi::c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *mut std::ffi::c_void);
}

fn get_click_binary_path() -> std::path::PathBuf {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        let candidate = parent.join("wechat_click");
        if candidate.exists() {
            return candidate;
        }
    }
    std::path::PathBuf::from("bin/wechat_click")
}

/// 调用原生工具或 macOS 原生 CoreGraphics 发送鼠标单击事件
pub async fn mouse_click(x: f64, y: f64) -> Result<()> {
    let click_bin = get_click_binary_path();
    info!(
        "执行鼠标点击: ({}, {}) [工具: {:?}, 是否存在: {}]",
        x,
        y,
        click_bin,
        click_bin.exists()
    );
    if click_bin.exists() {
        let status = Command::new(&click_bin)
            .kill_on_drop(true)
            .args([&x.to_string(), &y.to_string()])
            .status()
            .await
            .context("执行 wechat_click 失败")?;
        info!("wechat_click 执行结果: {:?}", status);
        if !status.success() {
            bail!("wechat_click 退出状态异常: {:?}", status.code());
        }
        return Ok(());
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        unsafe {
            let pt = CGPoint { x, y };
            // 1 = kCGEventLeftMouseDown, 2 = kCGEventLeftMouseUp, 0 = kCGMouseButtonLeft
            let down = CGEventCreateMouseEvent(std::ptr::null(), 1, pt, 0);
            let up = CGEventCreateMouseEvent(std::ptr::null(), 2, pt, 0);
            if !down.is_null() && !up.is_null() {
                CGEventPost(0, down); // 0 = kCGHIDEventTap
                std::thread::sleep(std::time::Duration::from_millis(60));
                CGEventPost(0, up);
            }
            if !down.is_null() {
                CFRelease(down);
            }
            if !up.is_null() {
                CFRelease(up);
            }
            if down.is_null() || up.is_null() {
                bail!("创建鼠标点击事件失败");
            }
            Ok(())
        }
    })
    .await
    .context("鼠标点击任务失败")?
}

/// 执行一段 AppleScript 代码，通过 stdin 输入以避免 shell 转义与参数长度问题
async fn run_applescript(script: &str) -> Result<String> {
    let mut child = Command::new("osascript")
        .kill_on_drop(true)
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

    let output = child
        .wait_with_output()
        .await
        .context("等待 osascript 执行失败")?;
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

    if normalize_chat_name(recipient).is_empty() {
        bail!("接收人规范化后不能为空");
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
        .kill_on_drop(true)
        .args([
            "-R",
            &format!(
                "{},{},{},{}",
                list_crop_x, list_crop_y, list_crop_w, list_crop_h
            ),
            "-x",
            &tmp_list_img,
        ])
        .status()
        .await;

    let mut found_in_list = false;
    if let Ok(st) = cap_res
        && st.success()
    {
        let ocr_bin = get_ocr_binary_path();
        if let Ok(ocr_out) = Command::new(&ocr_bin)
            .kill_on_drop(true)
            .arg(&tmp_list_img)
            .output()
            .await
        {
            let _ = std::fs::remove_file(&tmp_list_img);
            if ocr_out.status.success() {
                let json_str = String::from_utf8_lossy(&ocr_out.stdout);
                let items: Vec<OcrItem> = serde_json::from_str(json_str.trim()).unwrap_or_default();
                match match_name_column(&items, recipient) {
                    NameColumnMatch::Unique(item) => {
                        // 计算该项在屏幕上的物理 Y 坐标并点击 (稍微向下微调 4px 命中中心)
                        let target_y = list_crop_y as f64 + (item.y * list_crop_h as f64) + 4.0;
                        let target_x = win_x + 150.0;
                        info!(
                            "在会话列表中找到 [{}]，直接点击坐标: ({}, {})",
                            recipient, target_x, target_y
                        );
                        mouse_click(target_x, target_y).await?;
                        found_in_list = true;
                    }
                    NameColumnMatch::Ambiguous => {
                        warn!("会话名称 [{}] 存在歧义，转入搜索", recipient)
                    }
                    NameColumnMatch::Missing => {}
                }
            }
        }
    }
    let _ = std::fs::remove_file(&tmp_list_img);

    if found_in_list {
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
        return verify_chat_title(recipient).await;
    }

    // 方案二：如果左侧列表未直接包含，通过顶部搜索栏输入
    info!("[{}] 不在最近会话前排，启动全局搜索定位...", recipient);
    let search_result = async {
        let mut clipboard_guard = clipboard::ClipboardGuard::replace(recipient).await?;
        let operation_result = async {
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

            // 搜索结果坐标仅用于尝试打开；标题核验成功前不能发送正文。
            let click_x = win_x + 230.0;
            let click_y = win_y + 150.0;
            info!("尝试点击搜索结果: ({}, {})", click_x, click_y);
            mouse_click(click_x, click_y).await?;

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            verify_chat_title(recipient).await
        }
        .await;
        let restore_result = clipboard_guard.restore().await;
        if let Err(error) = &restore_result {
            warn!("搜索后恢复剪贴板失败，将同步兜底: {error:#}");
        }
        operation_result.and(restore_result)
    }
    .await;
    if search_result.is_err() {
        let close_result = run_applescript(
            r#"
            tell application "System Events"
                tell process "WeChat" to key code 53
            end tell
        "#,
        )
        .await;
        if let Err(error) = close_result {
            warn!("关闭微信搜索框失败: {error:#}");
        }
    }
    search_result
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
    mouse_click(input_x, input_y).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(350)).await;

    // 输入框点击可能改变焦点；在正文进入剪贴板之前再次核验标题。
    verify_chat_title(recipient).await?;

    // 3. 暂存剪贴板并将正文粘贴发送
    let mut clipboard_guard = clipboard::ClipboardGuard::replace(message).await?;

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
    let send_result = run_applescript(send_script).await;

    // 4. 恢复原剪贴板
    let restore_result = clipboard_guard.restore().await;
    if let Err(error) = &restore_result {
        warn!("发送后恢复剪贴板失败，将同步兜底: {error:#}");
    }
    send_result?;
    restore_result?;

    info!("已触发微信发送操作，送达状态未验证");
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct OcrItem {
    pub text: String,
    pub x: f64,
    pub y: f64,
}

fn is_name_boundary(c: char) -> bool {
    c.is_whitespace()
        || c.is_ascii_punctuation()
        || matches!(
            c,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '！'
                | '？'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '【'
                | '】'
                | '《'
                | '》'
                | '…'
                | '—'
                | '·'
        )
}

fn normalize_chat_name(name: &str) -> String {
    // 保留括号到人数后缀检查结束，避免先删掉右括号而留下数字。
    let name = name.trim_matches(|c| is_name_boundary(c) && !matches!(c, '(' | ')' | '（' | '）'));
    let mut without_count = name;
    for (open, close) in [('(', ')'), ('（', '）')] {
        if let Some(body) = name.strip_suffix(close)
            && let Some((prefix, count)) = body.rsplit_once(open)
            && !count.trim().is_empty()
            && count.trim().chars().all(|c| c.is_ascii_digit())
        {
            without_count = prefix;
            break;
        }
    }
    without_count.trim_matches(is_name_boundary).to_string()
}

#[derive(Debug)]
enum NameColumnMatch<'a> {
    Missing,
    Unique(&'a OcrItem),
    Ambiguous,
}

fn match_name_column<'a>(items: &'a [OcrItem], recipient: &str) -> NameColumnMatch<'a> {
    let recipient = normalize_chat_name(recipient);
    if recipient.is_empty() {
        return NameColumnMatch::Missing;
    }
    let mut ordered: Vec<_> = items
        .iter()
        .filter(|item| {
            item.x.is_finite()
                && item.y.is_finite()
                && (0.0..=1.0).contains(&item.x)
                && (0.0..=1.0).contains(&item.y)
        })
        .collect();
    ordered.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));

    // 与最近会话列表使用相同的 y 分组阈值，但每组只保留最上/最左的名称项。
    let mut groups: Vec<(f64, Option<&OcrItem>)> = Vec::new();
    for item in ordered {
        if let Some((_, name)) = groups
            .iter_mut()
            .find(|(base_y, _)| (item.y - base_y).abs() < 0.08)
        {
            if name.is_none() && item.x <= 0.55 {
                *name = Some(item);
            }
        } else {
            groups.push((item.y, (item.x <= 0.55).then_some(item)));
        }
    }
    let mut matches = groups
        .into_iter()
        .filter_map(|(_, name)| name)
        .filter(|item| normalize_chat_name(&item.text) == recipient);
    match (matches.next(), matches.next()) {
        (Some(_), Some(_)) => NameColumnMatch::Ambiguous,
        (Some(item), None) => NameColumnMatch::Unique(item),
        _ => NameColumnMatch::Missing,
    }
}

fn title_matches_recipient(title: &str, recipient: &str) -> bool {
    let recipient = normalize_chat_name(recipient);
    !recipient.is_empty() && normalize_chat_name(title) == recipient
}

fn check_chat_title(title: &str, recipient: &str) -> Result<()> {
    if !title_matches_recipient(title, recipient) {
        bail!(
            "聊天标题核验失败，接收人 [{}]，OCR 标题 [{}]，已停止操作",
            recipient,
            title
        );
    }
    Ok(())
}

/// 核验聊天区顶部标题的显示名称；这不是微信 ID 级身份核验。
pub async fn verify_chat_title(recipient: &str) -> Result<()> {
    let (win_x, win_y, win_w, _) = get_wechat_window_rect().await?;
    if win_w <= 325.0 {
        bail!("微信窗口过窄，无法核验聊天标题");
    }
    let region = format!(
        "{},{},{},60",
        (win_x + 315.0).round() as i64,
        win_y.round() as i64,
        (win_w - 325.0).round() as i64
    );
    let tmp_file = std::env::temp_dir().join(format!("wechat_title_{}.png", uuid::Uuid::new_v4()));
    let result = async {
        let status = Command::new("screencapture")
            .kill_on_drop(true)
            .args(["-R", &region, "-x"])
            .arg(&tmp_file)
            .status()
            .await
            .context("截取聊天标题栏失败")?;
        if !status.success() {
            bail!("截取聊天标题栏失败，已停止操作");
        }
        let output = Command::new(get_ocr_binary_path())
            .kill_on_drop(true)
            .arg(&tmp_file)
            .output()
            .await
            .context("执行聊天标题 OCR 失败")?;
        if !output.status.success() {
            bail!(
                "聊天标题 OCR 失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let mut items: Vec<OcrItem> =
            serde_json::from_slice(&output.stdout).context("聊天标题 OCR 返回不是有效 JSON")?;
        items.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        // 不在任意 OCR 碎片里找名字；整个标题文本必须相等，识别不全则拒绝。
        let title = items
            .iter()
            .map(|item| item.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        check_chat_title(&title, recipient)
    }
    .await;
    let _ = std::fs::remove_file(&tmp_file);
    result
}

pub fn get_ocr_binary_path() -> std::path::PathBuf {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        let candidate = parent.join("wechat_ocr");
        if candidate.exists() {
            return candidate;
        }
    }
    std::path::PathBuf::from("bin/wechat_ocr")
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
        .kill_on_drop(true)
        .args([
            "-R",
            &format!(
                "{},{},{},{}",
                list_crop_x, list_crop_y, list_crop_w, list_crop_h
            ),
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
        .kill_on_drop(true)
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
    let mut items: Vec<OcrItem> =
        serde_json::from_str(raw_json.trim()).context("OCR 返回不是有效 JSON")?;
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
        open_chat_conversation(target).await?;
        verify_chat_title(target).await?;
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

    info!(
        "读取聊天区域几何坐标: ({}, {}, {}, {})",
        chat_x, chat_y, chat_w, chat_h
    );

    let tmp_file = format!("/tmp/wechat_chat_{}.png", std::process::id());

    // 调用系统 screencapture
    let cap_status = Command::new("screencapture")
        .kill_on_drop(true)
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
        .kill_on_drop(true)
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
    info!("OCR 识别输出长度: {} 字节", raw_json.len());
    let items: Vec<OcrItem> =
        serde_json::from_str(raw_json.trim()).context("OCR 返回不是有效 JSON")?;

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
pub async fn wait_for_new_message(
    recipient: Option<&str>,
    timeout_seconds: u64,
) -> Result<Option<String>> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    let timeout = std::time::Duration::from_secs(timeout_seconds.max(5));
    let start = std::time::Instant::now();

    if let Some(target) = recipient.filter(|r| !r.trim().is_empty()) {
        // === 模式 A：单目标会话长轮询 ===
        info!(
            "开始长轮询监听目标 [{}] 的新消息，超时设定: {} 秒",
            target, timeout_seconds
        );
        open_chat_conversation(target).await?;

        let initial_lines = read_chat_messages(None).await?;
        let mut known_lines: std::collections::HashSet<String> =
            initial_lines.into_iter().collect();

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

            let current_lines = read_chat_messages(None).await?;
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
                info!("监听到会话文字变化，共 {} 行", new_lines.len());
                return Ok(Some(combined));
            }
        }

        Ok(None)
    } else {
        // === 模式 B：全局会话收件箱长轮询（支持任意好友/群聊新消息自动感知） ===
        info!(
            "开始全局长轮询监听微信新消息（收件箱模式），超时设定: {} 秒",
            timeout_seconds
        );

        // 获取初始会话列表基线
        let initial_sessions = get_recent_sessions().await?;
        // 保存 key: name -> (time, preview)
        let mut session_baseline: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        for s in initial_sessions {
            session_baseline.insert(s.name, (s.time, s.preview));
        }

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1800)).await;

            let current_sessions = get_recent_sessions().await?;
            for s in current_sessions {
                let is_changed = match session_baseline.get(&s.name) {
                    Some((old_time, old_preview)) => {
                        // 时间变动或最新消息预览变动且非空
                        (!s.preview.is_empty() && s.preview != *old_preview)
                            || (!s.time.is_empty() && s.time != *old_time)
                    }
                    None => {
                        // 新冒出的置顶会话且有消息
                        !s.preview.is_empty()
                    }
                };

                if is_changed {
                    info!("检测到会话列表变化，自动点击查看...");
                    // 点击该会话打开其聊天窗口
                    mouse_click(s.click_x, s.click_y).await?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // 读取该会话的右侧真实气泡记录
                    let full_chat = read_chat_messages(None).await?;
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

#[cfg(test)]
mod send_safety_tests {
    use super::*;

    fn item(text: &str, x: f64, y: f64) -> OcrItem {
        OcrItem {
            text: text.into(),
            x,
            y,
        }
    }

    #[test]
    fn names_trim_boundaries_and_numeric_member_suffixes() {
        for (raw, expected) in [
            ("  张三  ", "张三"),
            ("【研发群 (12)】！", "研发群"),
            ("“研发群（ 12 ）”", "研发群"),
            ("研发(内部)群", "研发(内部)群"),
            ("张 三", "张 三"),
            ("张三123", "张三123"),
            ("... （12） ...", ""),
            ("！ ， ", ""),
        ] {
            assert_eq!(normalize_chat_name(raw), expected, "{raw}");
        }
    }

    #[test]
    fn preview_lines_do_not_select_a_conversation() {
        for preview in ["张三: 好的", "张三"] {
            let items = [item(preview, 0.25, 0.14), item("工作群", 0.25, 0.10)];
            assert!(matches!(
                match_name_column(&items, "张三"),
                NameColumnMatch::Missing
            ));
        }
    }

    #[test]
    fn fragments_and_prefixes_do_not_match() {
        for (name, recipient) in [
            ("文件", "文件传输助手"),
            ("群", "研发群"),
            ("张三丰", "张三"),
            ("张三", "张三丰"),
            ("", "张三"),
        ] {
            let items = [item(name, 0.2, 0.1)];
            assert!(matches!(
                match_name_column(&items, recipient),
                NameColumnMatch::Missing
            ));
        }
    }

    #[test]
    fn only_top_left_name_column_item_can_match() {
        let items = [
            item("张三", 0.8, 0.10),
            item("李四", 0.2, 0.10),
            item("张三", 0.4, 0.10),
            item("张三", 0.2, 0.14),
        ];
        assert!(matches!(
            match_name_column(&items, "张三"),
            NameColumnMatch::Missing
        ));
        match match_name_column(&items, "李四") {
            NameColumnMatch::Unique(name) => assert_eq!(name.text, "李四"),
            other => panic!("名称列应命中李四: {other:?}"),
        }
    }

    #[test]
    fn unique_name_matches_after_normalization() {
        let items = [
            item("09:00", 0.85, 0.10),
            item("研发群 (12)", 0.55, 0.11),
            item("张三: 好的", 0.2, 0.15),
            item("其他群", 0.2, 0.25),
        ];
        match match_name_column(&items, "【研发群】") {
            NameColumnMatch::Unique(name) => assert_eq!(name.y, 0.11),
            other => panic!("唯一名称应命中: {other:?}"),
        }
    }

    #[test]
    fn duplicate_normalized_names_are_ambiguous() {
        let items = [item("研发群(12)", 0.2, 0.1), item("研发群（24）", 0.2, 0.3)];
        assert!(matches!(
            match_name_column(&items, "研发群"),
            NameColumnMatch::Ambiguous
        ));
    }

    #[test]
    fn invalid_coordinates_and_empty_targets_are_rejected() {
        let items = [
            item("张三", f64::NAN, 0.1),
            item("张三", 0.2, f64::INFINITY),
            item("张三", -0.1, 0.1),
            item("张三", 0.2, 1.1),
        ];
        assert!(matches!(
            match_name_column(&items, "张三"),
            NameColumnMatch::Missing
        ));
        assert!(matches!(
            match_name_column(&[item("...", 0.2, 0.1)], "..."),
            NameColumnMatch::Missing
        ));
    }

    #[test]
    fn titles_require_complete_normalized_equality() {
        assert!(title_matches_recipient("“研发群（12）”", "研发群"));
        assert!(title_matches_recipient("张三", " 张三 "));
        for (title, recipient) in [
            ("张三丰", "张三"),
            ("张", "张三"),
            ("工作群 张三", "张三"),
            ("张三 聊天记录", "张三"),
            ("", "张三"),
            ("...", "..."),
            ("alice", "Alice"),
            ("张 三", "张三"),
        ] {
            assert!(
                !title_matches_recipient(title, recipient),
                "{title} / {recipient}"
            );
        }
    }

    #[test]
    fn title_mismatch_error_includes_observed_title() {
        let error = check_chat_title("张三丰 (12)", "张三")
            .unwrap_err()
            .to_string();
        assert!(error.contains("张三丰 (12)"));
        assert!(error.contains("已停止操作"));
        assert!(check_chat_title("张三", "张三").is_ok());
    }
}
