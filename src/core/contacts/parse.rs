use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrLine {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFields {
    pub category: String,
    pub wechat_id: Option<String>,
    pub nickname: String,
    pub remark: Option<String>,
    pub region: Option<String>,
    pub source: Option<String>,
    pub added_on: Option<String>,
    pub common_groups: Option<i64>,
    pub enterprise: Option<String>,
    pub real_name: Option<String>,
    pub tags: Vec<String>,
    pub friend_permission: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileParse {
    pub fields: ProfileFields,
    pub missing: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagerRow {
    pub row_y: f64,
    pub name: Option<String>,
    pub remark: Option<String>,
    pub tags: Vec<String>,
    pub permission: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagerColumns {
    pub name_x: f64,
    pub remark_x: f64,
    pub tags_x: f64,
    pub permission_x: f64,
}

fn strip_prefix_and_colon<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if let Some(rest) = text.strip_prefix(prefix) {
        let rest = rest.trim_start();
        let rest = if let Some(stripped) = rest.strip_prefix('：') {
            stripped.trim_start()
        } else if let Some(stripped) = rest.strip_prefix(':') {
            stripped.trim_start()
        } else {
            rest
        };
        Some(rest)
    } else {
        None
    }
}

fn is_valid_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

fn parse_digits(s: &str) -> Option<i64> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().ok()
}

pub fn parse_profile(lines: &[OcrLine], category: &str) -> ProfileParse {
    let mut wechat_id = None;
    let mut remark = None;
    let mut region = None;
    let mut source = None;
    let mut added_on = None;
    let mut common_groups = None;
    let mut enterprise = None;
    let mut real_name = None;
    let mut friend_permission = None;

    let prefixes = [
        ("微信号", "wechat_id"),
        ("地区", "region"),
        ("备注", "remark"),
        ("来源", "source"),
        ("添加时间", "added_on"),
        ("共同群聊", "common_groups"),
        ("企业", "enterprise"),
        ("实名", "real_name"),
        ("朋友权限", "friend_permission"),
    ];

    let mut used_line_indices = std::collections::HashSet::new();

    // 1. 查找已知字段及其值（同一行或同行右侧）
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        if trimmed == "添加备注名" {
            used_line_indices.insert(idx);
            continue;
        }

        // 字段名只出现在一行的最左边：同一行（y±6）左侧还有别的文字时，这一段是值，不是字段名
        // （例如 "来源 | 企业微信" 中的"企业微信"不能被当成字段"企业"）。
        let has_left_peer = lines.iter().enumerate().any(|(p_idx, peer)| {
            p_idx != idx && (peer.y - line.y).abs() <= 6.0 && peer.x < line.x
        });
        if has_left_peer {
            continue;
        }

        for (prefix, field) in &prefixes {
            if let Some(val_str) = strip_prefix_and_colon(trimmed, prefix) {
                used_line_indices.insert(idx);
                let value_text = if !val_str.is_empty() {
                    val_str.to_string()
                } else {
                    // 在同一行右侧（容差 y±6.0，x > line.x）查找值
                    let mut peer_val = None;
                    for (p_idx, peer) in lines.iter().enumerate() {
                        if p_idx != idx
                            && (peer.y - line.y).abs() <= 6.0
                            && peer.x > line.x
                            && !used_line_indices.contains(&p_idx)
                        {
                            peer_val = Some((p_idx, peer.text.trim().to_string()));
                            break;
                        }
                    }
                    if let Some((p_idx, text)) = peer_val {
                        used_line_indices.insert(p_idx);
                        text
                    } else {
                        String::new()
                    }
                };

                let value_clean = value_text.trim();
                if value_clean.is_empty() {
                    continue;
                }

                match *field {
                    "wechat_id" => {
                        wechat_id = Some(value_clean.to_string());
                    }
                    "region" => {
                        region = Some(value_clean.to_string());
                    }
                    "remark" => {
                        if value_clean != "添加备注名" {
                            remark = Some(value_clean.to_string());
                        }
                    }
                    "source" => {
                        source = Some(value_clean.to_string());
                    }
                    "added_on" => {
                        if is_valid_date(value_clean) {
                            added_on = Some(value_clean.to_string());
                        }
                    }
                    "common_groups" => {
                        if let Some(n) = parse_digits(value_clean) {
                            common_groups = Some(n);
                        }
                    }
                    "enterprise" => {
                        enterprise = Some(value_clean.to_string());
                    }
                    "real_name" => {
                        real_name = Some(value_clean.to_string());
                    }
                    "friend_permission" => {
                        friend_permission = Some(value_clean.to_string());
                    }
                    _ => {}
                }
                break;
            }
        }
    }

    // 2. 昵称：取资料卡区域最上方一行（y 最小、x ≥ 500）
    let mut nickname_candidates: Vec<&OcrLine> = lines
        .iter()
        .enumerate()
        .filter(|(idx, l)| {
            if used_line_indices.contains(idx) {
                return false;
            }
            if l.x < 500.0 {
                return false;
            }
            let trimmed = l.text.trim();
            if trimmed.is_empty() || trimmed == "添加备注名" {
                return false;
            }
            for (p, _) in &prefixes {
                if trimmed.starts_with(p) {
                    return false;
                }
            }
            true
        })
        .map(|(_, l)| l)
        .collect();

    nickname_candidates.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    let nickname = nickname_candidates
        .first()
        .map(|l| l.text.trim().to_string())
        .unwrap_or_default();

    let mut missing = Vec::new();
    if wechat_id.is_none() {
        missing.push("wechat_id");
    }
    if nickname.is_empty() {
        missing.push("nickname");
    }

    ProfileParse {
        fields: ProfileFields {
            category: category.to_string(),
            wechat_id,
            nickname,
            remark,
            region,
            source,
            added_on,
            common_groups,
            enterprise,
            real_name,
            tags: Vec::new(),
            friend_permission,
        },
        missing,
    }
}

pub fn parse_manager_page(
    lines: &[OcrLine],
    first_row_y: f64,
    row_h: f64,
    rows: usize,
    columns: &ManagerColumns,
) -> Vec<ManagerRow> {
    let mut result = Vec::with_capacity(rows);

    for i in 0..rows {
        let target_y = first_row_y + i as f64 * row_h;
        let row_lines: Vec<&OcrLine> = lines
            .iter()
            .filter(|l| (l.y - target_y).abs() <= row_h / 2.0)
            .collect();

        let mut name = None;
        let mut remark = None;
        let mut tags = Vec::new();
        let mut permission = None;

        for l in row_lines {
            let d_name = (l.x - columns.name_x).abs();
            let d_remark = (l.x - columns.remark_x).abs();
            let d_tags = (l.x - columns.tags_x).abs();
            let d_perm = (l.x - columns.permission_x).abs();

            let min_d = d_name.min(d_remark).min(d_tags).min(d_perm);
            if (min_d - d_name).abs() < 1e-6 {
                let t = l.text.trim();
                if !t.is_empty() {
                    name = Some(t.to_string());
                }
            } else if (min_d - d_remark).abs() < 1e-6 {
                let t = l.text.trim();
                if !t.is_empty() && t != "添加备注名" {
                    remark = Some(t.to_string());
                }
            } else if (min_d - d_tags).abs() < 1e-6 {
                let t = l.text.trim();
                for part in t.split(&[',', '，', ';', '；'][..]) {
                    let part_trim = part.trim();
                    if !part_trim.is_empty() {
                        tags.push(part_trim.to_string());
                    }
                }
            } else {
                let t = l.text.trim();
                if !t.is_empty() {
                    permission = Some(t.to_string());
                }
            }
        }

        result.push(ManagerRow {
            row_y: target_y,
            name,
            remark,
            tags,
            permission,
        });
    }

    result
}

pub fn parse_group_headers(lines: &[OcrLine]) -> Vec<(String, u32, f64)> {
    let mut headers = Vec::new();

    for line in lines {
        let trimmed = line.text.trim();
        // 过滤展开箭头等前缀
        let clean = trimmed.trim_start_matches(|c: char| {
            c == '▶' || c == '▼' || c == '▷' || c == '▽' || c == '>' || c == 'v' || c == '^' || c.is_whitespace()
        });

        // 分组标题必须与微信的分组名完全一致（只比较汉字：箭头各种字形、标点、空白、数量都去掉），
        // 否则名字里带"联系人""群聊"的联系人行会被误当成分组。
        let normalized: String = clean
            .chars()
            .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
            .collect();
        let category = match normalized.as_str() {
            "公众号服务号" => Some("official"),
            "企业微信联系人" => Some("work"),
            "联系人" => Some("personal"),
            "群聊" => Some("group"),
            _ => None,
        };

        if let Some(cat) = category {
            let mut count = None;

            // 检查右侧同行 (y 容差 ±6.0, x > line.x) 是否有纯数字
            for peer in lines {
                if (peer.y - line.y).abs() <= 6.0 && peer.x > line.x {
                    let peer_trim = peer.text.trim();
                    if let Ok(c) = peer_trim.parse::<u32>() {
                        count = Some(c);
                        break;
                    }
                }
            }

            // 如果同侧未找到，尝试从自身文本末尾提取数字 (如 "(33)" 或 " 33")
            if count.is_none() {
                if let Some(c) = parse_digits(clean).and_then(|n| u32::try_from(n).ok()) {
                    count = Some(c);
                }
            }

            let final_count = count.unwrap_or(0);
            headers.push((cat.to_string(), final_count, line.y));
        }
    }

    headers.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: f64, y: f64, w: f64, h: f64) -> OcrLine {
        OcrLine {
            text: text.to_string(),
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn test_profile_label_value_split() {
        let lines = vec![
            line("测试甲", 520.0, 70.0, 80.0, 24.0),
            line("微信号：wxid_t1", 500.0, 110.0, 140.0, 18.0),
            line("地区中国 测试省", 500.0, 140.0, 120.0, 18.0),
            line("备注", 431.0, 200.0, 40.0, 18.0),
            line("甲备注", 504.0, 202.0, 60.0, 18.0),
            line("来源通过搜索手机号添加", 500.0, 240.0, 180.0, 18.0),
            line("添加时间2023-06-08", 500.0, 270.0, 160.0, 18.0),
            line("共同群聊1个", 500.0, 300.0, 100.0, 18.0),
        ];

        let parsed = parse_profile(&lines, "personal");
        assert_eq!(parsed.fields.category, "personal");
        assert_eq!(parsed.fields.nickname, "测试甲");
        assert_eq!(parsed.fields.wechat_id.as_deref(), Some("wxid_t1"));
        assert_eq!(parsed.fields.region.as_deref(), Some("中国 测试省"));
        assert_eq!(parsed.fields.remark.as_deref(), Some("甲备注"));
        assert_eq!(parsed.fields.source.as_deref(), Some("通过搜索手机号添加"));
        assert_eq!(parsed.fields.added_on.as_deref(), Some("2023-06-08"));
        assert_eq!(parsed.fields.common_groups, Some(1));
    }

    #[test]
    fn test_profile_remark_placeholder_is_none() {
        let lines = vec![
            line("测试甲", 520.0, 70.0, 80.0, 24.0),
            line("微信号：wxid_t1", 500.0, 110.0, 140.0, 18.0),
            line("添加备注名", 500.0, 160.0, 80.0, 18.0),
        ];

        let parsed = parse_profile(&lines, "personal");
        assert_eq!(parsed.fields.remark, None);
    }

    #[test]
    fn test_official_profile_wechat_id_gh() {
        let lines = vec![
            line("官方测试服务号", 520.0, 70.0, 120.0, 24.0),
            line("微信号gh_abc123", 500.0, 110.0, 150.0, 18.0),
            line("这是一段公众号简介", 500.0, 150.0, 200.0, 18.0),
        ];

        let parsed = parse_profile(&lines, "official");
        assert_eq!(parsed.fields.category, "official");
        assert_eq!(parsed.fields.nickname, "官方测试服务号");
        assert_eq!(parsed.fields.wechat_id.as_deref(), Some("gh_abc123"));
        assert!(parsed.missing.is_empty() || !parsed.missing.contains(&"wechat_id"));
    }

    #[test]
    fn test_work_profile_has_no_wechat_id() {
        let lines = vec![
            line("张三@某某科技", 520.0, 70.0, 120.0, 24.0),
            line("企业", 431.0, 220.0, 40.0, 18.0),
            line("某某科技有限公司", 504.0, 220.0, 140.0, 18.0),
            line("实名：张三", 431.0, 250.0, 80.0, 18.0),
        ];

        let parsed = parse_profile(&lines, "work");
        assert_eq!(parsed.fields.category, "work");
        assert_eq!(parsed.fields.nickname, "张三@某某科技");
        assert_eq!(parsed.fields.wechat_id, None);
        assert_eq!(parsed.fields.enterprise.as_deref(), Some("某某科技有限公司"));
        assert_eq!(parsed.fields.real_name.as_deref(), Some("张三"));
        assert!(parsed.missing.contains(&"wechat_id"));
    }

    #[test]
    fn test_profile_nickname_is_top_large_line() {
        let lines = vec![
            line("左侧列表联系人", 120.0, 50.0, 100.0, 20.0),
            line("大字昵称", 520.0, 80.0, 90.0, 28.0),
            line("微信号：wxid_abc", 500.0, 120.0, 120.0, 18.0),
            line("地区：中国", 500.0, 150.0, 100.0, 18.0),
        ];

        let parsed = parse_profile(&lines, "personal");
        assert_eq!(parsed.fields.nickname, "大字昵称");
    }

    #[test]
    fn test_manager_page_assigns_by_geometry_with_missing_row() {
        let cols = ManagerColumns {
            name_x: 300.0,
            remark_x: 385.0,
            tags_x: 500.0,
            permission_x: 620.0,
        };
        let mut lines = Vec::new();
        for i in 0..10 {
            if i == 2 || i == 7 {
                continue;
            }
            let row_y = 96.0 + i as f64 * 48.0;
            lines.push(line(&format!("名字_{}", i + 1), 300.0, row_y, 60.0, 20.0));
        }

        let rows = parse_manager_page(&lines, 96.0, 48.0, 10, &cols);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[2].name, None);
        assert_eq!(rows[7].name, None);
        assert_eq!(rows[0].name.as_deref(), Some("名字_1"));
        assert_eq!(rows[1].name.as_deref(), Some("名字_2"));
        assert_eq!(rows[3].name.as_deref(), Some("名字_4"));
        assert_eq!(rows[9].name.as_deref(), Some("名字_10"));
        assert!((rows[2].row_y - 192.0).abs() < 1e-3);
    }

    #[test]
    fn test_manager_columns_split() {
        let cols = ManagerColumns {
            name_x: 300.0,
            remark_x: 385.0,
            tags_x: 500.0,
            permission_x: 620.0,
        };
        let row_y = 96.0;
        let lines = vec![
            line("张三", 300.0, row_y, 40.0, 20.0),
            line("老张", 385.0, row_y, 40.0, 20.0),
            line("客户, VIP", 500.0, row_y, 80.0, 20.0),
            line("仅聊天", 620.0, row_y, 50.0, 20.0),
        ];

        let rows = parse_manager_page(&lines, 96.0, 48.0, 1, &cols);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name.as_deref(), Some("张三"));
        assert_eq!(row.remark.as_deref(), Some("老张"));
        assert_eq!(row.tags, vec!["客户".to_string(), "VIP".to_string()]);
        assert_eq!(row.permission.as_deref(), Some("仅聊天"));
    }

    /// 识别结果里值行排在字段名行前面时（真实 OCR 顺序不保证），值"企业微信"不能被当成字段"企业"。
    #[test]
    fn test_value_starting_with_label_word_is_not_a_label() {
        let lines = vec![
            line("企业微信", 505.0, 340.0, 59.0, 14.0),
            line("来源", 430.0, 341.0, 28.0, 14.0),
            line("测试企业有限公司", 506.0, 369.0, 137.0, 15.0),
            line("企业", 430.0, 367.0, 30.0, 15.0),
        ];
        let parsed = parse_profile(&lines, "work");
        assert_eq!(parsed.fields.source.as_deref(), Some("企业微信"));
        assert_eq!(parsed.fields.enterprise.as_deref(), Some("测试企业有限公司"));
    }

    /// 联系人名字里含"联系人""群聊"等字样时不能被当成分组标题。
    #[test]
    fn test_contact_named_like_group_is_not_a_header() {
        let lines = vec![
            line("> 公众号、服务号", 77.0, 113.0, 119.0, 15.0),
            line("33", 272.0, 116.0, 14.0, 10.0),
            line("群聊小助手", 133.0, 266.0, 80.0, 15.0),
            line("我的联系人管家", 133.0, 320.0, 90.0, 15.0),
            line("联系人", 76.0, 186.0, 62.0, 15.0),
            line("22", 272.0, 188.0, 15.0, 10.0),
        ];
        let headers = parse_group_headers(&lines);
        assert_eq!(
            headers,
            vec![("official".to_string(), 33, 113.0), ("personal".to_string(), 22, 186.0)]
        );
    }

    #[test]
    fn test_group_headers_counts() {
        let lines = vec![
            line("▶ 公众号、服务号", 100.0, 113.0, 120.0, 20.0),
            line("33", 272.0, 113.0, 25.0, 20.0),
            line("▼ 企业微信联系人", 100.0, 149.0, 130.0, 20.0),
            line("14", 272.0, 149.0, 25.0, 20.0),
            line("联系人", 100.0, 185.0, 80.0, 20.0),
            line("22", 272.0, 185.0, 25.0, 20.0),
        ];

        let headers = parse_group_headers(&lines);
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].0, "official");
        assert_eq!(headers[0].1, 33);
        assert!((headers[0].2 - 113.0).abs() < 1e-3);

        assert_eq!(headers[1].0, "work");
        assert_eq!(headers[1].1, 14);
        assert!((headers[1].2 - 149.0).abs() < 1e-3);

        assert_eq!(headers[2].0, "personal");
        assert_eq!(headers[2].1, 22);
        assert!((headers[2].2 - 185.0).abs() < 1e-3);
    }
}
