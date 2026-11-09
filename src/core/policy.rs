use crate::core::model::{Bubble, Frame};
use crate::core::state::write_private;
use anyhow::{Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

pub const PHASH_TOLERANCE: u32 = 12;
pub const FINGERPRINT_VERSION: u32 = 2;
pub const SCROLL_STEP_POINTS: i64 = 90;

pub type Fingerprint = (String, String, Option<String>);

pub fn fingerprint(bubble: &Bubble) -> Fingerprint {
    (
        bubble.direction.clone(),
        bubble.digest.clone(),
        bubble.phash.clone(),
    )
}

pub fn phash_distance(a: &str, b: &str) -> Result<u32> {
    let parse_hex = |s: &str| -> Result<u128> {
        let trimmed = s.trim();
        let hex_str = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed);
        if hex_str.is_empty() {
            bail!("empty hex string in phash: {}", s);
        }
        u128::from_str_radix(hex_str, 16)
            .map_err(|e| anyhow::anyhow!("invalid hex string in phash '{}': {}", s, e))
    };
    let va = parse_hex(a)?;
    let vb = parse_hex(b)?;
    Ok((va ^ vb).count_ones())
}

pub fn same_bubble(old: &Bubble, new: &Bubble) -> bool {
    if old.direction != new.direction || old.digest != new.digest {
        return false;
    }
    match (&old.phash, &new.phash) {
        (Some(pa), Some(pb)) => match phash_distance(pa, pb) {
            Ok(dist) => dist <= PHASH_TOLERANCE,
            Err(_) => false,
        },
        (None, None) => true,
        _ => false,
    }
}

pub fn certain_tail(old: &[Fingerprint], new: &[Fingerprint], current: &[Bubble]) -> Vec<Bubble> {
    let min_len = old.len().min(new.len());
    let mut max_end = None;
    for k in 1..=min_len {
        let old_suffix = &old[old.len() - k..];
        for i in 0..=(new.len() - k) {
            if &new[i..i + k] == old_suffix {
                let candidate = i + k;
                max_end = Some(max_end.map_or(candidate, |m: usize| m.max(candidate)));
            }
        }
    }
    let end = max_end.unwrap_or(new.len());
    if end >= current.len() {
        Vec::new()
    } else {
        current[end..].to_vec()
    }
}

fn time_label(bubble: &Bubble) -> Option<&str> {
    bubble.extra.get("time_label").and_then(Value::as_str)
}

/// Minutes since midnight for a plain "HH:MM" separator; prefixed labels ("昨天 21:30") return None.
fn clock_minutes(label: &str) -> Option<u32> {
    let (h, m) = label.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && label.len() <= 5).then_some(h * 60 + m)
}

/// Identical texts are told apart by the time separator WeChat draws above them. When the newest
/// separator in `current` was never seen in `previous` (and is later than every plain time there),
/// the bubbles below it arrived after the last capture; only the newest one is returned so a burst
/// of identical messages gets one reply. Empty when `previous` carries no labels (cannot compare).
pub fn separator_tail(previous: &[Bubble], current: &[Bubble]) -> Vec<Bubble> {
    let seen: Vec<&str> = previous.iter().filter_map(time_label).collect();
    if seen.is_empty() {
        return Vec::new();
    }
    let Some(index) = current.iter().rposition(|b| time_label(b).is_some()) else {
        return Vec::new();
    };
    let label = time_label(&current[index]).unwrap_or_default();
    if seen.contains(&label) {
        return Vec::new();
    }
    let latest = seen.iter().filter_map(|l| clock_minutes(l)).max();
    if let (Some(latest), Some(now)) = (latest, clock_minutes(label)) {
        if now <= latest {
            return Vec::new();
        }
    }
    current.last().into_iter().cloned().collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContinuityError {
    ContinuityGap,
    AmbiguousAnchor { certain: Vec<Bubble> },
}

impl std::fmt::Display for ContinuityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContinuityGap => write!(f, "visual continuity gap or ambiguous repeated anchor"),
            Self::AmbiguousAnchor { .. } => write!(f, "ambiguous repeated anchor"),
        }
    }
}

impl std::error::Error for ContinuityError {}

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyError {
    Continuity(ContinuityError),
    InvalidFrame(String),
    ValidationError(String),
}

impl From<ContinuityError> for PolicyError {
    fn from(err: ContinuityError) -> Self {
        Self::Continuity(err)
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continuity(e) => write!(f, "{}", e),
            Self::InvalidFrame(e) => write!(f, "{}", e),
            Self::ValidationError(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for PolicyError {}

pub fn new_bubbles(
    previous: &[Bubble],
    current: &[Bubble],
) -> std::result::Result<Vec<Bubble>, ContinuityError> {
    let old: Vec<Fingerprint> = previous.iter().map(fingerprint).collect();
    let new: Vec<Fingerprint> = current.iter().map(fingerprint).collect();
    if old == new {
        return Ok(Vec::new());
    }
    if old.is_empty() {
        return Ok(current.to_vec());
    }
    let min_len = old.len().min(new.len());
    let mut matches = Vec::new();
    for k in 1..=min_len {
        if old[old.len() - k..] == new[..k] {
            matches.push(k);
        }
    }
    if matches.is_empty() {
        return Err(ContinuityError::ContinuityGap);
    }
    let size = *matches.iter().max().unwrap();
    let anchor = &old[old.len() - size..];

    let mut periodic = false;
    for period in 1..size {
        if size % period == 0 {
            let is_rep = (0..size).all(|idx| anchor[idx] == anchor[idx % period]);
            if is_rep {
                periodic = true;
                break;
            }
        }
    }

    let unique_count = anchor.iter().collect::<HashSet<_>>().len();
    if matches.len() > 1 && (unique_count < 2 || periodic) {
        return Err(ContinuityError::AmbiguousAnchor {
            certain: certain_tail(&old, &new, current),
        });
    }

    let mut count = 0;
    for i in 0..=(new.len() - size) {
        if &new[i..i + size] == anchor {
            count += 1;
        }
    }
    if count != 1 {
        return Err(ContinuityError::AmbiguousAnchor {
            certain: certain_tail(&old, &new, current),
        });
    }

    Ok(current[size..].to_vec())
}

pub fn reconcile(previous: &[Bubble], current: &[Bubble]) -> Vec<Bubble> {
    let mut result = current.to_vec();
    let min_len = previous.len().min(current.len());
    for size in (1..=min_len).rev() {
        let prev_window = &previous[previous.len() - size..];
        let curr_window = &result[..size];
        let all_same = prev_window
            .iter()
            .zip(curr_window.iter())
            .all(|(o, n)| same_bubble(o, n));
        if !all_same {
            continue;
        }
        let all_images = curr_window
            .iter()
            .all(|n| n.kind.as_deref() == Some("image"));
        if all_images {
            continue;
        }
        for (o, n) in prev_window.iter().zip(result[..size].iter_mut()) {
            if n.phash.is_some() {
                n.phash = o.phash.clone();
            }
        }
        break;
    }
    result
}

pub fn relocate(items: &[Bubble], current: &[Bubble]) -> Vec<Bubble> {
    let mut used = HashSet::new();
    let mut result = Vec::new();
    for item in items {
        let mut matched = None;
        if matches!(item.kind.as_deref(), Some("image") | Some("file")) {
            let mut candidates = Vec::new();
            for (idx, b) in current.iter().enumerate() {
                if !used.contains(&idx) && b.kind == item.kind && same_bubble(item, b) {
                    let dist = match (&item.phash, &b.phash) {
                        (Some(ip), Some(bp)) => phash_distance(ip, bp).unwrap_or(0),
                        _ => 0,
                    };
                    candidates.push((idx, b, dist));
                }
            }
            if !candidates.is_empty() {
                candidates.sort_by_key(|c| c.2);
                let best = candidates[0];
                matched = Some((best.0, best.1));
            }
        }
        match matched {
            None => result.push(item.clone()),
            Some((idx, b)) => {
                used.insert(idx);
                let mut updated = b.clone();
                if item.phash.is_some() {
                    updated.phash = item.phash.clone();
                }
                result.push(updated);
            }
        }
    }
    result
}

pub fn merge_pages(
    pages: &[Vec<Bubble>],
    step: Option<i64>,
) -> std::result::Result<Vec<Bubble>, ContinuityError> {
    let step = step.unwrap_or(SCROLL_STEP_POINTS);
    let mut placed: Vec<(i64, Bubble)> = Vec::new();
    let mut offset: i64 = 0;

    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            let prev_page = &pages[index - 1];
            let mut deltas = Vec::new();
            for old in prev_page {
                for new in page {
                    if old.kind == new.kind && same_bubble(old, new) {
                        if let (Some(oy), Some(ny)) = (old.y, new.y) {
                            let diff = ny - oy;
                            if diff > 0 && diff < 300 {
                                deltas.push(diff);
                            }
                        }
                    }
                }
            }
            deltas.sort();
            let shift = if !deltas.is_empty() {
                deltas[deltas.len() / 2]
            } else {
                step
            };
            offset += shift;
        }

        for bubble in page {
            let pos = bubble.y.unwrap_or(0) - offset;
            let twin = placed
                .iter()
                .find(|(p, b)| b.direction == bubble.direction && (p - pos).abs() <= 8);
            match twin {
                None => placed.push((pos, bubble.clone())),
                Some((_, twin_b)) => {
                    if !same_bubble(twin_b, bubble) {
                        return Err(ContinuityError::ContinuityGap);
                    }
                }
            }
        }
    }

    placed.sort_by_key(|(pos, _)| *pos);
    Ok(placed.into_iter().map(|(_, b)| b).collect())
}

pub fn stitch(
    cursor: &[Bubble],
    pages: &[Vec<Bubble>],
) -> std::result::Result<Vec<Bubble>, ContinuityError> {
    for (index, page) in pages.iter().enumerate() {
        let mut changes = match new_bubbles(cursor, page) {
            Ok(c) => c,
            Err(ContinuityError::ContinuityGap) => continue,
            Err(e) => return Err(e),
        };

        for i in index..pages.len().saturating_sub(1) {
            let more = new_bubbles(&pages[i], &pages[i + 1])?;
            changes.extend(more);
        }
        return Ok(changes);
    }
    Err(ContinuityError::ContinuityGap)
}

pub fn strip_member_count(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    if chars.is_empty() {
        return title.to_string();
    }
    let mut end = chars.len();
    while end > 0 && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return title.to_string();
    }
    let last = chars[end - 1];
    if last != ')' && last != '）' {
        return title.to_string();
    }
    let mut i = end - 1;
    let mut has_digit = false;
    while i > 0 && chars[i - 1].is_ascii_digit() {
        has_digit = true;
        i -= 1;
    }
    if !has_digit || i == 0 {
        return title.to_string();
    }
    let open = chars[i - 1];
    let matched = (last == ')' && open == '(')
        || (last == '）' && open == '（')
        || open == '('
        || open == '（';
    if !matched {
        return title.to_string();
    }
    let mut cut = i - 1;
    while cut > 0 && chars[cut - 1].is_whitespace() {
        cut -= 1;
    }
    chars[..cut].iter().collect()
}

pub fn is_invisible(c: char) -> bool {
    (c >= '\u{200B}' && c <= '\u{200F}') || c == '\u{2060}' || c == '\u{FEFF}'
}

pub fn clean_title(raw: &str) -> String {
    raw.chars()
        .filter(|&c| !is_invisible(c))
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn validate_reply(value: &str) -> std::result::Result<String, PolicyError> {
    let trimmed = value.trim();
    let char_count = value.chars().count();
    if trimmed.is_empty() || char_count > 200 {
        return Err(PolicyError::ValidationError(
            "model reply must be 1..200 characters".to_string(),
        ));
    }
    if value.chars().any(|c| (c as u32) < 32 && c != '\n') {
        return Err(PolicyError::ValidationError(
            "reply contains control characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

/// Is `observed` (OCR of a bubble) our message `expected`? NFKC, whitespace ignored.
/// Replies longer than 10 characters tolerate OCR misreads: up to max(1, n/20) edits and length difference
/// (device 2026-11-08: unrecognised echoes were the right bubble with 1–2 misread characters). Short replies
/// stay exact because one character can change their meaning.
pub fn echo_matches(observed: &str, expected: &str) -> bool {
    let compact = |s: &str| -> Vec<char> { s.nfkc().filter(|c| !c.is_whitespace()).collect() };
    let (seen, want) = (compact(observed), compact(expected));
    if seen == want {
        return true;
    }
    let n = want.len();
    if n <= 10 {
        return false;
    }
    let allowed = std::cmp::max(1, n / 20);
    if seen.len().abs_diff(n) > allowed {
        return false;
    }
    edit_distance_within(&seen, &want, allowed)
}

/// Levenshtein distance between `a` and `b` is at most `limit` (early exit once every cell exceeds it).
fn edit_distance_within(a: &[char], b: &[char], limit: usize) -> bool {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(ca != cb));
        }
        if current.iter().all(|&d| d > limit) {
            return false;
        }
        previous = current;
    }
    previous[b.len()] <= limit
}

/// String escaping of Python `json.dumps(..., ensure_ascii=False)`: quote, backslash, the short forms
/// \n \r \t \b \f, other C0 controls as lowercase \u00xx; everything else (DEL, U+2028, non-ASCII) raw.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn format_fp(fp: &Fingerprint) -> String {
    match &fp.2 {
        Some(p) => format!(
            "[\"{}\", \"{}\", \"{}\"]",
            escape_json(&fp.0),
            escape_json(&fp.1),
            escape_json(p)
        ),
        None => format!(
            "[\"{}\", \"{}\", null]",
            escape_json(&fp.0),
            escape_json(&fp.1)
        ),
    }
}

fn format_identity_json(
    account: &str,
    recipient: &str,
    baseline: &[Fingerprint],
    candidate: &Fingerprint,
) -> String {
    let mut baseline_strs = Vec::new();
    for b in baseline {
        baseline_strs.push(format_fp(b));
    }
    let baseline_part = format!("[{}]", baseline_strs.join(", "));
    let candidate_part = format_fp(candidate);
    format!(
        "[\"{}\", \"{}\", {}, {}]",
        escape_json(account),
        escape_json(recipient),
        baseline_part,
        candidate_part
    )
}

pub struct Policy {
    pub account: String,
    pub recipient: String,
    pub trigger: String,
    pub accepted: HashSet<String>,
    pub group: bool,
}

impl Policy {
    pub fn new(
        account: impl Into<String>,
        recipient: impl Into<String>,
        trigger: impl Into<String>,
        names: Option<Vec<String>>,
        group: bool,
    ) -> std::result::Result<Self, PolicyError> {
        let account = account.into();
        let recipient = recipient.into();
        let trigger = trigger.into();
        if account.is_empty() || recipient.is_empty() || trigger.is_empty() {
            return Err(PolicyError::ValidationError(
                "explicit account, exact private-chat allowlist and test trigger required"
                    .to_string(),
            ));
        }
        let raw_names = names.unwrap_or_else(|| vec![recipient.clone()]);
        let accepted = raw_names.into_iter().map(|n| clean_title(&n)).collect();
        Ok(Self {
            account,
            recipient,
            trigger,
            accepted,
            group,
        })
    }

    pub fn check_frame(&self, frame: &Frame) -> std::result::Result<(), PolicyError> {
        let profile = frame.profile.as_deref().unwrap_or("");
        let valid_profile = profile == "wechat-4-dark" || profile == "wechat-4-light";
        let valid_scale = match &frame.scale {
            Some(serde_json::Value::Number(n)) => n.as_i64() == Some(1) || n.as_i64() == Some(2),
            _ => false,
        };
        if !valid_profile || !valid_scale {
            return Err(PolicyError::InvalidFrame(
                "uncalibrated visual profile".to_string(),
            ));
        }
        if frame.clipped_bubble == Some(true) {
            return Err(PolicyError::InvalidFrame(
                "incomplete bottom bubble; no automatic reply".to_string(),
            ));
        }
        let titles: Vec<String> = frame
            .title_lines
            .iter()
            .map(|x| {
                let cleaned = clean_title(x);
                if self.group {
                    strip_member_count(&cleaned)
                } else {
                    cleaned
                }
            })
            .collect();

        if titles.len() != 1 || !self.accepted.contains(&titles[0]) {
            return Err(PolicyError::InvalidFrame(
                "exact allowlisted title not verified".to_string(),
            ));
        }
        Ok(())
    }

    pub fn candidate(
        &self,
        previous: &Frame,
        current: &Frame,
        confirmed: &Frame,
    ) -> std::result::Result<Option<Bubble>, PolicyError> {
        self.check_frame(current)?;
        self.check_frame(confirmed)?;
        let changes = new_bubbles(&previous.bubbles, &current.bubbles)?;
        let curr_fps: Vec<_> = current.bubbles.iter().map(fingerprint).collect();
        let conf_fps: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
        if curr_fps != conf_fps {
            return Err(PolicyError::ValidationError(
                "chat changed during confirmation".to_string(),
            ));
        }
        let incoming: Vec<&Bubble> = changes
            .iter()
            .filter(|b| b.direction == "inbound_visual")
            .collect();
        if incoming.len() != 1 || changes.len() != 1 {
            return Ok(None);
        }
        let message = incoming[0];
        let message_compact: String = message.text.split_whitespace().collect();
        let trigger_compact: String = self.trigger.split_whitespace().collect();
        if !message_compact.starts_with(&trigger_compact) {
            return Ok(None);
        }
        Ok(Some(message.clone()))
    }

    pub fn key(&self, baseline: &Frame, candidate: &Bubble) -> String {
        let baseline_fps: Vec<Fingerprint> = baseline.bubbles.iter().map(fingerprint).collect();
        let cand_fp = fingerprint(candidate);
        let identity_json =
            format_identity_json(&self.account, &self.recipient, &baseline_fps, &cand_fp);
        let mut hasher = Sha256::new();
        hasher.update(identity_json.as_bytes());
        let hash: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        format!("bot-{}", hash)
    }
}

pub trait CaptureFrames {
    fn next_frame(&mut self) -> impl std::future::Future<Output = Result<Frame>> + Send;
}

pub trait PolicyToolClient {
    fn call_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>> + Send;
}

pub async fn run_bounded_bot<C, T, G, Fut>(
    root: &std::path::Path,
    policy: &Policy,
    client: &mut T,
    capture: &mut C,
    generate: G,
    duration: std::time::Duration,
) -> Result<()>
where
    C: CaptureFrames,
    T: PolicyToolClient,
    G: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let baseline = capture.next_frame().await?;
    policy
        .check_frame(&baseline)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    std::fs::create_dir_all(root)?;
    write_private(&root.join("baseline.json"), &baseline)?;

    let deadline = std::time::Instant::now() + duration;
    let mut current_baseline = baseline;

    while std::time::Instant::now() < deadline {
        let frame = capture.next_frame().await?;
        policy
            .check_frame(&frame)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let fp_base: Vec<_> = current_baseline.bubbles.iter().map(fingerprint).collect();
        let fp_cur: Vec<_> = frame.bubbles.iter().map(fingerprint).collect();
        if fp_base == fp_cur {
            continue;
        }

        let confirmed = capture.next_frame().await?;
        let candidate = policy
            .candidate(&current_baseline, &frame, &confirmed)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let Some(candidate) = candidate else {
            current_baseline = confirmed;
            continue;
        };

        let key = policy.key(&current_baseline, &candidate);
        let journal = root.join(format!("{key}.json"));
        if journal.exists() {
            return Ok(());
        }

        let mut record = serde_json::json!({
            "status": "generating",
            "account_id": policy.account,
            "recipient": policy.recipient,
            "input": candidate,
            "idempotency_key": key,
            "direction_evidence": "visual_candidate",
            "platform_message_verified": false,
        });
        write_private(&journal, &record)?;

        let generated_text = generate(candidate.text.clone()).await?;
        let reply = validate_reply(&generated_text).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        record["status"] = serde_json::json!("generated");
        record["reply"] = serde_json::json!(reply);
        write_private(&journal, &record)?;

        let latest = capture.next_frame().await?;
        policy
            .check_frame(&latest)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let fp_latest: Vec<_> = latest.bubbles.iter().map(fingerprint).collect();
        let fp_conf: Vec<_> = confirmed.bubbles.iter().map(fingerprint).collect();
        if fp_latest != fp_conf {
            record["status"] = serde_json::json!("chat_changed_before_send");
            write_private(&journal, &record)?;
            bail!("chat changed before sending; reply retained, no send");
        }

        record["status"] = serde_json::json!("attempting");
        write_private(&journal, &record)?;

        let result = client
            .call_tool(
                "send_message",
                serde_json::json!({
                    "recipient": policy.recipient,
                    "message": reply,
                    "idempotency_key": key,
                }),
            )
            .await?;

        let durable = client
            .call_tool(
                "get_send_status",
                serde_json::json!({
                    "idempotency_key": key,
                }),
            )
            .await?;

        record["status"] = serde_json::json!("send_returned");
        record["send_result"] = result;
        record["durable_status"] = durable;
        write_private(&journal, &record)?;

        let after = capture.next_frame().await?;
        policy
            .check_frame(&after)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let echoed = after
            .bubbles
            .iter()
            .any(|x| x.direction == "outbound_visual" && echo_matches(&x.text, &reply));

        record["client_echo_observed"] = serde_json::json!(echoed);
        record["delivery"] = serde_json::json!("unknown");
        write_private(&journal, &record)?;

        return Ok(());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::time::Duration;

    fn make_bubble(text: &str, direction: Option<&str>, digest: Option<&str>) -> Bubble {
        Bubble {
            text: text.to_string(),
            digest: digest.unwrap_or(text).to_string(),
            direction: direction.unwrap_or("inbound_visual").to_string(),
            kind: None,
            media_id: None,
            phash: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        }
    }

    fn make_image(phash: &str, digest: Option<&str>, media_id: Option<&str>) -> Bubble {
        let digest = digest.unwrap_or("image:50:40").to_string();
        let mid = media_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| phash.repeat(4));
        Bubble {
            text: "[图片]".to_string(),
            digest,
            direction: "inbound_visual".to_string(),
            kind: Some("image".to_string()),
            media_id: Some(mid),
            phash: Some(phash.to_string()),
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        }
    }

    fn at_y(mut b: Bubble, y: i64, height: Option<i64>) -> Bubble {
        b.y = Some(y);
        b.height = Some(height.unwrap_or(40));
        b
    }

    fn make_frame(bubbles: Vec<Bubble>, title: Option<&str>) -> Frame {
        Frame {
            profile: Some("wechat-4-dark".to_string()),
            scale: Some(json!(1)),
            title_lines: vec![title.unwrap_or("示例联系人甲").to_string()],
            bubbles,
            platform_message_verified: None,
            sender_verified: None,
            clipped_bubble: None,
            width: None,
            height: None,
            captured_width: None,
            snapshot_id: None,
            window_id: None,
            observed_title: None,
            captured_at_ms: None,
            capture_seconds: None,
            ocr_cached: None,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn policy_test_exact_allowlist_and_private_title() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        for title in ["其他人", "示例联系人甲群", "示例联系人甲 (3)", "示例联系人甲2"] {
            let res = policy.check_frame(&make_frame(vec![], Some(title)));
            assert!(
                matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("allowlisted title"))
            );
        }
    }

    #[test]
    fn policy_test_only_a_new_stable_incoming_test_message_triggers() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let message = make_bubble("Bot验证1104 你好", None, None);
        let current = make_frame(vec![make_bubble("old", None, None), message.clone()], None);
        let candidate = policy.candidate(&baseline, &current, &current).unwrap();
        assert_eq!(candidate, Some(message));
        let none_cand = policy.candidate(&current, &current, &current).unwrap();
        assert_eq!(none_cand, None);
    }

    #[test]
    fn policy_test_own_echo_unknown_direction_and_unrequested_messages_do_not_trigger() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        for msg in [
            make_bubble("Bot验证1104", Some("outbound_visual"), None),
            make_bubble("Bot验证1104", Some("unknown"), None),
            make_bubble("你好", None, None),
        ] {
            let current = make_frame(vec![make_bubble("old", None, None), msg], None);
            let res = policy.candidate(&baseline, &current, &current).unwrap();
            assert_eq!(res, None);
        }
    }

    #[test]
    fn policy_test_two_frames_must_agree() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let current = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104", None, None),
            ],
            None,
        );
        let changed = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("other", None, None),
            ],
            None,
        );
        let err = policy.candidate(&baseline, &current, &changed).unwrap_err();
        assert!(
            matches!(err, PolicyError::ValidationError(ref msg) if msg.contains("changed during confirmation"))
        );
    }

    #[test]
    fn policy_test_gap_and_repeated_anchor_stop() {
        let err1 = new_bubbles(
            &[make_bubble("old", None, None)],
            &[make_bubble("new", None, None)],
        )
        .unwrap_err();
        assert_eq!(err1, ContinuityError::ContinuityGap);

        let same = make_bubble("same", None, None);
        let err2 = new_bubbles(
            &[same.clone(), same.clone()],
            &[same.clone(), same.clone(), make_bubble("extra", None, None)],
        )
        .unwrap_err();
        assert!(matches!(err2, ContinuityError::AmbiguousAnchor { .. }));
    }

    #[test]
    fn policy_test_repeated_short_greeting_does_not_override_long_unique_context() {
        let old: Vec<Bubble> = ["hi", "hi", "there", "question", "received", "hi"]
            .iter()
            .map(|x| make_bubble(x, None, None))
            .collect();
        let mut new1 = old[1..].to_vec();
        new1.push(make_bubble("reply", Some("outbound_visual"), None));
        assert_eq!(new_bubbles(&old, &new1).unwrap(), &new1[new1.len() - 1..]);

        let mut new2 = old[2..].to_vec();
        new2.push(make_bubble("reply", Some("outbound_visual"), None));
        new2.push(make_bubble("next question", None, None));
        assert_eq!(new_bubbles(&old, &new2).unwrap(), &new2[new2.len() - 2..]);
    }

    #[test]
    fn policy_test_ambiguous_anchor_exposes_only_certainly_new_tail() {
        let same = make_bubble("same", None, None);
        let err1 = new_bubbles(&[same.clone()], &[same.clone(), same.clone()]).unwrap_err();
        match err1 {
            ContinuityError::AmbiguousAnchor { certain } => {
                assert_eq!(certain, Vec::<Bubble>::new())
            }
            _ => panic!("expected AmbiguousAnchor"),
        }

        let err2 = new_bubbles(
            &[same.clone(), same.clone()],
            &[same.clone(), same.clone(), make_bubble("next", None, None)],
        )
        .unwrap_err();
        match err2 {
            ContinuityError::AmbiguousAnchor { certain } => {
                assert_eq!(certain, vec![make_bubble("next", None, None)])
            }
            _ => panic!("expected AmbiguousAnchor"),
        }
    }

    #[test]
    fn policy_test_periodic_repeated_context_is_still_ambiguous() {
        let old: Vec<Bubble> = ["a", "b", "a", "b"]
            .iter()
            .map(|x| make_bubble(x, None, None))
            .collect();
        let mut new1 = old.clone();
        new1.push(make_bubble("next", None, None));
        let err = new_bubbles(&old, &new1).unwrap_err();
        assert!(matches!(err, ContinuityError::AmbiguousAnchor { .. }));
    }

    #[test]
    fn policy_test_multiple_new_messages_are_not_replied_to_as_one() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let current = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104", None, None),
                make_bubble("second", None, None),
            ],
            None,
        );
        let res = policy.candidate(&baseline, &current, &current).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn policy_test_same_body_different_occurrences_remain_distinct() {
        let old = vec![
            make_bubble("anchor", None, None),
            make_bubble("same", None, None),
        ];
        let mut current = old.clone();
        current.push(make_bubble("same", None, None));
        assert_eq!(
            new_bubbles(&old, &current).unwrap(),
            vec![make_bubble("same", None, None)]
        );
    }

    #[test]
    fn policy_test_key_scoped_to_account_recipient_and_input() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let message = make_bubble("Bot验证1104", None, None);
        let key = policy.key(&baseline, &message);
        assert_eq!(key, policy.key(&baseline.clone(), &message.clone()));
        assert_ne!(
            key,
            Policy::new("other", "示例联系人甲", "Bot验证1104", None, false)
                .unwrap()
                .key(&baseline, &message)
        );
        assert_ne!(
            key,
            Policy::new("fixture", "其他人", "Bot验证1104", None, false)
                .unwrap()
                .key(&baseline, &message)
        );
        assert!(key.starts_with("bot-"));
        assert_eq!(key.len(), 4 + 64);
    }

    #[test]
    fn policy_test_policy_accepts_alias_titles_only() {
        let policy = Policy::new(
            "fixture",
            "🍓示例联系人甲",
            "service",
            Some(vec!["🍓示例联系人甲".to_string(), "示例联系人甲".to_string()]),
            false,
        )
        .unwrap();
        assert!(
            policy
                .check_frame(&make_frame(
                    vec![make_bubble("x", None, None)],
                    Some("示例联系人甲")
                ))
                .is_ok()
        );
        assert!(
            policy
                .check_frame(&make_frame(
                    vec![make_bubble("x", None, None)],
                    Some("\u{200b}🍓示例联系人甲 ")
                ))
                .is_ok()
        );
        let res = policy.check_frame(&make_frame(
            vec![make_bubble("x", None, None)],
            Some("别人"),
        ));
        assert!(
            matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("allowlisted title"))
        );
    }

    #[test]
    fn policy_test_unknown_theme_rejected() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let cases = [
            (Some("light"), Some(json!(1))),
            (Some("wechat-4-dark-1x"), Some(json!(1))),
            (Some("wechat-4-dark"), Some(json!(3))),
            (Some("wechat-4-light"), None),
        ];
        for (profile, scale) in cases {
            let mut f = make_frame(vec![], None);
            f.profile = profile.map(|s| s.to_string());
            f.scale = scale;
            let res = policy.check_frame(&f);
            assert!(
                matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("uncalibrated"))
            );
        }
    }

    #[test]
    fn policy_test_calibrated_themes_and_scales_accepted() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        for profile in ["wechat-4-dark", "wechat-4-light"] {
            for scale in [1, 2] {
                let mut ok = make_frame(vec![make_bubble("x", None, None)], None);
                ok.profile = Some(profile.to_string());
                ok.scale = Some(json!(scale));
                assert!(policy.check_frame(&ok).is_ok());
            }
        }
    }

    #[test]
    fn policy_test_incomplete_latest_bubble_blocks_auto_reply() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let mut partial = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104", None, None),
            ],
            None,
        );
        partial.clipped_bubble = Some(true);
        let res = policy.candidate(&baseline, &partial, &partial);
        assert!(
            matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("incomplete bottom bubble"))
        );
    }

    #[test]
    fn policy_test_reply_bounds() {
        assert_eq!(
            validate_reply("你好，这是自动回复测试。").unwrap(),
            "你好，这是自动回复测试。"
        );
        for body in ["", "  ", &"x".repeat(201), "你好\x00"] {
            assert!(validate_reply(body).is_err());
        }
    }

    #[test]
    fn policy_test_loading_image_reconciles_to_same_fingerprint() {
        let prev = vec![
            make_bubble("hi", None, None),
            make_image("0f0f0f0f0f0f0f0f", None, None),
        ];
        let cur = vec![
            make_bubble("hi", None, None),
            make_image("0f0f0f0f0f0f0f00", None, None),
        ];
        let fixed = reconcile(&prev, &cur);
        assert_eq!(fingerprint(&fixed[1]), fingerprint(&prev[1]));
        assert_eq!(fixed[1].media_id, cur[1].media_id);
        assert_eq!(new_bubbles(&prev, &fixed).unwrap(), Vec::<Bubble>::new());
        assert_eq!(cur[1].phash.as_deref(), Some("0f0f0f0f0f0f0f00"));
    }

    #[test]
    fn policy_test_same_size_different_images_are_distinct() {
        let a = make_image("0000000000000000", None, None);
        let b = make_image("ffffffff00000000", None, None);
        let c = make_image("00000000ffffffff", None, None);
        assert_eq!(
            new_bubbles(&[a.clone(), b.clone()], &[a, b, c.clone()]).unwrap(),
            vec![c]
        );
    }

    #[test]
    fn policy_test_far_phash_is_not_reconciled() {
        let prev = vec![make_image("0000000000000000", None, None)];
        let cur = vec![make_image("00000000000fffff", None, None)];
        assert_eq!(
            reconcile(&prev, &cur)[0].phash.as_deref(),
            Some("00000000000fffff")
        );
    }

    #[test]
    fn policy_test_stitch_joins_pages_after_cursor() {
        let a = make_bubble("a", None, None);
        let b = make_bubble("b", None, None);
        let c = make_bubble("c", None, None);
        let d = make_bubble("D", None, None);
        assert_eq!(
            stitch(
                &[a.clone(), b.clone()],
                &[vec![a, b, c.clone()], vec![c.clone(), d.clone()]]
            )
            .unwrap(),
            vec![c, d]
        );
    }

    #[test]
    fn policy_test_stitch_without_overlap_raises() {
        let res = stitch(
            &[make_bubble("a", None, None)],
            &[
                vec![make_bubble("x", None, None)],
                vec![make_bubble("y", None, None)],
            ],
        );
        assert_eq!(res, Err(ContinuityError::ContinuityGap));
    }

    #[test]
    fn policy_test_similar_new_image_is_not_taken_for_scrolled_away_one() {
        let a = make_image("0000000000000000", None, None);
        let b = make_image("0000000000000fff", None, None);
        let c = make_image("ffffffff00000000", None, None);
        let d = make_image("00000000ffffffff", None, None);
        let prev = vec![make_bubble("x", None, None), a];
        let cur = vec![b.clone(), c, d];
        let fixed = reconcile(&prev, &cur);
        assert_eq!(fixed[0].phash, b.phash);
        let res = new_bubbles(&prev, &fixed);
        assert_eq!(res, Err(ContinuityError::ContinuityGap));
    }

    #[test]
    fn policy_test_loading_image_beside_text_still_reconciles() {
        let prev = vec![
            make_bubble("x", None, None),
            make_image("0000000000000000", None, None),
        ];
        let cur = vec![
            make_bubble("x", None, None),
            make_image("0000000000000fff", None, None),
            make_bubble("y", None, None),
        ];
        assert_eq!(
            new_bubbles(&prev, &reconcile(&prev, &cur)).unwrap(),
            vec![cur[2].clone()]
        );
    }

    #[test]
    fn policy_test_pages_merge_by_position_across_empty_pages() {
        let a = make_bubble("a", None, None);
        let b = make_bubble("b", None, None);
        let i1 = make_image("0000000000000000", None, None);
        let i2 = make_image("ffffffff00000000", None, None);
        let pages = vec![
            vec![at_y(i2.clone(), 23, Some(247))],
            vec![at_y(i2.clone(), 113, Some(247))],
            vec![],
            vec![at_y(i1.clone(), 26, Some(247))],
            vec![
                at_y(a.clone(), 40, None),
                at_y(b.clone(), 100, None),
                at_y(i1.clone(), 116, Some(247)),
            ],
        ];
        let merged = merge_pages(&pages, Some(90)).unwrap();
        let keys: Vec<String> = merged
            .into_iter()
            .map(|m| {
                if m.kind.as_deref() == Some("image") {
                    m.phash.unwrap()
                } else {
                    m.text
                }
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                "a",
                "b",
                i1.phash.unwrap().as_str(),
                i2.phash.unwrap().as_str()
            ]
        );
    }

    #[test]
    fn policy_test_measured_offset_overrides_nominal_step() {
        let x = make_bubble("x", None, None);
        let y = make_bubble("y", None, None);
        let pages = vec![
            vec![at_y(y.clone(), 300, None)],
            vec![at_y(x.clone(), 200, None), at_y(y, 380, None)],
        ];
        let merged = merge_pages(&pages, Some(90)).unwrap();
        let texts: Vec<String> = merged.into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["x", "y"]);
    }

    #[test]
    fn policy_test_disagreeing_pages_raise_gap() {
        let pages = vec![
            vec![at_y(make_bubble("y", None, None), 300, None)],
            vec![at_y(make_bubble("z", None, None), 390, None)],
        ];
        let res = merge_pages(&pages, Some(90));
        assert_eq!(res, Err(ContinuityError::ContinuityGap));
    }

    #[test]
    fn policy_test_echo_normalizes_width_but_not_changed_numbers() {
        assert!(echo_matches("收到~", "收到～"));
        assert!(!echo_matches("金额100", "金额100.00"));
    }

    #[test]
    fn policy_test_private_title_with_parentheses_is_not_stripped() {
        let p1 = Policy::new("fixture", "张三", "service", None, false).unwrap();
        assert!(
            p1.check_frame(&make_frame(vec![], Some("张三 (3)")))
                .is_err()
        );

        let p2 = Policy::new("fixture", "示例群", "service", None, true).unwrap();
        assert!(
            p2.check_frame(&make_frame(vec![], Some("示例群 (3)")))
                .is_ok()
        );
    }

    #[test]
    fn policy_test_relocate_replaces_image_and_file_with_current() {
        let old_img = make_image("0000000000000000", None, Some("old_media_id"));
        let cur_img = make_image("0000000000000001", None, Some("new_media_id"));
        let mut cur_img_with_geom = cur_img.clone();
        cur_img_with_geom.x = Some(10);
        cur_img_with_geom.y = Some(20);

        let relocated = relocate(&[old_img.clone()], &[cur_img_with_geom]);
        assert_eq!(relocated.len(), 1);
        assert_eq!(relocated[0].media_id.as_deref(), Some("new_media_id"));
        assert_eq!(relocated[0].phash, old_img.phash);
        assert_eq!(relocated[0].x, Some(10));
        assert_eq!(relocated[0].y, Some(20));
    }

    #[test]
    fn policy_test_phash_distance_invalid_hex() {
        assert!(phash_distance("invalid", "0000").is_err());
        assert!(phash_distance("0000", "xyz").is_err());
        assert!(phash_distance("", "0000").is_err());
        assert!(phash_distance("0000", "").is_err());
        assert!(phash_distance("0xZZ", "00").is_err());
        assert_eq!(phash_distance("0000", "0001").unwrap(), 1);
        assert_eq!(phash_distance("0x0000", "0x0001").unwrap(), 1);
        assert_eq!(phash_distance("0f0f", "0f00").unwrap(), 4);
    }

    #[test]
    fn policy_test_echo_normalizes_fullwidth_and_newlines() {
        assert!(echo_matches("收到～\n好的", "收到~\n  好的\t"));
        assert!(echo_matches("全角　空格\r\n测试", "全角 空格 测试"));
        assert!(echo_matches("客服　电话：１２３４５", "客服 电话: 12345"));
        assert!(!echo_matches("收到～\n好的", "收到～\n不好"));
    }

    #[test]
    fn test_echo_matches_tolerates_ocr_misreads_on_long_replies() {
        // Device 2026-11-08: most unrecognised echoes were the right bubble with 1–2 characters misread by OCR.
        let reply =
            "您好，我是AI客服助手，请问有什么可以帮您？订单问题请提供订单号，我们会尽快为您处理。";
        let one_off = reply.replacen("订单号", "订单弓", 1);
        let two_off = one_off.replacen("处理", "处埋", 1);
        assert!(echo_matches(&one_off, reply));
        assert!(echo_matches(&two_off, reply));
        // Too many differences, or a clearly different message, is not our echo.
        // 43 characters allow 2 edits; three differ here.
        let many = reply.replace("订单", "汀单").replacen("客服", "客眼", 1);
        assert!(!echo_matches(&many, reply));
        assert!(!echo_matches(
            "今天天气很好，适合出去走走，记得带伞以防下雨，路上注意安全哦。",
            reply
        ));
        // Short replies stay exact: one character changes the meaning.
        assert!(!echo_matches("好的谢谢", "好吧谢谢"));
        assert!(!echo_matches("金额100", "金额100.00"));
    }

    #[test]
    fn test_key_matches_python_golden() {
        // Expected keys computed by scripts/wechat_bot/policy.py Policy.key (json.dumps ensure_ascii=False).
        // A different key after switching from Python to Rust would let the same turn be sent twice.
        fn fp(direction: &str, digest: &str, phash: Option<&str>) -> Bubble {
            let mut b = make_bubble(digest, Some(direction), Some(digest));
            b.phash = phash.map(str::to_string);
            b
        }
        let cases: Vec<(&str, &str, Vec<Bubble>, Bubble, &str)> = vec![
            (
                "test-desktop",
                "示例群",
                vec![fp("inbound_visual", "你好", None)],
                fp("inbound_visual", "在吗", None),
                "bot-ab507730f719356627784f02651896ad60d40a37fe49425f5be84558b67ca4ee",
            ),
            (
                "acc",
                "示例联系人甲☕",
                vec![fp(
                    "outbound_visual",
                    "引\"号\\反斜杠",
                    Some("00ff00ff00ff00ff"),
                )],
                fp(
                    "inbound_visual",
                    "换行\n制表\t控制\u{1}\u{7f}末尾 😀",
                    Some("0123456789abcdef"),
                ),
                "bot-b80fe1c960d3bcc0891c7f19043572e29054fff7136a435e925cb9b3b11bb994",
            ),
            (
                "a",
                "b",
                vec![],
                fp("inbound_visual", "\u{2028}\u{e9}", None),
                "bot-fe2f1fa9b5c4bb4fde22f14dcf0fd69cc54e1d809218e6efb009b7ca16266f63",
            ),
        ];
        for (account, recipient, baseline, candidate, expected) in cases {
            let policy = Policy::new(account, recipient, "t", None, false).unwrap();
            let mut frame: Frame = serde_json::from_value(json!({"bubbles": []})).unwrap();
            frame.bubbles = baseline;
            assert_eq!(
                policy.key(&frame, &candidate),
                expected,
                "{account}/{recipient}"
            );
        }
    }

    struct TestDir(std::path::PathBuf);
    impl TestDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("test-policy-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct MockCapture {
        frames: Vec<Frame>,
        index: usize,
    }

    impl MockCapture {
        fn new(frames: Vec<Frame>) -> Self {
            Self { frames, index: 0 }
        }
    }

    impl CaptureFrames for MockCapture {
        async fn next_frame(&mut self) -> Result<Frame> {
            if self.index < self.frames.len() {
                let f = self.frames[self.index].clone();
                self.index += 1;
                Ok(f)
            } else {
                bail!("no more frames");
            }
        }
    }

    struct MockClient {
        calls: Vec<(String, Value)>,
        send_error: bool,
    }

    impl MockClient {
        fn new(send_error: bool) -> Self {
            Self {
                calls: Vec::new(),
                send_error,
            }
        }
    }

    impl PolicyToolClient for MockClient {
        async fn call_tool(&mut self, name: &str, args: Value) -> Result<Value> {
            self.calls.push((name.to_string(), args));
            if name == "send_message" && self.send_error {
                bail!("send outcome unknown");
            }
            Ok(json!({"status": "triggered", "delivery": "unknown"}))
        }
    }

    #[tokio::test]
    async fn test_one_model_reply_through_existing_idempotent_send() {
        let td = TestDir::new();
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let old = make_frame(vec![make_bubble("old", None, None)], None);
        let new_frame = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104 你好", None, None),
            ],
            None,
        );
        let mut after_bubbles = new_frame.bubbles.clone();
        after_bubbles.push(make_bubble(
            "你好，这是自动回复测试。",
            Some("outbound_visual"),
            None,
        ));
        let after = make_frame(after_bubbles, None);

        let mut capture = MockCapture::new(vec![
            old,
            new_frame.clone(),
            new_frame.clone(),
            new_frame,
            after,
        ]);
        let mut client = MockClient::new(false);
        let gen_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gen_count_clone = gen_count.clone();

        run_bounded_bot(
            td.path(),
            &policy,
            &mut client,
            &mut capture,
            move |_| {
                gen_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Ok("你好，这是自动回复测试。".to_string()) }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let sent: Vec<_> = client
            .calls
            .iter()
            .filter(|(n, _)| n == "send_message")
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1["recipient"], "示例联系人甲");
        assert_eq!(sent[0].1["message"], "你好，这是自动回复测试。");
        assert!(
            sent[0].1["idempotency_key"]
                .as_str()
                .unwrap()
                .starts_with("bot-")
        );

        let mut found_record = None;
        for entry in std::fs::read_dir(td.path()).unwrap() {
            let path = entry.unwrap().path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("bot-") && name.ends_with(".json") {
                    let content = std::fs::read_to_string(&path).unwrap();
                    found_record = Some(serde_json::from_str::<Value>(&content).unwrap());
                    break;
                }
            }
        }
        let record = found_record.expect("expected bot-*.json journal");
        assert_eq!(record["client_echo_observed"], true);
        assert_eq!(record["delivery"], "unknown");
        assert_eq!(gen_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_model_latency_cannot_authorize_sending_into_changed_chat() {
        let td = TestDir::new();
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let old = make_frame(vec![make_bubble("old", None, None)], None);
        let new_frame = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104 你好", None, None),
            ],
            None,
        );
        let mut changed_bubbles = new_frame.bubbles.clone();
        changed_bubbles.push(make_bubble("another", None, None));
        let changed = make_frame(changed_bubbles, None);

        let mut capture = MockCapture::new(vec![old, new_frame.clone(), new_frame, changed]);
        let mut client = MockClient::new(false);

        let err = run_bounded_bot(
            td.path(),
            &policy,
            &mut client,
            &mut capture,
            |_| async { Ok("你好，这是自动回复测试。".to_string()) },
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("changed before sending"),
            "got err: {}",
            err
        );
        assert!(!client.calls.iter().any(|(n, _)| n == "send_message"));
    }

    #[tokio::test]
    async fn test_non_allowlisted_title_never_reaches_model() {
        let td = TestDir::new();
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let bad_frame = make_frame(vec![], Some("其他人"));

        let mut capture = MockCapture::new(vec![bad_frame]);
        let mut client = MockClient::new(false);
        let gen_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gen_called_clone = gen_called.clone();

        let err = run_bounded_bot(
            td.path(),
            &policy,
            &mut client,
            &mut capture,
            move |_| {
                gen_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                async { Ok("reply".to_string()) }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("allowlisted title"),
            "got err: {}",
            err
        );
        assert!(!gen_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!client.calls.iter().any(|(n, _)| n == "send_message"));
    }

    #[tokio::test]
    async fn test_unknown_send_is_not_regenerated_or_resent_on_restart() {
        let td = TestDir::new();
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let old = make_frame(vec![make_bubble("old", None, None)], None);
        let new_frame = make_frame(
            vec![
                make_bubble("old", None, None),
                make_bubble("Bot验证1104 你好", None, None),
            ],
            None,
        );

        let mut capture1 = MockCapture::new(vec![
            old.clone(),
            new_frame.clone(),
            new_frame.clone(),
            new_frame.clone(),
        ]);
        let mut client1 = MockClient::new(true); // send_error = true
        let gen_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let gen_c1 = gen_count.clone();
        let err = run_bounded_bot(
            td.path(),
            &policy,
            &mut client1,
            &mut capture1,
            move |_| {
                gen_c1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Ok("你好，这是自动回复测试。".to_string()) }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("send outcome unknown"));

        // Restart with the same state directory
        let mut capture2 = MockCapture::new(vec![old, new_frame.clone(), new_frame]);
        let mut client2 = MockClient::new(false);
        let gen_c2 = gen_count.clone();

        run_bounded_bot(
            td.path(),
            &policy,
            &mut client2,
            &mut capture2,
            move |_| {
                gen_c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { Ok("你好，这是自动回复测试。".to_string()) }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let total_sends = client1
            .calls
            .iter()
            .filter(|(n, _)| n == "send_message")
            .count()
            + client2
                .calls
                .iter()
                .filter(|(n, _)| n == "send_message")
                .count();
        assert_eq!(total_sends, 1);
        assert_eq!(gen_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
