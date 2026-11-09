use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use crate::core::contacts::parse::{OcrLine, parse_group_headers, parse_profile};
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

/// 把所有展开的分组收起，让各分组标题都回到列表顶部一屏（实机：遗留的展开分组会把后面的标题挤出屏幕、
/// 还会让上一个分组一路滚进下一个分组，把别的分组的人记错分类）。
/// 判断展开：标题右侧没有人数，并且标题正下方紧跟着字母索引或联系人行。
async fn collapse_all_groups(capture: &mut dyn CoordinatorCapture) -> Result<Vec<OcrLine>> {
    let mut lines: Vec<OcrLine> = Vec::new();
    for _ in 0..5 {
        let _ = capture
            .call(
                "contacts_scroll_list",
                json!({"direction": "up", "ticks": 20}),
            )
            .await;
        let v = capture.call("contacts_headers", json!({})).await?;
        lines = v
            .get("lines")
            .and_then(|l| serde_json::from_value(l.clone()).ok())
            .unwrap_or_default();
        let headers: Vec<f64> = lines
            .iter()
            .filter(|l| l.x < 115.0 && GROUP_TITLES_CJK.contains(&cjk_only(&l.text).as_str()))
            .map(|l| l.y)
            .collect();
        let expanded = headers
            .iter()
            .copied()
            .find(|hy| group_expanded(&lines, *hy));
        match expanded {
            Some(hy) => {
                let _ = capture
                    .call("contacts_click_list", json!({"x": 180.0, "y": hy}))
                    .await?;
            }
            None => return Ok(lines),
        }
    }
    Ok(lines)
}

const GROUP_TITLES_CJK: [&str; 4] = ["公众号服务号", "企业微信联系人", "联系人", "群聊"];

/// 分组是否展开：只看标题正下方（约 110 点内）有没有字母索引行或联系人行。
/// 不能看人数：实机展开后人数照样显示；标题到第一个联系人约 80 点，中间字母行可能识别不出。
fn group_expanded(lines: &[OcrLine], header_y: f64) -> bool {
    // 只看到下一个分组标题为止：收起的分组下面紧挨着的是下一个分组（它若展开，字母行会落在范围内）
    let next_header = lines
        .iter()
        .filter(|l| {
            l.x < 115.0
                && l.y > header_y + 5.0
                && GROUP_TITLES_CJK.contains(&cjk_only(&l.text).as_str())
        })
        .map(|l| l.y)
        .fold(f64::INFINITY, f64::min);
    let upper = (header_y + 110.0).min(next_header - 5.0);
    lines.iter().any(|l| {
        l.y > header_y + 15.0
            && l.y < upper
            && !GROUP_TITLES_CJK.contains(&cjk_only(&l.text).as_str())
            && !(l.x > 250.0 && l.text.trim().chars().all(|c| c.is_ascii_digit()))
            && !cjk_only(&l.text).contains("通讯录管理")
    })
}

/// 名字比对用的规范化：只保留汉字、字母、数字（小写）。去掉截断省略号、空白、表情、性别图标等
fn name_key_for_match(text: &str) -> String {
    text.chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c) || c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 列表行名字与资料卡名字是否指同一人：规范化后相等，或一方包含另一方且较短一方至少 2 个字
/// （列表名字会被截断，资料卡名字可能带额外图标）
fn names_match(row: &str, profile: &str) -> bool {
    let a = name_key_for_match(row);
    let b = name_key_for_match(profile);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.chars().count() <= b.chars().count() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if short.chars().count() >= 2 && long.contains(short.as_str()) {
        return true;
    }
    // 同长度、4 字及以上，只差 1 个字：识别个别字出入（实机：长度相同却判为不一致）
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    ac.len() == bc.len() && ac.len() >= 4 && ac.iter().zip(&bc).filter(|(x, y)| x != y).count() <= 1
}

/// 只保留汉字（去掉箭头、图标、标点、数字、空白），用于识别界面固有文字
fn cjk_only(text: &str) -> String {
    text.chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .collect()
}

fn group_title_cjk(category: &str) -> &'static str {
    match category {
        "official" => "公众号服务号",
        "work" => "企业微信联系人",
        "personal" => "联系人",
        "group" => "群聊",
        _ => "",
    }
}

/// 账号只进入摘要，既防止路径注入，也不在文件名泄露微信号。
pub fn account_state_path(state_dir: &Path, account_id: &str, kind: &str) -> PathBuf {
    let key = Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    state_dir.join(format!("contacts-sync-{kind}-{key}.json"))
}

pub fn write_sync_result(state_dir: &Path, account_id: &str, result: &Value) -> Result<()> {
    let mut result = result.clone();
    result["account_id"] = json!(account_id);
    write_private(
        &account_state_path(state_dir, account_id, "result"),
        &result,
    )?;
    if result["complete"] == true {
        write_private(
            &account_state_path(state_dir, account_id, "success"),
            &result,
        )?;
    }
    // 保留已有控制命令的响应位置，但消费者必须校验账号和请求 ID。
    write_private(&state_dir.join("contacts-sync-result.json"), &result)
}

/// 只有分类、名字精确一致且唯一的有效强身份才可免开资料卡。
fn incremental_contact<'a>(existing: &'a [Value], category: &str, name: &str) -> Option<&'a str> {
    let key = crate::core::config::name_key(name);
    let mut matches = existing.iter().filter(|p| {
        p["category"] == category
            && ["nickname", "remark"].iter().any(|field| {
                p[*field]
                    .as_str()
                    .is_some_and(|n| crate::core::config::name_key(n) == key)
            })
    });
    let first = matches.next()?;
    if matches.next().is_some() || first["identity"] != "strong" || !first["removed_at"].is_null() {
        return None;
    }
    first["contact_id"].as_str()
}

pub fn should_attempt_daily_sync(
    state_dir: &Path,
    account_id: &str,
    is_idle: bool,
    now_sec: f64,
) -> bool {
    if !is_idle {
        return false;
    }
    let attempt_file = account_state_path(state_dir, account_id, "daily-attempt");
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

    let res_file = account_state_path(state_dir, account_id, "result");
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

pub fn record_daily_sync_attempt(state_dir: &Path, account_id: &str, now_sec: f64) -> Result<()> {
    let attempt_file = account_state_path(state_dir, account_id, "daily-attempt");
    write_private(
        &attempt_file,
        &json!({"account_id": account_id, "last_attempt_sec": now_sec}),
    )
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

pub async fn run_sync(
    capture: &mut dyn CoordinatorCapture,
    store: &HistoryStore,
    account_id: &str,
    mode: SyncMode,
    checkpoint: &Path,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    now_ms: i64,
) -> Result<SyncReport> {
    // 先独占微信（对话监控已停、网关等其他进程也不能在两次操作之间插进来），整个同步结束再释放
    if let Err(e) = capture.call("contacts_lease_begin", json!({})).await {
        let _ = capture.call("contacts_lease_end", json!({})).await;
        if let Some(dir) = checkpoint.parent() {
            let _ = write_sync_result(
                dir,
                account_id,
                &json!({"finished_at_ms": now_ms, "complete": false, "error": format!("desktop_busy: {e:#}")}),
            );
        }
        return Err(e);
    }
    let result = run_sync_inner(
        capture,
        store,
        account_id,
        mode,
        checkpoint,
        should_yield,
        now_ms,
    )
    .await;
    if let Err(e) = &result {
        // 出错不自动续做：清掉断点并写明错误，否则协调器每轮续做、立刻失败，形成死循环（实机 347 次，机器人停摆）
        let _ = fs::remove_file(checkpoint);
        if let Some(dir) = checkpoint.parent() {
            let _ = write_sync_result(
                dir,
                account_id,
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
    // 无论成败都释放独占
    if let Err(e) = capture.call("contacts_lease_end", json!({})).await {
        tracing::warn!("contacts_lease_end failed: {e:#}");
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
    // 不再断点续做：中途停下就下次从头来（按微信号更新，重复跑不会重复记录）
    let _ = fs::remove_file(checkpoint);

    // 停对话监控：写暂停标记（用户已手动暂停则保留用户的标记）
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
            if self.should_clean {
                let _ = fs::remove_file(self.pause_file);
            }
        }
    }
    let _pause_guard = PauseGuard {
        pause_file: &pause_file,
        should_clean: !initially_paused,
    };

    anyhow::ensure!(!should_yield(), "contacts_sync_interrupted");
    capture.call("contacts_enter", json!({})).await?;
    let top = collapse_all_groups(capture).await?;
    let groups = parse_group_headers(&top);
    info!(
        lines = top.len(),
        headers = groups.len(),
        "contacts_sync_headers"
    );
    let existing = store.list_profiles(account_id, None)?;

    let mut per_category = Vec::new();
    let mut unreadable = 0u32;
    let mut needs_review = 0u32;
    let mut stopped = false;
    for (cat, expected, _) in &groups {
        if cat == "official" {
            info!("contacts_sync_skip_official");
            continue; // 公众号/服务号不采集（用户 2026-11-08）
        }
        if should_yield() {
            stopped = true;
            break;
        }
        let mut r = sync_group(
            capture,
            store,
            account_id,
            cat,
            mode,
            &existing,
            should_yield,
            now_ms,
        )
        .await?;
        r.stopped |= should_yield();
        if !r.stopped
            && r.unreadable == 0
            && r.needs_review == 0
            && r.ids.len() == *expected as usize
        {
            store.mark_missing(account_id, cat, &r.ids, now_ms)?;
        }
        let synced = r.ids.len() as u32;
        // 期望数与已同步数的差额都算"未读到"，不悄悄放过
        unreadable += r.unreadable.max(expected.saturating_sub(synced));
        needs_review += r.needs_review;
        per_category.push((cat.clone(), *expected, synced));
        if r.stopped {
            stopped = true;
            break;
        }
    }

    capture.call("contacts_leave", json!({})).await?;

    let complete = !stopped
        && !per_category.is_empty()
        && per_category.iter().all(|(_, e, s)| e == s)
        && unreadable == 0
        && needs_review == 0;
    let report = SyncReport {
        per_category: per_category.clone(),
        unreadable_rows: unreadable,
        needs_review,
        yielded: stopped,
        complete,
    };
    write_sync_result(
        state_dir,
        account_id,
        &json!({
            "finished_at_ms": now_ms,
            "complete": complete,
            "per_category": per_category.iter().map(|(c, e, s)| json!({"category": c, "expected": e, "synced": s})).collect::<Vec<_>>(),
            "unreadable_rows": unreadable,
            "needs_review": needs_review,
            "stopped": stopped,
        }),
    )?;
    info!(complete, stopped, unreadable, needs_review, "sync finished");
    Ok(report)
}

struct GroupResult {
    ids: Vec<String>,
    unreadable: u32,
    needs_review: u32,
    stopped: bool,
}

/// 一屏的识别结果：文字行 + 头像中心 y（动作组件按像素找的，只有联系人行有头像）
struct Screen {
    lines: Vec<OcrLine>,
    avatars: Vec<f64>,
}

fn screen_of(v: &Value) -> Screen {
    Screen {
        lines: lines_of(v),
        avatars: v
            .get("avatars")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default(),
    }
}

fn line_center(l: &OcrLine) -> f64 {
    l.y + l.h / 2.0
}

/// 本屏属于本分组的联系人行（从上到下）。
/// 实测（2026-11-08 截图）：每个字母单独占一行（约 36 点高，字母常识别不出），联系人行约 54 点高；
/// 只有联系人行左侧有头像，所以"行"按头像定，名字只用来核对。
/// 只取本分组标题下方、下一个分组标题上方的头像；返回 (头像 y, 同一高度识别出的名字) 和下一个分组标题是否在本屏。
fn group_rows(screen: &Screen, category: &str) -> (Vec<(f64, Option<String>)>, bool) {
    let own = group_title_cjk(category);
    let is_header =
        |l: &OcrLine| l.x < 115.0 && GROUP_TITLES_CJK.contains(&cjk_only(&l.text).as_str());
    let own_header = screen
        .lines
        .iter()
        .filter(|l| is_header(l) && cjk_only(&l.text) == own)
        .map(line_center)
        .fold(None, |acc: Option<f64>, y| {
            Some(acc.map_or(y, |a: f64| a.max(y)))
        });
    let next_header = screen
        .lines
        .iter()
        .filter(|l| is_header(l) && cjk_only(&l.text) != own)
        .map(line_center)
        .filter(|y| own_header.map_or(true, |h| *y > h))
        .fold(None, |acc: Option<f64>, y| {
            Some(acc.map_or(y, |a: f64| a.min(y)))
        });
    let mut rows: Vec<(f64, Option<String>)> = screen
        .avatars
        .iter()
        .copied()
        .filter(|y| own_header.map_or(true, |h| *y > h + 20.0))
        .filter(|y| next_header.map_or(true, |n| *y < n - 20.0))
        .map(|ay| {
            let name = screen
                .lines
                .iter()
                .filter(|l| l.x >= 115.0 && (line_center(l) - ay).abs() < 18.0)
                .filter(|l| !is_header(l) && !cjk_only(&l.text).contains("通讯录管理"))
                .filter(|l| !(l.x > 250.0 && l.text.trim().chars().all(|c| c.is_ascii_digit())))
                .min_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
                .map(|l| l.text.trim().to_string());
            (ay, name)
        })
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    (rows, next_header.is_some())
}

fn lines_of(v: &serde_json::Value) -> Vec<OcrLine> {
    v.get("lines")
        .and_then(|l| serde_json::from_value(l.clone()).ok())
        .unwrap_or_default()
}

/// 滚动后新屏相对旧屏移动了多少：
/// 1. 用旧屏上识别出名字的行，在新屏找同名行；
/// 2. 找不到时用头像位置对齐（旧屏头像整体平移 d 后与新屏头像重合最多的 d）；
/// 3. 两屏完全一样 → 0（到底）。都不行返回 None（跟丢了）。
fn measure_shift(
    prev: &[(f64, Option<String>)],
    prev_screen: &Screen,
    next: &Screen,
    expect: f64,
) -> Option<f64> {
    let same = prev_screen.avatars == next.avatars
        && prev_screen
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            == next
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>();
    if same {
        return Some(0.0);
    }
    for (py, name) in prev.iter().rev() {
        let Some(name) = name else { continue };
        let cand = next
            .lines
            .iter()
            .filter(|l| l.x >= 115.0 && names_match(name, &l.text))
            .map(|l| py - line_center(l))
            .filter(|d| *d >= -2.0)
            .min_by(|a, b| {
                (a - expect)
                    .abs()
                    .partial_cmp(&(b - expect).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(d) = cand {
            return Some(d.max(0.0));
        }
    }
    let mut best: Option<(usize, f64)> = None;
    for pa in &prev_screen.avatars {
        for na in &next.avatars {
            let d = pa - na;
            if d < 1.0 {
                continue;
            }
            let hits = prev_screen
                .avatars
                .iter()
                .filter(|p| next.avatars.iter().any(|n| (*p - d - n).abs() < 3.0))
                .count();
            if hits >= 2
                && best.map_or(true, |(h, bd)| {
                    hits > h || (hits == h && (d - expect).abs() < (bd - expect).abs())
                })
            {
                best = Some((hits, d));
            }
        }
    }
    best.map(|(_, d)| d)
}

/// key 是否按顺序出现在 long 里（不要求连续）
fn is_subsequence(key: &[char], long: &[char]) -> bool {
    let mut it = long.iter();
    key.iter().all(|c| it.any(|x| x == c))
}

/// 列表上这一行与点开的资料卡是不是同一个人。
/// 1. 原规则（names_match）：规范化后相等、包含（≥2 字）、同长度差 1 字；
/// 2. 列表名字 ≥4 字、按顺序全在资料卡名字里（实机：名字里的字母被识别成 "@"）；
/// 3. 企业微信（实测 2026-11-08）：列表显示"名字…@企业"，资料卡第一行是完整名字、另有一行单独是"@企业"。
///    按 @ 拆开：资料卡上有"@企业"行时，企业名必须对上（截断时按开头），名字要是资料卡名字的开头、允许差 1 字；
///    资料卡上没有"@企业"行时，名字是资料卡名字的开头即可（≥2 字）。
fn row_matches_profile(row: &str, prof: &[OcrLine], nickname: &str, remark: Option<&str>) -> bool {
    let names: Vec<&str> = std::iter::once(nickname)
        .chain(remark)
        .filter(|n| !n.trim().is_empty())
        .collect();
    if names.iter().any(|n| names_match(row, n)) {
        return true;
    }
    let rk: Vec<char> = name_key_for_match(row).chars().collect();
    if rk.len() >= 4
        && names
            .iter()
            .any(|n| is_subsequence(&rk, &name_key_for_match(n).chars().collect::<Vec<_>>()))
    {
        return true;
    }
    let Some(at) = row.find(['@', '＠']) else {
        return false;
    };
    let name_part: Vec<char> = name_key_for_match(&row[..at]).chars().collect();
    let company_row = name_key_for_match(&row[at..]);
    if name_part.is_empty() {
        return false;
    }
    let company_card = prof
        .iter()
        .map(|l| l.text.trim())
        .find(|t| t.starts_with('@') || t.starts_with('＠'))
        .map(name_key_for_match);
    let company_ok = match &company_card {
        Some(card) => {
            !company_row.is_empty()
                && (card.starts_with(&company_row) || company_row.starts_with(card.as_str()))
        }
        None => false,
    };
    names.iter().any(|n| {
        let nk: Vec<char> = name_key_for_match(n).chars().collect();
        if nk.len() < name_part.len() {
            return false;
        }
        let diff = name_part.iter().zip(&nk).filter(|(a, b)| a != b).count();
        match &company_card {
            Some(_) => company_ok && (diff == 0 || (diff == 1 && name_part.len() >= 2)),
            None => diff == 0 && name_part.len() >= 2,
        }
    })
}

/// 资料卡一行的结构（只用于日志）：界面固有文字原样保留，其余汉字记 c、字母数字记 a、空格记 _，符号原样
fn card_line_shape(text: &str) -> String {
    const UI_WORDS: [&str; 16] = [
        "企业",
        "实名",
        "来源",
        "备注",
        "添加备注名",
        "朋友圈",
        "视频号",
        "更多信息",
        "地区",
        "微信号",
        "共同群聊",
        "添加时间",
        "发消息",
        "语音聊天",
        "视频聊天",
        "企业微信",
    ];
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(w) = UI_WORDS
            .iter()
            .filter(|w| rest.starts_with(**w))
            .max_by_key(|w| w.len())
        {
            out.push('[');
            out.push_str(w);
            out.push(']');
            rest = &rest[w.len()..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            'c'
        } else if ch.is_ascii_alphanumeric() {
            'a'
        } else if ch == ' ' {
            '_'
        } else {
            ch
        });
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// 文字的短指纹（只用于日志比对两张资料卡是否相同，不含文字本身）
fn short_fp(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .take(3)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 字形结构（只用于日志，不含文字本身）：在 other 里出现的字记 "="，其余汉字记 "c"、字母数字记 "a"，
/// 空格记 "_"，@ … ( ) 原样保留，其他符号记 "."
fn shape_against(text: &str, other: &str) -> String {
    let mut pool: Vec<char> = name_key_for_match(other).chars().collect();
    text.chars()
        .map(|ch| {
            let low = ch.to_lowercase().next().unwrap_or(ch);
            if ('\u{4e00}'..='\u{9fff}').contains(&ch) || ch.is_ascii_alphanumeric() {
                if let Some(pos) = pool.iter().position(|x| *x == low) {
                    pool.remove(pos);
                    '='
                } else if ch.is_ascii_alphanumeric() {
                    'a'
                } else {
                    'c'
                }
            } else if ch == ' ' {
                '_'
            } else if "@…()（）＠".contains(ch) {
                ch
            } else {
                '.'
            }
        })
        .collect()
}

/// 资料卡哪一行和列表名字最像（只用于日志，输出数字不输出名字）：(行号, 相同字数)
fn best_profile_line(row: &str, prof: &[OcrLine]) -> (usize, usize) {
    let rk: Vec<char> = name_key_for_match(row).chars().collect();
    let mut best = (usize::MAX, 0usize);
    for (i, l) in prof.iter().enumerate() {
        let mut lk: Vec<char> = name_key_for_match(&l.text).chars().collect();
        let mut common = 0;
        for c in &rk {
            if let Some(pos) = lk.iter().position(|x| x == c) {
                lk.remove(pos);
                common += 1;
            }
        }
        if common > best.1 {
            best = (i, common);
        }
    }
    best
}

/// 固定流程：收起所有分组 → 展开本分组 → 每屏按头像从上到下逐个点开采集 → 出现下一个分组标题或滚不动就结束 → 否则滚一屏
#[allow(clippy::too_many_arguments)]
async fn sync_group(
    capture: &mut dyn CoordinatorCapture,
    store: &HistoryStore,
    account_id: &str,
    category: &str,
    mode: SyncMode,
    existing: &[serde_json::Value],
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    now_ms: i64,
) -> Result<GroupResult> {
    // 每格滚多少点事先不知道（实机 5 格就超过一屏、把人跳过去了）：先滚 1 格量出来，之后按实测算格数
    let mut px_per_tick: Option<f64> = None;
    let mut r = GroupResult {
        ids: Vec::new(),
        unreadable: 0,
        needs_review: 0,
        stopped: false,
    };

    // 1. 收起所有分组、回到顶部，找到本分组标题并展开（按标题下方内容确认，最多点两次）
    let top = collapse_all_groups(capture).await?;
    let Some(hy) = top
        .iter()
        .find(|l| l.x < 115.0 && cjk_only(&l.text) == group_title_cjk(category))
        .map(|l| l.y)
    else {
        info!(category, "contacts_sync_group_header_missing");
        return Ok(r);
    };
    let mut screen = Screen {
        lines: top,
        avatars: Vec::new(),
    };
    for _ in 0..2 {
        if group_expanded(&screen.lines, hy) {
            break;
        }
        screen = screen_of(
            &capture
                .call("contacts_click_list", json!({"x": 180.0, "y": hy}))
                .await?,
        );
    }
    if screen.avatars.is_empty() {
        // 已展开但还没有头像数据（例如展开前就是展开状态）：重新取一次本屏
        screen = screen_of(&capture.call("contacts_headers", json!({})).await?);
    }

    // 2. 逐屏处理。done_until：本屏上已处理到的位置（这个 y 及以上的行都处理过）
    let mut done_until = 0.0f64;
    for _page in 0..200 {
        let (rows, next_visible) = group_rows(&screen, category);
        info!(
            category,
            rows = rows.len(),
            named = rows.iter().filter(|(_, n)| n.is_some()).count(),
            next_group_visible = next_visible,
            "contacts_sync_screen"
        );
        let limit = done_until + 10.0;
        for (y, name) in rows.iter().filter(|(y, _)| *y > limit) {
            if should_yield() {
                r.stopped = true;
                return Ok(r);
            }
            done_until = *y;

            if mode == SyncMode::Incremental {
                if let Some(name) = name {
                    let unique_on_screen = rows
                        .iter()
                        .filter(|(_, other)| {
                            other.as_deref().is_some_and(|other| {
                                crate::core::config::name_key(other)
                                    == crate::core::config::name_key(name)
                            })
                        })
                        .count()
                        == 1;
                    if let Some(cid) =
                        incremental_contact(existing, category, name).filter(|_| unique_on_screen)
                    {
                        if !r.ids.iter().any(|x| x == cid) {
                            r.ids.push(cid.to_string());
                            continue;
                        }
                    }
                }
            }

            let resp = capture
                .call("contacts_open_row", json!({"y": y}))
                .await
                .unwrap_or_default();
            if should_yield() {
                r.stopped = true;
                return Ok(r);
            }
            let prof = lines_of(&resp);
            // 资料卡画面没变：只有本行名字和资料卡对得上才算（点的正好是已显示的那个人），否则不记
            let changed = resp
                .get("changed")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if prof.is_empty() {
                info!(category, row_y = *y, "contacts_sync_profile_empty");
                r.unreadable += 1;
                continue;
            }
            let f = parse_profile(&prof, category).fields;
            if category == "work" {
                // 诊断：企业微信资料卡逐行结构（界面固有文字原样，其余只记字形），用于核对名字/企业的解析
                let layout: Vec<String> = prof
                    .iter()
                    .map(|l| format!("{}@{},{}", card_line_shape(&l.text), l.x as i64, l.y as i64))
                    .collect();
                info!(
                    category,
                    row_y = *y,
                    card = layout.join(" | ").as_str(),
                    "contacts_sync_card_layout"
                );
            }
            let matched = match name {
                Some(name) => row_matches_profile(name, &prof, &f.nickname, f.remark.as_deref()),
                None => changed && !f.nickname.trim().is_empty(),
            };
            if !matched {
                info!(
                    category,
                    row_y = *y,
                    changed,
                    profile_lines = prof.len(),
                    row_len = name
                        .as_deref()
                        .map_or(0, |n| name_key_for_match(n).chars().count()),
                    nickname_len = name_key_for_match(&f.nickname).chars().count(),
                    has_wechat_id = f.wechat_id.is_some(),
                    best_line = name.as_deref().map_or(-1, |n| {
                        let (i, _) = best_profile_line(n, &prof);
                        if i == usize::MAX { -1 } else { i as i64 }
                    }),
                    best_common = name.as_deref().map_or(0, |n| best_profile_line(n, &prof).1),
                    nickname_common = name.as_deref().map_or(0, |n| best_profile_line(
                        n,
                        &[OcrLine {
                            text: f.nickname.clone(),
                            x: 0.0,
                            y: 0.0,
                            w: 0.0,
                            h: 0.0
                        }]
                    )
                    .1),
                    row_shape = name
                        .as_deref()
                        .map_or(String::new(), |n| shape_against(n, &f.nickname)),
                    nick_shape = name
                        .as_deref()
                        .map_or(String::new(), |n| shape_against(&f.nickname, n)),
                    line0_shape = name.as_deref().map_or(String::new(), |n| prof
                        .first()
                        .map_or(String::new(), |l| shape_against(&l.text, n))),
                    line1_shape = name.as_deref().map_or(String::new(), |n| prof
                        .get(1)
                        .map_or(String::new(), |l| shape_against(&l.text, n))),
                    "contacts_sync_profile_name_mismatch"
                );
                r.unreadable += 1;
                continue;
            }
            let input = ProfileInput {
                category: category.to_string(),
                wechat_id: f.wechat_id,
                nickname: f.nickname,
                remark: f.remark,
                region: f.region,
                source: f.source,
                added_on: f.added_on,
                common_groups: f.common_groups,
                enterprise: f.enterprise,
                real_name: f.real_name,
                tags: f.tags,
                friend_permission: f.friend_permission,
            };
            let outcome = store.upsert_profile(account_id, &input, now_ms);
            {
                let (kind, cid_tail) = match &outcome {
                    Ok(UpsertOutcome::Created(c)) => {
                        ("created", c.chars().rev().take(4).collect::<String>())
                    }
                    Ok(UpsertOutcome::Updated(c)) => {
                        ("updated", c.chars().rev().take(4).collect::<String>())
                    }
                    Ok(UpsertOutcome::NeedsReview { .. }) => ("needs_review", String::new()),
                    Err(_) => ("error", String::new()),
                };
                info!(
                    category,
                    row_y = *y,
                    row_named = name.is_some(),
                    changed,
                    has_wechat_id = input.wechat_id.is_some(),
                    outcome = kind,
                    cid_tail = cid_tail.as_str(),
                    row_shape = name
                        .as_deref()
                        .map_or(String::new(), |n| shape_against(n, &input.nickname)),
                    card_fp = short_fp(
                        &prof
                            .iter()
                            .map(|l| l.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                    .as_str(),
                    nick_fp = short_fp(&input.nickname).as_str(),
                    ent_fp = short_fp(input.enterprise.as_deref().unwrap_or("")).as_str(),
                    "contacts_sync_row_ok"
                );
            }
            match outcome {
                Ok(UpsertOutcome::Created(cid)) | Ok(UpsertOutcome::Updated(cid)) => {
                    if !r.ids.contains(&cid) {
                        r.ids.push(cid);
                    }
                }
                Ok(UpsertOutcome::NeedsReview { .. }) => r.needs_review += 1,
                Err(_) => r.unreadable += 1,
            }
        }

        // 3. 本屏已到下一个分组：本分组结束
        if next_visible {
            break;
        }
        // 4. 滚动：让本屏最后一个联系人滚到列表上部（还留在屏幕上），这样绝不会跳过人
        let last_y = rows.last().map(|(y, _)| *y).unwrap_or(400.0);
        let ticks = match px_per_tick {
            Some(px) if px > 1.0 => (((last_y - 130.0) / px).floor() as i64).max(1) as u32,
            _ => 1,
        };
        let next = screen_of(
            &capture
                .call(
                    "contacts_scroll_list",
                    json!({"direction": "down", "ticks": ticks}),
                )
                .await?,
        );
        let expect = px_per_tick.map_or(100.0, |px| px * ticks as f64);
        let shift = measure_shift(&rows, &screen, &next, expect);
        info!(
            category,
            ticks,
            shift = shift.unwrap_or(-1.0),
            "contacts_sync_scrolled"
        );
        match shift {
            Some(d) if d < 1.0 => break, // 滚不动：到底
            Some(d) => {
                if px_per_tick.is_none() || ticks > 1 {
                    px_per_tick = Some(d / ticks as f64);
                }
                done_until -= d;
            }
            None => {
                // 跟丢了：宁可把本屏都再核一遍（按名字核对、按微信号更新，不会记错人），也不跳过
                info!(category, "contacts_sync_lost_track");
                done_until = 0.0;
            }
        }
        screen = next;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contacts::parse::ManagerRow;
    use crate::core::model::Frame;
    use crate::core::worker_client::OcrStatus;
    use anyhow::bail;
    use serde_json::Value;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "wechat-sync-test-{}",
                uuid::Uuid::new_v4().simple()
            ));
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
            self.letter_sections
                .iter()
                .map(|(_, items)| items.len() as u32)
                .sum()
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
        /// 真机：分组展开后标题旁的人数仍然显示（不能用人数判断是否展开）
        pub counts_always: bool,
        /// 真机：字母行常识别不出来
        pub letters_unread: bool,
        /// 当前资料卡显示的是谁（点同一个人资料卡不变化）
        pub shown_profile: Option<String>,
        /// 每格滚动的点数（真机事先未知）
        pub tick_px: f64,
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
                counts_always: false,
                letters_unread: false,
                shown_profile: None,
                tick_px: 90.0,
            }
        }

        fn render_list(&self) -> (Vec<OcrLine>, Vec<f64>) {
            let mut lines = Vec::new();
            let mut avatars: Vec<f64> = Vec::new();

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

            // 通讯录管理按钮：实测 x≈131、y≈59，识别结果带图标前缀字符（6 字），与联系人名字同列
            let mgr_y = 59.0 - self.scroll_offset;
            if mgr_y >= 40.0 && mgr_y <= 640.0 {
                lines.push(OcrLine {
                    text: "8通讯录管理".to_string(),
                    x: 131.0,
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
                    if !g.expanded || self.counts_always {
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
                        // 字母索引行高 36（实测）
                        let letter_center_y = cur_y + 18.0 - self.scroll_offset;
                        if letter_center_y >= 40.0
                            && letter_center_y <= 640.0
                            && !self.letters_unread
                        {
                            lines.push(OcrLine {
                                text: letter.clone(),
                                x: 95.0,
                                y: letter_center_y,
                                w: 20.0,
                                h: 20.0,
                            });
                        }
                        cur_y += 36.0;

                        for item in items {
                            // 联系人行高 54（实测），左侧头像 28 高、与名字同高
                            let item_center_y = cur_y + 27.0 - self.scroll_offset;
                            if item_center_y - 14.0 >= 0.0 && item_center_y + 14.0 <= 640.0 {
                                avatars.push(item_center_y);
                            }
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
                            cur_y += 54.0;
                        }
                    }
                }
            }

            // 真机识别结果的 y 是文字框上沿
            for l in &mut lines {
                l.y -= l.h / 2.0;
            }
            (lines, avatars)
        }

        /// 列表内容总高度（虚拟坐标，从 0 起）
        fn content_height(&self) -> f64 {
            let mut cur_y = 90.0;
            for g in &self.groups {
                cur_y += 40.0;
                if g.expanded {
                    for (_l, items) in &g.letter_sections {
                        cur_y += 36.0 + 54.0 * items.len() as f64;
                    }
                }
            }
            cur_y
        }

        fn find_item_at_virtual_y(
            &self,
            vy: f64,
        ) -> Option<(&VirtualGroup, Option<&VirtualContactItem>, bool, bool)> {
            // 返回 (group, Option<item>, is_header, is_letter)
            let mut cur_y = 90.0;
            for g in &self.groups {
                if vy >= cur_y && vy < cur_y + 40.0 {
                    return Some((g, None, true, false));
                }
                cur_y += 40.0;

                if g.expanded {
                    for (_letter, items) in &g.letter_sections {
                        if vy >= cur_y && vy < cur_y + 36.0 {
                            return Some((g, None, false, true));
                        }
                        cur_y += 36.0;

                        for item in items {
                            if vy >= cur_y && vy < cur_y + 54.0 {
                                return Some((g, Some(item), false, false));
                            }
                            cur_y += 54.0;
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
        fn scroll<'a>(
            &'a mut self,
            _direction: &'a str,
            _amount: u32,
        ) -> Pin<Box<dyn Future<Output = Result<Frame>> + Send + 'a>> {
            Box::pin(async { bail!("not implemented") })
        }
        fn call<'a>(
            &'a mut self,
            op: &'a str,
            params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
            let calls = self.calls.clone();
            calls.lock().unwrap().push((op.to_string(), params.clone()));

            if self.fail_op.as_deref() == Some(op) {
                return Box::pin(async {
                    bail!(
                        "private visual operation failed: invalid(\"layout_mismatch:input_divider\")"
                    )
                });
            }
            if op == "contacts_enter" {
                self.scroll_offset = 0.0;
                return Box::pin(async { Ok(json!({"ok": true})) });
            }
            if op == "contacts_leave" {
                return Box::pin(async { Ok(json!({"ok": true})) });
            }
            if op == "contacts_headers" {
                let (lines, avatars) = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines, "avatars": avatars})) });
            }
            if op == "contacts_scroll_list" {
                let dir = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                let ticks = params.get("ticks").and_then(|v| v.as_u64()).unwrap_or(1) as f64;
                if dir == "down" {
                    // 与真机一致：滚到底就停住
                    let max_offset = (self.content_height() - 600.0).max(0.0);
                    self.scroll_offset =
                        (self.scroll_offset + ticks * self.tick_px).min(max_offset);
                } else {
                    self.scroll_offset = (self.scroll_offset - ticks * self.tick_px).max(0.0);
                }
                let (lines, avatars) = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines, "avatars": avatars})) });
            }
            if op == "contacts_click_list" {
                let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if x < 60.0 || x > 300.0 || y < 40.0 || y > 640.0 {
                    return Box::pin(async { bail!("contacts_click_outside_list") });
                }
                self.list_clicks.push((x, y));
                let vy = y + self.scroll_offset;
                if (vy - 59.0).abs() <= 15.0 {
                    self.manager_open = true;
                    self.calls.lock().unwrap().push((
                        "contacts_manager_opened_by_click".to_string(),
                        json!({"y": y}),
                    ));
                    let (lines, avatars) = self.render_list();
                    return Box::pin(
                        async move { Ok(json!({"lines": lines, "avatars": avatars})) },
                    );
                }
                let mut cur_y = 90.0;
                for g in &mut self.groups {
                    if vy >= cur_y && vy < cur_y + 40.0 {
                        g.expanded = !g.expanded;
                        break;
                    }
                    cur_y += 40.0;
                    if g.expanded {
                        for (_letter, items) in &g.letter_sections {
                            cur_y += 36.0;
                            cur_y += 54.0 * items.len() as f64;
                        }
                    }
                }
                let (lines, avatars) = self.render_list();
                return Box::pin(async move { Ok(json!({"lines": lines, "avatars": avatars})) });
            }
            if op == "contacts_open_row" {
                let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if y < 40.0 || y > 640.0 {
                    return Box::pin(async { bail!("contacts_click_outside_list") });
                }
                self.row_open_clicks.push(y);
                let vy = y + self.scroll_offset;
                if (vy - 59.0).abs() <= 15.0 {
                    // 点到「通讯录管理」按钮：真机会打开管理窗口
                    self.manager_open = true;
                    self.calls.lock().unwrap().push((
                        "contacts_manager_opened_by_click".to_string(),
                        json!({"y": y}),
                    ));
                    return Box::pin(async { Ok(json!({"lines": []})) });
                }
                let found = self.find_item_at_virtual_y(vy);
                if let Some((_group, Some(item), false, false)) = found {
                    let changed = self.shown_profile.as_deref() != Some(item.name.as_str());
                    if let Some(ref mis) = item.misaligned_profile {
                        let lines = mis.clone();
                        return Box::pin(
                            async move { Ok(json!({"lines": lines, "changed": true})) },
                        );
                    }
                    let item_name = item.name.clone();
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
                    self.shown_profile = Some(item_name);
                    return Box::pin(
                        async move { Ok(json!({"lines": lines, "changed": changed})) },
                    );
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
                                cur_y += 36.0 + 54.0 * items.len() as f64;
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
            if op == "contacts_manager_page"
                || op == "contacts_manager_scroll"
                || op == "contacts_manager_filter"
            {
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
                        if let Some((tag, _)) = self
                            .manager_tags
                            .iter()
                            .find(|(_, ty)| (ty - y).abs() < 10.0)
                        {
                            self.current_filter_tag = Some(tag.clone());
                        } else {
                            return Box::pin(async { bail!("contacts_filter_position_invalid") });
                        }
                    } else {
                        self.current_filter_tag = None;
                    }
                    self.manager_scroll_row = 0;
                } else if op == "contacts_manager_scroll" {
                    let dir = params
                        .get("direction")
                        .and_then(|v| v.as_str())
                        .unwrap_or("down");
                    let ticks = params.get("ticks").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                    let filtered_rows: Vec<_> = self
                        .manager_rows
                        .iter()
                        .filter(|r| {
                            if let Some(ref tag) = self.current_filter_tag {
                                r.tags.contains(tag)
                            } else {
                                true
                            }
                        })
                        .collect();
                    let max_start = filtered_rows.len().saturating_sub(10);
                    if dir == "down" {
                        self.manager_scroll_row = (self.manager_scroll_row + ticks).min(max_start);
                    } else {
                        self.manager_scroll_row = self.manager_scroll_row.saturating_sub(ticks);
                    }
                }

                let filtered_rows: Vec<_> = self
                    .manager_rows
                    .iter()
                    .filter(|r| {
                        if let Some(ref tag) = self.current_filter_tag {
                            r.tags.contains(tag)
                        } else {
                            true
                        }
                    })
                    .collect();

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
        fn retarget<'a>(
            &'a mut self,
            _recipient: &'a str,
            _directory: &'a Path,
            _names: &'a [String],
            _group: bool,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn sessions<'a>(
            &'a mut self,
            _names: &'a [String],
        ) -> Pin<
            Box<dyn Future<Output = Result<Vec<crate::core::scheduler::SessionRow>>> + Send + 'a>,
        > {
            Box::pin(async { Ok(vec![]) })
        }
        fn download<'a>(
            &'a mut self,
            _media_id: &'a str,
            _kind: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<PathBuf>> + Send + 'a>> {
            Box::pin(async { bail!("not implemented") })
        }
        fn last_switch(&self) -> Option<Value> {
            None
        }
        fn ocr_status(&self) -> Option<OcrStatus> {
            None
        }
        fn is_connected(&self) -> bool {
            true
        }
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
            VirtualContactItem {
                name: "张三".to_string(),
                remark: None,
                wechat_id: Some("wxid_p1".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
            VirtualContactItem {
                name: "李四".to_string(),
                remark: None,
                wechat_id: Some("wxid_p2".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
            VirtualContactItem {
                name: "王五".to_string(),
                remark: None,
                wechat_id: Some("wxid_p3".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
        ];
        let w_items = vec![VirtualContactItem {
            name: "企微员工赵".to_string(),
            remark: None,
            wechat_id: None,
            enterprise: Some("某某公司".to_string()),
            real_name: Some("赵六".to_string()),
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }];
        let o_items = vec![
            VirtualContactItem {
                name: "公众号一".to_string(),
                remark: None,
                wechat_id: Some("gh_off1".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
            VirtualContactItem {
                name: "公众号二".to_string(),
                remark: None,
                wechat_id: Some("gh_off2".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
        ];

        let g_personal = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("P".to_string(), p_items)],
            expanded: false,
        };
        let g_work = VirtualGroup {
            category: "work".to_string(),
            title: "企业微信联系人".to_string(),
            letter_sections: vec![("W".to_string(), w_items)],
            expanded: false,
        };
        let g_official = VirtualGroup {
            category: "official".to_string(),
            title: "公众号、服务号".to_string(),
            letter_sections: vec![("O".to_string(), o_items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g_personal, g_work, g_official]);
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

        assert!(report.complete, "report: {report:?}");
        assert!(!report.yielded);
        assert_eq!(report.unreadable_rows, 0);
        let profiles = store.list_profiles("test_acct", None).unwrap();
        // 公众号不采集（用户 2026-11-08）：只有 3 个联系人 + 1 个企业微信联系人
        assert_eq!(profiles.len(), 4);
        assert!(
            profiles.iter().all(|p| p["category"] != "official"),
            "{profiles:?}"
        );
        assert!(report.per_category.iter().all(|(c, _, _)| c != "official"));
    }

    #[tokio::test]
    async fn test_sync_reports_incomplete_when_rows_unreadable() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![
            VirtualContactItem {
                name: "张三".to_string(),
                remark: None,
                wechat_id: Some("wxid_p1".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            },
            // 资料卡读不出任何文字
            VirtualContactItem {
                name: "李四".to_string(),
                remark: None,
                wechat_id: Some("wxid_p2".to_string()),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: Some(vec![]),
            },
        ];
        let g_personal = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("P".to_string(), p_items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g_personal]);
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
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

        let p_items = (1..=4)
            .map(|i| VirtualContactItem {
                name: format!("用户{i}"),
                remark: None,
                wechat_id: Some(format!("wxid_p{i}")),
                enterprise: None,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            })
            .collect();
        let g_personal = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("U".to_string(), p_items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g_personal]);
        ui.manager_rows = (1..=4)
            .map(|i| ManagerRow {
                row_y: 96.0 + 48.0 * ((i - 1) as f64),
                name: Some(format!("用户{i}")),
                remark: None,
                tags: vec![],
                permission: None,
            })
            .collect();

        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Incremental,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert!(report.complete);
        let open_row_calls = ui
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| op == "contacts_open_row")
            .count();
        assert_eq!(
            open_row_calls, 1,
            "Only the 4th new user should trigger contacts_open_row"
        );
    }

    #[tokio::test]
    async fn test_sync_never_requests_forbidden_click() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let p_items = vec![VirtualContactItem {
            name: "张三".to_string(),
            remark: None,
            wechat_id: Some("wxid_p1".to_string()),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }];
        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("Z".to_string(), p_items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

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

        let p_items = vec![VirtualContactItem {
            name: "张三".to_string(),
            remark: None,
            wechat_id: Some("wxid_p1".to_string()),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }];
        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("Z".to_string(), p_items)],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);

        // Case A: pause_file did not exist initially -> removed after sync
        assert!(!pause_file.exists());
        let no_yield = || false;
        run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert!(!pause_file.exists());

        // Case B: pause_file existed initially -> remains after sync
        fs::write(&pause_file, b"{\"paused\":true}").unwrap();
        run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            2000,
        )
        .await
        .unwrap();
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
        let p_items = vec![VirtualContactItem {
            name: secret_name.to_string(),
            remark: None,
            wechat_id: Some(secret_wxid.to_string()),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        }];
        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![("M".to_string(), p_items)],
            expanded: false,
        };

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
        run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        drop(_guard);

        let logs = String::from_utf8(log_buffer.lock().unwrap().clone()).unwrap();
        assert!(
            !logs.contains(secret_name),
            "Logs leaked secret name: {}",
            logs
        );
        assert!(
            !logs.contains(secret_wxid),
            "Logs leaked secret wxid: {}",
            logs
        );
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
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

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
            letter_sections: vec![(
                "G".to_string(),
                vec![VirtualContactItem {
                    name: "公众号甲".to_string(),
                    remark: None,
                    wechat_id: Some("gh_111".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: None,
                }],
            )],
            expanded: false,
        };
        let g2 = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![(
                "Z".to_string(),
                vec![VirtualContactItem {
                    name: "张三".to_string(),
                    remark: None,
                    wechat_id: Some("wxid_zs".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: None,
                }],
            )],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g1, g2]);
        let no_yield = || false;
        run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

        // 检查所有 open_row 点击的 y，不得落在 header 或字母行
        for &y in &ui.row_open_clicks {
            let vy = y + ui.scroll_offset;
            let found = ui.find_item_at_virtual_y(vy);
            if let Some((_, _, is_header, is_letter)) = found {
                assert!(
                    !is_header,
                    "contacts_open_row clicked on group header at y={y}"
                );
                assert!(
                    !is_letter,
                    "contacts_open_row clicked on letter row at y={y}"
                );
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
            letter_sections: vec![(
                "Z".to_string(),
                vec![VirtualContactItem {
                    name: "张三".to_string(),
                    remark: None,
                    wechat_id: Some("wxid_zhangsan".to_string()),
                    enterprise: None,
                    real_name: None,
                    tags: vec![],
                    friend_permission: None,
                    unreadable: false,
                    misaligned_profile: Some(misaligned_lines),
                }],
            )],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

        assert!(!report.complete);
        assert_eq!(report.unreadable_rows, 1);
        assert!(
            ui.row_open_clicks.len() >= 1,
            "Must have opened the row to detect misalignment"
        );
        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(
            profiles.len(),
            0,
            "Misaligned profile must not be written to db"
        );
    }

    #[tokio::test]
    async fn test_row_without_readable_name_is_opened_by_avatar_without_misattribution() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let g = VirtualGroup {
            category: "personal".to_string(),
            title: "联系人".to_string(),
            letter_sections: vec![(
                "A".to_string(),
                vec![
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
                ],
            )],
            expanded: false,
        };

        let mut ui = VirtualContactsUi::new(vec![g]);
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "test_acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

        // 列表上认不出名字的行仍按头像点开，资料卡换成了新的人就记下；三个人的微信号各归各位
        assert!(report.complete, "{report:?}");
        let profiles = store.list_profiles("test_acct", None).unwrap();
        assert_eq!(profiles.len(), 3);
        for (nick, wid) in [
            ("用户A", "wxid_a"),
            ("用户B", "wxid_b"),
            ("用户C", "wxid_c"),
        ] {
            assert!(
                profiles
                    .iter()
                    .any(|p| p["nickname"] == nick && p["wechat_id"] == wid),
                "{nick} 应对应 {wid}: {profiles:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_daily_sync_attempted_at_most_once_per_day_even_if_incomplete() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let res_path = account_state_path(&state_dir, "test_acct", "result");
        let _attempt_path = state_dir.join("contacts-sync-daily-attempt.json");

        // 写入一次不完整同步结果
        fs::write(
            &res_path,
            serde_json::to_string(&json!({
                "finished_at_ms": 1000000000000_i64,
                "complete": false,
            }))
            .unwrap(),
        )
        .unwrap();

        // 检查在同一天（未跨天且未满20小时）是否会再次触发
        let now_sec = 1000000000.0 + 3600.0; // 1小时后
        assert!(!should_attempt_daily_sync(
            &state_dir,
            "test_acct",
            true,
            now_sec
        ));

        // 记录当天 attempt
        record_daily_sync_attempt(&state_dir, "test_acct", now_sec).unwrap();
        assert!(!should_attempt_daily_sync(
            &state_dir,
            "test_acct",
            true,
            now_sec + 3600.0
        ));
    }

    #[test]
    fn test_stale_contacts_sync_pause_cleared_on_startup() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();
        let pause_file = state_dir.join("admin-pause.json");

        // Case 1: reason == contacts_sync -> 清理
        fs::write(
            &pause_file,
            serde_json::to_string(&json!({
                "paused": true,
                "reason": "contacts_sync"
            }))
            .unwrap(),
        )
        .unwrap();
        clean_stale_contacts_sync_pause(&state_dir);
        assert!(
            !pause_file.exists(),
            "Stale contacts_sync pause must be removed"
        );

        // Case 2: reason == manual -> 不清理
        fs::write(
            &pause_file,
            serde_json::to_string(&json!({
                "paused": true,
                "reason": "manual_admin"
            }))
            .unwrap(),
        )
        .unwrap();
        clean_stale_contacts_sync_pause(&state_dir);
        assert!(pause_file.exists(), "User manual pause must NOT be removed");
    }
    fn letter_group(
        category: &str,
        title: &str,
        prefix: &str,
        sections: &[(&str, usize)],
    ) -> VirtualGroup {
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
        VirtualGroup {
            category: category.to_string(),
            title: title.to_string(),
            letter_sections,
            expanded: false,
        }
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
        let report = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();

        assert_eq!(ui.header_open_clicks, 0, "不得用打开资料卡点分组标题");
        assert!(report.complete, "{report:?}");
        assert_eq!(report.per_category, vec![("personal".to_string(), 7, 7)]);
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
        let res = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await;
        assert!(res.is_err(), "读分组失败应当返回错误");
        let ops: Vec<String> = ui
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(op, _)| op.clone())
            .collect();
        let enter = ops
            .iter()
            .position(|o| o == "contacts_enter")
            .expect("entered");
        assert!(
            ops[enter..].iter().any(|o| o == "contacts_leave"),
            "进过通讯录就必须离开: {ops:?}"
        );
        assert!(
            !state_dir.join("admin-pause.json").exists(),
            "同步写的暂停标记必须清除"
        );
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
            let report = run_sync(&mut ui, &store, "acct", mode, &checkpoint, &no_yield, 1000)
                .await
                .unwrap();
            assert!(report.complete, "{mode:?}: {report:?}");
        }
        let ops: Vec<String> = ui
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(op, _)| op.clone())
            .collect();
        assert!(
            !ops.iter().any(|o| o.starts_with("contacts_manager_")),
            "不得打开通讯录管理: {ops:?}"
        );
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

        fs::write(&checkpoint, "{}").unwrap(); // 旧版本留下的断点
        ui.fail_op = Some("contacts_scroll_list".to_string());
        let no_yield = || false;
        assert!(
            run_sync(
                &mut ui,
                &store,
                "acct",
                SyncMode::Full,
                &checkpoint,
                &no_yield,
                2000
            )
            .await
            .is_err()
        );
        assert!(
            !checkpoint.exists(),
            "出错后必须清掉断点，否则协调器会每轮续做、陷入循环"
        );
        let result: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(state_dir.join("contacts-sync-result.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(result["complete"], false);
        assert!(
            result["error"]
                .as_str()
                .unwrap_or("")
                .contains("layout_mismatch"),
            "{result}"
        );
    }

    /// 中途停下不做断点续做：下次从头按固定流程再走一遍，按微信号更新，不重复记录
    #[tokio::test]
    async fn test_stopped_run_restarts_from_top_without_duplicates() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let personal = letter_group("personal", "联系人", "人", &[("A", 5), ("B", 5)]);
        let mut ui = VirtualContactsUi::new(vec![personal]);
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n2 = n.clone();
        let stop_soon = move || n2.fetch_add(1, Ordering::SeqCst) >= 4;
        let first = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &stop_soon,
            1000,
        )
        .await
        .unwrap();
        assert!(first.yielded && !first.complete, "{first:?}");
        assert!(!checkpoint.exists());
        let no_yield = || false;
        let second = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            2000,
        )
        .await
        .unwrap();
        assert!(second.complete, "{second:?}");
        assert_eq!(store.list_profiles("acct", None).unwrap().len(), 10);
    }

    /// 实测（2026-11-08 截图）：每个字母单独占一行（36 点高），联系人行 54 点高，字母常识别不出。
    /// 按头像逐个点开、点完一屏滚一屏：每个人只点一次、全部记对。
    #[tokio::test]
    async fn test_unread_letter_rows_and_scrolling_open_each_contact_once() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let work = letter_group(
            "work",
            "企业微信联系人",
            "企",
            &[("A", 3), ("C", 1), ("D", 2), ("L", 4), ("Z", 4)],
        );
        let personal = letter_group(
            "personal",
            "联系人",
            "人",
            &[("K", 1), ("L", 1), ("M", 2), ("N", 1), ("W", 6), ("X", 11)],
        );
        let mut ui = VirtualContactsUi::new(vec![work, personal]);
        ui.letters_unread = true;
        ui.counts_always = true;
        ui.header_arrows = true;
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        assert_eq!(
            report.per_category,
            vec![
                ("work".to_string(), 14, 14),
                ("personal".to_string(), 22, 22)
            ]
        );
        let profiles = store.list_profiles("acct", None).unwrap();
        assert_eq!(profiles.len(), 36);
        for p in &profiles {
            let nick = p["nickname"].as_str().unwrap();
            let want = if p["category"] == "work" {
                "企"
            } else {
                "人"
            };
            assert!(nick.starts_with(want), "分类记错: {p:?}");
        }
        let opens = ui
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| op == "contacts_open_row")
            .count();
        assert_eq!(opens, 36, "每个人只点一次");
    }

    /// 实机 2026-11-08：每格滚得比估计的多，5 格就超过一屏，第二屏开始跳人。
    /// 每格距离事先未知（这里 140 点/格），也必须每人只点一次、一个不漏。
    /// 用户 2026-11-08：微信团队（weixin）、文件传输助手（filehelper）在微信里就归「联系人」分组，
    /// 和普通联系人一样能收发消息和文件，不能单独归为官方号；分组人数照微信显示的核对。全量、增量都一样。
    #[tokio::test]
    async fn test_wechat_built_in_accounts_stay_in_contacts_group() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let mut personal = letter_group("personal", "联系人", "人", &[("A", 2), ("W", 1)]);
        let sys = |name: &str, id: &str| VirtualContactItem {
            name: name.to_string(),
            remark: None,
            wechat_id: Some(id.to_string()),
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        };
        personal
            .letter_sections
            .push(("W".to_string(), vec![sys("微信团队", "weixin")]));
        personal.letter_sections.insert(
            1,
            ("F".to_string(), vec![sys("文件传输助手", "filehelper")]),
        );
        let mut ui = VirtualContactsUi::new(vec![personal]);
        let no_yield = || false;
        for (mode, now) in [(SyncMode::Full, 1000), (SyncMode::Incremental, 2000)] {
            let report = run_sync(&mut ui, &store, "acct", mode, &checkpoint, &no_yield, now)
                .await
                .unwrap();
            assert!(report.complete, "{mode:?}: {report:?}");
            assert_eq!(
                report.per_category,
                vec![("personal".to_string(), 5, 5)],
                "{mode:?}"
            );
        }
        let profiles = store.list_profiles("acct", None).unwrap();
        assert_eq!(profiles.len(), 5);
        for id in ["weixin", "filehelper"] {
            let p = profiles.iter().find(|p| p["wechat_id"] == id).expect(id);
            assert_eq!(p["category"], "personal", "{p:?}");
            assert!(p["removed_at"].is_null(), "{p:?}");
        }
    }

    #[tokio::test]
    async fn test_unknown_scroll_distance_never_skips_contacts() {
        for tick_px in [40.0, 90.0, 140.0, 230.0] {
            let temp = TestTempDir::new();
            let state_dir = temp.path().join("state");
            let store = HistoryStore::open(&state_dir).unwrap();
            let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
            let personal = letter_group(
                "personal",
                "联系人",
                "人",
                &[("A", 2), ("K", 1), ("L", 1), ("M", 2), ("W", 6), ("X", 10)],
            );
            let mut ui = VirtualContactsUi::new(vec![personal]);
            ui.letters_unread = true;
            ui.tick_px = tick_px;
            let no_yield = || false;
            let report = run_sync(
                &mut ui,
                &store,
                "acct",
                SyncMode::Full,
                &checkpoint,
                &no_yield,
                1000,
            )
            .await
            .unwrap();
            assert!(report.complete, "tick_px={tick_px}: {report:?}");
            let opens = ui
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(op, _)| op == "contacts_open_row")
                .count();
            assert_eq!(opens, 22, "tick_px={tick_px}: 每人只点一次");
        }
    }

    /// 实机：联系人分组每一行都"资料卡名字与列表名字不一致"、一个也没写入。列表名字会被截断（…），
    /// 资料卡名字后可能多出性别图标等字符。比对时只看汉字/字母/数字，一方包含另一方（≥2 字）即视为同一人。
    /// 实机 2026-11-08（字形结构日志）：企业微信列表显示"名字…@企业"（名字过长被截断），
    /// 资料卡第一行是完整名字，下一行单独是"@企业"。按 @ 拆开分别核对。
    #[test]
    fn test_work_row_matches_card_by_name_prefix_and_company_line() {
        let card = |nick: &str, company: &str| {
            vec![
                line(nick, 520.0, 60.0),
                line(company, 520.0, 90.0),
                line("备注", 430.0, 200.0),
            ]
        };
        // 名字截断
        assert!(row_matches_profile(
            "王小明…@腾讯科技",
            &card("王小明明白", "@腾讯科技"),
            "王小明明白",
            None
        ));
        // 企业名也被截断
        assert!(row_matches_profile(
            "王小明..@腾讯科",
            &card("王小明明白", "@腾讯科技有限公司"),
            "王小明明白",
            None
        ));
        // 名字有 1 个字识别不同，企业对得上
        assert!(row_matches_profile(
            "李西@某某公司",
            &card("李四", "@某某公司"),
            "李四",
            None
        ));
        assert!(row_matches_profile(
            "季小 @某某公司",
            &card("李小", "@某某公司"),
            "李小",
            None
        ));
        // 企业对不上：不是同一人
        assert!(!row_matches_profile(
            "李西@某某公司",
            &card("李四", "@另一家公司"),
            "李四",
            None
        ));
        // 名字差太多：不是同一人
        assert!(!row_matches_profile(
            "张大山@某某公司",
            &card("李四光", "@某某公司"),
            "李四光",
            None
        ));
    }

    /// 实机：名字里的英文字母在列表上被识别成 "@"，列表名字的字按顺序都在资料卡名字里（≥4 字）也算同一人
    #[test]
    fn test_row_chars_in_order_within_profile_name_match() {
        let card = vec![line("春a夏秋冬东南西北xyz", 520.0, 60.0)];
        assert!(row_matches_profile(
            "春@夏秋冬东南…",
            &card,
            "春a夏秋冬东南西北xyz",
            None
        ));
        // 少于 4 个字不按此规则
        assert!(!row_matches_profile(
            "春冬",
            &card,
            "春a夏秋冬东南西北xyz",
            None
        ));
        // 顺序不对不算
        assert!(!row_matches_profile(
            "冬秋夏春",
            &card,
            "春a夏秋冬东南西北xyz",
            None
        ));
    }

    #[test]
    fn test_card_line_shape_keeps_ui_words_only() {
        assert_eq!(card_line_shape("企业某某科技◎"), "[企业]cccc◎");
        assert_eq!(card_line_shape("来源 企业微信"), "[来源]_[企业微信]");
        assert_eq!(card_line_shape("@某某 Ab"), "@cc_aa");
    }

    #[test]
    fn test_shape_log_hides_text() {
        assert_eq!(shape_against("张三@腾讯", "张三"), "==@cc");
        assert_eq!(shape_against("Ab 1…", "ab"), "==_a…");
    }

    #[test]
    fn test_row_and_profile_names_match_after_normalization() {
        assert!(names_match("测试甲", "测试甲 ♂"));
        assert!(names_match("很长的客户名字…", "很长的客户名字后面还有"));
        assert!(names_match("Coffee 咖啡", "coffee咖啡"));
        assert!(names_match("😀小王", "小王"));
        assert!(!names_match("张三", "李四"));
        assert!(!names_match("王", "王五")); // 不足 2 字不做包含匹配
        assert!(!names_match("", "王五"));
        // 实机：同长度名字个别字识别有出入（4 字及以上允许差 1 个字）
        assert!(names_match("测试客户甲乙", "测试客户甲已"));
        assert!(names_match("咖啡小站", "咖啡小沾"));
        assert!(!names_match("张三", "李三")); // 短名字不放宽
        assert!(!names_match("咖啡小站", "茶饮大站")); // 差 2 个字不算
    }

    /// 用户要求（2026-11-08）：一屏里逐个点开采集，点完往下滚一整屏，不重复点。
    /// 原实现每次只滚一格（90 点）、按文字判断是否处理过，识别稍有出入就重复点同一行。
    #[tokio::test]
    async fn test_each_row_opened_once_and_list_scrolls_by_screen() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let personal = letter_group(
            "personal",
            "联系人",
            "人",
            &[("A", 4), ("B", 5), ("C", 4), ("D", 5), ("E", 4)],
        );
        let mut ui = VirtualContactsUi::new(vec![personal]);
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        assert_eq!(
            ui.row_open_clicks.len(),
            22,
            "每人只点开一次，实际 {} 次",
            ui.row_open_clicks.len()
        );
        let downs: Vec<u64> = ui
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, p)| op == "contacts_scroll_list" && p["direction"] == "down")
            .map(|(_, p)| p["ticks"].as_u64().unwrap_or(1))
            .collect();
        // 第一次滚 1 格量出每格距离，之后每次把本屏最后一人滚到列表上部（一屏一滚）
        assert_eq!(downs.first(), Some(&1), "{downs:?}");
        assert!(
            downs.iter().skip(1).all(|t| *t >= 3),
            "之后应按整屏滚动: {downs:?}"
        );
        assert!(
            downs.len() <= 6,
            "22 人约 3 屏，滚动次数不应过多: {downs:?}"
        );
    }

    /// 实机：开始时"企业微信联系人"处于展开状态（之前操作遗留），把"联系人"标题挤出第一屏，
    /// 只读到 2 个分组；处理完公众号后也找不到下一个分组标题。每个分组开始前必须先收起所有展开的分组。
    #[tokio::test]
    async fn test_groups_left_expanded_are_collapsed_before_each_group() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let official = letter_group("official", "公众号、服务号", "号", &[("G", 6), ("H", 6)]);
        let mut work = letter_group("work", "企业微信联系人", "企", &[("Q", 5), ("R", 5)]);
        work.expanded = true; // 遗留的展开状态
        let personal = letter_group("personal", "联系人", "人", &[("A", 4), ("B", 4)]);
        let mut ui = VirtualContactsUi::new(vec![official, work, personal]);
        ui.header_arrows = true;
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert_eq!(
            report.per_category,
            vec![("work".to_string(), 10, 10), ("personal".to_string(), 8, 8)],
            "{report:?}"
        );
        assert!(report.complete, "{report:?}");
        assert_eq!(ui.header_open_clicks, 0);
    }

    /// 实机：分组展开后人数照样显示，标题与第一个联系人之间隔着（可能没识别出的）字母行约 80 点。
    /// 用人数判断展开会把收起的当成展开（不去点开、0 个联系人），把展开的当成收起（不收起、挤掉别的标题）。
    #[tokio::test]
    async fn test_expansion_judged_by_content_not_counts() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let official = letter_group("official", "公众号、服务号", "号", &[("G", 6), ("H", 6)]);
        let work = letter_group("work", "企业微信联系人", "企", &[("Q", 5)]);
        let mut personal = letter_group("personal", "联系人", "人", &[("A", 4), ("B", 4)]);
        personal.expanded = true; // 遗留展开，且人数照样显示
        let mut ui = VirtualContactsUi::new(vec![official, work, personal]);
        ui.header_arrows = true;
        ui.counts_always = true;
        let no_yield = || false;
        let report = run_sync(
            &mut ui,
            &store,
            "acct",
            SyncMode::Full,
            &checkpoint,
            &no_yield,
            1000,
        )
        .await
        .unwrap();
        assert_eq!(
            report.per_category,
            vec![("work".to_string(), 5, 5), ("personal".to_string(), 8, 8)],
            "{report:?}"
        );
        assert!(report.complete, "{report:?}");
    }

    /// 用户要求：初始化时先停对话监控、整个过程独占微信。同步第一步拿租约（在进入通讯录之前），
    /// 最后一步释放（在离开通讯录之后），出错也必须释放。
    #[tokio::test]
    async fn test_sync_holds_exclusive_lease_for_whole_run() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");
        let personal = letter_group("personal", "联系人", "人", &[("A", 3)]);
        let no_yield = || false;
        for fail in [None, Some("contacts_headers".to_string())] {
            let mut ui = VirtualContactsUi::new(vec![personal.clone()]);
            ui.fail_op = fail.clone();
            let _ = run_sync(
                &mut ui,
                &store,
                "acct",
                SyncMode::Full,
                &checkpoint,
                &no_yield,
                1000,
            )
            .await;
            let ops: Vec<String> = ui
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(op, _)| op.clone())
                .collect();
            assert_eq!(
                ops.first().map(String::as_str),
                Some("contacts_lease_begin"),
                "{fail:?}: {ops:?}"
            );
            assert_eq!(
                ops.last().map(String::as_str),
                Some("contacts_lease_end"),
                "{fail:?}: {ops:?}"
            );
            let leave = ops
                .iter()
                .rposition(|o| o == "contacts_leave")
                .expect("leave");
            assert!(leave < ops.len() - 1, "先离开通讯录再释放租约");
        }
    }

    fn fixture_profile(
        category: &str,
        name: &str,
        wid: Option<&str>,
        enterprise: Option<&str>,
    ) -> ProfileInput {
        ProfileInput {
            category: category.into(),
            wechat_id: wid.map(str::to_string),
            nickname: name.into(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: enterprise.map(str::to_string),
            real_name: None,
            tags: vec![],
            friend_permission: None,
        }
    }

    #[test]
    fn test_incremental_matching_requires_category_unique_exact_active_strong_identity() {
        let profiles = vec![
            json!({"contact_id":"personal", "category":"personal", "nickname":"同名客户", "identity":"strong"}),
            json!({"contact_id":"work", "category":"work", "nickname":"同名客户", "identity":"weak"}),
        ];
        assert_eq!(
            incremental_contact(&profiles, "personal", "同名客户"),
            Some("personal")
        );
        assert_eq!(incremental_contact(&profiles, "work", "同名客户"), None);
        assert_eq!(incremental_contact(&profiles, "personal", "同名"), None);
        let mut duplicates = profiles.clone();
        duplicates.push(profiles[0].clone());
        assert_eq!(
            incremental_contact(&duplicates, "personal", "同名客户"),
            None
        );
        let removed = vec![
            json!({"contact_id":"removed", "category":"personal", "nickname":"同名客户", "identity":"strong", "removed_at":1}),
        ];
        assert_eq!(incremental_contact(&removed, "personal", "同名客户"), None);
    }

    #[test]
    fn test_sync_state_and_daily_attempt_are_account_scoped() {
        let temp = TestTempDir::new();
        write_sync_result(
            temp.path(),
            "account-a",
            &json!({"complete":true,"finished_at_ms":1000000}),
        )
        .unwrap();
        record_daily_sync_attempt(temp.path(), "account-a", 1000.0).unwrap();
        assert!(!should_attempt_daily_sync(
            temp.path(),
            "account-a",
            true,
            1001.0
        ));
        assert!(should_attempt_daily_sync(
            temp.path(),
            "account-b",
            true,
            1001.0
        ));
        write_sync_result(
            temp.path(),
            "account-b",
            &json!({"complete":false,"finished_at_ms":1001000}),
        )
        .unwrap();
        write_sync_result(
            temp.path(),
            "account-a",
            &json!({"complete":false,"finished_at_ms":1002000}),
        )
        .unwrap();
        let success: Value = serde_json::from_slice(
            &fs::read(account_state_path(temp.path(), "account-a", "success")).unwrap(),
        )
        .unwrap();
        assert_eq!(success["finished_at_ms"], 1000000);
        assert_eq!(success["account_id"], "account-a");
        assert!(!account_state_path(temp.path(), "account-b", "success").exists());
        assert_eq!(
            account_state_path(temp.path(), "../account-a", "result").parent(),
            Some(temp.path())
        );
    }

    #[tokio::test]
    async fn test_incomplete_sync_preserves_missing_contact_and_other_account() {
        let temp = TestTempDir::new();
        let store = HistoryStore::open(temp.path()).unwrap();
        let p = fixture_profile("personal", "未读客户", Some("wxid_unread"), None);
        store.upsert_profile("account-a", &p, 1).unwrap();
        store.upsert_profile("account-b", &p, 1).unwrap();
        let item = VirtualContactItem {
            name: p.nickname,
            remark: None,
            wechat_id: p.wechat_id,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: Some(vec![]),
        };
        let mut ui = VirtualContactsUi::new(vec![VirtualGroup {
            category: "personal".into(),
            title: "联系人".into(),
            letter_sections: vec![("A".into(), vec![item])],
            expanded: false,
        }]);
        let report = run_sync(
            &mut ui,
            &store,
            "account-a",
            SyncMode::Full,
            &temp.path().join("contacts-sync-checkpoint.json"),
            &|| false,
            2000,
        )
        .await
        .unwrap();
        assert!(!report.complete);
        for account in ["account-a", "account-b"] {
            assert!(store.list_profiles(account, None).unwrap()[0]["removed_at"].is_null());
        }
    }

    #[tokio::test]
    async fn test_incremental_reappearance_reopens_card_and_restores_contact() {
        let temp = TestTempDir::new();
        let store = HistoryStore::open(temp.path()).unwrap();
        let p = fixture_profile("personal", "返回客户", Some("wxid_returned"), None);
        store.upsert_profile("account-a", &p, 1).unwrap();
        store.mark_missing("account-a", "personal", &[], 2).unwrap();
        let item = VirtualContactItem {
            name: p.nickname,
            remark: None,
            wechat_id: p.wechat_id,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        };
        let mut ui = VirtualContactsUi::new(vec![VirtualGroup {
            category: "personal".into(),
            title: "联系人".into(),
            letter_sections: vec![("A".into(), vec![item])],
            expanded: false,
        }]);
        let report = run_sync(
            &mut ui,
            &store,
            "account-a",
            SyncMode::Incremental,
            &temp.path().join("contacts-sync-checkpoint.json"),
            &|| false,
            2000,
        )
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        let profiles = store.list_profiles("account-a", None).unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0]["removed_at"].is_null());
        assert!(
            ui.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(op, _)| op == "contacts_open_row")
        );
    }

    #[tokio::test]
    async fn test_account_change_during_card_read_cannot_write_or_mark_missing() {
        let temp = TestTempDir::new();
        let store = HistoryStore::open(temp.path()).unwrap();
        let p = fixture_profile("personal", "切换测试", Some("wxid_change"), None);
        store.upsert_profile("account-a", &p, 1).unwrap();
        let item = VirtualContactItem {
            name: p.nickname,
            remark: None,
            wechat_id: p.wechat_id,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        };
        let mut ui = VirtualContactsUi::new(vec![VirtualGroup {
            category: "personal".into(),
            title: "联系人".into(),
            letter_sections: vec![("A".into(), vec![item])],
            expanded: false,
        }]);
        let calls = ui.calls.clone();
        let account_changed = move || {
            calls
                .lock()
                .unwrap()
                .iter()
                .any(|(op, _)| op == "contacts_open_row")
        };
        let report = run_sync(
            &mut ui,
            &store,
            "account-a",
            SyncMode::Full,
            &temp.path().join("contacts-sync-checkpoint.json"),
            &account_changed,
            2000,
        )
        .await
        .unwrap();
        assert!(!report.complete);
        assert!(report.yielded);
        let profiles = store.list_profiles("account-a", None).unwrap();
        assert_eq!(profiles[0]["last_synced_at"], 1);
        assert!(profiles[0]["removed_at"].is_null());
        assert!(store.list_profiles("account-b", None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_incremental_same_name_work_contacts_keep_both_enterprises() {
        let temp = TestTempDir::new();
        let store = HistoryStore::open(temp.path()).unwrap();
        let mut items = Vec::new();
        for ent in ["测试企业甲", "测试企业乙"] {
            let p = fixture_profile("work", "同名员工", None, Some(ent));
            store.upsert_profile("account-a", &p, 1).unwrap();
            items.push(VirtualContactItem {
                name: p.nickname,
                remark: None,
                wechat_id: None,
                enterprise: p.enterprise,
                real_name: None,
                tags: vec![],
                friend_permission: None,
                unreadable: false,
                misaligned_profile: None,
            });
        }
        let mut ui = VirtualContactsUi::new(vec![VirtualGroup {
            category: "work".into(),
            title: "企业微信联系人".into(),
            letter_sections: vec![("A".into(), items)],
            expanded: false,
        }]);
        let report = run_sync(
            &mut ui,
            &store,
            "account-a",
            SyncMode::Incremental,
            &temp.path().join("contacts-sync-checkpoint.json"),
            &|| false,
            2000,
        )
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        assert_eq!(report.per_category, vec![("work".into(), 2, 2)]);
        let profiles = store.list_profiles("account-a", None).unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(
            profiles
                .iter()
                .all(|p| p["removed_at"].is_null() && p["last_synced_at"] == 2000)
        );
    }

    #[tokio::test]
    async fn test_incremental_does_not_reuse_another_accounts_same_name() {
        let temp = TestTempDir::new();
        let store = HistoryStore::open(temp.path()).unwrap();
        let p = fixture_profile("personal", "跨号客户", Some("wxid_shared"), None);
        store.upsert_profile("account-a", &p, 1).unwrap();
        let item = VirtualContactItem {
            name: p.nickname,
            remark: None,
            wechat_id: p.wechat_id,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
            unreadable: false,
            misaligned_profile: None,
        };
        let mut ui = VirtualContactsUi::new(vec![VirtualGroup {
            category: "personal".into(),
            title: "联系人".into(),
            letter_sections: vec![("A".into(), vec![item])],
            expanded: false,
        }]);
        let report = run_sync(
            &mut ui,
            &store,
            "account-b",
            SyncMode::Incremental,
            &temp.path().join("contacts-sync-checkpoint.json"),
            &|| false,
            2000,
        )
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        let a = store.list_profiles("account-a", None).unwrap();
        let b = store.list_profiles("account-b", None).unwrap();
        assert_eq!(a[0]["last_synced_at"], 1);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0]["last_synced_at"], 2000);
        assert_ne!(a[0]["contact_id"], b[0]["contact_id"]);
    }
}
