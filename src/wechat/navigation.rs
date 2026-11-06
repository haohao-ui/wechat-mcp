//! Native visual navigation verifies the search query and an exact result before clicking.
use super::{
    NameColumnMatch, OcrItem, SessionScreenshot, Snapshot, WechatWindow, WindowRect, clipboard,
    ensure_native_snapshot_current, get_ocr_binary_path, key_press, longest_cjk_run, match_name_column,
    mouse_click, native_items, normalize_chat_name, query_wechat_window, verify_chat_title,
    verify_title_items,
};
use crate::desktop::{protocol::Region, runtime};
use anyhow::{Context, Result, bail};
use std::time::Duration;
use tokio::process::Command;

fn search_field_hint(text: &str, recipient: &str) -> bool {
    if normalize_chat_name(text) == normalize_chat_name(recipient) {
        return true;
    }
    // Placeholder hints may be imperfect; query and recipient gates remain exact.
    matches!(longest_cjk_run(text), "搜索" | "搜尋" | "搜察" | "搜素")
        || text.trim().eq_ignore_ascii_case("search")
}

fn query_text_matches(text: &str, recipient: &str) -> bool {
    let value = text.trim();
    let target = recipient.trim();
    value == target
        || value
            .strip_prefix('Q')
            .is_some_and(|suffix| suffix.trim() == target)
}

async fn legacy_region(rect: WindowRect, role: &str) -> Result<Vec<OcrItem>> {
    if !rect.is_valid() {
        bail!("读取区域无效")
    }
    let path = std::env::temp_dir().join(format!("wechat_{role}_{}.png", uuid::Uuid::new_v4()));
    let image = SessionScreenshot(path);
    let status = crate::desktop::overlay::capture(
        Command::new("screencapture")
            .kill_on_drop(true)
            .args([
                "-R",
                &format!(
                    "{},{},{},{}",
                    rect.x.round(),
                    rect.y.round(),
                    rect.width.round(),
                    rect.height.round()
                ),
                "-x",
            ])
            .arg(&image.0)
            .status(),
    )
    .await
    .context("读取导航区域失败")?;
    if !status.success() {
        bail!("导航区域采集失败")
    }
    let result = Command::new(get_ocr_binary_path())
        .kill_on_drop(true)
        .arg(&image.0)
        .output()
        .await?;
    if !result.status.success() {
        bail!("导航区域无法识别文字")
    }
    serde_json::from_slice(&result.stdout).context("导航区域OCR响应无效")
}

async fn unchanged_window(expected: &WechatWindow) -> Result<()> {
    let current = query_wechat_window().await?;
    if current.window_id != expected.window_id
        || current.rect != expected.rect
        || !current.on_screen
    {
        bail!("定位期间微信窗口变化，已停止操作")
    }
    Ok(())
}

/// The legacy capture backend uses the same query/readback/unique-result gates.
pub(super) async fn open_legacy_search(recipient: &str, window: &WechatWindow) -> Result<()> {
    let rect = window.rect;
    let search = WindowRect {
        x: rect.x + 65.0,
        y: rect.y,
        width: 250.0,
        height: 55.0,
    };
    let fields = legacy_region(search, "search").await?;
    let mut matches = fields
        .iter()
        .filter(|item| search_field_hint(&item.text, recipient));
    let field = matches.next().context("无法定位搜索输入框，已停止操作")?;
    if matches.next().is_some() {
        bail!("搜索输入框存在歧义")
    }
    unchanged_window(window).await?;
    mouse_click(
        search.x + field.x * search.width + 4.0,
        search.y + field.y * search.height + 4.0,
    )
    .await?;
    let mut guard = clipboard::ClipboardGuard::replace(recipient).await?;
    let result = async {
        key_press("cmd+a").await?;
        key_press("cmd+v").await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let query = legacy_region(search, "search").await?;
        if !query
            .iter()
            .any(|item| query_text_matches(&item.text, recipient))
        {
            bail!("搜索正文读回与目标不一致，未点击搜索结果");
        }
        let list = WindowRect {
            x: rect.x + 65.0,
            y: rect.y + 60.0,
            width: 250.0,
            height: rect.height - 60.0,
        };
        let items = legacy_region(list, "list").await?;
        let item = match match_name_column(&items, recipient) {
            NameColumnMatch::Unique(item) => item,
            NameColumnMatch::Missing => bail!("搜索结果未找到完整目标名称，未点击搜索结果"),
            NameColumnMatch::Ambiguous => bail!("搜索结果有同名候选，未点击搜索结果"),
        };
        unchanged_window(window).await?;
        mouse_click(
            list.x + item.x * list.width + 4.0,
            list.y + item.y * list.height + 4.0,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        verify_chat_title(recipient).await
    }
    .await;
    let restored = guard.restore().await;
    result.and(restored)
}

fn name_location(snapshot: &Snapshot, recipient: &str) -> Result<Option<(f64, f64)>> {
    snapshot.validate()?;
    let items = native_items(snapshot);
    match match_name_column(&items, recipient) {
        NameColumnMatch::Unique(item) => {
            let line = snapshot
                .lines
                .iter()
                .find(|line| {
                    line.text == item.text && line.bounds.x == item.x && line.bounds.y == item.y
                })
                .context("name candidate lost its source rectangle")?;
            let region = snapshot.region;
            let bounds = line.bounds;
            Ok(Some((
                snapshot.window.x
                    + (region.x + (bounds.x + bounds.width / 2.0) * region.width)
                        * snapshot.window.width,
                snapshot.window.y
                    + (region.y + (bounds.y + bounds.height / 2.0) * region.height)
                        * snapshot.window.height,
            )))
        }
        NameColumnMatch::Missing => Ok(None),
        NameColumnMatch::Ambiguous => bail!("搜索结果名称重复，无法安全选择目标"),
    }
}

async fn click_result(snapshot: &Snapshot, recipient: &str) -> Result<bool> {
    let Some((x, y)) = name_location(snapshot, recipient)? else {
        return Ok(false);
    };
    ensure_native_snapshot_current(snapshot).await?;
    mouse_click(x, y).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    verify_chat_title(recipient).await?;
    Ok(true)
}

pub(super) async fn open_if_native(recipient: &str) -> Result<bool> {
    let Some(title) = runtime::shared().snapshot(Region::Title, false).await? else {
        return Ok(false);
    };
    ensure_native_snapshot_current(&title).await?;
    if verify_title_items(native_items(&title), recipient).is_ok() {
        return Ok(true);
    }
    let sessions = runtime::shared()
        .snapshot(Region::Sessions, false)
        .await?
        .context("native conversation list unavailable")?;
    if title.lines.is_empty() && sessions.lines.is_empty() {
        bail!("微信窗口可见但标题和会话区均无可读文字，无法核验目标；请检查窗口内容或采集通道");
    }
    if click_result(&sessions, recipient).await? {
        return Ok(true);
    }

    let field = runtime::shared()
        .snapshot(Region::Search, false)
        .await?
        .context("native search capture unavailable")?;
    let mut fields = field
        .lines
        .iter()
        .filter(|line| search_field_hint(&line.text, recipient));
    let placeholder = fields
        .next()
        .context("未识别到搜索输入区域，已停止定位；请标定 search 区域")?;
    if fields.next().is_some() {
        bail!("搜索输入区域有多个候选，已停止定位")
    }
    ensure_native_snapshot_current(&field).await?;
    let region = field.region;
    mouse_click(
        field.window.x
            + (region.x + (placeholder.bounds.x + placeholder.bounds.width / 2.0) * region.width)
                * field.window.width,
        field.window.y
            + (region.y + (placeholder.bounds.y + placeholder.bounds.height / 2.0) * region.height)
                * field.window.height,
    )
    .await?;
    let mut guard = clipboard::ClipboardGuard::replace(recipient).await?;
    let operation = async {
        key_press("cmd+a").await?;
        key_press("cmd+v").await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let query = runtime::shared()
            .snapshot(Region::Search, false)
            .await?
            .context("native search readback unavailable")?;
        ensure_native_snapshot_current(&query).await?;
        if query.epoch != field.epoch || query.window != field.window {
            bail!("搜索输入期间窗口变化，已停止定位");
        }
        if !query
            .lines
            .iter()
            .any(|line| query_text_matches(&line.text, recipient))
        {
            bail!("搜索正文读回与目标不一致，未点击任何搜索结果");
        }
        let results = runtime::shared()
            .snapshot(Region::Sessions, false)
            .await?
            .context("native search results unavailable")?;
        if results.epoch != query.epoch || results.window != query.window {
            bail!("搜索结果窗口变化，已停止定位");
        }
        if !click_result(&results, recipient).await? {
            bail!("搜索结果未找到唯一完整目标名称，未点击搜索结果");
        }
        Ok(true)
    }
    .await;
    let restored = guard.restore().await;
    operation.and(restored.map(|()| true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_readback_removes_only_the_known_magnifier_prefix() {
        for value in ["示例联系人甲", "Q示例联系人甲", "Q 示例联系人甲"] {
            assert!(query_text_matches(value, "示例联系人甲"));
        }
        for value in ["午夜咖", "Q午夜咖", "示例联系人甲x", "Wrong"] {
            assert!(!query_text_matches(value, "示例联系人甲"));
        }
        assert!(query_text_matches("Q示例联系人甲", "Q示例联系人甲"));
    }

    #[test]
    fn search_hints_tolerate_placeholder_noise_without_fuzzy_recipient_matching() {
        for hint in ["搜索", "Q 搜察", "搜素", "Search", "示例联系人甲"] {
            assert!(search_field_hint(hint, "示例联系人甲"), "{hint}");
        }
        for text in ["发送", "新的朋友", "午夜咖", "搜狗"] {
            assert!(!search_field_hint(text, "示例联系人甲"), "{text}");
        }
    }

    fn snapshot() -> Snapshot {
        serde_json::from_value(json!({
            "epoch":"test","sequence":1,"captured_at_ms":1,"digest":"a".repeat(64),"ocr_cached":false,
            "window":{"window_id":1,"pid":2,"x":-100,"y":40,"width":1000,"height":800,"on_screen":true},
            "region":{"x":0.1,"y":0.1,"width":0.3,"height":0.8},
            "lines":[{"text":"示例联系人甲","confidence":0.9,"bounds":{"x":0.2,"y":0.3,"width":0.4,"height":0.1}}]
        })).unwrap()
    }

    #[test]
    fn name_click_uses_region_and_text_bounds_in_desktop_points() {
        let (x, y) = name_location(&snapshot(), "示例联系人甲").unwrap().unwrap();
        assert!((x - 120.0).abs() < 1e-9);
        assert!((y - 344.0).abs() < 1e-9);
    }

    #[test]
    fn ambiguous_preview_or_invalid_candidates_cannot_produce_clicks() {
        let mut frame = snapshot();
        let mut duplicate = frame.lines[0].clone();
        duplicate.bounds.y = 0.6;
        frame.lines.push(duplicate);
        assert!(name_location(&frame, "示例联系人甲").is_err());
        frame.lines.pop();
        frame.lines[0].bounds.x = 0.8;
        frame.lines[0].bounds.width = 0.1;
        assert!(name_location(&frame, "示例联系人甲").unwrap().is_none());
        frame.lines[0].bounds.width = 2.0;
        assert!(name_location(&frame, "示例联系人甲").is_err());
    }
}
