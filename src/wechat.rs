use anyhow::{Context, Result, anyhow, bail};
use std::hash::Hasher;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{info, warn};

use crate::clipboard;
use crate::desktop::{protocol::Region, runtime};
use crate::domain::observation::Snapshot;
use crate::messaging::reconcile::{Continuity, reconcile};

mod navigation;
pub mod files;

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
    if let Ok(path) = std::env::var("WECHAT_CLICK_BIN") {
        return std::path::PathBuf::from(path);
    }
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
    crate::desktop::overlay::click(x, y, false).await;
    let click_bin = get_click_binary_path();
    info!(
        "执行鼠标点击: ({}, {}) [工具: {:?}, 是否存在: {}]",
        x,
        y,
        click_bin,
        click_bin.exists()
    );
    if click_bin.exists() {
        let output = Command::new(&click_bin)
            .kill_on_drop(true)
            .args([&x.to_string(), &y.to_string()])
            .output()
            .await
            .context("执行 wechat_click 失败")?;
        info!("wechat_click 执行结果: {:?}", output.status);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                bail!("wechat_click 退出状态异常: {:?}", output.status.code());
            } else {
                bail!(
                    "wechat_click 退出状态异常: {:?}: {}",
                    output.status.code(),
                    stderr
                );
            }
        }
        crate::desktop::overlay::click(x, y, true).await;
        return Ok(());
    }

    let result = tokio::task::spawn_blocking(move || -> Result<()> {
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
    .context("鼠标点击任务失败")?;
    if result.is_ok() {
        crate::desktop::overlay::click(x, y, true).await;
    }
    result
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
    let output = crate::desktop::overlay::capture(
        Command::new(get_window_binary_path())
            .kill_on_drop(true)
            .args(args)
            .output(),
    )
    .await
    .context("执行 wechat_window 失败，请先运行 scripts/build.sh 编译原生工具")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn query_wechat_window() -> Result<WechatWindow> {
    let window = parse_wechat_window(&run_window_tool(&[]).await?)?;
    if window.on_screen {
        let rect = window.rect;
        crate::desktop::overlay::window(rect.x, rect.y, rect.width, rect.height).await;
    } else {
        crate::desktop::overlay::hide().await;
    }
    Ok(window)
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

    if normalize_chat_name(recipient).is_empty() {
        bail!("接收人规范化后不能为空");
    }
    let (win_x, win_y, _win_w, win_h) = focus_wechat().await?;
    info!("正在定位联系人/群聊 [{}]...", recipient);

    let window = query_wechat_window().await?;
    ensure_window_observable(&window)?;
    if navigation::open_if_native(recipient).await? {
        return Ok(());
    }
    if verify_chat_title(recipient).await.is_ok() {
        return Ok(());
    }
    if let Some((cached_x, cached_y)) =
        cached_chat_location(recipient, window.window_id, window.rect, Instant::now())
    {
        info!(
            "复用会话位置缓存 [{}]：({}, {})",
            recipient, cached_x, cached_y
        );
        mouse_click(cached_x, cached_y).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
        if verify_chat_title(recipient).await.is_ok() {
            remember_chat_location(recipient, window.window_id, window.rect, cached_x, cached_y);
            return Ok(());
        }
        warn!("会话位置缓存 [{}] 已失效，回退到 OCR 定位", recipient);
        forget_chat_location();
    }

    // 方案一：优先通过 Vision OCR 扫描左侧会话列表，实现无感精准直达
    let list_crop_x = (win_x + 65.0).round() as i64;
    let list_crop_y = (win_y + 60.0).round() as i64;
    let list_crop_w = 250i64;
    let list_crop_h = (win_h - 60.0).round() as i64;

    let tmp_list_img = format!("/tmp/wechat_list_{}.png", std::process::id());
    let cap_res = crate::desktop::overlay::capture(
        Command::new("screencapture")
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
            .status(),
    )
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
                        remember_chat_location(
                            recipient,
                            window.window_id,
                            window.rect,
                            target_x,
                            target_y,
                        );
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
    let search_result = navigation::open_legacy_search(recipient, &window).await;
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

#[derive(Debug)]
pub enum SendError {
    NotSent(anyhow::Error),
    Unknown(anyhow::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NotSent(err) => {
                let s = format!("{err:#}");
                if s.contains("未触发发送") {
                    write!(f, "{s}")
                } else {
                    write!(f, "未触发发送: {s}")
                }
            }
            SendError::Unknown(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::NotSent(err) => Some(err.as_ref()),
            SendError::Unknown(err) => Some(err.as_ref()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFault {
    Click(Option<&'static str>),
    AfterEnter(Option<&'static str>),
}

tokio::task_local! {
    pub static TEST_FAULT: Option<SendFault>;
}

pub async fn with_test_send_fault<F, R>(fault: SendFault, f: F) -> R
where
    F: std::future::Future<Output = R>,
{
    TEST_FAULT.scope(Some(fault), f).await
}

/// 发送微信消息给指定联系人或群组
pub async fn send_message(recipient: &str, message: &str) -> Result<(), SendError> {
    if let Some(fault) = TEST_FAULT.try_with(|f| *f).ok().flatten() {
        match fault {
            SendFault::Click(msg) => {
                let err_detail = msg.unwrap_or("accessibility_permission_missing_for_executing_process");
                return Err(SendError::NotSent(anyhow!(
                    "wechat_click 退出状态异常: Some(1): {err_detail}"
                )));
            }
            SendFault::AfterEnter(msg) => {
                let err_detail = msg.unwrap_or("applescript execution failed after enter");
                return Err(SendError::Unknown(anyhow!("{err_detail}")));
            }
        }
    }

    if !is_wechat_running().await.map_err(SendError::NotSent)? {
        return Err(SendError::NotSent(anyhow!("微信客户端未运行，请先启动并登录微信")));
    }

    if recipient.trim().is_empty() {
        return Err(SendError::NotSent(anyhow!("接收人不能为空")));
    }

    if message.trim().is_empty() {
        return Err(SendError::NotSent(anyhow!("消息内容不能为空")));
    }

    info!("准备向 [{}] 发送微信消息...", recipient);

    // 1. 定位并打开目标会话
    open_chat_conversation(recipient)
        .await
        .map_err(SendError::NotSent)?;

    // 2. 获取当前窗口坐标并精准点击聊天输入框中心，确保光标聚焦
    let (win_x, win_y, _win_w, win_h) = get_wechat_window_rect()
        .await
        .map_err(SendError::NotSent)?;
    let input_x = win_x + 450.0;
    let input_y = win_y + win_h - 80.0;
    info!("点击聊天输入框激活光标焦点: ({}, {})", input_x, input_y);
    mouse_click(input_x, input_y)
        .await
        .map_err(SendError::NotSent)?;
    tokio::time::sleep(tokio::time::Duration::from_millis(350)).await;

    // 输入框点击可能改变焦点；在正文进入剪贴板之前再次核验标题。
    verify_chat_title(recipient)
        .await
        .map_err(SendError::NotSent)?;

    // 3. 暂存剪贴板并将正文粘贴发送
    let mut clipboard_guard = clipboard::ClipboardGuard::replace(message)
        .await
        .map_err(SendError::NotSent)?;

    let paste_script = r#"
        tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                keystroke "v" using {command down}
                delay 0.3
            end tell
        end tell
    "#;

    let prepare_result = async {
        run_applescript(paste_script).await?;
        // A sentinel prevents a failed Copy from being mistaken for a matching clipboard.
        clipboard::set_clipboard(&format!("wechat-readback-{}", uuid::Uuid::new_v4())).await?;
        run_applescript(
            r#"tell application "System Events"
            tell process "WeChat"
                keystroke "a" using {command down}
                keystroke "c" using {command down}
                delay 0.15
            end tell
        end tell"#,
        )
        .await?;
        let actual = clipboard::get_clipboard()
            .await?
            .context("发送前无法读取输入框正文，未触发发送")?;
        if !same_input_text(&actual, message) {
            bail!("发送前输入正文读回不一致，未触发发送")
        }
        verify_chat_title(recipient).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(err) = prepare_result {
        let _ = clipboard_guard.restore().await;
        return Err(SendError::NotSent(err));
    }

    let enter_result = run_applescript(
        r#"tell application "System Events"
        tell process "WeChat" to key code 36
    end tell"#,
    )
    .await;

    // 4. 恢复原剪贴板
    let restore_result = clipboard_guard.restore().await;
    if let Err(error) = &restore_result {
        warn!("发送后恢复剪贴板失败，将同步兜底: {error:#}");
    }

    if let Err(err) = enter_result {
        return Err(SendError::Unknown(err));
    }
    if let Err(err) = restore_result {
        return Err(SendError::Unknown(err));
    }

    info!("已触发微信发送操作，送达状态未验证");
    Ok(())
}

/// 发送本机文件（路径已由 files::prepare_send_file 校验）。
/// 粘贴后先 OCR 输入框确认出现该文件，再复核标题后回车；任何一步失败都清空输入框且不回车。
pub async fn send_file(recipient: &str, file: &files::FileToSend) -> Result<()> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }
    info!("准备向 [{}] 发送文件 {}...", recipient, file.name);
    open_chat_conversation(recipient).await?;
    let (win_x, win_y, win_w, win_h) = get_wechat_window_rect().await?;
    mouse_click(win_x + 450.0, win_y + win_h - 80.0).await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(350)).await;
    verify_chat_title(recipient).await?;

    // 先保存文本剪贴板；文件引用写入后，结束时恢复原文本。
    let mut clipboard_guard =
        clipboard::ClipboardGuard::replace(&format!("wechat-file-{}", uuid::Uuid::new_v4())).await?;
    let clear_input = r#"tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                keystroke "a" using {command down}
                key code 51
                delay 0.2
            end tell
        end tell"#;
    let send_result = async {
        // 输入框里残留的草稿不能和文件一起发出。
        run_applescript(clear_input).await?;
        set_clipboard_file(&file.path).await?;
        run_applescript(
            r#"tell application "System Events"
            tell process "WeChat"
                set frontmost to true
                keystroke "v" using {command down}
                delay 1.0
            end tell
        end tell"#,
        )
        .await?;
        let lines = ocr_screen_region(
            win_x + 315.0,
            win_y + win_h - 160.0,
            win_w - 325.0,
            150.0,
        )
        .await?;
        if !files::input_shows_file(&lines, &file.name) {
            let _ = run_applescript(clear_input).await;
            bail!("粘贴后输入框未出现文件 {}，已清空且未触发发送", file.name);
        }
        if let Err(error) = verify_chat_title(recipient).await {
            let _ = run_applescript(clear_input).await;
            return Err(error);
        }
        run_applescript(
            r#"tell application "System Events"
            tell process "WeChat" to key code 36
        end tell"#,
        )
        .await
        .map(|_| ())
    }
    .await;
    let restore_result = clipboard_guard.restore().await;
    if let Err(error) = &restore_result {
        warn!("发送文件后恢复剪贴板失败，将同步兜底: {error:#}");
    }
    send_result?;
    restore_result?;
    info!("已触发微信文件发送操作，送达状态未验证");
    Ok(())
}

/// 把一个文件引用写入通用剪贴板（与 Finder 拷贝文件相同的 NSURL 形式）。
async fn set_clipboard_file(path: &std::path::Path) -> Result<()> {
    let script = r#"ObjC.import('AppKit');
function run(argv) {
    const board = $.NSPasteboard.generalPasteboard;
    board.clearContents;
    const ok = board.writeObjects($([$.NSURL.fileURLWithPath(argv[0])]));
    return ok ? 'ok' : 'failed';
}"#;
    let output = Command::new("osascript")
        .kill_on_drop(true)
        .args(["-l", "JavaScript", "-e", script])
        .arg(path)
        .output()
        .await
        .context("执行文件剪贴板脚本失败")?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "ok" {
        bail!(
            "无法把文件写入剪贴板: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// 截取屏幕区域（点坐标）并 OCR，返回识别出的文字行。
async fn ocr_screen_region(x: f64, y: f64, width: f64, height: f64) -> Result<Vec<String>> {
    if width <= 0.0 || height <= 0.0 {
        bail!("截图区域无效");
    }
    let region = format!(
        "{},{},{},{}",
        x.round() as i64,
        y.round() as i64,
        width.round() as i64,
        height.round() as i64
    );
    let tmp_file = std::env::temp_dir().join(format!("wechat_input_{}.png", uuid::Uuid::new_v4()));
    let result = async {
        let status = crate::desktop::overlay::capture(
            Command::new("screencapture")
                .kill_on_drop(true)
                .args(["-R", &region, "-x"])
                .arg(&tmp_file)
                .status(),
        )
        .await
        .context("截取输入框失败")?;
        if !status.success() {
            bail!("截取输入框失败，已停止操作");
        }
        let output = Command::new(get_ocr_binary_path())
            .kill_on_drop(true)
            .arg(&tmp_file)
            .output()
            .await
            .context("执行输入框 OCR 失败")?;
        if !output.status.success() {
            bail!("输入框 OCR 失败: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        let items: Vec<OcrItem> =
            serde_json::from_slice(&output.stdout).context("输入框 OCR 返回不是有效 JSON")?;
        Ok(items.into_iter().map(|item| item.text).collect())
    }
    .await;
    let _ = std::fs::remove_file(&tmp_file);
    result
}

fn same_input_text(actual: &str, expected: &str) -> bool {
    let normalize = |text: &str| text.replace("\r\n", "\n").replace('\r', "\n");
    normalize(actual) == normalize(expected)
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

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// Tesseract 兜底时标题两侧常粘连图标/英文碎片；取最长连续中文片段作为候选。
/// 仍要求与接收人规范化后完全相等，不接受“包含”关系。
fn longest_cjk_run(text: &str) -> &str {
    let mut best = (0, 0);
    let mut start = None;
    for (index, c) in text.char_indices() {
        if is_cjk(c) {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take()
            && index - begin > best.1 - best.0
        {
            best = (begin, index);
        }
    }
    if let Some(begin) = start
        && text.len() - begin > best.1 - best.0
    {
        best = (begin, text.len());
    }
    &text[best.0..best.1]
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
    if let Some(snapshot) = runtime::shared().snapshot(Region::Title, true).await? {
        ensure_native_snapshot_current(&snapshot).await?;
        return verify_title_items(native_items(&snapshot), recipient);
    }
    let (win_x, win_y, win_w, _) = get_wechat_window_rect().await?;
    if win_w <= 325.0 {
        bail!("微信窗口过窄，无法核验聊天标题");
    }
    // 群标题文字可能从聊天区左侧留白开始；从 250pt 起截，避免切掉首个汉字。
    let region = format!(
        "{},{},{},50",
        (win_x + 250.0).round() as i64,
        win_y.round() as i64,
        (win_w - 260.0).round() as i64
    );
    let tmp_file = std::env::temp_dir().join(format!("wechat_title_{}.png", uuid::Uuid::new_v4()));
    let result = async {
        let status = crate::desktop::overlay::capture(
            Command::new("screencapture")
                .kill_on_drop(true)
                .args(["-R", &region, "-x"])
                .arg(&tmp_file)
                .status(),
        )
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
        let items: Vec<OcrItem> =
            serde_json::from_slice(&output.stdout).context("聊天标题 OCR 返回不是有效 JSON")?;
        verify_title_items(items, recipient)
    }
    .await;
    let _ = std::fs::remove_file(&tmp_file);
    result
}

fn verify_title_items(mut items: Vec<OcrItem>, recipient: &str) -> Result<()> {
    items.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    // 优先要求整个标题文本相等；Tesseract 兜底时标题前后常带图标碎片，
    // 允许标题栏内某个完整文本块单独相等（仍不接受名字片段）。
    let title = items
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if check_chat_title(&title, recipient).is_ok() {
        return Ok(());
    }
    for candidate in items
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
    {
        let cjk = longest_cjk_run(candidate);
        if !cjk.is_empty()
            && (check_chat_title(candidate, recipient).is_ok()
                || check_chat_title(cjk, recipient).is_ok())
        {
            return Ok(());
        }
    }
    check_chat_title(&title, recipient)
}

fn native_items(snapshot: &Snapshot) -> Vec<OcrItem> {
    snapshot
        .lines
        .iter()
        .map(|line| OcrItem {
            text: line.text.clone(),
            x: line.bounds.x,
            y: line.bounds.y,
        })
        .collect()
}

async fn ensure_native_snapshot_current(snapshot: &Snapshot) -> Result<()> {
    let current = query_wechat_window().await?;
    ensure_window_observable(&current)?;
    let window = &snapshot.window;
    if current.window_id != window.window_id
        || current.rect
            != (WindowRect {
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
            })
    {
        bail!("微信窗口在原生观察期间发生变化，已丢弃本轮结果");
    }
    Ok(())
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

/// 对截图文件做快速内容摘要，用于判断界面是否发生变化，避免每次轮询都执行 OCR。
fn file_digest(path: &std::path::Path) -> Result<u64> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("无法读取截图 {}", path.display()))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .with_context(|| format!("读取截图 {} 失败", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
    }
    Ok(hasher.finish())
}

const CHAT_LOCATION_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq)]
struct CachedChatLocation {
    window_id: u32,
    rect: WindowRect,
    click_x: f64,
    click_y: f64,
    captured_at: Instant,
}

impl CachedChatLocation {
    fn is_reusable(self, window_id: u32, rect: WindowRect, now: Instant) -> bool {
        self.window_id == window_id
            && self.rect == rect
            && now.duration_since(self.captured_at) <= CHAT_LOCATION_TTL
    }
}

#[derive(Debug)]
struct ChatLocationCache {
    recipient: String,
    location: CachedChatLocation,
}

fn chat_location_cache() -> &'static Mutex<Option<ChatLocationCache>> {
    static CACHE: OnceLock<Mutex<Option<ChatLocationCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_chat_location(
    recipient: &str,
    window_id: u32,
    rect: WindowRect,
    now: Instant,
) -> Option<(f64, f64)> {
    let key = normalize_chat_name(recipient);
    let guard = chat_location_cache().lock().ok()?;
    let cache = guard.as_ref()?;
    if cache.recipient != key || !cache.location.is_reusable(window_id, rect, now) {
        return None;
    }
    Some((cache.location.click_x, cache.location.click_y))
}

fn remember_chat_location(
    recipient: &str,
    window_id: u32,
    rect: WindowRect,
    click_x: f64,
    click_y: f64,
) {
    let entry = ChatLocationCache {
        recipient: normalize_chat_name(recipient),
        location: CachedChatLocation {
            window_id,
            rect,
            click_x,
            click_y,
            captured_at: Instant::now(),
        },
    };
    if let Ok(mut guard) = chat_location_cache().lock() {
        *guard = Some(entry);
    }
}

fn forget_chat_location() {
    if let Ok(mut guard) = chat_location_cache().lock() {
        *guard = None;
    }
}

/// 一帧会话列表截图；截图文件在 Drop 时删除。
struct SessionFrame {
    screenshot: SessionScreenshot,
    window: WechatWindow,
    pixels: PixelRect,
    list_crop_y: f64,
    list_crop_h: f64,
    click_x: f64,
    digest: u64,
}

async fn capture_session_frame() -> Result<SessionFrame> {
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
    let capture = crate::desktop::overlay::capture(
        Command::new("screencapture")
            .kill_on_drop(true)
            .args(["-l", &window.window_id.to_string(), "-x", "-o", "-t", "png"])
            .arg(&tmp_file.0)
            .output(),
    )
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
    let digest = file_digest(&tmp_file.0)?;
    // 像素裁剪的边界经过取整，反算点坐标避免 Retina 下点击偏移。
    let scale_x = image_width as f64 / window.rect.width;
    let scale_y = image_height as f64 / window.rect.height;
    let list_crop_y = window.rect.y + pixels.y as f64 / scale_y;
    let list_crop_h = pixels.height as f64 / scale_y;
    let list_left = window.rect.x + pixels.x as f64 / scale_x;
    let list_width = pixels.width as f64 / scale_x;
    let click_x = (window.rect.x + 150.0).clamp(list_left, list_left + list_width / 2.0);
    Ok(SessionFrame {
        screenshot: tmp_file,
        window,
        pixels,
        list_crop_y,
        list_crop_h,
        click_x,
        digest,
    })
}

async fn parse_session_frame(frame: &SessionFrame) -> Result<Vec<ChatSession>> {
    let ocr_bin = get_ocr_binary_path();
    let ocr_output = Command::new(&ocr_bin)
        .kill_on_drop(true)
        .arg(&frame.screenshot.0)
        .args([
            "--crop",
            &frame.pixels.x.to_string(),
            &frame.pixels.y.to_string(),
            &frame.pixels.width.to_string(),
            &frame.pixels.height.to_string(),
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
    if current.window_id != frame.window.window_id || current.rect != frame.window.rect {
        bail!("微信窗口位置或尺寸已变化，本轮后台观察跳过");
    }
    Ok(group_recent_sessions(
        items,
        frame.click_x,
        frame.list_crop_y,
        frame.list_crop_h,
    ))
}

/// 获取当前微信左侧最近会话列表及各会话的最新消息预览
pub(crate) struct SessionRead {
    pub sessions: Vec<ChatSession>,
    pub observation: Option<Snapshot>,
}

pub(crate) async fn read_session_observation() -> Result<SessionRead> {
    if let Some(snapshot) = runtime::shared().snapshot(Region::Sessions, false).await? {
        ensure_native_snapshot_current(&snapshot).await?;
        let window = &snapshot.window;
        let sessions = group_recent_sessions(
            native_items(&snapshot),
            window.x
                + snapshot.region.x * window.width
                + snapshot.region.width * window.width / 3.0,
            window.y + snapshot.region.y * window.height,
            snapshot.region.height * window.height,
        );
        return Ok(SessionRead {
            sessions,
            observation: Some(snapshot),
        });
    }
    let frame = capture_session_frame().await?;
    Ok(SessionRead {
        sessions: parse_session_frame(&frame).await?,
        observation: None,
    })
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

/// 聊天区域截图；文件在 Drop 时删除。
enum ChatFrame {
    Legacy {
        screenshot: ChatScreenshot,
        digest: u64,
    },
    Native(Snapshot),
}

impl ChatFrame {
    fn digest(&self) -> u64 {
        match self {
            Self::Legacy { digest, .. } => *digest,
            Self::Native(snapshot) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                hasher.write(snapshot.digest.as_bytes());
                hasher.finish()
            }
        }
    }

    fn epoch(&self) -> String {
        match self {
            Self::Native(snapshot) => snapshot.epoch.clone(),
            Self::Legacy { .. } => "legacy".into(),
        }
    }
}

struct ChatScreenshot(std::path::PathBuf);

impl Drop for ChatScreenshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn chat_text_rect(window: WindowRect) -> Result<WindowRect> {
    let rect = WindowRect {
        x: window.x + 315.0,
        y: window.y + 60.0,
        width: window.width - 325.0,
        height: window.height - 220.0,
    };
    if !window.is_valid() || !rect.is_valid() {
        bail!("微信窗口过小，无法截取聊天正文");
    }
    Ok(rect)
}

async fn capture_chat_frame() -> Result<ChatFrame> {
    if let Some(snapshot) = runtime::shared().snapshot(Region::Chat, false).await? {
        ensure_native_snapshot_current(&snapshot).await?;
        return Ok(ChatFrame::Native(snapshot));
    }
    let (win_x, win_y, win_w, win_h) = get_wechat_window_rect().await?;

    // 计算右侧真实聊天气泡区域：
    // 微信左侧导航栏 (~65px) + 会话列表栏 (~250px) 共约 315px
    // 避开顶部标题栏 (~60pt) 与底部工具栏、输入框 (~160pt)。
    let chat = chat_text_rect(WindowRect {
        x: win_x,
        y: win_y,
        width: win_w,
        height: win_h,
    })?;
    let chat_x = chat.x.round() as i64;
    let chat_y = chat.y.round() as i64;
    let chat_w = chat.width.round() as i64;
    let chat_h = chat.height.round() as i64;

    info!(
        "读取聊天区域几何坐标: ({}, {}, {}, {})",
        chat_x, chat_y, chat_w, chat_h
    );

    let tmp_file = ChatScreenshot(
        std::env::temp_dir().join(format!("wechat_chat_{}.png", uuid::Uuid::new_v4())),
    );

    // 调用系统 screencapture
    let cap_status = crate::desktop::overlay::capture(
        Command::new("screencapture")
            .kill_on_drop(true)
            .args([
                "-R",
                &format!("{},{},{},{}", chat_x, chat_y, chat_w, chat_h),
                "-x",
            ])
            .arg(&tmp_file.0)
            .status(),
    )
    .await
    .context("执行 screencapture 失败")?;

    if !cap_status.success() {
        bail!("截取聊天区域失败");
    }

    let digest = file_digest(&tmp_file.0)?;
    Ok(ChatFrame::Legacy {
        screenshot: tmp_file,
        digest,
    })
}

async fn parse_chat_frame(frame: &ChatFrame) -> Result<Vec<String>> {
    let screenshot = match frame {
        ChatFrame::Native(snapshot) => {
            return Ok(snapshot
                .lines
                .iter()
                .map(|line| line.text.trim().to_owned())
                .collect());
        }
        ChatFrame::Legacy { screenshot, .. } => screenshot,
    };
    let ocr_bin = get_ocr_binary_path();
    let ocr_output = Command::new(&ocr_bin)
        .kill_on_drop(true)
        .arg(&screenshot.0)
        .output()
        .await
        .context("执行 wechat_ocr 失败")?;

    if !ocr_output.status.success() {
        let err = String::from_utf8_lossy(&ocr_output.stderr);
        bail!("Apple Vision OCR 识别失败: {}", err);
    }

    let raw_json = String::from_utf8_lossy(&ocr_output.stdout);
    info!("OCR 识别输出长度: {} 字节", raw_json.len());
    let items: Vec<OcrItem> =
        serde_json::from_str(raw_json.trim()).context("OCR 返回不是有效 JSON")?;

    Ok(items
        .into_iter()
        .map(|item| item.text.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect())
}

/// 读取指定联系人/群聊（或当前打开窗口）的聊天记录文字（纯净右侧气泡区域）
pub async fn read_chat_messages(recipient: Option<&str>) -> Result<Vec<String>> {
    Ok(read_chat_observation(recipient).await?.text_lines)
}

pub(crate) struct ChatRead {
    pub text_lines: Vec<String>,
    pub observation: Option<Snapshot>,
}

pub(crate) async fn read_chat_observation(recipient: Option<&str>) -> Result<ChatRead> {
    if !is_wechat_running().await? {
        bail!("微信客户端未运行，请先启动并登录微信");
    }

    if let Some(target) = recipient {
        open_chat_conversation(target).await?;
        verify_chat_title(target).await?;
    } else {
        focus_wechat().await?;
    }

    let frame = capture_chat_frame().await?;
    if let Some(target) = recipient {
        verify_frame_title(&frame, target).await?;
    }
    let text_lines = parse_chat_frame(&frame).await?;
    let observation = match frame {
        ChatFrame::Native(snapshot) => Some(snapshot),
        ChatFrame::Legacy { .. } => None,
    };
    Ok(ChatRead {
        text_lines,
        observation,
    })
}

async fn verify_frame_title(frame: &ChatFrame, target: &str) -> Result<()> {
    match frame {
        ChatFrame::Native(snapshot) => verify_title_items(
            snapshot
                .title_lines
                .iter()
                .map(|line| OcrItem {
                    text: line.text.clone(),
                    x: line.bounds.x,
                    y: line.bounds.y,
                })
                .collect(),
            target,
        ),
        ChatFrame::Legacy { .. } => verify_chat_title(target).await,
    }
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

        let initial_frame = capture_chat_frame().await?;
        verify_frame_title(&initial_frame, target).await?;
        let mut last_digest = initial_frame.digest();
        let mut epoch = initial_frame.epoch();
        let mut previous_lines = parse_chat_frame(&initial_frame).await?;

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

            // 先比较截图摘要；界面没变化就不跑 OCR。
            let frame = capture_chat_frame().await?;
            if frame.digest() == last_digest && frame.epoch() == epoch {
                continue;
            }
            // A manually switched chat must never be attributed to the old target.
            verify_frame_title(&frame, target).await?;
            last_digest = frame.digest();
            let current_lines = parse_chat_frame(&frame).await?;
            let result = reconcile(&previous_lines, &current_lines, frame.epoch() == epoch);
            epoch = frame.epoch();
            previous_lines = current_lines;
            if matches!(result.continuity, Continuity::Gap | Continuity::Ambiguous) {
                return Ok(Some("会话文字连续性无法确认，可能有滚动、窗口重建或重复正文；需要重新读取，未判定为新入站消息".into()));
            }
            let new_lines = result.new_lines;

            if !new_lines.is_empty() {
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

        // 获取初始会话列表基线和截图摘要
        let initial_frame = capture_session_frame().await?;
        let mut last_digest = initial_frame.digest;
        let initial_sessions = parse_session_frame(&initial_frame).await?;
        // 保存 key: name -> (time, preview)
        let mut session_baseline: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        for s in initial_sessions {
            session_baseline.insert(s.name, (s.time, s.preview));
        }

        while start.elapsed() < timeout {
            tokio::time::sleep(tokio::time::Duration::from_millis(1800)).await;

            // 先比较截图摘要；界面没变化就不跑 OCR。
            let frame = capture_session_frame().await?;
            if frame.digest == last_digest {
                continue;
            }
            last_digest = frame.digest;
            let current_sessions = parse_session_frame(&frame).await?;
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
                    info!("检测到会话列表变化，尝试安全打开会话...");
                    // 后台截图坐标可能被其他窗口遮挡，须激活微信并核验会话身份。
                    if let Err(error) = open_chat_conversation(&s.name).await {
                        return Ok(Some(format!(
                            "检测到会话 [{}] 的列表变化：\n【时间】: {}\n【消息摘要】: {}\n未能安全打开会话，未读取聊天气泡\n【错误原因】: {error:#}",
                            s.name, s.time, s.preview
                        )));
                    }

                    // 读取该会话的右侧真实气泡记录
                    let full_chat = read_chat_messages(Some(&s.name)).await?;
                    let recent_bubbles = if full_chat.len() > 5 {
                        full_chat[full_chat.len() - 5..].join("\n")
                    } else {
                        full_chat.join("\n")
                    };

                    let result_msg = format!(
                        "观察到会话 [{}] 的列表变化（未确认收发方向）：\n【时间】: {}\n【列表摘要】: {}\n【可见聊天文字】:\n{}",
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

#[cfg(test)]
mod send_safety_tests {
    use super::*;

    #[test]
    fn chat_crop_excludes_tested_input_area_and_never_expands_small_windows() {
        let chat = chat_text_rect(rect(29.0, 143.0, 880.0, 640.0)).unwrap();
        // Test-machine input starts at window-relative y=498.
        assert!(chat.y + chat.height < 143.0 + 498.0);
        for (width, height) in [(500.0, 400.0), (715.0, 721.0)] {
            let chat = chat_text_rect(rect(0.0, 0.0, width, height)).unwrap();
            assert!(chat.x + chat.width <= width);
            assert!(chat.y + chat.height <= height);
        }
        assert!(chat_text_rect(rect(0.0, 0.0, 325.0, 640.0)).is_err());
        assert!(chat_text_rect(rect(0.0, 0.0, 880.0, 220.0)).is_err());
    }

    #[test]
    fn input_readback_preserves_content_and_normalizes_only_line_endings() {
        assert!(same_input_text("第一行\r\n第二行", "第一行\n第二行"));
        assert!(same_input_text(
            "微信 RPA 测试消息，请忽略。",
            "微信 RPA 测试消息，请忽略。"
        ));
        assert!(!same_input_text(
            "微信RPA测试消息，请忽略。",
            "微信 RPA 测试消息，请忽略。"
        ));
        assert!(!same_input_text(
            "旧草稿微信 RPA 测试消息，请忽略。",
            "微信 RPA 测试消息，请忽略。"
        ));
        assert!(!same_input_text("金额 100.00", "金额 100"));
        assert!(!same_input_text(" 正文", "正文"));
    }

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

    #[test]
    fn screenshot_digest_detects_content_changes() {
        let path =
            std::env::temp_dir().join(format!("wechat_digest_test_{}.bin", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"first frame").unwrap();
        let first = file_digest(&path).unwrap();
        std::fs::write(&path, b"second frame").unwrap();
        let second = file_digest(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_ne!(first, second);
    }

    #[test]
    fn cached_location_requires_matching_window_rect_and_ttl() {
        let now = Instant::now();
        let location = CachedChatLocation {
            window_id: 7,
            rect: rect(10.0, 20.0, 800.0, 600.0),
            click_x: 160.0,
            click_y: 240.0,
            captured_at: now,
        };
        assert!(location.is_reusable(
            7,
            rect(10.0, 20.0, 800.0, 600.0),
            now + Duration::from_secs(60)
        ));
        assert!(!location.is_reusable(
            8,
            rect(10.0, 20.0, 800.0, 600.0),
            now + Duration::from_secs(60)
        ));
        assert!(!location.is_reusable(
            7,
            rect(11.0, 20.0, 800.0, 600.0),
            now + Duration::from_secs(60)
        ));
        assert!(!location.is_reusable(
            7,
            rect(10.0, 20.0, 800.0, 600.0),
            now + CHAT_LOCATION_TTL + Duration::from_secs(1)
        ));
    }

    #[test]
    fn cjk_run_ignores_tesseract_title_noise() {
        let noisy = ">)常德长沙拼车微信交流二群(477)OrvEe-";
        assert_eq!(longest_cjk_run(noisy), "常德长沙拼车微信交流二群");
        assert!(title_matches_recipient(
            longest_cjk_run(noisy),
            "常德长沙拼车微信交流二群"
        ));
        assert!(!title_matches_recipient(
            longest_cjk_run(noisy),
            "常德长沙拼车微信交流一群"
        ));
    }

    #[tokio::test]
    async fn mouse_click_includes_stderr_on_failure() {
        let tmp_dir = std::env::temp_dir();
        let script_path = tmp_dir.join(format!("fake_click_{}.sh", uuid::Uuid::new_v4()));
        std::fs::write(
            &script_path,
            "#!/bin/sh\necho 'accessibility_permission_missing_for_executing_process' >&2\nexit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        unsafe {
            std::env::set_var("WECHAT_CLICK_BIN", &script_path);
        }
        let result = mouse_click(100.0, 100.0).await;
        unsafe {
            std::env::remove_var("WECHAT_CLICK_BIN");
        }
        let _ = std::fs::remove_file(&script_path);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains(
                "wechat_click 退出状态异常: Some(1): accessibility_permission_missing_for_executing_process"
            ),
            "Unexpected error message: {err_msg}"
        );
    }
}
