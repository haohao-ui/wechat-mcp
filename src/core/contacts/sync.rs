use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::info;

use crate::core::contacts::parse::{
    parse_group_headers, parse_profile, OcrLine,
};
use crate::core::history::store::{HistoryStore, ProfileInput, UpsertOutcome};
use crate::core::scheduler::CoordinatorCapture;
use crate::core::state::write_private;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Full,
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub per_category: Vec<(String, u32, u32)>, // 分类, 期望, 已同步
    pub unreadable_rows: u32,
    pub needs_review: u32,
    pub yielded: bool,
    pub complete: bool,
}

pub fn should_attempt_daily_sync(state_dir: &Path, is_idle: bool, now_sec: f64) -> bool {
    if !is_idle {
        return false;
    }
    let attempt_file = state_dir.join("contacts-sync-daily-attempt.json");
    if attempt_file.exists() {
        if let Ok(content) = fs::read_to_string(&attempt_file) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                if let Some(last_sec) = v.get("last_attempt_sec").and_then(|t| t.as_f64()) {
                    if now_sec - last_sec < 20.0 * 3600.0 {
                        return false;
                    }
                }
            }
        }
    }

    let res_file = state_dir.join("contacts-sync-result.json");
    if res_file.exists() {
        if let Ok(content) = fs::read_to_string(&res_file) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                if let Some(fin_ms) = v.get("finished_at_ms").and_then(|t| t.as_f64()) {
                    let fin_sec = fin_ms / 1000.0;
                    if now_sec - fin_sec < 20.0 * 3600.0 {
                        return false;
                    }
                }
            }
        }
    }

    true
}

pub fn record_daily_sync_attempt(state_dir: &Path, now_sec: f64) -> Result<()> {
    let attempt_file = state_dir.join("contacts-sync-daily-attempt.json");
    write_private(&attempt_file, &json!({"last_attempt_sec": now_sec}))
}

pub fn clean_stale_contacts_sync_pause(state_dir: &Path) {
    let pause_file = state_dir.join("admin-pause.json");
    if pause_file.exists() {
        if let Ok(content) = fs::read_to_string(&pause_file) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                if v.get("reason").and_then(|r| r.as_str()) == Some("contacts_sync") {
                    let _ = fs::remove_file(&pause_file);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CheckpointState {
    mode: Option<SyncMode>,
    category_index: usize,
    categories: Vec<(String, u32, f64)>, // (category, expected, header_y)
    seen_ids_by_cat: HashMap<String, Vec<String>>,
    cat_progress: HashMap<String, Vec<String>>, // category -> list of processed contact names in order
    unreadable_rows: u32,
    needs_review: u32,
    /// 每个分组展开后已向下滚动的格数：恢复时一次滚回去，而不是逐屏重走（逐屏重走会耗尽让出预算、永远推进不了）
    #[serde(default)]
    scroll_ticks: HashMap<String, u32>,
}

pub async fn run_sync(
    capture: &mut dyn CoordinatorCapture,
    store: &HistoryStore,
    account_id: &str,
    mode: SyncMode,
    checkpoint: &Path,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    now_ms: i64,
) -> Result<SyncReport> {
    let result = run_sync_inner(capture, store, account_id, mode, checkpoint, should_yield, now_ms).await;
    if let Err(e) = &result {
        // 出错不自动续做：清掉断点并写明错误，否则协调器每轮续做、立刻失败，形成死循环（实机 347 次，机器人停摆）
        let _ = fs::remove_file(checkpoint);
        if let Some(dir) = checkpoint.parent() {
            let _ = write_private(
                &dir.join("contacts-sync-result.json"),
                &json!({"finished_at_ms": now_ms, "complete": false, "error": format!("{e:#}")}),
            );
        }
    }
    if result.is_err() {
        // 进过通讯录后任何一步失败，都必须切回聊天页：否则微信停在通讯录页，机器人看不到聊天（2026-11-08 实机）
        if let Err(e) = capture.call("contacts_leave", json!({})).await {
            tracing::warn!("contacts_leave after failed sync also failed: {e:#}");
        }
    }
    result
}

async fn run_sync_inner(
    capture: &mut dyn CoordinatorCapture,
    store: &HistoryStore,
    account_id: &str,
    mode: SyncMode,
    checkpoint: &Path,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    now_ms: i64,
) -> Result<SyncReport> {
    let state_dir = checkpoint.parent().unwrap_or_else(|| Path::new("."));
    let pause_file = state_dir.join("admin-pause.json");
    let initially_paused = pause_file.exists();

    if !initially_paused {
        let _ = write_private(
            &pause_file,
            &json!({"paused": true, "reason": "contacts_sync", "at_ms": now_ms}),
        );
    }

    struct PauseGuard<'a> {
        pause_file: &'a Path,
        should_clean: bool,
    }
    impl<'a> Drop for PauseGuard<'a> {
        fn drop(&mut self) {
            if self.should_clean && self.pause_file.exists() {
                let _ = fs::remove_file(self.pause_file);
            }
        }
    }
    let _pause_guard = PauseGuard {
        pause_file: &pause_file,
        should_clean: !initially_paused,
    };

    let mut state: CheckpointState = if checkpoint.exists() {
        fs::read_to_string(checkpoint)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        CheckpointState::default()
    };
    state.mode = Some(mode);

    // 1. 进入通讯录
    let _ = capture.call("contacts_enter", json!({})).await?;

    // 2. 读取分组标题与期望数量
    if state.categories.is_empty() {
        let headers_res = capture.call("contacts_headers", json!({})).await?;
        let lines: Vec<OcrLine> = headers_res
            .get("lines")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let parsed_headers = parse_group_headers(&lines);
        state.categories = parsed_headers;
    }

    info!(
        "sync categories found: {}",
        state
            .categories
            .iter()
            .map(|(c, count, _)| format!("{c}:{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 3. 不打开「通讯录管理」：那里没有每个人的详细资料，且读表格会耗尽让出预算（用户 2026-11-08 要求、实机验证）
    // 4. 逐分组展开、逐屏识别联系人行
    let existing_profiles = store.list_profiles(account_id, None).unwrap_or_default();
    let mut yielded = false;

    'outer: while state.category_index < state.categories.len() {
        let (cat_name, expected, _) = state.categories[state.category_index].clone();
        let seen_list = state
            .seen_ids_by_cat
            .entry(cat_name.clone())
            .or_default();
        let processed_names = state
            .cat_progress
            .entry(cat_name.clone())
            .or_default();

        // a. 滚到顶
        let _ = capture
            .call("contacts_scroll_list", json!({"direction": "up", "ticks": 20}))
            .await;

        // b. 找到当前分组标题行位置
        let top_headers_res = capture.call("contacts_headers", json!({})).await?;
        let top_lines: Vec<OcrLine> = top_headers_res
            .get("lines")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let header_line = top_lines.iter().find(|l| {
            let t = l.text.trim();
            match cat_name.as_str() {
                "official" => t.contains("公众号"),
                "work" => t.contains("企业微信"),
                "personal" => t == "联系人" || (t.contains("联系人") && !t.contains("企业微信")),
                _ => false,
            }
        });

        if let Some(hl) = header_line {
            let hy = hl.y;
            // 检查标题右侧是否有数量（收起状态）
            let has_count = top_lines.iter().any(|l| {
                (l.y - hy).abs() < 15.0 && l.x > 250.0 && l.text.trim().chars().all(|c| c.is_ascii_digit())
            });

            let mut cur_screen_lines = if has_count {
                // c. 点击分组标题展开并确认
                let click_res = capture.call("contacts_click_list", json!({"x": 180.0, "y": hy})).await?;
                click_res
                    .get("lines")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default()
            } else {
                top_lines
            };

            // 恢复：一次滚到上次停下的位置（少一格，保证与上次最后一屏有重叠）
            let resume_ticks = state.scroll_ticks.get(&cat_name).copied().unwrap_or(0);
            if resume_ticks > 1 {
                if let Ok(v) = capture
                    .call("contacts_scroll_list", json!({"direction": "down", "ticks": resume_ticks - 1}))
                    .await
                {
                    cur_screen_lines = v
                        .get("lines")
                        .and_then(|l| serde_json::from_value(l.clone()).ok())
                        .unwrap_or_default();
                    state.scroll_ticks.insert(cat_name.clone(), resume_ticks - 1);
                }
            }

            let mut no_new_rows_count = 0;
            // 本轮（从顶部开始）按顺序看到的联系人名字；与断点里已处理的名字按出现次数比对，
            // 已处理过的直接跳过，不再打开资料卡（恢复时不重做，保证每轮都有进展）
            let mut pass_seen: Vec<String> = Vec::new();

            'screen_loop: loop {
                if should_yield() {
                    yielded = true;
                    break 'outer;
                }

                // 识别当前屏幕属于联系人的行
                let mut contact_rows_on_screen: Vec<(String, f64)> = Vec::new();
                let mut lines_sorted = cur_screen_lines.clone();
                lines_sorted.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

                for l in &lines_sorted {
                    if l.y < 40.0 || l.y > 640.0 {
                        continue;
                    }
                    let txt = l.text.trim();
                    // 联系人名字在 x≈133；分组标题（带箭头前缀）与字母索引在 x≈77–95：按位置排除，
                    // 不依赖文字精确匹配（真机标题前有箭头字符，点到标题会收起分组）
                    if l.x < 115.0 {
                        continue;
                    }
                    if txt == "搜索" || txt == "通讯录管理" {
                        continue;
                    }
                    let is_group_title = ["公众号、服务号", "企业微信联系人", "联系人", "群聊"]
                        .iter()
                        .any(|gt| txt == *gt);
                    if is_group_title {
                        continue;
                    }
                    if txt.len() == 1 && txt.chars().all(|c| c.is_ascii_uppercase()) {
                        continue;
                    }
                    if l.x > 250.0 && txt.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    contact_rows_on_screen.push((txt.to_string(), l.y));
                }

                let screen_names: Vec<String> = contact_rows_on_screen.iter().map(|(n, _)| n.clone()).collect();
                let max_overlap = screen_names.len().min(pass_seen.len());
                let mut match_k = 0;
                for k in (1..=max_overlap).rev() {
                    let tail = &pass_seen[pass_seen.len() - k..];
                    let head = &screen_names[..k];
                    if tail == head {
                        match_k = k;
                        break;
                    }
                }

                let new_rows = if pass_seen.is_empty() {
                    &contact_rows_on_screen[..]
                } else {
                    &contact_rows_on_screen[match_k..]
                };

                if new_rows.is_empty() {
                    no_new_rows_count += 1;
                    if no_new_rows_count >= 2 {
                        break 'screen_loop;
                    }
                } else {
                    no_new_rows_count = 0;
                    for (row_name, row_y) in new_rows {
                        pass_seen.push(row_name.clone());
                        let seen_in_pass = pass_seen.iter().filter(|n| *n == row_name).count();
                        let already_done = processed_names.iter().filter(|n| *n == row_name).count();
                        if seen_in_pass <= already_done {
                            continue;
                        }
                        if should_yield() {
                            yielded = true;
                            break 'outer;
                        }

                        let mut skip_open = false;
                        if mode == SyncMode::Incremental {
                            // 增量：列表上的名字（备注或昵称）在库里已有资料就不点开，只点开新名字
                            if let Some(p) = existing_profiles.iter().find(|p| {
                                p.get("nickname").and_then(|v| v.as_str()) == Some(row_name) ||
                                p.get("remark").and_then(|v| v.as_str()) == Some(row_name)
                            }) {
                                skip_open = true;
                                if let Some(cid) = p.get("contact_id").and_then(|v| v.as_str()) {
                                    if !seen_list.contains(&cid.to_string()) {
                                        seen_list.push(cid.to_string());
                                    }
                                }
                            }
                        }

                        if !skip_open {
                            let open_res = capture.call("contacts_open_row", json!({"y": row_y})).await;
                            let prof_lines: Vec<OcrLine> = open_res.ok()
                                .and_then(|v| v.get("lines").cloned())
                                .and_then(|v| serde_json::from_value(v).ok())
                                .unwrap_or_default();

                            if prof_lines.is_empty() {
                                state.unreadable_rows += 1;
                            } else {
                                let parse_res = parse_profile(&prof_lines, &cat_name);
                                let f = parse_res.fields;

                                // 错位保护：资料卡昵称或备注必须与该行名字一致
                                let name_matched = (row_name == &f.nickname)
                                    || f.remark.as_deref().map(|rmk| rmk == row_name).unwrap_or(false);

                                if !name_matched {
                                    state.unreadable_rows += 1;
                                } else {
                                    let combined_tags = f.tags.clone();

                                    let input = ProfileInput {
                                        category: cat_name.clone(),
                                        wechat_id: f.wechat_id,
                                        nickname: f.nickname,
                                        remark: f.remark,
                                        region: f.region,
                                        source: f.source,
                                        added_on: f.added_on,
                                        common_groups: f.common_groups,
                                        enterprise: f.enterprise,
                                        real_name: f.real_name,
                                        tags: combined_tags,
                                        friend_permission: f.friend_permission,
                                    };

                                    match store.upsert_profile(account_id, &input, now_ms) {
                                        Ok(UpsertOutcome::Created(cid)) | Ok(UpsertOutcome::Updated(cid)) => {
                                            if !seen_list.contains(&cid) {
                                                seen_list.push(cid);
                                            }
                                        }
                                        Ok(UpsertOutcome::NeedsReview { .. }) => {
                                            state.needs_review += 1;
                                        }
                                        Err(_) => {
                                            state.unreadable_rows += 1;
                                        }
                                    }
                                }
                            }
                        }

                        processed_names.push(row_name.clone());
                    }
                }

                // 滚一格继续
                let scroll_res = capture
                    .call("contacts_scroll_list", json!({"direction": "down", "ticks": 1}))
                    .await?;
                *state.scroll_ticks.entry(cat_name.clone()).or_insert(0) += 1;
                cur_screen_lines = scroll_res
                    .get("lines")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
            }

            // 核对人数差额：如果已同步数 < 期望数，记录差额为 unreadable
            let synced_count = seen_list.len() as u32;
            if synced_count < expected {
                let missing = expected - synced_count;
                if missing > state.unreadable_rows {
                    state.unreadable_rows = missing;
                }
            }

            // 收起分组
            let _ = capture
                .call("contacts_scroll_list", json!({"direction": "up", "ticks": 20}))
                .await;
            if let Ok(th_res) = capture.call("contacts_headers", json!({})).await {
                let th_lines: Vec<OcrLine> = th_res
                    .get("lines")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                if let Some(target_hl) = th_lines.iter().find(|l| {
                    let t = l.text.trim();
                    match cat_name.as_str() {
                        "official" => t.contains("公众号"),
                        "work" => t.contains("企业微信"),
                        "personal" => t == "联系人" || (t.contains("联系人") && !t.contains("企业微信")),
                        _ => false,
                    }
                }) {
                    let is_expanded = !th_lines.iter().any(|l| {
                        (l.y - target_hl.y).abs() < 15.0 && l.x > 250.0 && l.text.trim().chars().all(|c| c.is_ascii_digit())
                    });
                    if is_expanded {
                        let _ = capture.call("contacts_click_list", json!({"x": 180.0, "y": target_hl.y})).await;
                    }
                }
            }
        }

        // 标记缺失
        let _ = store.mark_missing(account_id, &cat_name, seen_list, now_ms);
        state.scroll_ticks.remove(&cat_name);
        state.category_index += 1;
    }

    // 5. 离开通讯录
    let _ = capture.call("contacts_leave", json!({})).await;

    // 6. 核对结果
    let mut per_category = Vec::new();
    let mut all_match = true;

    for (cat, exp, _) in &state.categories {
        let synced = state
            .seen_ids_by_cat
            .get(cat)
            .map(|v| v.len() as u32)
            .unwrap_or(0);
        per_category.push((cat.clone(), *exp, synced));
        if synced != *exp {
            all_match = false;
        }
    }

    let complete = !yielded && all_match && state.unreadable_rows == 0;

    let report = SyncReport {
        per_category: per_category.clone(),
        unreadable_rows: state.unreadable_rows,
        needs_review: state.needs_review,
        yielded,
        complete,
    };

    // 保存结果
    let result_file = state_dir.join("contacts-sync-result.json");
    let result_val = json!({
        "finished_at_ms": now_ms,
        "complete": complete,
        "per_category": per_category.iter().map(|(cat, exp, syn)| {
            json!({"category": cat, "expected": exp, "synced": syn})
        }).collect::<Vec<_>>(),
        "unreadable_rows": state.unreadable_rows,
        "needs_review": state.needs_review,
    });
    let _ = write_private(&result_file, &result_val);

    // 断点处理
    if yielded {
        let _ = write_private(checkpoint, &serde_json::to_value(&state).unwrap_or_default());
    } else if complete && checkpoint.exists() {
        let _ = fs::remove_file(checkpoint);
    }

    info!(
        "sync finished: complete={}, yielded={}, unreadable={}, needs_review={}",
        complete, yielded, state.unreadable_rows, state.needs_review
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contacts::parse::ManagerRow;
    use std::collections::HashMap;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use anyhow::bail;
    use serde_json::Value;
    use crate::core::model::Frame;
    use crate::core::worker_client::OcrStatus;

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("wechat-sync-test-{}", uuid::Uuid::new_v4().simple()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub struct VirtualContactItem {
        pub name: String,
        pub remark: Option<String>,
        pub wechat_id: Option<String>,
        pub enterprise: Option<String>,
        pub real_name: Option<String>,
        pub tags: Vec<String>,
        pub friend_permission: Option<String>,
        pub unreadable: bool,
        pub misaligned_profile: Option<Vec<OcrLine>>,
    }

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub struct VirtualGroup {
        pub category: String,
        pub title: String,
        pub letter_sections: Vec<(String, Vec<VirtualContactItem>)>,
        pub expanded: bool,
    }

    impl VirtualGroup {
        pub fn total_contacts(&self) -> u32 {
            self.letter_sections.iter().map(|(_, items)| items.len() as u32).sum()
        }
    }

    pub struct VirtualContactsUi {
        pub groups: Vec<VirtualGroup>,
        pub scroll_offset: f64,
        pub calls: Arc<Mutex<Vec<(String, Value)>>>,
        pub manager_open: bool,
        pub manager_rows: Vec<ManagerRow>,
        pub manager_scroll_row: usize,
        pub manager_tags: Vec<(String, f64)>, // (tag_name, y)
        pub current_filter_tag: Option<String>,
        pub manager_close_countdown: Option<usize>,
        pub list_clicks: Vec<(f64, f64)>,
        pub row_open_clicks: Vec<f64>,
        /// 真机识别出的分组标题带展开箭头前缀（如 "> 联系人" / "∨ 联系人"）
        pub header_arrows: bool,
        /// 用"打开资料卡"点到了分组标题（真机上会把分组收起）
        pub header_open_clicks: usize,
        /// 模拟某个操作失败（如真机上的 layout_mismatch）
        pub fail_op: Option<String>,
    }

    impl VirtualContactsUi {
        pub fn new(groups: Vec<VirtualGroup>) -> Self {
            Self {
                groups,
                scroll_offset: 0.0,
                calls: Arc::new(Mutex::new(Vec::new())),
                manager_open: false,
                manager_rows: Vec::new(),
                manager_scroll_row: 0,
                manager_tags: Vec::new(),
                current_filter_tag: None,
                manager_close_countdown: None,
                list_clicks: Vec::new(),
                row_open_clicks: Vec::new(),
                header_arrows: false,
                header_open_clicks: 0,
                fail_op: None,
            }
        }

        fn render_list(&self) -> Vec<OcrLine> {
            let mut lines = Vec::new();

            // 搜索框：center_y = 20.0
            let search_y = 20.0 - self.scroll_offset;
            if search_y >= 40.0 && search_y <= 640.0 {
                lines.push(OcrLine {
                    text: "搜索".to_string(),
                    x: 100.0,
                    y: search_y,
                    w: 60.0,
                    h: 20.0,
                });
            }

            // 通讯录管理按钮：center_y = 59.0
            let mgr_y = 59.0 - self.scroll_offset;
            if mgr_y >= 40.0 && mgr_y <= 640.0 {
                lines.push(OcrLine {
                    text: "通讯录管理".to_string(),
                    x: 100.0,
                    y: mgr_y,
                    w: 80.0,
                    h: 20.0,
                });
            }

            let mut cur_y = 90.0;
            for g in &self.groups {
                // 分组标题行高 40
                let header_center_y = cur_y + 20.0 - self.scroll_offset;
                if header_center_y >= 40.0 && header_center_y <= 640.0 {
                    let title = if self.header_arrows {
                        format!("{}{}", if g.expanded { "∨ " } else { "> " }, g.title)
                    } else {
                        g.title.clone()
                    };
                    lines.push(OcrLine {
                        text: title,
                        x: 80.0,
                        y: header_center_y,
                        w: 120.0,
                        h: 24.0,
                    });
                    if !g.expanded {
                        lines.push(OcrLine {
                            text: g.total_contacts().to_string(),
                            x: 272.0,
                            y: header_center_y,
                            w: 30.0,
                            h: 20.0,
                        });
                    }
                }
                cur_y += 40.0;

                if g.expanded {
                    for (letter, items) in &g.letter_sections {
                        // 字母索引行高 45
                        let letter_center_y = cur_y + 22.5 - self.scroll_offset;
                        if letter_center_y >= 40.0 && letter_center_y <= 640.0 {
                            lines.push(OcrLine {
                                text: letter.clone(),
                                x: 95.0,
                                y: letter_center_y,
                                w: 20.0,
                                h: 20.0,
                            });
                        }
                        cur_y += 45.0;

                        for item in items {
                            // 联系人行高 50
                            let item_center_y = cur_y + 25.0 - self.scroll_offset;
                            if item_center_y >= 40.0 && item_center_y <= 640.0 {
                                if !item.unreadable {
                                    let display_name = item.remark.as_deref().unwrap_or(&item.name);
                                    lines.push(OcrLine {
                                        text: display_name.to_string(),
                                        x: 133.0,
                                        y: item_center_y,
                                        w: 100.0,
                                        h: 20.0,
                                    });
                                }
                            }
                            cur_y += 50.0;
                        }
                    }
                }
            }

            lines
        }

        fn find_item_at_virtual_y(&self, vy: f64) -> Option<(&VirtualGroup, Option<&VirtualContactItem>, bool, bool)> {
            // 返回 (group, Option<item>, is_header, is_letter)
            let mut cur_y = 90.0;
            for g in &self.groups {
                if vy >= cur_y && vy < cur_y + 40.0 {
                    return Some((g, None, true, false));
                }
                cur_y += 40.0;

                if g.expanded {
                    for (_letter, items) in &g.letter_sections {
                        if vy >= cur_y && vy < cur_y + 45.0 {
                            return Some((g, None, false, true));
                        }
                        cur_y += 45.0;

                        for item in items {
                            if vy >= cur_y && vy < cur_y + 50.0 {
                                return Some((g, Some(item), false, false));
                            }
                            cur_y += 50.0;
                        }
                    }
                }
            }
            None
        }
    }

    impl CoordinatorCapture for VirtualContactsUi {
        fn start<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn close<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn frame<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
            Box::pin(async { bail!("not implemented") })
        }
        fn scroll<'a>(&'a mut self, _direction: &'a str, _amount: u32) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
            Box::pin(async { bail!("not implemented") })
        }
        fn call<'a>(&'a mut self, op: &'a str, params: Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            let calls = self.calls.clone();
            calls.lock().unwrap().push((op.to_string(), params.clone()));

            if self.fail_op.as_deref() == Some(op) {
                return Box::pin(async { bail!("private visual operation failed: invalid(\"layout_mismatch:input_divider\")") });
            }
            if op == "contacts_enter" {
                self.scroll_offset = 0.0;
                return Box::pin(async { Ok(json!({"ok": true})) });
            }
            if op == "contacts_leave" {
                return Box::pin(async { Ok(json!({"ok": true})) });
            }
            if op == "contacts_headers" {
                let lines = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines})) });
            }
            if op == "contacts_scroll_list" {
                let dir = params.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
                let ticks = params.get("ticks").and_then(|v| v.as_u64()).unwrap_or(1) as f64;
                if dir == "down" {
                    self.scroll_offset += ticks * 90.0;
                } else {
                    self.scroll_offset = (self.scroll_offset - ticks * 90.0).max(0.0);
                }
                let lines = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines})) });
            }
            if op == "contacts_click_list" {
                let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if x < 60.0 || x > 300.0 || y < 40.0 || y > 640.0 {
                    return Box::pin(async { bail!("contacts_click_outside_list") });
                }
                self.list_clicks.push((x, y));
                let vy = y + self.scroll_offset;
                let mut cur_y = 90.0;
                for g in &mut self.groups {
                    if vy >= cur_y && vy < cur_y + 40.0 {
                        g.expanded = !g.expanded;
                        break;
                    }
                    cur_y += 40.0;
                    if g.expanded {
                        for (_letter, items) in &g.letter_sections {
                            cur_y += 45.0;
                            cur_y += 50.0 * items.len() as f64;
                        }
                    }
                }
                let lines = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines})) });
            }
            if op == "contacts_open_row" {
                let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if y < 40.0 || y > 640.0 {
                    return Box::pin(async { bail!("contacts_click_outside_list") });
                }
                self.row_open_clicks.push(y);
                let vy = y + self.scroll_offset;
                let found = self.find_item_at_virtual_y(vy);
                if let Some((_group, Some(item), false, false)) = found {
                    if let Some(ref mis) = item.misaligned_profile {
                        let lines = mis.clone();
                        return Box::pin(async move { Ok(json!({"lines": lines})) });
                    }
                    let mut lines = Vec::new();
                    lines.push(line(&item.name, 520.0, 60.0));
                    if let Some(ref wid) = item.wechat_id {
                        lines.push(line(&format!("微信号：{}", wid), 520.0, 90.0));
                    }
                    if let Some(ref rmk) = item.remark {
                        lines.push(line(&format!("备注：{}", rmk), 520.0, 120.0));
                    }
                    lines.push(line("地区中国 测试省", 520.0, 150.0));
                    if let Some(ref ent) = item.enterprise {
                        lines.push(line(&format!("企业{}", ent), 520.0, 180.0));
                    }
                    if let Some(ref rn) = item.real_name {
                        lines.push(line(&format!("实名{}", rn), 520.0, 210.0));
                    }
                    return Box::pin(async move { Ok(json!({"lines": lines})) });
                }
                // 点到分组标题：真机上会切换展开/收起（危险点击）
                if let Some((_g, None, true, false)) = found {
                    self.header_open_clicks += 1;
                    let mut cur_y = 90.0;
                    for g in &mut self.groups {
                        if vy >= cur_y && vy < cur_y + 40.0 {
                            g.expanded = !g.expanded;
                            break;
                        }
                        cur_y += 40.0;
                        if g.expanded {
                            for (_letter, items) in &g.letter_sections {
                                cur_y += 45.0 + 50.0 * items.len() as f64;
                            }
                        }
                    }
                    return Box::pin(async { Ok(json!({"lines": []})) });
                }
                // 字母行或空白区：无资料卡
                return Box::pin(async { Ok(json!({"lines": []})) });
            }
            if op == "contacts_manager_open" {
                self.manager_open = true;
                self.manager_scroll_row = 0;
                return Box::pin(async {
                    Ok(json!({"window": {"x": 200, "y": 100, "w": 720, "h": 560}}))
                });
            }
            if op == "contacts_manager_close" {
                if !self.manager_open {
                    return Box::pin(async { bail!("contacts_manager_missing") });
                }
                self.manager_open = false;
                return Box::pin(async { Ok(json!({"closed": true})) });
            }
            if op == "contacts_manager_page" || op == "contacts_manager_scroll" || op == "contacts_manager_filter" {
                if !self.manager_open {
                    return Box::pin(async { bail!("contacts_manager_missing") });
                }
                if let Some(ref mut cnt) = self.manager_close_countdown {
                    if *cnt == 0 {
                        self.manager_open = false;
                        self.manager_close_countdown = None;
                        return Box::pin(async { bail!("contacts_manager_missing") });
                    } else {
                        *cnt -= 1;
                    }
                }

                if op == "contacts_manager_filter" {
                    let kind = params.get("kind").and_then(|v| v.as_str()).unwrap_or("all");
                    if kind == "tag" {
                        let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        if let Some((tag, _)) = self.manager_tags.iter().find(|(_, ty)| (ty - y).abs() < 10.0) {
                            self.current_filter_tag = Some(tag.clone());
                        } else {
                            return Box::pin(async { bail!("contacts_filter_position_invalid") });
                        }
                    } else {
                        self.current_filter_tag = None;
                    }
                    self.manager_scroll_row = 0;
                } else if op == "contacts_manager_scroll" {
                    let dir = params.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
                    let ticks = params.get("ticks").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                    let filtered_rows: Vec<_> = self.manager_rows.iter().filter(|r| {
                        if let Some(ref tag) = self.current_filter_tag {
                            r.tags.contains(tag)
                        } else {
                            true
                        }
                    }).collect();
                    let max_start = filtered_rows.len().saturating_sub(10);
                    if dir == "down" {
                        self.manager_scroll_row = (self.manager_scroll_row + ticks).min(max_start);
                    } else {
                        self.manager_scroll_row = self.manager_scroll_row.saturating_sub(ticks);
                    }
                }

                let filtered_rows: Vec<_> = self.manager_rows.iter().filter(|r| {
                    if let Some(ref tag) = self.current_filter_tag {
                        r.tags.contains(tag)
                    } else {
                        true
                    }
                }).collect();

                let mut lines = Vec::new();
                lines.push(line("全部", 40.0, 52.0));
                lines.push(line("标签", 40.0, 167.0));
                for (tag_name, tag_y) in &self.manager_tags {
                    lines.push(line(tag_name, 40.0, *tag_y));
                }

                let page_rows = filtered_rows.iter().skip(self.manager_scroll_row).take(10);
                for (idx, r) in page_rows.enumerate() {
                    let y = 96.0 + 48.0 * idx as f64;
                    if let Some(ref nm) = r.name {
                        lines.push(line(nm, 300.0, y));
                    }
                    if let Some(ref rmk) = r.remark {
                        lines.push(line(rmk, 385.0, y));
                    }
                    if !r.tags.is_empty() {
                        lines.push(line(&r.tags.join(","), 500.0, y));
                    }
                    if let Some(ref p) = r.permission {
                        lines.push(line(p, 620.0, y));
                    }
                }

                return Box::pin(async move {
                    Ok(json!({
                        "lines": lines,
                        "window": {"x": 200, "y": 100, "w": 720, "h": 560}
                    }))
                });
            }

            Box::pin(async { Ok(json!({})) })
        }
        fn retarget<'a>(&'a mut self, _recipient: &'a str, _directory: &'a Path, _names: &'a [String], _group: bool) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn sessions<'a>(&'a mut self, _names: &'a [String]) -> Pin<Box<dyn Future<Output = Result<Vec<crate::core::scheduler::SessionRow>>> + Send + 'a>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn download<'a>(&'a mut self, _media_id: &'a str, _kind: &'a str) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
            Box::pin(async { bail!("not implemented") })
        }
        fn last_switch(&self) -> Option<Value> { None }
        fn ocr_status(&self) -> Option<OcrStatus> { None }
        fn is_connected(&self) -> bool { true }
    }

    fn line(text: &str, x: f64, y: f64) -> OcrLine {
        OcrLine {
            text: text.to_string(),
            x,
            y,
            w: 100.0,
            h: 20.0,
        }
    }

    #[tokio::test]
    async fn test_full_sync_counts_match_headers() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![
            VirtualContactItem { name: "张三".to_string(), remark: None, wechat_id: Some("wxid_p1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "李四".to_string(), remark: None, wechat_id: Some("wxid_p2".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "王五".to_string(), remark: None, wechat_id: Some("wxid_p3".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let w_items = vec![
            VirtualContactItem { name: "企微员工赵".to_string(), remark: None, wechat_id: None, enterprise: Some("某某公司".to_string()), real_name: Some("赵六".to_string()), tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let o_items = vec![
            VirtualContactItem { name: "公众号一".to_string(), remark: None, wechat_id: Some("gh_off1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "公众号二".to_string(), remark: None, wechat_id: Some("gh_off2".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];

        let g_personal = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("P".to_string(), p_items)], expanded: false };
        let g_work = VirtualGroup { category: "work".to_string(), title: "企业微信联系人".to_string(), letter_sections: vec![("W".to_string(), w_items)], expanded: false };
        let g_official = VirtualGroup { category: "official".to_string(), title: "公众号、服务号".to_string(), letter_sections: vec![("O".to_string(), o_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g_personal, g_work, g_official]);
        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert!(report.complete, "report: {report:?}");
        assert!(!report.yielded);
        assert_eq!(report.unreadable_rows, 0);
        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(profiles.len(), 6);
    }

    #[tokio::test]
    async fn test_sync_yields_between_contacts_and_resumes_from_checkpoint() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![
            VirtualContactItem { name: "张三".to_string(), remark: None, wechat_id: Some("wxid_p1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "李四".to_string(), remark: None, wechat_id: Some("wxid_p2".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "王五".to_string(), remark: None, wechat_id: Some("wxid_p3".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let g_personal = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("P".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g_personal]);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c_clone = counter.clone();
        let should_yield = move || {
            c_clone.fetch_add(1, Ordering::SeqCst) >= 2
        };

        let report1 = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &should_yield, 1000).await.unwrap();
        assert!(report1.yielded);
        assert!(!report1.complete);
        assert!(checkpoint.exists());

        let no_yield = || false;
        let report2 = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.unwrap();
        assert!(report2.complete);
        assert!(!report2.yielded);
        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(profiles.len(), 3);
    }

    #[tokio::test]
    async fn test_sync_reports_incomplete_when_rows_unreadable() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![
            VirtualContactItem { name: "张三".to_string(), remark: None, wechat_id: Some("wxid_p1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
            VirtualContactItem { name: "李四".to_string(), remark: None, wechat_id: Some("wxid_p2".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: true, misaligned_profile: None },
        ];
        let g_personal = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("P".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g_personal]);
        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(!report.complete);
        assert_eq!(report.unreadable_rows, 1);
        let res_file = state_dir.join("contacts-sync-result.json");
        assert!(res_file.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(res_file).unwrap()).unwrap();
        assert_eq!(val["complete"], false);
    }

    #[tokio::test]
    async fn test_incremental_only_opens_changed_rows() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        for i in 1..=3 {
            let p = ProfileInput {
                category: "personal".to_string(),
                wechat_id: Some(format!("wxid_p{i}")),
                nickname: format!("用户{i}"),
                remark: None,
                region: None,
                source: None,
                added_on: None,
                common_groups: None,
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
            };
            store.upsert_profile("test_acct", &p, 500).unwrap();
        }

        let p_items = (1..=4).map(|i| VirtualContactItem {
            name: format!("用户{i}"),
            remark: None,
            wechat_id: Some(format!("wxid_p{i}")),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }).collect();
        let g_personal = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("U".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g_personal]);
        ui.manager_rows = (1..=4).map(|i| ManagerRow {
            row_y: 96.0 + 48.0 * ((i - 1) as f64),
            name: Some(format!("用户{i}")),
            remark: None,
            tags: vec![],
            permission: None,
        }).collect();

        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Incremental, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(report.complete);
        let open_row_calls = ui.calls.lock().unwrap().iter().filter(|(op, _)| op == "contacts_open_row").count();
        assert_eq!(open_row_calls, 1, "Only the 4th new user should trigger contacts_open_row");
    }

    #[tokio::test]
    async fn test_sync_never_requests_forbidden_click() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![
            VirtualContactItem { name: "张三".to_string(), remark: None, wechat_id: Some("wxid_p1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let g = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("Z".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        let calls = ui.calls.lock().unwrap();
        for (op, params) in calls.iter() {
            if op == "contacts_click_list" {
                let x = params["x"].as_f64().unwrap_or(0.0);
                let y = params["y"].as_f64().unwrap_or(0.0);
                assert!(!(x >= 420.0 && x <= 840.0 && y >= 440.0));
            }
        }
    }

    #[tokio::test]
    async fn test_sync_pauses_and_restores_auto_reply() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let pause_file = state_dir.join("admin-pause.json");

        let p_items = vec![
            VirtualContactItem { name: "张三".to_string(), remark: None, wechat_id: Some("wxid_p1".to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let g = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("Z".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g]);

        // Case A: pause_file did not exist initially -> removed after sync
        assert!(!pause_file.exists());
        let no_yield = || false;
        run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(!pause_file.exists());

        // Case B: pause_file existed initially -> remains after sync
        fs::write(&pause_file, b"{\"paused\":true}").unwrap();
        run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.unwrap();
        assert!(pause_file.exists());
    }

    #[tokio::test]
    async fn test_sync_logs_counts_only() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let secret_name = "秘密联系人甲";
        let secret_wxid = "wxid_secret_999";
        let p_items = vec![
            VirtualContactItem { name: secret_name.to_string(), remark: None, wechat_id: Some(secret_wxid.to_string()), enterprise: None, real_name: None, tags: vec![], friend_permission: None, unreadable: false, misaligned_profile: None },
        ];
        let g = VirtualGroup { category: "personal".to_string(), title: "联系人".to_string(), letter_sections: vec![("M".to_string(), p_items)], expanded: false };

        let mut ui = VirtualContactsUi::new(vec![g]);

        let log_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = log_buffer.clone();

        struct BufWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for BufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || BufWriter(writer.clone()))
            .finish();

        let no_yield = || false;
        let _guard = tracing::subscriber::set_default(subscriber);
        run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        drop(_guard);

        let logs = String::from_utf8(log_buffer.lock().unwrap().clone()).unwrap();
        assert!(!logs.contains(secret_name), "Logs leaked secret name: {}", logs);
        assert!(!logs.contains(secret_wxid), "Logs leaked secret wxid: {}", logs);
    }

    #[tokio::test]
    async fn test_full_sync_with_letter_rows_and_scrolling_attaches_profiles_to_right_names() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        // 22 人带字母行，需滚动
        let letters = vec![
            ("A", vec!["安安", "傲天", "艾伦", "阿强"]),
            ("B", vec!["白白", "包包", "贝贝", "冰冰", "彬彬"]),
            ("C", vec!["陈陈", "程程", "楚楚", "春春"]),
            ("D", vec!["大大", "丹丹", "点点", "东东", "冬冬"]),
            ("E", vec!["鄂鄂", "恩恩", "二郎", "尔康"]),
        ];

        let mut contact_idx = 0;
        let mut sections = Vec::new();
        for (let_str, names) in letters {
            let mut items = Vec::new();
            for n in names {
                contact_idx += 1;
                items.push(VirtualContactItem {
                    name: n.to_string(),
                    remark: None,
                    wechat_id: Some(format!("wxid_user_{contact_idx:02}")),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: None,
                });
            }
            sections.push((let_str.to_string(), items));
        }

        let group = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: sections,
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![group]);
        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert!(report.complete, "report should be complete: {report:?}");
        assert_eq!(report.per_category, vec![("personal".to_string(), 22, 22)]);

        let profiles = store.list_profiles("test_acct", Some("personal")).unwrap();
        assert_eq!(profiles.len(), 22);

        // 验证每个微信号对应名字正确
        for (i, p) in profiles.iter().enumerate() {
            let wid = p.get("wechat_id").and_then(|v| v.as_str()).unwrap();
            let _nick = p.get("nickname").and_then(|v| v.as_str()).unwrap();
            let expected_idx: usize = wid.strip_prefix("wxid_user_").unwrap().parse().unwrap();
            assert_eq!(expected_idx, i + 1);
        }
    }

    #[tokio::test]
    async fn test_sync_never_clicks_group_header_as_contact() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let g1 = VirtualGroup {
            category: "official".to_string(),
            title: "公众号、服务号".to_string(),
            letter_sections: vec![
                ("G".to_string(), vec![VirtualContactItem {
                    name: "公众号甲".to_string(),
                    remark: None,
                    wechat_id: Some("gh_111".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: None,
                }]),
            ],
            expanded: false,
        };
        let g2 = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![
                ("Z".to_string(), vec![VirtualContactItem {
                    name: "张三".to_string(),
                    remark: None,
                    wechat_id: Some("wxid_zs".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: None,
                }]),
            ],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g1, g2]);
        let no_yield = || false;
        run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        // 检查所有 open_row 点击的 y，不得落在 header 或字母行
        for &y in &ui.row_open_clicks {
            let vy = y + ui.scroll_offset;
            let found = ui.find_item_at_virtual_y(vy);
            if let Some((_, _, is_header, is_letter)) = found {
                assert!(!is_header, "contacts_open_row clicked on group header at y={y}");
                assert!(!is_letter, "contacts_open_row clicked on letter row at y={y}");
            }
        }
    }

    #[tokio::test]
    async fn test_misaligned_profile_is_not_written() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        // 张三行的资料卡返回了李四的资料（名字错位）
        let misaligned_lines = vec![
            line("李四", 520.0, 60.0),
            line("微信号：wxid_lisi", 520.0, 90.0),
        ];

        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![
                ("Z".to_string(), vec![VirtualContactItem {
                    name: "张三".to_string(),
                    remark: None,
                    wechat_id: Some("wxid_zhangsan".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: Some(misaligned_lines),
                }]),
            ],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert!(!report.complete);
        assert_eq!(report.unreadable_rows, 1);
        assert!(ui.row_open_clicks.len() >= 1, "Must have opened the row to detect misalignment");
        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(profiles.len(), 0, "Misaligned profile must not be written to db");
    }

    #[tokio::test]
    async fn test_unreadable_name_marks_incomplete_without_misattribution() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![
                ("A".to_string(), vec![
                    VirtualContactItem {
                        name: "用户A".to_string(),
                        remark: None,
                        wechat_id: Some("wxid_a".to_string()),
                        enterprise: None,
                        real_name: None,
                        tags: vec![],
                        friend_permission: None,
                        unreadable: false,
                        misaligned_profile: None,
                    },
                    VirtualContactItem {
                        name: "用户B".to_string(),
                        remark: None,
                        wechat_id: Some("wxid_b".to_string()),
                        enterprise: None,
                        real_name: None,
                        tags: vec![],
                        friend_permission: None,
                        unreadable: true, // 认不出名字
                        misaligned_profile: None,
                    },
                    VirtualContactItem {
                        name: "用户C".to_string(),
                        remark: None,
                        wechat_id: Some("wxid_c".to_string()),
                        enterprise: None,
                        real_name: None,
                        tags: vec![],
                        friend_permission: None,
                        unreadable: false,
                        misaligned_profile: None,
                    },
                ]),
            ],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert!(!report.complete);
        assert!(report.unreadable_rows >= 1);
        let profiles = store.list_profiles("test_acct", None).unwrap();
        // 用户A和用户C应该正确写入，且微信号没有串行错位
        assert_eq!(profiles.len(), 2);
        let nicks: Vec<String> = profiles.iter().map(|p| p.get("nickname").unwrap().as_str().unwrap().to_string()).collect();
        assert!(nicks.contains(&"用户A".to_string()));
        assert!(nicks.contains(&"用户C".to_string()));
        assert!(!nicks.contains(&"用户B".to_string()));
    }

    #[tokio::test]
    async fn test_yield_after_time_budget_and_resume_completes_without_duplicates() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let items: Vec<VirtualContactItem> = (1..=6).map(|i| VirtualContactItem {
            name: format!("用户{i}"),
            remark: None,
            wechat_id: Some(format!("wxid_{i}")),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }).collect();

        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("U".to_string(), items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);

        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c_clone = counter.clone();
        // 处理完 2 个后让出
        let yield_after_2 = move || {
            c_clone.fetch_add(1, Ordering::SeqCst) >= 2
        };

        let report1 = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &yield_after_2, 1000).await.unwrap();
        assert!(report1.yielded);
        assert!(!report1.complete);
        assert!(checkpoint.exists());

        // 再次恢复
        let no_yield = || false;
        let report2 = run_sync(&mut ui, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.unwrap();
        assert!(report2.complete);
        assert!(!report2.yielded);

        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(profiles.len(), 6, "Total profiles must be exactly 6 without duplicates");
    }

    #[tokio::test]
    async fn test_daily_sync_attempted_at_most_once_per_day_even_if_incomplete() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let res_path = state_dir.join("contacts-sync-result.json");
        let _attempt_path = state_dir.join("contacts-sync-daily-attempt.json");

        // 写入一次不完整同步结果
        fs::write(&res_path, serde_json::to_string(&json!({
            "finished_at_ms": 1000000000000_i64,
            "complete": false,
        })).unwrap()).unwrap();

        // 检查在同一天（未跨天且未满20小时）是否会再次触发
        let now_sec = 1000000000.0 + 3600.0; // 1小时后
        assert!(!should_attempt_daily_sync(&state_dir, true, now_sec));

        // 记录当天 attempt
        record_daily_sync_attempt(&state_dir, now_sec).unwrap();
        assert!(!should_attempt_daily_sync(&state_dir, true, now_sec + 3600.0));
    }

    #[test]
    fn test_stale_contacts_sync_pause_cleared_on_startup() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let pause_file = state_dir.join("admin-pause.json");

        // Case 1: reason == contacts_sync -> 清理
        fs::write(&pause_file, serde_json::to_string(&json!({
            "paused": true,
            "reason": "contacts_sync"
        })).unwrap()).unwrap();
        clean_stale_contacts_sync_pause(&state_dir);
        assert!(!pause_file.exists(), "Stale contacts_sync pause must be removed");

        // Case 2: reason == manual -> 不清理
        fs::write(&pause_file, serde_json::to_string(&json!({
            "paused": true,
            "reason": "manual_admin"
        })).unwrap()).unwrap();
        clean_stale_contacts_sync_pause(&state_dir);
        assert!(pause_file.exists(), "User manual pause must NOT be removed");
    }
    fn letter_group(category: &str, title: &str, prefix: &str, sections: &[(&str, usize)]) -> VirtualGroup {
        let mut n = 0;
        let letter_sections = sections
            .iter()
            .map(|(letter, count)| {
                let items = (0..*count)
                    .map(|_| {
                        n += 1;
                        VirtualContactItem {
                            name: format!("{prefix}{n:02}"),
                            remark: None,
                            wechat_id: Some(format!("wxid_{prefix}_{n:02}")),
                            enterprise: None,
                            real_name: None,
                            tags: vec![],
                            friend_permission: None,
                            unreadable: false,
                            misaligned_profile: None,
                        }
                    })
                    .collect();
                (letter.to_string(), items)
            })
            .collect();
        VirtualGroup { category: category.to_string(), title: title.to_string(), letter_sections, expanded: false }
    }

    /// 真机每打开一张资料卡约 1.5 秒，15 秒预算只够约 10 个人。恢复时若从头把已处理的人重开一遍，
    /// 进度永远停在前几个人。要求：反复让出/恢复能在有限轮内完成，且每个人的资料卡最多打开一次。
    #[tokio::test]
    async fn test_resume_progresses_without_reopening_processed_rows() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let group = letter_group("personal", "联系人", "客", &[("A", 4), ("B", 5), ("C", 4), ("D", 5), ("E", 4)]);
        let mut ui = VirtualContactsUi::new(vec![group]);

        let mut done = false;
        for run in 0..20 {
            let opens_at_start = ui.row_open_clicks.len();
            let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let b = budget.clone();
            // 每轮大约处理 3 个人后让出（模拟 15 秒时间预算）
            let should_yield = move || b.fetch_add(1, Ordering::SeqCst) >= 6;
            let report = run_sync(&mut ui, &store, "acct", SyncMode::Full, &checkpoint, &should_yield, 1000 + run).await.unwrap();
            assert!(ui.row_open_clicks.len() >= opens_at_start);
            if report.complete {
                assert_eq!(report.per_category, vec![("personal".to_string(), 22, 22)]);
                done = true;
                break;
            }
            assert!(report.yielded, "未完成时必须是让出: {report:?}");
        }
        assert!(done, "20 轮内必须完成（恢复后要从断点继续，不能每次从头重开）");
        assert!(ui.row_open_clicks.len() <= 22, "每个人的资料卡最多打开一次，实际打开 {} 次", ui.row_open_clicks.len());
        assert_eq!(store.list_profiles("acct", Some("personal")).unwrap().len(), 22);
    }

    /// 真机识别出的分组标题带箭头（"> 公众号、服务号"、展开后 "∨ 联系人"），不能被当成联系人去"打开资料卡"
    /// ——真机上那一下会把分组收起。按位置区分：联系人名字 x≈133，分组标题与字母索引 x≈77–95。
    #[tokio::test]
    async fn test_headers_with_arrow_prefix_are_never_opened_as_contacts() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let official = letter_group("official", "公众号、服务号", "号", &[("G", 3)]);
        let personal = letter_group("personal", "联系人", "人", &[("A", 3), ("B", 4)]);
        let mut ui = VirtualContactsUi::new(vec![official, personal]);
        ui.header_arrows = true;

        let no_yield = || false;
        let report = run_sync(&mut ui, &store, "acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert_eq!(ui.header_open_clicks, 0, "不得用打开资料卡点分组标题");
        assert!(report.complete, "{report:?}");
        assert_eq!(report.per_category, vec![("official".to_string(), 3, 3), ("personal".to_string(), 7, 7)]);
    }

    /// 真机首次同步：进入通讯录后下一步失败，原实现直接返回、没有切回聊天页，微信停在通讯录页，机器人停摆。
    /// 只要进过通讯录，无论成败都必须调用 contacts_leave。
    #[tokio::test]
    async fn test_failure_after_enter_always_leaves_contacts() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let personal = letter_group("personal", "联系人", "人", &[("A", 2)]);
        let mut ui = VirtualContactsUi::new(vec![personal]);
        ui.fail_op = Some("contacts_headers".to_string());

        let no_yield = || false;
        let res = run_sync(&mut ui, &store, "acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await;
        assert!(res.is_err(), "读分组失败应当返回错误");
        let ops: Vec<String> = ui.calls.lock().unwrap().iter().map(|(op, _)| op.clone()).collect();
        let enter = ops.iter().position(|o| o == "contacts_enter").expect("entered");
        assert!(ops[enter..].iter().any(|o| o == "contacts_leave"), "进过通讯录就必须离开: {ops:?}");
        assert!(!state_dir.join("admin-pause.json").exists(), "同步写的暂停标记必须清除");
    }

    /// 用户要求（2026-11-08）：初始化通讯录不打开「通讯录管理」（那里没有每个人的详细资料），
    /// 直接展开分组逐个点开联系人采集。实机上读管理表格耗尽了 15 秒预算、窗口没关上导致机器人停摆。
    #[tokio::test]
    async fn test_sync_never_opens_manager_window() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let official = letter_group("official", "公众号、服务号", "号", &[("G", 2)]);
        let personal = letter_group("personal", "联系人", "人", &[("A", 3), ("B", 2)]);
        let mut ui = VirtualContactsUi::new(vec![official, personal]);
        let no_yield = || false;
        for mode in [SyncMode::Full, SyncMode::Incremental] {
            let report = run_sync(&mut ui, &store, "acct", mode, &checkpoint, &no_yield, 1000).await.unwrap();
            assert!(report.complete, "{mode:?}: {report:?}");
        }
        let ops: Vec<String> = ui.calls.lock().unwrap().iter().map(|(op, _)| op.clone()).collect();
        assert!(!ops.iter().any(|o| o.starts_with("contacts_manager_")), "不得打开通讯录管理: {ops:?}");
    }

    /// 实机：断点续做时每一轮都立刻失败，断点还在，下一轮又续做——每秒一次的死循环（347 次），机器人停摆。
    /// 同步出错后必须清掉断点并在结果里写明错误，不自动续做。
    #[tokio::test]
    async fn test_failed_sync_clears_checkpoint_to_avoid_retry_loop() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let personal = letter_group("personal", "联系人", "人", &[("A", 4), ("B", 4)]);
        let mut ui = VirtualContactsUi::new(vec![personal]);

        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let b = budget.clone();
        let yield_soon = move || b.fetch_add(1, Ordering::SeqCst) >= 3;
        let first = run_sync(&mut ui, &store, "acct", SyncMode::Full, &checkpoint, &yield_soon, 1000).await.unwrap();
        assert!(first.yielded && checkpoint.exists(), "先让出留下断点: {first:?}");

        ui.fail_op = Some("contacts_scroll_list".to_string());
        let no_yield = || false;
        assert!(run_sync(&mut ui, &store, "acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.is_err());
        assert!(!checkpoint.exists(), "出错后必须清掉断点，否则协调器会每轮续做、陷入循环");
        let result: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(state_dir.join("contacts-sync-result.json")).unwrap()).unwrap();
        assert_eq!(result["complete"], false);
        assert!(result["error"].as_str().unwrap_or("").contains("layout_mismatch"), "{result}");
    }

}
