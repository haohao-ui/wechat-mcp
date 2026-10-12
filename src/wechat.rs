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
pub fn mouse_click(x: f64, y: f64) {
    let click_bin = get_click_binary_path();
    info!(
        "执行鼠标点击: ({}, {}) [工具: {:?}, 是否存在: {}]",
        x,
        y,
        click_bin,
        click_bin.exists()
    );
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
    let rect = query_wechat_window().await?.rect;
    Ok((rect.x, rect.y, rect.width, rect.height))
}

/// 获取微信当前窗口屏幕坐标原点 (x, y)
#[allow(dead_code)]
pub async fn get_wechat_window_pos() -> Result<(f64, f64)> {
    let (x, y, _, _) = get_wechat_window_rect().await?;
    Ok((x, y))
}

/// 前台激活微信；仅在窗口过小或超出所有屏幕范围时调整窗口
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
            end tell
        end tell
    "#;

    run_applescript(script).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let window = query_wechat_window().await?;
    let screens: Vec<WindowRect> = serde_json::from_str(&run_window_tool(&["--screens"]).await?)
        .context("无法解析屏幕范围 JSON")?;
    if let Some(target) = adjusted_window_rect(window.rect, &screens)? {
        // AX 窗口排序与 CGWindow 不同，以面积最大的窗口与查询工具保持一致。
        let script = format!(
            r#"
            tell application "System Events"
                tell process "WeChat"
                    set targetWin to missing value
                    set largestArea to 0
                    repeat with w in (every window)
                        set sz to size of w
                        set area to (item 1 of sz) * (item 2 of sz)
                        if area > largestArea then
                            set largestArea to area
                            set targetWin to w
                        end if
                    end repeat
                    if targetWin is missing value then error "未找到微信主窗口"
                    set size of targetWin to {{{}, {}}}
                    set position of targetWin to {{{}, {}}}
                end tell
            end tell
            "#,
            target.width.round(),
            target.height.round(),
            target.x.round(),
            target.y.round()
        );
        run_applescript(&script).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }
    get_wechat_window_rect().await
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
struct WindowRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl WindowRect {
    fn is_valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .iter()
        .all(|value| value.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Debug, serde::Deserialize)]
struct WechatWindow {
    window_id: u32,
    #[serde(flatten)]
    rect: WindowRect,
    on_screen: bool,
}

fn parse_wechat_window(json: &str) -> Result<WechatWindow> {
    let window: WechatWindow = serde_json::from_str(json).context("无法解析微信窗口 JSON")?;
    if window.window_id == 0 || !window.rect.is_valid() {
        bail!("微信窗口 JSON 包含无效的窗口 ID 或矩形");
    }
    Ok(window)
}

fn get_window_binary_path() -> std::path::PathBuf {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        let candidate = parent.join("wechat_window");
        if candidate.exists() {
            return candidate;
        }
    }
    std::path::PathBuf::from("bin/wechat_window")
}

async fn run_window_tool(args: &[&str]) -> Result<String> {
    let output = Command::new(get_window_binary_path())
        .kill_on_drop(true)
        .args(args)
        .output()
        .await
        .context("执行 wechat_window 失败，请先运行 scripts/build.sh 编译原生工具")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn query_wechat_window() -> Result<WechatWindow> {
    parse_wechat_window(&run_window_tool(&[]).await?)
}

/// 检查屏幕矩形的并集，允许窗口跨相邻屏幕及位于负坐标屏幕。
fn window_inside_screens(window: WindowRect, screens: &[WindowRect]) -> bool {
    let mut xs = vec![window.x, window.x + window.width];
    let mut ys = vec![window.y, window.y + window.height];
    for screen in screens {
        xs.extend(
            [screen.x, screen.x + screen.width]
                .into_iter()
                .filter(|x| *x > window.x && *x < window.x + window.width),
        );
        ys.extend(
            [screen.y, screen.y + screen.height]
                .into_iter()
                .filter(|y| *y > window.y && *y < window.y + window.height),
        );
    }
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    xs.dedup();
    ys.dedup();
    xs.windows(2).all(|x_pair| {
        ys.windows(2).all(|y_pair| {
            let x = (x_pair[0] + x_pair[1]) / 2.0;
            let y = (y_pair[0] + y_pair[1]) / 2.0;
            screens.iter().any(|screen| {
                x >= screen.x
                    && x <= screen.x + screen.width
                    && y >= screen.y
                    && y <= screen.y + screen.height
            })
        })
    })
}

fn adjusted_window_rect(window: WindowRect, screens: &[WindowRect]) -> Result<Option<WindowRect>> {
    if !window.is_valid() || screens.is_empty() || screens.iter().any(|screen| !screen.is_valid()) {
        bail!("微信窗口或屏幕范围无效，无法安全调整窗口");
    }
    if window.width >= 800.0 && window.height >= 600.0 && window_inside_screens(window, screens) {
        return Ok(None);
    }
    let center_x = window.x + window.width / 2.0;
    let center_y = window.y + window.height / 2.0;
    let distance = |screen: &WindowRect| {
        (center_x - center_x.clamp(screen.x, screen.x + screen.width)).powi(2)
            + (center_y - center_y.clamp(screen.y, screen.y + screen.height)).powi(2)
    };
    let screen = screens
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .unwrap();
    let width = window.width.max(800.0).min(screen.width);
    let height = window.height.max(600.0).min(screen.height);
    Ok(Some(WindowRect {
        x: window.x.clamp(screen.x, screen.x + screen.width - width),
        y: window.y.clamp(screen.y, screen.y + screen.height - height),
        width,
        height,
    }))
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
                for item in items {
                    if item.text.contains(recipient) || recipient.contains(&item.text) {
                        // 计算该项在屏幕上的物理 Y 坐标并点击 (稍微向下微调 4px 命中中心)
                        let target_y = list_crop_y as f64 + (item.y * list_crop_h as f64) + 4.0;
                        let target_x = win_x + 150.0;
                        info!(
                            "在会话列表中找到 [{}]，直接点击坐标: ({}, {})",
                            recipient, target_x, target_y
                        );
                        mouse_click(target_x, target_y);
                        found_in_list = true;
                        break;
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
    if let Some(old_content) = saved_clipboard
        && let Err(e) = clipboard::set_clipboard(&old_content).await
    {
        warn!("恢复原剪贴板内容失败: {:?}", e);
    }

    info!("已触发微信发送操作，送达状态未验证");
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct OcrItem {
    pub text: String,
    pub x: f64,
    pub y: f64,
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

    let window = query_wechat_window().await?;
    ensure_window_observable(&window)?;
    let crop = session_list_rect(window.rect)?;
    let tmp_file = SessionScreenshot(
        std::env::temp_dir().join(format!("wechat_sessions_{}.png", uuid::Uuid::new_v4())),
    );
    // 去掉窗口阴影，使图片边界与 CGWindow 的窗口点尺寸对应。
    let capture = Command::new("screencapture")
        .kill_on_drop(true)
        .args(["-l", &window.window_id.to_string(), "-x", "-o", "-t", "png"])
        .arg(&tmp_file.0)
        .output()
        .await
        .context("截图微信窗口失败，请在系统设置中授予屏幕录制权限")?;

    if !capture.status.success() {
        // 截图期间最小化时，优先报告暂停原因。
        if let Ok(current) = query_wechat_window().await {
            ensure_window_observable(&current)?;
        }
        bail!(
            "截图微信窗口失败，请在系统设置中授予屏幕录制权限: {}",
            String::from_utf8_lossy(&capture.stderr).trim()
        );
    }

    let (image_width, image_height) = screenshot_pixel_size(&tmp_file.0)?;
    let pixels = crop_pixel_rect(crop, window.rect, image_width, image_height)?;
    let ocr_bin = get_ocr_binary_path();
    let ocr_output = Command::new(&ocr_bin)
        .kill_on_drop(true)
        .arg(&tmp_file.0)
        .args([
            "--crop",
            &pixels.x.to_string(),
            &pixels.y.to_string(),
            &pixels.width.to_string(),
            &pixels.height.to_string(),
        ])
        .output()
        .await
        .context("执行 OCR 解析会话列表失败")?;

    if !ocr_output.status.success() {
        let err = String::from_utf8_lossy(&ocr_output.stderr);
        bail!("OCR 会话列表识别失败: {}", err);
    }

    let raw_json = String::from_utf8_lossy(&ocr_output.stdout);
    let items: Vec<OcrItem> =
        serde_json::from_str(raw_json.trim()).context("OCR 返回不是有效 JSON")?;

    let current = query_wechat_window().await?;
    ensure_window_observable(&current)?;
    if current.window_id != window.window_id || current.rect != window.rect {
        bail!("微信窗口位置或尺寸已变化，本轮后台观察跳过");
    }
    // 像素裁剪的边界经过取整，反算点坐标避免 Retina 下点击偏移。
    let scale_x = image_width as f64 / window.rect.width;
    let scale_y = image_height as f64 / window.rect.height;
    let list_crop_y = window.rect.y + pixels.y as f64 / scale_y;
    let list_crop_h = pixels.height as f64 / scale_y;
    let list_left = window.rect.x + pixels.x as f64 / scale_x;
    let list_width = pixels.width as f64 / scale_x;
    let click_x = (window.rect.x + 150.0).clamp(list_left, list_left + list_width / 2.0);
    Ok(group_recent_sessions(
        items,
        click_x,
        list_crop_y,
        list_crop_h,
    ))
}

struct SessionScreenshot(std::path::PathBuf);

impl Drop for SessionScreenshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn ensure_window_observable(window: &WechatWindow) -> Result<()> {
    if !window.on_screen {
        bail!("微信窗口已最小化，后台观察暂停");
    }
    Ok(())
}

fn session_list_rect(window: WindowRect) -> Result<WindowRect> {
    let rect = WindowRect {
        x: 65.0,
        y: 60.0,
        width: (window.width - 65.0).min(250.0),
        height: window.height - 60.0,
    };
    if !window.is_valid() || !rect.is_valid() {
        bail!("微信窗口过小，无法截取会话列表，后台观察暂停");
    }
    Ok(rect)
}

#[derive(Debug, PartialEq)]
struct PixelRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn crop_pixel_rect(
    crop: WindowRect,
    window: WindowRect,
    image_width: u32,
    image_height: u32,
) -> Result<PixelRect> {
    if !window.is_valid()
        || !crop.is_valid()
        || image_width == 0
        || image_height == 0
        || crop.x < 0.0
        || crop.y < 0.0
        || crop.x + crop.width > window.width
        || crop.y + crop.height > window.height
    {
        bail!("会话列表裁剪范围或截图像素尺寸无效");
    }
    let scale_x = image_width as f64 / window.width;
    let scale_y = image_height as f64 / window.height;
    let x = (crop.x * scale_x).round() as u32;
    let y = (crop.y * scale_y).round() as u32;
    let right = ((crop.x + crop.width) * scale_x)
        .round()
        .min(image_width as f64) as u32;
    let bottom = ((crop.y + crop.height) * scale_y)
        .round()
        .min(image_height as f64) as u32;
    if right <= x || bottom <= y {
        bail!("会话列表裁剪后的图片为空");
    }
    Ok(PixelRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

fn screenshot_pixel_size(path: &std::path::Path) -> Result<(u32, u32)> {
    use std::io::Read;
    let mut header = [0u8; 24];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .context("无法读取微信窗口截图，请检查屏幕录制权限")?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        bail!("微信窗口截图不是有效 PNG，请检查屏幕录制权限");
    }
    let width = u32::from_be_bytes(header[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(header[20..24].try_into().unwrap());
    if width == 0 || height == 0 {
        bail!("微信窗口截图为空，请检查屏幕录制权限");
    }
    Ok((width, height))
}

fn group_recent_sessions(
    mut items: Vec<OcrItem>,
    click_x: f64,
    list_crop_y: f64,
    list_crop_h: f64,
) -> Vec<ChatSession> {
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
        let target_y = list_crop_y + (g.base_y * list_crop_h) + 6.0;
        result.push(ChatSession {
            name: g.name.trim().to_string(),
            time: g.time.trim().to_string(),
            preview: g.preview_parts.join(" ").trim().to_string(),
            click_x,
            click_y: target_y,
        });
    }

    result
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
                    mouse_click(s.click_x, s.click_y);
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
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> WindowRect {
        WindowRect {
            x,
            y,
            width,
            height,
        }
    }

    fn item(text: &str, x: f64, y: f64) -> OcrItem {
        OcrItem {
            text: text.into(),
            x,
            y,
        }
    }

    #[test]
    fn window_json_preserves_actual_geometry_and_visibility() {
        for on_screen in [true, false] {
            let json = format!(
                r#"{{"window_id":42,"x":-1200.5,"y":80,"width":950,"height":680,"on_screen":{on_screen}}}"#
            );
            let window = parse_wechat_window(&json).unwrap();
            assert_eq!(window.window_id, 42);
            assert_eq!(window.rect, rect(-1200.5, 80.0, 950.0, 680.0));
            assert_eq!(window.on_screen, on_screen);
            let result = ensure_window_observable(&window);
            if on_screen {
                result.unwrap();
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "微信窗口已最小化，后台观察暂停"
                );
            }
        }
    }

    #[test]
    fn window_json_rejects_missing_malformed_or_invalid_fields() {
        for json in [
            "not json",
            r#"{"window_id":42,"x":0,"y":0,"width":950,"height":680}"#,
            r#"{"window_id":0,"x":0,"y":0,"width":950,"height":680,"on_screen":true}"#,
            r#"{"window_id":42,"x":0,"y":0,"width":0,"height":680,"on_screen":true}"#,
            r#"{"window_id":42,"x":0,"y":0,"width":950,"height":-680,"on_screen":true}"#,
            r#"{"window_id":"42","x":0,"y":0,"width":950,"height":680,"on_screen":true}"#,
        ] {
            assert!(parse_wechat_window(json).is_err(), "{json}");
        }
    }

    #[test]
    fn valid_windows_on_negative_screens_and_across_screens_are_unchanged() {
        let screens = [
            rect(-1440.0, 0.0, 1440.0, 900.0),
            rect(0.0, 0.0, 1440.0, 900.0),
        ];
        for window in [
            rect(-1300.0, 80.0, 950.0, 680.0),
            rect(-400.0, 0.0, 950.0, 900.0),
        ] {
            assert_eq!(adjusted_window_rect(window, &screens).unwrap(), None);
        }
    }

    #[test]
    fn screen_gaps_are_not_treated_as_visible_window_area() {
        let screens = [
            rect(-1440.0, 0.0, 1440.0, 900.0),
            rect(100.0, 0.0, 1440.0, 900.0),
        ];
        let window = rect(-400.0, 80.0, 950.0, 680.0);
        assert!(!window_inside_screens(window, &screens));
        let adjusted = adjusted_window_rect(window, &screens).unwrap().unwrap();
        assert!(window_inside_screens(adjusted, &screens));
    }

    #[test]
    fn offscreen_and_small_windows_are_adjusted_only_as_needed() {
        let screens = [
            rect(-1440.0, 0.0, 1440.0, 900.0),
            rect(0.0, 0.0, 1440.0, 900.0),
        ];
        assert_eq!(
            adjusted_window_rect(rect(-1500.0, 80.0, 950.0, 680.0), &screens).unwrap(),
            Some(rect(-1440.0, 80.0, 950.0, 680.0))
        );
        assert_eq!(
            adjusted_window_rect(rect(-1300.0, 80.0, 700.0, 500.0), &screens).unwrap(),
            Some(rect(-1300.0, 80.0, 800.0, 600.0))
        );
        assert_eq!(
            adjusted_window_rect(rect(100.0, 100.0, 950.0, 680.0), &screens).unwrap(),
            None
        );
        assert_eq!(
            adjusted_window_rect(rect(100.0, 100.0, 800.0, 600.0), &screens).unwrap(),
            None
        );
    }

    #[test]
    fn window_adjustment_handles_small_screens_and_rejects_invalid_bounds() {
        let screen = rect(0.0, 0.0, 640.0, 480.0);
        assert_eq!(
            adjusted_window_rect(rect(100.0, 100.0, 700.0, 500.0), &[screen]).unwrap(),
            Some(screen)
        );
        assert!(adjusted_window_rect(screen, &[]).is_err());
        assert!(adjusted_window_rect(screen, &[rect(0.0, 0.0, 0.0, 480.0)]).is_err());
        assert!(adjusted_window_rect(rect(f64::NAN, 0.0, 800.0, 600.0), &[screen]).is_err());
    }

    #[test]
    fn session_crop_uses_actual_image_scale_for_retina_and_standard_displays() {
        let window = rect(-1300.0, 80.0, 950.0, 680.0);
        let crop = session_list_rect(window).unwrap();
        assert_eq!(crop, rect(65.0, 60.0, 250.0, 620.0));
        for scale in [1, 2] {
            assert_eq!(
                crop_pixel_rect(crop, window, 950 * scale, 680 * scale).unwrap(),
                PixelRect {
                    x: 65 * scale,
                    y: 60 * scale,
                    width: 250 * scale,
                    height: 620 * scale
                }
            );
        }
    }

    #[test]
    fn pixel_crop_rounds_edges_and_uses_independent_image_axes() {
        let window = rect(0.0, 0.0, 950.0, 680.0);
        let crop = session_list_rect(window).unwrap();
        assert_eq!(
            crop_pixel_rect(crop, window, 1425, 1360).unwrap(),
            PixelRect {
                x: 98,
                y: 120,
                width: 375,
                height: 1240
            }
        );
    }

    #[test]
    fn small_window_crop_is_clipped_without_resizing_the_window() {
        let window = rect(20.0, 30.0, 300.0, 400.0);
        let crop = session_list_rect(window).unwrap();
        assert_eq!(crop, rect(65.0, 60.0, 235.0, 340.0));
        assert_eq!(
            crop_pixel_rect(crop, window, 600, 800).unwrap(),
            PixelRect {
                x: 130,
                y: 120,
                width: 470,
                height: 680
            }
        );
        assert!(session_list_rect(rect(0.0, 0.0, 65.0, 680.0)).is_err());
        assert!(session_list_rect(rect(0.0, 0.0, 950.0, 60.0)).is_err());
    }

    #[test]
    fn pixel_crop_rejects_empty_or_out_of_bounds_images() {
        let window = rect(0.0, 0.0, 950.0, 680.0);
        let crop = session_list_rect(window).unwrap();
        assert!(crop_pixel_rect(crop, window, 0, 1360).is_err());
        assert!(crop_pixel_rect(crop, window, 1, 1).is_err());
        assert!(crop_pixel_rect(rect(-1.0, 0.0, 250.0, 620.0), window, 1900, 1360).is_err());
        assert!(crop_pixel_rect(rect(65.0, 60.0, 950.0, 620.0), window, 1900, 1360).is_err());
    }

    #[test]
    fn session_grouping_preserves_sorting_names_times_previews_and_screen_coordinates() {
        let groups = group_recent_sessions(
            vec![
                item(" 后一条 ", 0.1, 0.3),
                item(" 第二段 ", 0.1, 0.15),
                item(" 09:00 ", 0.8, 0.1),
                item(" 第一条 ", 0.1, 0.1),
                item(" 第一段 ", 0.1, 0.13),
                item(" 10:00 ", 0.8, 0.3),
            ],
            -1150.0,
            140.0,
            620.0,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "第一条");
        assert_eq!(groups[0].time, "09:00");
        assert_eq!(groups[0].preview, "第一段   第二段");
        assert_eq!(groups[0].click_x, -1150.0);
        assert_eq!(groups[0].click_y, 208.0);
        assert_eq!(groups[1].name, "后一条");
        assert_eq!(groups[1].time, "10:00");
        assert_eq!(groups[1].click_y, 332.0);
    }

    #[test]
    fn session_grouping_keeps_threshold_and_discards_groups_without_names() {
        let groups = group_recent_sessions(
            vec![
                item("仅时间", 0.8, 0.0),
                item("   ", 0.1, 0.2),
                item("第一条", 0.55, 0.4),
                item("摘要", 0.1, 0.479),
                item("第二条", 0.1, 0.481),
                item("第二条时间", 0.56, 0.481),
            ],
            350.0,
            260.0,
            620.0,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "第一条");
        assert_eq!(groups[0].preview, "摘要");
        assert_eq!(groups[1].name, "第二条");
        assert_eq!(groups[1].time, "第二条时间");
        assert!(group_recent_sessions(vec![], 350.0, 260.0, 620.0).is_empty());
    }

    #[test]
    fn temporary_screenshot_is_removed_on_error_or_cancellation_drop() {
        let path =
            std::env::temp_dir().join(format!("wechat_cleanup_{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"partial screenshot").unwrap();
        {
            let _screenshot = SessionScreenshot(path.clone());
        }
        assert!(!path.exists());
    }

    #[test]
    fn screenshot_dimensions_are_read_from_png_and_invalid_files_are_rejected() {
        let screenshot = SessionScreenshot(
            std::env::temp_dir().join(format!("wechat_header_{}.png", uuid::Uuid::new_v4())),
        );
        let mut header = Vec::from(&b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR"[..]);
        header.extend(1900u32.to_be_bytes());
        header.extend(1360u32.to_be_bytes());
        std::fs::write(&screenshot.0, &header).unwrap();
        assert_eq!(screenshot_pixel_size(&screenshot.0).unwrap(), (1900, 1360));
        header[16..20].fill(0);
        std::fs::write(&screenshot.0, &header).unwrap();
        assert!(screenshot_pixel_size(&screenshot.0).is_err());
        std::fs::write(&screenshot.0, b"not a png").unwrap();
        assert!(screenshot_pixel_size(&screenshot.0).is_err());
    }
}
