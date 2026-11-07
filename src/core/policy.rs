use std::collections::HashSet;
use crate::core::model::{Bubble, Frame};

pub const PHASH_TOLERANCE: u32 = 12;
pub const FINGERPRINT_VERSION: u32 = 2;
pub const SCROLL_STEP_POINTS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub direction: String,
    pub digest: String,
    pub phash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AmbiguousAnchor {
    pub message: String,
    pub certain: Vec<Bubble>,
}

impl std::fmt::Display for AmbiguousAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AmbiguousAnchor {}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinuityGap(pub String);

impl std::fmt::Display for ContinuityGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ContinuityGap {}

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyError {
    ContinuityGap(ContinuityGap),
    AmbiguousAnchor(AmbiguousAnchor),
    InvalidFrame(String),
    ValidationError(String),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContinuityGap(e) => write!(f, "{}", e),
            Self::AmbiguousAnchor(e) => write!(f, "{}", e),
            Self::InvalidFrame(e) => write!(f, "{}", e),
            Self::ValidationError(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for PolicyError {}

pub fn fingerprint(bubble: &Bubble) -> Fingerprint {
    Fingerprint {
        direction: bubble.direction.clone(),
        digest: bubble.digest.clone(),
        phash: bubble.phash.clone(),
    }
}

pub fn phash_distance(a: &str, b: &str) -> u32 {
    let va = u64::from_str_radix(a.trim(), 16).unwrap_or(0);
    let vb = u64::from_str_radix(b.trim(), 16).unwrap_or(0);
    (va ^ vb).count_ones()
}

pub fn same_bubble(old: &Bubble, new: &Bubble) -> bool {
    if old.direction != new.direction || old.digest != new.digest {
        return false;
    }
    match (&old.phash, &new.phash) {
        (Some(pa), Some(pb)) => phash_distance(pa, pb) <= PHASH_TOLERANCE,
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

pub fn new_bubbles(previous: &[Bubble], current: &[Bubble]) -> Result<Vec<Bubble>, PolicyError> {
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
        return Err(PolicyError::ContinuityGap(ContinuityGap(
            "visual continuity gap or ambiguous repeated anchor".to_string(),
        )));
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
        return Err(PolicyError::AmbiguousAnchor(AmbiguousAnchor {
            message: "ambiguous repeated anchor".to_string(),
            certain: certain_tail(&old, &new, current),
        }));
    }

    let mut count = 0;
    for i in 0..=(new.len() - size) {
        if &new[i..i + size] == anchor {
            count += 1;
        }
    }
    if count != 1 {
        return Err(PolicyError::AmbiguousAnchor(AmbiguousAnchor {
            message: "repeated visual anchor".to_string(),
            certain: certain_tail(&old, &new, current),
        }));
    }

    Ok(current[size..].to_vec())
}

pub fn reconcile(previous: &[Bubble], current: &[Bubble]) -> Vec<Bubble> {
    let mut result = current.to_vec();
    let min_len = previous.len().min(current.len());
    for size in (1..=min_len).rev() {
        let prev_window = &previous[previous.len() - size..];
        let curr_window = &result[..size];
        let all_same = prev_window.iter().zip(curr_window.iter()).all(|(o, n)| same_bubble(o, n));
        if !all_same {
            continue;
        }
        let all_images = curr_window.iter().all(|n| n.kind.as_deref() == Some("image"));
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
                        (Some(ip), Some(bp)) => phash_distance(ip, bp),
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

pub fn merge_pages(pages: &[Vec<Bubble>], step: Option<i64>) -> Result<Vec<Bubble>, PolicyError> {
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
            let twin = placed.iter().find(|(p, b)| b.direction == bubble.direction && (p - pos).abs() <= 8);
            match twin {
                None => placed.push((pos, bubble.clone())),
                Some((_, twin_b)) => {
                    if !same_bubble(twin_b, bubble) {
                        return Err(PolicyError::ContinuityGap(ContinuityGap(
                            "scrolled pages disagree at one position".to_string(),
                        )));
                    }
                }
            }
        }
    }

    placed.sort_by_key(|(pos, _)| *pos);
    Ok(placed.into_iter().map(|(_, b)| b).collect())
}

pub fn stitch(cursor: &[Bubble], pages: &[Vec<Bubble>]) -> Result<Vec<Bubble>, PolicyError> {
    for (index, page) in pages.iter().enumerate() {
        let mut changes = match new_bubbles(cursor, page) {
            Ok(c) => c,
            Err(PolicyError::ContinuityGap(_)) => continue,
            Err(e) => return Err(e),
        };

        for i in index..pages.len().saturating_sub(1) {
            let more = new_bubbles(&pages[i], &pages[i + 1])?;
            changes.extend(more);
        }
        return Ok(changes);
    }
    Err(PolicyError::ContinuityGap(ContinuityGap(
        "no scrolled page overlaps the cursor".to_string(),
    )))
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
    raw.chars().filter(|&c| !is_invisible(c)).collect::<String>().trim().to_string()
}

pub fn validate_reply(value: &str) -> Result<String, PolicyError> {
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

pub fn normalize_for_echo(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{3000}' {
            continue;
        }
        let u = c as u32;
        let mapped = if (0xFF01..=0xFF5E).contains(&u) {
            char::from_u32(u - 0xFEE0).unwrap_or(c)
        } else {
            c
        };
        out.push(mapped);
    }
    out
}

pub fn echo_matches(observed: &str, expected: &str) -> bool {
    normalize_for_echo(observed) == normalize_for_echo(expected)
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]
    )
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_fp(fp: &Fingerprint) -> String {
    match &fp.phash {
        Some(p) => format!(
            "[\"{}\", \"{}\", \"{}\"]",
            escape_json(&fp.direction),
            escape_json(&fp.digest),
            escape_json(p)
        ),
        None => format!(
            "[\"{}\", \"{}\", null]",
            escape_json(&fp.direction),
            escape_json(&fp.digest)
        ),
    }
}

fn format_identity_json(account: &str, recipient: &str, baseline: &[Fingerprint], candidate: &Fingerprint) -> String {
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
    ) -> Result<Self, PolicyError> {
        let account = account.into();
        let recipient = recipient.into();
        let trigger = trigger.into();
        if account.is_empty() || recipient.is_empty() || trigger.is_empty() {
            return Err(PolicyError::ValidationError(
                "explicit account, exact private-chat allowlist and test trigger required".to_string(),
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

    pub fn check_frame(&self, frame: &Frame) -> Result<(), PolicyError> {
        let profile = frame.profile.as_deref().unwrap_or("");
        let valid_profile = profile == "wechat-4-dark" || profile == "wechat-4-light";
        let valid_scale = match &frame.scale {
            Some(serde_json::Value::Number(n)) => n.as_i64() == Some(1) || n.as_i64() == Some(2),
            _ => false,
        };
        if !valid_profile || !valid_scale {
            return Err(PolicyError::InvalidFrame("uncalibrated visual profile".to_string()));
        }
        if frame.clipped_bubble == Some(true) {
            return Err(PolicyError::InvalidFrame(
                "incomplete bottom bubble; no automatic reply".to_string(),
            ));
        }
        let titles: Vec<String> = frame.title_lines.iter().map(|x| {
            let cleaned = clean_title(x);
            if self.group {
                strip_member_count(&cleaned)
            } else {
                cleaned
            }
        }).collect();

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
    ) -> Result<Option<Bubble>, PolicyError> {
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
        let identity_json = format_identity_json(&self.account, &self.recipient, &baseline_fps, &cand_fp);
        let hash = sha256_hex(identity_json.as_bytes());
        format!("bot-{}", hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let mid = media_id.map(|s| s.to_string()).unwrap_or_else(|| phash.repeat(4));
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
            assert!(matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("allowlisted title")));
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
        let current = make_frame(vec![make_bubble("old", None, None), make_bubble("Bot验证1104", None, None)], None);
        let changed = make_frame(vec![make_bubble("old", None, None), make_bubble("other", None, None)], None);
        let err = policy.candidate(&baseline, &current, &changed).unwrap_err();
        assert!(matches!(err, PolicyError::ValidationError(ref msg) if msg.contains("changed during confirmation")));
    }

    #[test]
    fn policy_test_gap_and_repeated_anchor_stop() {
        let err1 = new_bubbles(&[make_bubble("old", None, None)], &[make_bubble("new", None, None)]).unwrap_err();
        assert!(matches!(err1, PolicyError::ContinuityGap(_)));

        let same = make_bubble("same", None, None);
        let err2 = new_bubbles(&[same.clone(), same.clone()], &[same.clone(), same.clone(), make_bubble("extra", None, None)]).unwrap_err();
        assert!(matches!(err2, PolicyError::AmbiguousAnchor(_)));
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
            PolicyError::AmbiguousAnchor(caught) => assert_eq!(caught.certain, Vec::<Bubble>::new()),
            _ => panic!("expected AmbiguousAnchor"),
        }

        let err2 = new_bubbles(&[same.clone(), same.clone()], &[same.clone(), same.clone(), make_bubble("next", None, None)]).unwrap_err();
        match err2 {
            PolicyError::AmbiguousAnchor(caught) => assert_eq!(caught.certain, vec![make_bubble("next", None, None)]),
            _ => panic!("expected AmbiguousAnchor"),
        }
    }

    #[test]
    fn policy_test_periodic_repeated_context_is_still_ambiguous() {
        let old: Vec<Bubble> = ["a", "b", "a", "b"].iter().map(|x| make_bubble(x, None, None)).collect();
        let mut new1 = old.clone();
        new1.push(make_bubble("next", None, None));
        let err = new_bubbles(&old, &new1).unwrap_err();
        assert!(matches!(err, PolicyError::AmbiguousAnchor(_)));
    }

    #[test]
    fn policy_test_multiple_new_messages_are_not_replied_to_as_one() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let current = make_frame(vec![
            make_bubble("old", None, None),
            make_bubble("Bot验证1104", None, None),
            make_bubble("second", None, None),
        ], None);
        let res = policy.candidate(&baseline, &current, &current).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn policy_test_same_body_different_occurrences_remain_distinct() {
        let old = vec![make_bubble("anchor", None, None), make_bubble("same", None, None)];
        let mut current = old.clone();
        current.push(make_bubble("same", None, None));
        assert_eq!(new_bubbles(&old, &current).unwrap(), vec![make_bubble("same", None, None)]);
    }

    #[test]
    fn policy_test_key_scoped_to_account_recipient_and_input() {
        let policy = Policy::new("fixture", "示例联系人甲", "Bot验证1104", None, false).unwrap();
        let baseline = make_frame(vec![make_bubble("old", None, None)], None);
        let message = make_bubble("Bot验证1104", None, None);
        let key = policy.key(&baseline, &message);
        assert_eq!(key, policy.key(&baseline.clone(), &message.clone()));
        assert_ne!(key, Policy::new("other", "示例联系人甲", "Bot验证1104", None, false).unwrap().key(&baseline, &message));
        assert_ne!(key, Policy::new("fixture", "其他人", "Bot验证1104", None, false).unwrap().key(&baseline, &message));
        // 校验实际格式
        assert!(key.starts_with("bot-"));
        assert_eq!(key.len(), 4 + 64);
    }

    #[test]
    fn policy_test_policy_accepts_alias_titles_only() {
        let policy = Policy::new("fixture", "🍓示例联系人甲", "service", Some(vec!["🍓示例联系人甲".to_string(), "示例联系人甲".to_string()]), false).unwrap();
        assert!(policy.check_frame(&make_frame(vec![make_bubble("x", None, None)], Some("示例联系人甲"))).is_ok());
        assert!(policy.check_frame(&make_frame(vec![make_bubble("x", None, None)], Some("\u{200b}🍓示例联系人甲 "))).is_ok());
        let res = policy.check_frame(&make_frame(vec![make_bubble("x", None, None)], Some("别人")));
        assert!(matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("allowlisted title")));
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
            assert!(matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("uncalibrated")));
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
        let mut partial = make_frame(vec![make_bubble("old", None, None), make_bubble("Bot验证1104", None, None)], None);
        partial.clipped_bubble = Some(true);
        let res = policy.candidate(&baseline, &partial, &partial);
        assert!(matches!(res, Err(PolicyError::InvalidFrame(ref msg)) if msg.contains("incomplete bottom bubble")));
    }

    #[test]
    fn policy_test_reply_bounds() {
        assert_eq!(validate_reply("你好，这是自动回复测试。").unwrap(), "你好，这是自动回复测试。");
        for body in ["", "  ", &"x".repeat(201), "你好\x00"] {
            assert!(validate_reply(body).is_err());
        }
    }

    #[test]
    fn policy_test_loading_image_reconciles_to_same_fingerprint() {
        let prev = vec![make_bubble("hi", None, None), make_image("0f0f0f0f0f0f0f0f", None, None)];
        let cur = vec![make_bubble("hi", None, None), make_image("0f0f0f0f0f0f0f00", None, None)];
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
        assert_eq!(new_bubbles(&[a.clone(), b.clone()], &[a, b, c.clone()]).unwrap(), vec![c]);
    }

    #[test]
    fn policy_test_far_phash_is_not_reconciled() {
        let prev = vec![make_image("0000000000000000", None, None)];
        let cur = vec![make_image("00000000000fffff", None, None)];
        assert_eq!(reconcile(&prev, &cur)[0].phash.as_deref(), Some("00000000000fffff"));
    }

    #[test]
    fn policy_test_stitch_joins_pages_after_cursor() {
        let a = make_bubble("a", None, None);
        let b = make_bubble("b", None, None);
        let c = make_bubble("c", None, None);
        let d = make_bubble("D", None, None);
        assert_eq!(
            stitch(&[a.clone(), b.clone()], &[vec![a, b, c.clone()], vec![c.clone(), d.clone()]]).unwrap(),
            vec![c, d]
        );
    }

    #[test]
    fn policy_test_stitch_without_overlap_raises() {
        let res = stitch(&[make_bubble("a", None, None)], &[vec![make_bubble("x", None, None)], vec![make_bubble("y", None, None)]]);
        assert!(matches!(res, Err(PolicyError::ContinuityGap(_))));
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
        assert!(matches!(res, Err(PolicyError::ContinuityGap(_))));
    }

    #[test]
    fn policy_test_loading_image_beside_text_still_reconciles() {
        let prev = vec![make_bubble("x", None, None), make_image("0000000000000000", None, None)];
        let cur = vec![make_bubble("x", None, None), make_image("0000000000000fff", None, None), make_bubble("y", None, None)];
        assert_eq!(new_bubbles(&prev, &reconcile(&prev, &cur)).unwrap(), vec![cur[2].clone()]);
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
            vec![at_y(a.clone(), 40, None), at_y(b.clone(), 100, None), at_y(i1.clone(), 116, Some(247))],
        ];
        let merged = merge_pages(&pages, Some(90)).unwrap();
        let keys: Vec<String> = merged.into_iter().map(|m| {
            if m.kind.as_deref() == Some("image") {
                m.phash.unwrap()
            } else {
                m.text
            }
        }).collect();
        assert_eq!(keys, vec!["a", "b", i1.phash.unwrap().as_str(), i2.phash.unwrap().as_str()]);
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
        assert!(matches!(res, Err(PolicyError::ContinuityGap(_))));
    }

    #[test]
    fn policy_test_echo_normalizes_width_but_not_changed_numbers() {
        assert!(echo_matches("收到~", "收到～"));
        assert!(!echo_matches("金额100", "金额100.00"));
    }

    #[test]
    fn policy_test_private_title_with_parentheses_is_not_stripped() {
        let p1 = Policy::new("fixture", "张三", "service", None, false).unwrap();
        assert!(p1.check_frame(&make_frame(vec![], Some("张三 (3)"))).is_err());

        let p2 = Policy::new("fixture", "示例群", "service", None, true).unwrap();
        assert!(p2.check_frame(&make_frame(vec![], Some("示例群 (3)"))).is_ok());
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
}
