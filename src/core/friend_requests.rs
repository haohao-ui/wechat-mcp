//! Rule decisions and conservative, leased friend-request processing.
use super::{
    scheduler::CoordinatorCapture,
    state::{read_json, write_private},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    DryRun,
    Live,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    #[default]
    All,
    Any,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub mode: Mode,
    pub accept_sources: Vec<String>,
    pub accept_keywords: Vec<String>,
    pub r#match: Match,
    pub daily_limit: usize,
    pub check_interval_seconds: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::DryRun,
            // An empty list means "any source"; default to the agreed scan/search rule instead.
            accept_sources: vec!["扫一扫".to_string(), "搜索".to_string()],
            accept_keywords: vec![],
            r#match: Match::All,
            daily_limit: 20,
            check_interval_seconds: 120,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Accepted,
    DryRunMatched,
    NotMatched,
    LimitReached,
    NeedsReview,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub account_hash: String,
    #[serde(default)]
    pub rule_hash: String,
    pub identity: String,
    pub name_length: usize,
    pub day: String,
    pub time: f64,
    pub mode: Mode,
    pub decision: Decision,
    pub hits: Vec<String>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub account_hash: String,
    #[serde(default)]
    pub rule_hash: String,
    pub records: Vec<Record>,
    pub last_check: Option<f64>,
    pub last_scan: Option<f64>,
    pub pending: bool,
}
pub fn identity(account: &str, name: &str, message: &str, source: &str) -> String {
    let bytes =
        serde_json::to_vec(&(account, name, message, source)).expect("string tuple serializes");
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Classifies sources using stable anchors even when OCR misreads surrounding text.
pub fn source_category(source: &str) -> Option<&'static str> {
    let source: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    if ["手机号", "微信号", "QQ号"]
        .iter()
        .any(|s| source.contains(s))
    {
        Some("搜索")
    } else if source.contains('扫') {
        Some("扫一扫")
    } else if source.contains('群') {
        Some("群聊")
    } else if source.contains("名片") {
        Some("名片分享")
    } else if source.contains("附近") {
        Some("附近的人")
    } else if source.contains('摇') {
        Some("摇一摇")
    } else {
        None
    }
}

fn rule_hash(c: &Config) -> Result<String> {
    Ok(identity(
        "rules",
        &serde_json::to_string(c)?,
        "source_category_v1",
        "",
    ))
}

pub fn matches_rules(c: &Config, source: &str, message: &str) -> (bool, Vec<String>) {
    let category = source_category(source);
    let source_hits: Vec<_> = c
        .accept_sources
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            source.contains(s.as_str()) || category.is_some_and(|v| v.contains(s.as_str()))
        })
        .map(|(i, _)| format!("source:{i}"))
        .collect();
    let keyword_hits: Vec<_> = c
        .accept_keywords
        .iter()
        .enumerate()
        .filter(|(_, s)| message.contains(s.as_str()))
        .map(|(i, _)| format!("keyword:{i}"))
        .collect();
    let a = c.accept_sources.is_empty() || !source_hits.is_empty();
    let b = c.accept_keywords.is_empty() || !keyword_hits.is_empty();
    (
        match c.r#match {
            Match::All => a && b,
            Match::Any => a || b,
        },
        source_hits.into_iter().chain(keyword_hits).collect(),
    )
}
pub fn should_check(c: &Config, paused: bool, idle: bool, now: f64, last: Option<f64>) -> bool {
    c.enabled
        && !paused
        && idle
        && last.is_none_or(|t| now - t >= c.check_interval_seconds.max(1) as f64)
}
impl State {
    pub fn bind_account(&mut self, account: &str) {
        let key = identity(account, "", "", "");
        if self.account_hash != key {
            self.account_hash = key;
            self.last_check = None;
            self.last_scan = None;
            self.pending = false;
        }
    }

    pub fn load(root: &Path) -> Result<Self> {
        let p = root.join("friend-requests.json");
        if p.exists() {
            read_json(p)
        } else {
            Ok(Self::default())
        }
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        write_private(root.join("friend-requests.json"), self)
    }
    pub fn decide(
        &self,
        c: &Config,
        id: &str,
        day: &str,
        source: &str,
        message: &str,
    ) -> Option<Decision> {
        if let Some(r) = self.records.iter().rev().find(|r| {
            r.identity == id
                && (r.decision == Decision::NeedsReview
                    || r.decision == Decision::Accepted
                    || (r.mode == c.mode && r.decision == Decision::DryRunMatched)
                    || (r.decision == Decision::NotMatched && r.rule_hash == self.rule_hash))
        }) {
            return Some(r.decision);
        }
        if !matches_rules(c, source, message).0 {
            return Some(Decision::NotMatched);
        }
        let count = self
            .records
            .iter()
            .filter(|r| {
                r.account_hash == self.account_hash
                    && r.day == day
                    && r.mode == c.mode
                    && matches!(r.decision, Decision::Accepted | Decision::DryRunMatched)
            })
            .count();
        if count >= c.daily_limit {
            Some(Decision::LimitReached)
        } else {
            None
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        id: &str,
        name_length: usize,
        day: &str,
        time: f64,
        mode: Mode,
        decision: Decision,
        hits: Vec<String>,
    ) {
        self.records.push(Record {
            account_hash: self.account_hash.clone(),
            rule_hash: self.rule_hash.clone(),
            identity: id.into(),
            name_length,
            day: day.into(),
            time,
            mode,
            decision,
            hits,
        });
    }
}
/// Uses the host's local timezone, including DST, rather than a UTC day bucket.
pub fn local_day(now: f64) -> Result<String> {
    let out = std::process::Command::new("date")
        .args(["-r", &format!("{:.0}", now.floor()), "+%Y-%m-%d"])
        .output()?;
    if !out.status.success() {
        bail!("local calendar unavailable")
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}
#[derive(Debug, Deserialize)]
struct Row {
    row_y: f64,
    name: String,
    status: String,
    message: String,
}
#[derive(Debug, Default)]
pub struct ScanReport {
    pub pending: bool,
    pub accepted: usize,
    pub events: Vec<Value>,
}
/// Every exit after acquiring the lease attempts both navigation cleanup and release.
pub async fn scan(
    capture: &mut dyn CoordinatorCapture,
    c: &Config,
    root: &Path,
    account: &str,
    now: f64,
    stop: &dyn Fn() -> bool,
) -> Result<ScanReport> {
    capture.call("contacts_lease_begin", json!({})).await?;
    let result = scan_inner(capture, c, root, account, now, stop).await;
    let leave = capture.call("contacts_leave", json!({})).await;
    let release = capture.call("contacts_lease_end", json!({})).await;
    leave?;
    release?;
    result
}
async fn scan_inner(
    capture: &mut dyn CoordinatorCapture,
    c: &Config,
    root: &Path,
    account: &str,
    now: f64,
    stop: &dyn Fn() -> bool,
) -> Result<ScanReport> {
    let mut state = State::load(root)?;
    state.bind_account(account);
    state.rule_hash = rule_hash(c)?;
    let day = local_day(now)?;
    let mut report = ScanReport::default();
    capture.call("contacts_enter", json!({})).await?;
    let value = capture.call("friend_request_scan", json!({})).await?;
    let rows: Vec<Row> =
        serde_json::from_value(value.get("rows").cloned().context("missing request rows")?)?;
    report.pending = rows.iter().any(|r| r.status == "等待验证");
    state.pending = report.pending;
    state.last_scan = Some(now);
    state.save(root)?;
    let mut processed = 0;
    for row in rows.into_iter().filter(|r| r.status == "等待验证") {
        if processed >= 5 {
            break;
        }
        if stop() {
            break;
        }
        let detail = match capture
            .call(
                "friend_request_detail",
                json!({"row_y":row.row_y,"name":row.name}),
            )
            .await
        {
            Ok(v) => v,
            Err(error) => {
                report
                    .events
                    .push(json!({"event":"friend_request_scan","reason":"detail_failed","error":format!("{error:#}")}));
                break;
            }
        };
        let source = detail["source"].as_str().unwrap_or("");
        let message = detail["message"].as_str().unwrap_or(&row.message);
        let id = identity(account, &row.name, message, source);
        if state.records.iter().any(|r| {
            r.identity == id
                && (matches!(r.decision, Decision::Accepted | Decision::NeedsReview)
                    || (r.mode == c.mode && r.decision == Decision::DryRunMatched)
                    || (r.decision == Decision::NotMatched && r.rule_hash == state.rule_hash))
        }) {
            continue;
        }
        processed += 1;
        let (_, hits) = matches_rules(c, source, message);
        let mut decision = state
            .decide(c, &id, &day, source, message)
            .unwrap_or(Decision::NeedsReview);
        if state.decide(c, &id, &day, source, message).is_none() {
            // Persist an ambiguous attempt before touching the unobserved dialog: a crash must never retry it.
            state.record(
                &id,
                row.name.chars().count(),
                &day,
                now,
                c.mode,
                Decision::NeedsReview,
                hits.clone(),
            );
            state.save(root)?;
            if !stop() && !source.is_empty() && !detail["verify_button"].is_null() {
                let v=capture.call("friend_request_verify",json!({"mode":c.mode,"row_y":row.row_y,"name":row.name,"diagnostics_dir":root.join("diagnostics/friend-requests").join(format!("{now:.0}-{}",uuid::Uuid::new_v4()))})).await;
                if let Err(error) = &v {
                    report.events.push(json!({"event":"friend_request_scan","reason":"verify_failed","error":format!("{error:#}")}));
                }
                if let Ok(v) = v {
                    if v["verified"] == true {
                        decision = if c.mode == Mode::Live {
                            Decision::Accepted
                        } else {
                            Decision::DryRunMatched
                        };
                    }
                }
            }
            state.records.pop();
        }
        if decision == Decision::Accepted {
            report.accepted += 1;
        }
        state.record(
            &id,
            row.name.chars().count(),
            &day,
            now,
            c.mode,
            decision,
            hits,
        );
        state.save(root)?;
        report.events.push(json!({"event":"friend_request_decision","decision":decision,"name_length":row.name.chars().count(),"message_length":message.chars().count(),"source":source,"source_category":source_category(source)}));
    }
    Ok(report)
}
/// Queues the same request consumed by control contacts-sync; never overwrites an operator request.
pub fn queue_contacts_sync(
    root: &Path,
    account: &str,
    now: f64,
    mode: Mode,
    accepted: usize,
) -> Result<bool> {
    if mode != Mode::Live || accepted == 0 {
        return Ok(false);
    }
    let p = root.join("contacts-sync-request.json");
    let request = json!({"id":uuid::Uuid::new_v4().to_string(),"account_id":account,"mode":"incremental","created_at":now,"origin":"friend_requests"});
    if p.exists() {
        write_private(root.join("friend-requests-sync-pending.json"), &request)?;
        return Ok(false);
    }
    write_private(p, &request)?;
    Ok(true)
}

/// Preserves a concurrent operator request, then promotes our incremental request when it drains.
pub fn promote_contacts_sync(root: &Path, account: &str, now: f64) -> Result<bool> {
    let pending = root.join("friend-requests-sync-pending.json");
    let request = root.join("contacts-sync-request.json");
    if request.exists() || !pending.exists() {
        return Ok(false);
    }
    let mut value: Value = read_json(&pending)?;
    if value["account_id"] != account {
        return Ok(false);
    }
    value["created_at"] = json!(now);
    write_private(request, &value)?;
    std::fs::remove_file(pending)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_categories() {
        for (source, expected) in [
            ("通过搜索微信号添加", Some("搜索")),
            ("通过扫一扫添加", Some("扫一扫")),
            ("通过群聊添加", Some("群聊")),
            ("通过名片分享添加", Some("名片分享")),
            ("通过附近添加", Some("附近的人")),
            ("通过摇一摇添加", Some("摇一摇")),
            ("通过 QQ 号添加", Some("搜索")),
            ("手 机\n号 扫群名片附近摇", Some("搜索")),
            ("扫群名片附近摇", Some("扫一扫")),
            ("群名片附近摇", Some("群聊")),
            ("名片附近摇", Some("名片分享")),
            ("附近摇", Some("附近的人")),
            ("乱码", None),
        ] {
            assert_eq!(source_category(source), expected, "{source}");
        }
    }

    #[test]
    fn rule_hash_changes_with_algorithm_version() {
        let c = Config::default();
        let old_hash = identity("rules", &serde_json::to_string(&c).unwrap(), "", "");
        let new_hash = rule_hash(&c).unwrap();
        assert_ne!(old_hash, new_hash);
        let mut state = State {
            rule_hash: old_hash,
            ..State::default()
        };
        let source = "通过搜察手机号添加";
        let id = identity("account", "name", "", source);
        state.record(
            &id,
            4,
            "2026-11-09",
            1.,
            c.mode,
            Decision::NotMatched,
            vec![],
        );
        assert_eq!(
            state.decide(&c, &id, "2026-11-09", source, ""),
            Some(Decision::NotMatched)
        );
        state.rule_hash = new_hash;
        assert_eq!(state.decide(&c, &id, "2026-11-09", source, ""), None);
        for decision in [Decision::NeedsReview, Decision::Accepted] {
            state.records[0].decision = decision;
            assert_eq!(
                state.decide(&c, &id, "2026-11-09", source, ""),
                Some(decision)
            );
        }
    }

    #[test]
    fn ocr_misread_search_source_matches() {
        assert!(matches_rules(&Config::default(), "通过搜察手机号添加", "").0);
    }

    #[test]
    fn group_source_still_not_matched_by_default() {
        assert!(!matches_rules(&Config::default(), "通过群聊添加", "").0);
    }

    #[test]
    fn default_rules_accept_only_scan_and_search() {
        let c = Config::default();
        assert!(matches_rules(&c, "通过搜索手机号添加", "").0);
        assert!(matches_rules(&c, "通过扫一扫添加", "").0);
        assert!(!matches_rules(&c, "通过群聊添加", "").0);
        assert!(!matches_rules(&c, "通过名片分享添加", "").0);
    }

    #[test]
    fn rules_sources_keywords_all_any_empty() {
        let mut c = Config {
            accept_sources: vec![],
            ..Config::default()
        };
        assert!(matches_rules(&c, "任何", "任意").0);
        c.accept_sources = vec!["搜索".into()];
        c.accept_keywords = vec!["合作".into()];
        assert!(matches_rules(&c, "通过搜索手机号添加", "希望合作").0);
        assert!(!matches_rules(&c, "扫一扫", "希望合作").0);
        c.r#match = Match::Any;
        assert!(matches_rules(&c, "扫一扫", "希望合作").0);
        assert!(!matches_rules(&c, "扫一扫", "你好").0);
        c.accept_keywords.clear();
        assert!(matches_rules(&c, "扫一扫", "你好").0);
    }
    #[test]
    fn limits_dedup_modes_and_days() {
        let mut s = State::default();
        let mut c = Config::default();
        c.daily_limit = 1;
        let a = identity("acct", "甲", "msg", "搜索");
        let b = identity("acct", "乙", "msg", "搜索");
        assert_ne!(a, identity("other", "甲", "msg", "搜索"));
        assert_eq!(s.decide(&c, &a, "2026-11-09", "搜索", "msg"), None);
        s.record(
            &a,
            1,
            "2026-11-09",
            1.,
            Mode::DryRun,
            Decision::DryRunMatched,
            vec![],
        );
        assert_eq!(
            s.decide(&c, &a, "2026-11-09", "搜索", "msg"),
            Some(Decision::DryRunMatched)
        );
        assert_eq!(
            s.decide(&c, &b, "2026-11-09", "搜索", "msg"),
            Some(Decision::LimitReached)
        );
        c.mode = Mode::Live;
        assert_eq!(s.decide(&c, &a, "2026-11-09", "搜索", "msg"), None);
        s.record(
            &a,
            1,
            "2026-11-09",
            2.,
            Mode::Live,
            Decision::NeedsReview,
            vec![],
        );
        assert_eq!(
            s.decide(&c, &a, "2026-11-10", "搜索", "msg"),
            Some(Decision::NeedsReview)
        );
        assert_eq!(s.decide(&c, &b, "2026-11-10", "搜索", "msg"), None);
    }
    #[test]
    fn disabled_and_paused_do_not_trigger() {
        let mut c = Config::default();
        assert!(!should_check(&c, false, true, 1000., None));
        c.enabled = true;
        assert!(!should_check(&c, true, true, 1000., None));
        assert!(!should_check(&c, false, false, 1000., None));
        assert!(should_check(&c, false, true, 1000., None));
        assert!(!should_check(&c, false, true, 1000., Some(999.)));
    }
    #[test]
    fn live_quota_resets_on_local_day_and_is_account_scoped() {
        let mut state = State::default();
        let mut c = Config {
            accept_sources: vec![],
            ..Config::default()
        };
        c.mode = Mode::Live;
        c.daily_limit = 1;
        state.bind_account("a");
        let id = identity("a", "one", "m", "s");
        state.record(
            &id,
            3,
            "2026-11-09",
            1.0,
            Mode::Live,
            Decision::Accepted,
            vec![],
        );
        let next = identity("a", "two", "m", "s");
        assert_eq!(
            state.decide(&c, &next, "2026-11-09", "s", "m"),
            Some(Decision::LimitReached)
        );
        assert_eq!(state.decide(&c, &next, "2026-11-10", "s", "m"), None);
        state.bind_account("b");
        assert_eq!(
            state.decide(&c, &identity("b", "one", "m", "s"), "2026-11-09", "s", "m"),
            None
        );
    }
}
