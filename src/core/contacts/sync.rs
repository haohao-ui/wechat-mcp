use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use crate::core::contacts::parse::{
    parse_group_headers, parse_manager_page, parse_profile, ManagerColumns, ManagerRow, OcrLine,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CheckpointState {
    mode: Option<SyncMode>,
    category_index: usize,
    categories: Vec<(String, u32, f64)>, // (category, expected, header_y)
    seen_ids_by_cat: HashMap<String, Vec<String>>,
    cat_progress: HashMap<String, usize>, // category -> processed count
    unreadable_rows: u32,
    needs_review: u32,
    manager_done: bool,
    manager_names: Vec<String>,
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

    // 3. 打开管理窗口读取表格（收集已有名字/信息，以便增量对比）
    let mut manager_contacts: Vec<ManagerRow> = Vec::new();
    if !state.manager_done {
        let mut open_res = capture.call("contacts_manager_open", json!({})).await;
        if open_res.is_err() {
            // 如果出错尝试再次打开
            open_res = capture.call("contacts_manager_open", json!({})).await;
        }

        if open_res.is_ok() {
            let cols = ManagerColumns {
                name_x: 300.0,
                remark_x: 385.0,
                tags_x: 500.0,
                permission_x: 620.0,
            };

            let mut page_count = 0;
            loop {
                page_count += 1;
                let page_res = capture.call("contacts_manager_page", json!({})).await;
                let page_val = match page_res {
                    Ok(val) => val,
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("contacts_manager_missing") {
                            // 重新打开
                            let _ = capture.call("contacts_manager_open", json!({})).await;
                            capture
                                .call("contacts_manager_page", json!({}))
                                .await
                                .unwrap_or_else(|_| json!({"lines": []}))
                        } else {
                            json!({"lines": []})
                        }
                    }
                };

                let lines: Vec<OcrLine> = page_val
                    .get("lines")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                if lines.is_empty() {
                    break;
                }

                let rows = parse_manager_page(&lines, 96.0, 48.0, 10, &cols);
                let had_names = rows.iter().any(|r| r.name.is_some());
                if !had_names {
                    break;
                }

                for r in rows {
                    if let Some(ref name) = r.name {
                        if !state.manager_names.contains(name) {
                            state.manager_names.push(name.clone());
                        }
                    }
                    manager_contacts.push(r);
                }

                if page_count >= 5 {
                    break;
                }

                let scroll_res = capture
                    .call(
                        "contacts_manager_scroll",
                        json!({"direction": "down", "ticks": 10}),
                    )
                    .await;
                if let Err(e) = scroll_res {
                    if e.to_string().contains("contacts_manager_missing") {
                        let _ = capture.call("contacts_manager_open", json!({})).await;
                    }
                }
            }

            let _ = capture.call("contacts_manager_close", json!({})).await;
            state.manager_done = true;
        }
    }

    // 4. 逐分组、逐行点开资料卡
    let existing_profiles = store.list_profiles(account_id, None).unwrap_or_default();
    let existing_names: HashSet<String> = existing_profiles
        .iter()
        .filter_map(|p| {
            p.get("nickname")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let row_height = 50.0;
    let mut yielded = false;

    'outer: while state.category_index < state.categories.len() {
        let (cat_name, expected, header_y) = state.categories[state.category_index].clone();
        let seen_list = state
            .seen_ids_by_cat
            .entry(cat_name.clone())
            .or_insert_with(Vec::new);
        let mut processed = *state.cat_progress.entry(cat_name.clone()).or_insert(0);

        while processed < expected as usize {
            // 每处理完一个联系人前/后检查 should_yield
            if should_yield() {
                yielded = true;
                break 'outer;
            }

            let row_idx = processed;
            let row_y = header_y + row_height * (row_idx as f64 + 1.0);

            // 增量模式优化：如果在管理表格里已经有记录，并且数据库里已有且无变化，跳过 open_row
            let mut skip_open = false;
            if mode == SyncMode::Incremental && !existing_profiles.is_empty() {
                // 如果管理表格第 row_idx 项已在 existing_names 中
                if let Some(m_name) = state.manager_names.get(row_idx) {
                    if existing_names.contains(m_name) {
                        skip_open = true;
                        // 找到该联系人的 contact_id 加入 seen
                        if let Some(p) = existing_profiles.iter().find(|p| {
                            p.get("nickname").and_then(|v| v.as_str()) == Some(m_name.as_str())
                        }) {
                            if let Some(cid) = p.get("contact_id").and_then(|v| v.as_str()) {
                                if !seen_list.contains(&cid.to_string()) {
                                    seen_list.push(cid.to_string());
                                }
                            }
                        }
                    }
                }
            }

            if !skip_open {
                let open_res = capture
                    .call("contacts_open_row", json!({"y": row_y}))
                    .await;
                let prof_lines: Vec<OcrLine> = open_res
                    .ok()
                    .and_then(|v| v.get("lines").cloned())
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();

                if prof_lines.is_empty() {
                    state.unreadable_rows += 1;
                } else {
                    let parse_result = parse_profile(&prof_lines, &cat_name);
                    let f = parse_result.fields;
                    if f.nickname.trim().is_empty() && f.wechat_id.is_none() {
                        state.unreadable_rows += 1;
                    } else {
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
                            tags: f.tags,
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
                            Err(e) => {
                                warn!("upsert profile error: {e:#}");
                                state.unreadable_rows += 1;
                            }
                        }
                    }
                }
            }

            processed += 1;
            state.cat_progress.insert(cat_name.clone(), processed);

            if should_yield() {
                yielded = true;
                break 'outer;
            }
        }

        // 标记缺失
        let _ = store.mark_missing(account_id, &cat_name, seen_list, now_ms);

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

    struct TestCapture {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        headers_lines: Vec<OcrLine>,
        manager_pages: Arc<Mutex<Vec<Result<Vec<OcrLine>, String>>>>,
        manager_page_index: Arc<Mutex<usize>>,
        profile_responses: HashMap<i64, Vec<OcrLine>>,
    }

    impl TestCapture {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                headers_lines: Vec::new(),
                manager_pages: Arc::new(Mutex::new(Vec::new())),
                manager_page_index: Arc::new(Mutex::new(0)),
                profile_responses: HashMap::new(),
            }
        }
    }

    impl CoordinatorCapture for TestCapture {
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
            let op_str = op.to_string();
            let p = params.clone();
            calls.lock().unwrap().push((op_str.clone(), p));

            if op == "contacts_enter" || op == "contacts_leave" {
                return Box::pin(async { Ok(json!({"ok": true})) });
            }
            if op == "contacts_headers" {
                let lines = self.headers_lines.clone();
                return Box::pin(async move { Ok(json!({"lines": lines})) });
            }
            if op == "contacts_manager_open" {
                return Box::pin(async {
                    Ok(json!({
                        "window": {"x": 200, "y": 100, "w": 720, "h": 560}
                    }))
                });
            }
            if op == "contacts_manager_close" {
                return Box::pin(async { Ok(json!({"closed": true})) });
            }
            if op == "contacts_manager_page" || op == "contacts_manager_scroll" || op == "contacts_manager_filter" {
                let idx_arc = self.manager_page_index.clone();
                let pages_arc = self.manager_pages.clone();
                return Box::pin(async move {
                    let mut idx = idx_arc.lock().unwrap();
                    let pages = pages_arc.lock().unwrap();
                    if *idx >= pages.len() {
                        return Ok(json!({"lines": [], "window": {"x": 200, "y": 100, "w": 720, "h": 560}}));
                    }
                    let res = pages[*idx].clone();
                    *idx += 1;
                    match res {
                        Ok(lines) => Ok(json!({
                            "lines": lines,
                            "window": {"x": 200, "y": 100, "w": 720, "h": 560}
                        })),
                        Err(e) => bail!(e),
                    }
                });
            }
            if op == "contacts_open_row" {
                let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0).round() as i64;
                let prof = self.profile_responses.get(&y).cloned().unwrap_or_default();
                return Box::pin(async move {
                    Ok(json!({"lines": prof}))
                });
            }

            Box::pin(async move { Ok(json!({})) })
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

        let mut cap = TestCapture::new();
        // Mock headers: personal 3, work 1, official 2
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("3", 260.0, 100.0),
            line("企业微信联系人", 80.0, 300.0),
            line("1", 260.0, 300.0),
            line("公众号、服务号", 80.0, 400.0),
            line("2", 260.0, 400.0),
        ];
        // Profiles for each row y
        // Personal rows at y=150, 200, 250
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);
        cap.profile_responses.insert(200, vec![
            line("李四", 520.0, 60.0),
            line("微信号：wxid_p2", 520.0, 90.0),
        ]);
        cap.profile_responses.insert(250, vec![
            line("王五", 520.0, 60.0),
            line("微信号：wxid_p3", 520.0, 90.0),
        ]);
        // Work row at y=350
        cap.profile_responses.insert(350, vec![
            line("企微员工赵", 520.0, 60.0),
            line("企业某某公司", 520.0, 90.0),
            line("实名赵六", 520.0, 120.0),
        ]);
        // Official rows at y=450, 500
        cap.profile_responses.insert(450, vec![
            line("公众号一", 520.0, 60.0),
            line("微信号gh_off1", 520.0, 90.0),
        ]);
        cap.profile_responses.insert(500, vec![
            line("公众号二", 520.0, 60.0),
            line("微信号gh_off2", 520.0, 90.0),
        ]);

        let no_yield = || false;
        let report = run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        assert!(report.complete);
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

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("3", 260.0, 100.0),
        ];
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);
        cap.profile_responses.insert(200, vec![
            line("李四", 520.0, 60.0),
            line("微信号：wxid_p2", 520.0, 90.0),
        ]);
        cap.profile_responses.insert(250, vec![
            line("王五", 520.0, 60.0),
            line("微信号：wxid_p3", 520.0, 90.0),
        ]);

        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c_clone = counter.clone();
        // yield after 2 contacts
        let should_yield = move || {
            c_clone.fetch_add(1, Ordering::SeqCst) >= 2
        };

        let report1 = run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &should_yield, 1000).await.unwrap();
        assert!(report1.yielded);
        assert!(!report1.complete);
        assert!(checkpoint.exists());

        let no_yield = || false;
        let report2 = run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.unwrap();
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

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("2", 260.0, 100.0),
        ];
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);
        // Row 2 is unreadable (empty OCR lines or no nickname found)
        cap.profile_responses.insert(200, vec![]);

        let no_yield = || false;
        let report = run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(!report.complete);
        assert_eq!(report.unreadable_rows, 1);
        let res_file = state_dir.join("contacts-sync-result.json");
        assert!(res_file.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(res_file).unwrap()).unwrap();
        assert_eq!(val["complete"], false);
    }

    #[tokio::test]
    async fn test_manager_window_closed_midway_is_reopened() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("1", 260.0, 100.0),
        ];
        // First manager page call succeeds, second fails with contacts_manager_missing, third succeeds
        *cap.manager_pages.lock().unwrap() = vec![
            Ok(vec![line("张三", 300.0, 96.0)]),
            Err("invalid(\"contacts_manager_missing\")".to_string()),
            Ok(vec![]),
        ];
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);

        let no_yield = || false;
        let report = run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(report.complete);
        // Verify contacts_manager_open was called more than once
        let open_calls = cap.calls.lock().unwrap().iter().filter(|(op, _)| op == "contacts_manager_open").count();
        assert!(open_calls >= 2);
    }

    #[tokio::test]
    async fn test_incremental_only_opens_changed_rows() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        // Seed store with 3 contacts
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

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("4", 260.0, 100.0),
        ];
        // Manager table has 4 users: 用户1, 用户2, 用户3, 用户4
        *cap.manager_pages.lock().unwrap() = vec![
            Ok(vec![
                line("用户1", 300.0, 96.0),
                line("用户2", 300.0, 144.0),
                line("用户3", 300.0, 192.0),
                line("用户4", 300.0, 240.0),
            ]),
        ];
        // Profile for only the new user (row 4 -> y = 100 + 50 * 4 = 300)
        cap.profile_responses.insert(300, vec![
            line("用户4", 520.0, 60.0),
            line("微信号：wxid_p4", 520.0, 90.0),
        ]);

        let no_yield = || false;
        let report = run_sync(&mut cap, &store, "test_acct", SyncMode::Incremental, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(report.complete);
        let open_row_calls = cap.calls.lock().unwrap().iter().filter(|(op, _)| op == "contacts_open_row").count();
        assert_eq!(open_row_calls, 1);
    }

    #[tokio::test]
    async fn test_sync_never_requests_forbidden_click() {
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("1", 260.0, 100.0),
        ];
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);

        let no_yield = || false;
        run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();

        let calls = cap.calls.lock().unwrap();
        for (op, params) in calls.iter() {
            if op == "contacts_click_list" {
                let x = params["x"].as_f64().unwrap_or(0.0);
                let y = params["y"].as_f64().unwrap_or(0.0);
                // forbidden area: x in 420..840 and y >= 440
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

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("1", 260.0, 100.0),
        ];
        cap.profile_responses.insert(150, vec![
            line("张三", 520.0, 60.0),
            line("微信号：wxid_p1", 520.0, 90.0),
        ]);

        // Case A: pause_file did not exist initially -> removed after sync
        assert!(!pause_file.exists());
        let no_yield = || false;
        run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        assert!(!pause_file.exists());

        // Case B: pause_file existed initially -> remains after sync
        fs::write(&pause_file, b"{\"paused\":true}").unwrap();
        run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 2000).await.unwrap();
        assert!(pause_file.exists());
    }

    #[tokio::test]
    async fn test_sync_logs_counts_only() {
        // Collect tracing logs during sync and assert no synthetic name or wechat_id leaked
        let temp = TestTempDir::new();
        let state_dir = temp.path().join("state");
        let store = HistoryStore::open(&state_dir).unwrap();
        let checkpoint = state_dir.join("contacts-sync-checkpoint.json");

        let mut cap = TestCapture::new();
        cap.headers_lines = vec![
            line("联系人", 80.0, 100.0),
            line("1", 260.0, 100.0),
        ];
        let secret_name = "秘密联系人甲";
        let secret_wxid = "wxid_secret_999";
        cap.profile_responses.insert(150, vec![
            line(secret_name, 520.0, 60.0),
            line(&format!("微信号：{secret_wxid}"), 520.0, 90.0),
        ]);

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
        run_sync(&mut cap, &store, "test_acct", SyncMode::Full, &checkpoint, &no_yield, 1000).await.unwrap();
        drop(_guard);

        let logs = String::from_utf8(log_buffer.lock().unwrap().clone()).unwrap();
        assert!(!logs.contains(secret_name), "Logs leaked secret name: {}", logs);
        assert!(!logs.contains(secret_wxid), "Logs leaked secret wxid: {}", logs);
    }
}
