use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 轮次对外认领信息（Spec 8 契约）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimInfo {
    pub claim_token: String,
    pub lease_until: u64,
    pub deadline_at: u64,
}

/// 媒体查询接口：对外暴露前将本地路径映射为媒体 ID
pub trait MediaLookup {
    fn media_id_for_path(&self, path: &Path) -> Option<String>;
}

/// 基于 SQLite 历史库与 media 根目录实现的媒体解析器
pub struct HistoryMediaLookup<'a> {
    pub conn: &'a rusqlite::Connection,
    pub media_root: PathBuf,
}

impl<'a> MediaLookup for HistoryMediaLookup<'a> {
    fn media_id_for_path(&self, path: &Path) -> Option<String> {
        let root_canon = self.media_root.canonicalize().ok()?;
        let path_canon = path.canonicalize().ok()?;
        let rel = path_canon.strip_prefix(&root_canon).ok()?;
        let rel_str = rel.to_str()?;
        let mut stmt = self
            .conn
            .prepare("SELECT media_id FROM media WHERE path = ?1")
            .ok()?;
        stmt.query_row([rel_str], |row| row.get(0)).ok()
    }
}

/// 将 Unix 秒级时间戳格式化为 RFC 3339 UTC 字符串（使用整数日历算法，不引入外部依赖）
pub fn rfc3339(secs: u64) -> String {
    let sec = (secs % 60) as u32;
    let rem = secs / 60;
    let min = (rem % 60) as u32;
    let hour = ((rem / 60) % 24) as u32;
    let days = (rem / (60 * 24)) as i64;

    // Howard Hinnant's civil_from_days algorithm
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// 将内部 Turn 与认领上下文转为对外 v1 契约数据结构
/// Failure reason recorded for the same attachment, matched by its original label.
fn failure_for<'a>(
    turn: &'a crate::brain::Turn,
    media_ref: &crate::brain::MediaRef,
) -> Option<&'a str> {
    let label = media_ref.label.as_deref()?;
    turn.failed
        .iter()
        .find(|f| f.label == label)
        .and_then(|f| f.reason.as_deref())
}

pub fn turn_v1(
    turn: &crate::brain::Turn,
    claim: &ClaimInfo,
    contact_id: &str,
    media: &dyn MediaLookup,
) -> Value {
    let mut messages = Vec::new();
    let mut unavailable = Vec::new();

    // 1. 本轮文本（若非空）
    if !turn.text.is_empty() {
        messages.push(serde_json::json!({
            "kind": "text",
            "message_id": Value::Null,
            "source": "desktop_ocr",
            "text": turn.text,
        }));
    }

    // 2 & 3. 本轮图片与文件：media_refs 非空时优先使用，为空时保留基于 images/files 路径反查的后备
    if !turn.media_refs.is_empty() {
        for mr in &turn.media_refs {
            let kind = mr.kind.as_str();
            if kind != "image" && kind != "file" {
                continue;
            }
            // The same attachment may also be listed in `failed`; report it once.
            let failure = failure_for(turn, mr);
            let Some(ref mid) = mr.media_id else {
                unavailable.push(serde_json::json!({
                    "kind": kind,
                    "reason": failure.unwrap_or("media_id_unknown"),
                }));
                continue;
            };
            let mut msg = serde_json::json!({
                "kind": kind,
                "media_id": mid,
                "message_id": Value::Null,
            });
            let fields = msg.as_object_mut().unwrap();
            if kind == "file" {
                fields.insert(
                    "filename".to_string(),
                    serde_json::json!(mr.filename.as_deref().unwrap_or("")),
                );
            }
            if let Some(ref und) = mr.understanding {
                fields.insert("understanding".to_string(), serde_json::json!(und));
            }
            if let Some(reason) = failure {
                fields.insert("processing_failed".to_string(), serde_json::json!(reason));
            }
            messages.push(msg);
        }
    } else {
        // 后备路径：按 images 路径反查
        for img_path in &turn.images {
            match media.media_id_for_path(img_path) {
                Some(media_id) => {
                    messages.push(serde_json::json!({
                        "kind": "image",
                        "media_id": media_id,
                        "message_id": Value::Null,
                    }));
                }
                None => {
                    unavailable.push(serde_json::json!({
                        "kind": "image",
                        "reason": "media_not_in_history",
                    }));
                }
            }
        }

        // 后备路径：按 files 对象与路径反查
        for file_val in &turn.files {
            let filename = file_val
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let media_id = if let Some(mid) = file_val.get("media_id").and_then(|v| v.as_str()) {
                Some(mid.to_string())
            } else if let Some(p) = file_val.get("path").and_then(|v| v.as_str()) {
                media.media_id_for_path(Path::new(p))
            } else {
                None
            };

            match media_id {
                Some(mid) => {
                    messages.push(serde_json::json!({
                        "filename": filename,
                        "kind": "file",
                        "media_id": mid,
                        "message_id": Value::Null,
                    }));
                }
                None => {
                    unavailable.push(serde_json::json!({
                        "kind": "file",
                        "reason": "media_not_in_history",
                    }));
                }
            }
        }
    }

    // 4. 附件接收/解析失败项
    for failed in &turn.failed {
        if turn
            .media_refs
            .iter()
            .any(|mr| mr.label.as_deref() == Some(failed.label.as_str()))
        {
            continue; // already reported with its attachment above
        }
        let kind = failed.kind.as_deref().unwrap_or("unknown");
        let reason = failed.reason.as_deref().unwrap_or("unknown");
        unavailable.push(serde_json::json!({
            "kind": kind,
            "reason": reason,
        }));
    }

    // 5. 上下文角色映射：user/customer -> customer, assistant -> assistant, 其他丢弃
    let mut context = Vec::new();
    for item in &turn.context {
        let role = match item.role.as_str() {
            "user" | "customer" => "customer",
            "assistant" => "assistant",
            _ => continue,
        };

        if let Some(ref m) = item.media {
            let kind = m.kind.as_deref().unwrap_or("image");
            let media_id_val = m
                .path
                .as_deref()
                .and_then(|p| media.media_id_for_path(p))
                .map(Value::String)
                .unwrap_or(Value::Null);

            let mut obj = serde_json::Map::new();
            obj.insert("kind".to_string(), serde_json::json!(kind));
            obj.insert("media_id".to_string(), media_id_val);
            obj.insert("message_id".to_string(), Value::Null);
            obj.insert("role".to_string(), serde_json::json!(role));
            obj.insert("text".to_string(), serde_json::json!(item.text));
            context.push(Value::Object(obj));
        } else {
            context.push(serde_json::json!({
                "kind": "text",
                "message_id": Value::Null,
                "role": role,
                "text": item.text,
            }));
        }
    }

    serde_json::json!({
        "account": {
            "account_id": turn.account_id,
        },
        "api_version": "1",
        "claim": {
            "claim_token": claim.claim_token,
            "lease_expires_at": rfc3339(claim.lease_until),
        },
        "contact": {
            "contact_id": contact_id,
            "display_name": turn.contact.name,
            "kind": turn.contact.kind,
        },
        "context": context,
        "deadline_at": rfc3339(claim.deadline_at),
        "messages": messages,
        "turn_id": turn.turn_id,
        "unavailable": unavailable,
    })
}

pub fn turn_v2(
    turn: &crate::brain::Turn,
    claim: &ClaimInfo,
    contact_id: &str,
    media: &dyn MediaLookup,
    snapshot: Value,
) -> Value {
    let mut v = turn_v1(turn, claim, contact_id, media);
    v["api_version"] = serde_json::json!("2");
    v["brain_snapshot"] = snapshot;
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{ContactInfo, ContextItem, ContextMedia, FailedAttachment, Turn};
    use std::collections::BTreeMap;
    use std::collections::HashMap;

    struct MockMediaLookup {
        mapping: HashMap<PathBuf, String>,
    }

    impl MediaLookup for MockMediaLookup {
        fn media_id_for_path(&self, path: &Path) -> Option<String> {
            self.mapping.get(path).cloned()
        }
    }

    fn sort_json(val: &Value) -> Value {
        match val {
            Value::Object(map) => {
                let sorted: BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), sort_json(v))).collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(arr) => Value::Array(arr.iter().map(sort_json).collect()),
            other => other.clone(),
        }
    }

    fn assert_json_matches_snapshot(actual: &Value, snapshot_rel_path: &str) {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let full_path = manifest_dir
            .join("src/core/open_api/snapshots")
            .join(snapshot_rel_path);
        let expected_raw = std::fs::read_to_string(&full_path)
            .unwrap_or_else(|e| panic!("failed to read snapshot at {}: {e}", full_path.display()));
        let expected: Value = serde_json::from_str(&expected_raw).unwrap_or_else(|e| {
            panic!(
                "failed to parse snapshot JSON at {}: {e}",
                full_path.display()
            )
        });

        let sorted_actual = sort_json(actual);
        let sorted_expected = sort_json(&expected);

        let actual_pretty = serde_json::to_string_pretty(&sorted_actual).unwrap();
        let expected_pretty = serde_json::to_string_pretty(&sorted_expected).unwrap();

        assert_eq!(
            actual_pretty, expected_pretty,
            "Snapshot mismatch for {snapshot_rel_path}"
        );
    }

    #[test]
    fn rfc3339_known_values() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1700000000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339(951782400), "2000-02-29T00:00:00Z"); // 闰日
    }

    #[test]
    fn turn_v1_snapshot_text() {
        let turn = Turn {
            turn_id: "t_1001".to_string(),
            account_id: "wx_bot_main".to_string(),
            contact: ContactInfo {
                name: "张三".to_string(),
                kind: "private".to_string(),
            },
            text: "你好，请问客服在吗？".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_claim_text".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(),
        };

        let result = turn_v1(&turn, &claim, "c_zhangsan_01", &lookup);
        assert_json_matches_snapshot(&result, "turn_v1_text.json");
    }

    #[test]
    fn turn_v2_snapshot_test() {
        let turn = Turn {
            turn_id: "t_1001".to_string(),
            account_id: "wx_bot_main".to_string(),
            contact: ContactInfo {
                name: "张三".to_string(),
                kind: "private".to_string(),
            },
            text: "你好，请问客服在吗？".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_claim_text".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(),
        };
        let snapshot = serde_json::json!({
            "snapshot_id": "snap_v2_1001",
            "assembly_sha256": "sha_v2_snap",
            "compiled": "你好，请问客服在吗？"
        });

        let result = turn_v2(&turn, &claim, "c_zhangsan_01", &lookup, snapshot);
        assert_json_matches_snapshot(&result, "turn_v2.json");
    }

    #[test]
    fn turn_v1_snapshot_images() {
        let img1 = PathBuf::from("/media/img1.jpg");
        let img2 = PathBuf::from("/media/img2.jpg");
        let mut mapping = HashMap::new();
        mapping.insert(img1.clone(), "media_img_001".to_string());
        mapping.insert(img2.clone(), "media_img_002".to_string());

        let turn = Turn {
            turn_id: "t_1002".to_string(),
            account_id: "wx_bot_main".to_string(),
            contact: ContactInfo {
                name: "李四".to_string(),
                kind: "private".to_string(),
            },
            text: "发了两张图片和一个文件，还有一张图没加载出来".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![img1, img2],
            files: vec![serde_json::json!({
                "filename": "价格表.xlsx",
                "media_id": "media_file_price_01",
            })],
            notes: vec![],
            failed: vec![FailedAttachment {
                label: "[图片]".to_string(),
                kind: Some("image".to_string()),
                reason: Some("download_timeout".to_string()),
                hint: Some("请重试".to_string()),
            }],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_claim_img".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup { mapping };

        let result = turn_v1(&turn, &claim, "c_lisi_02", &lookup);
        assert_json_matches_snapshot(&result, "turn_v1_images.json");
    }

    #[test]
    fn turn_v1_snapshot_group_context() {
        let ctx_img = PathBuf::from("/media/ctx_err.png");
        let mut mapping = HashMap::new();
        mapping.insert(ctx_img.clone(), "media_ctx_err_99".to_string());

        let turn = Turn {
            turn_id: "t_1003".to_string(),
            account_id: "wx_bot_main".to_string(),
            contact: ContactInfo {
                name: "VIP客户群".to_string(),
                kind: "group".to_string(),
            },
            text: "请问这个报错怎么解决？".to_string(),
            mode: "service".to_string(),
            chat_kind: "group".to_string(),
            context: vec![
                ContextItem {
                    role: "user".to_string(),
                    text: "你好，我想问个问题".to_string(),
                    media: None,
                },
                ContextItem {
                    role: "assistant".to_string(),
                    text: "您好，请发一下具体的报错信息或截图".to_string(),
                    media: None,
                },
                ContextItem {
                    role: "user".to_string(),
                    text: "[图片]".to_string(),
                    media: Some(ContextMedia {
                        kind: Some("image".to_string()),
                        filename: None,
                        understanding: Some("错误日志截图".to_string()),
                        path: Some(ctx_img),
                    }),
                },
            ],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_claim_grp".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup { mapping };

        let result = turn_v1(&turn, &claim, "c_group_vip_03", &lookup);
        assert_json_matches_snapshot(&result, "turn_v1_group_context.json");
    }

    #[test]
    fn turn_v1_never_contains_local_paths() {
        let state_dir = format!("{}/testuser/.wechat-rpa/state", "/Users");
        let local_img = PathBuf::from(format!("{state_dir}/history/media/images/secret.jpg"));
        let local_file = format!("{state_dir}/history/media/docs/secret.pdf");

        let mut mapping = HashMap::new();
        mapping.insert(local_img.clone(), "media_safe_img_777".to_string());
        mapping.insert(PathBuf::from(&local_file), "media_safe_doc_888".to_string());

        let turn = Turn {
            turn_id: "t_path_check".to_string(),
            account_id: "wx_acc".to_string(),
            contact: ContactInfo {
                name: "Bob".to_string(),
                kind: "private".to_string(),
            },
            text: "Hello".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![ContextItem {
                role: "user".to_string(),
                text: "Context item".to_string(),
                media: Some(ContextMedia {
                    kind: Some("image".to_string()),
                    filename: None,
                    understanding: None,
                    path: Some(local_img.clone()),
                }),
            }],
            images: vec![local_img],
            files: vec![serde_json::json!({
                "filename": "document.pdf",
                "path": local_file,
            })],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_safe_test".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup { mapping };

        let output = turn_v1(&turn, &claim, "c_bob", &lookup);
        let serialized = serde_json::to_string(&output).unwrap();

        assert!(!serialized.contains("/Users/"), "must not contain /Users/");
        assert!(
            !serialized.contains(&state_dir),
            "must not contain state_dir"
        );

        // 递归检查任何字符串值不以 '/' 开头
        fn verify_no_leading_slash(val: &Value) {
            match val {
                Value::String(s) => {
                    assert!(
                        !s.starts_with('/'),
                        "String value should not start with slash (local path): {s}"
                    );
                }
                Value::Array(arr) => {
                    for item in arr {
                        verify_no_leading_slash(item);
                    }
                }
                Value::Object(map) => {
                    for (_k, v) in map {
                        verify_no_leading_slash(v);
                    }
                }
                _ => {}
            }
        }
        verify_no_leading_slash(&output);
    }

    #[test]
    fn unresolvable_image_goes_to_unavailable() {
        let unresolvable_img = PathBuf::from("/nonexistent/outside/image.png");
        let turn = Turn {
            turn_id: "t_unresolvable".to_string(),
            account_id: "wx_acc".to_string(),
            contact: ContactInfo {
                name: "Charlie".to_string(),
                kind: "private".to_string(),
            },
            text: "Testing unresolvable".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![unresolvable_img],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_test".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(), // 空 lookup，任何路径都返回 None
        };

        let result = turn_v1(&turn, &claim, "c_charlie", &lookup);
        let messages = result.get("messages").and_then(|v| v.as_array()).unwrap();
        assert!(
            messages
                .iter()
                .all(|m| m.get("kind").and_then(|k| k.as_str()) != Some("image")),
            "Unresolvable image should not appear in messages"
        );

        let unavailable = result
            .get("unavailable")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(
            unavailable,
            &vec![serde_json::json!({
                "kind": "image",
                "reason": "media_not_in_history"
            })]
        );
    }

    #[test]
    fn context_roles_mapped() {
        let turn = Turn {
            turn_id: "t_roles".to_string(),
            account_id: "wx_acc".to_string(),
            contact: ContactInfo {
                name: "Dave".to_string(),
                kind: "private".to_string(),
            },
            text: "Testing roles".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![
                ContextItem {
                    role: "user".to_string(),
                    text: "User message".to_string(),
                    media: None,
                },
                ContextItem {
                    role: "assistant".to_string(),
                    text: "Assistant response".to_string(),
                    media: None,
                },
                ContextItem {
                    role: "system".to_string(),
                    text: "System instruction".to_string(),
                    media: None,
                },
                ContextItem {
                    role: "other".to_string(),
                    text: "Other message".to_string(),
                    media: None,
                },
            ],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![],
        };
        let claim = ClaimInfo {
            claim_token: "tok_roles".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(),
        };

        let result = turn_v1(&turn, &claim, "c_dave", &lookup);
        let context = result.get("context").and_then(|v| v.as_array()).unwrap();
        assert_eq!(context.len(), 2);
        assert_eq!(
            context[0].get("role").and_then(|r| r.as_str()),
            Some("customer")
        );
        assert_eq!(
            context[1].get("role").and_then(|r| r.as_str()),
            Some("assistant")
        );
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "contract-test-{}-{}",
                name,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn history_media_lookup_resolves_and_denies_outside() {
        let temp_dir = TempDir::new("media-lookup");
        let media_root = temp_dir.path().join("media");
        std::fs::create_dir_all(&media_root).unwrap();

        // 真实存在的文件
        let inside_file = media_root.join("test.jpg");
        std::fs::write(&inside_file, b"test image data").unwrap();

        // 根目录之外的文件
        let outside_file = temp_dir.path().join("outside.jpg");
        std::fs::write(&outside_file, b"outside data").unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE media (media_id TEXT PRIMARY KEY, path TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO media (media_id, path) VALUES (?1, ?2)",
            ["mid_12345", "test.jpg"],
        )
        .unwrap();

        let lookup = HistoryMediaLookup {
            conn: &conn,
            media_root: media_root.clone(),
        };

        // 内部合法路径查得到
        assert_eq!(
            lookup.media_id_for_path(&inside_file),
            Some("mid_12345".to_string())
        );

        // 外部路径必须拒绝 (None)
        assert_eq!(lookup.media_id_for_path(&outside_file), None);

        // 不存在的文件必须拒绝 (None)
        assert_eq!(
            lookup.media_id_for_path(&media_root.join("missing.jpg")),
            None
        );
    }

    #[test]
    fn turn_serialization_unchanged_by_media_refs() {
        use crate::brain::MediaRef;
        let turn = Turn {
            turn_id: "t_serde".to_string(),
            account_id: "acc_test".to_string(),
            contact: ContactInfo {
                name: "Alice".to_string(),
                kind: "private".to_string(),
            },
            text: "Hello".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![MediaRef {
                kind: "image".to_string(),
                media_id: Some("mid_001".to_string()),
                filename: None,
                understanding: Some("理解文字".to_string()),
                label: None,
            }],
        };

        let val = serde_json::to_value(&turn).unwrap();
        assert!(
            val.get("media_refs").is_none(),
            "Turn 序列化结果绝对不能包含 media_refs 键"
        );
    }

    #[test]
    fn turn_v1_prefers_media_refs() {
        use crate::brain::MediaRef;
        let turn = Turn {
            turn_id: "t_1004".to_string(),
            account_id: "wx_bot_main".to_string(),
            contact: ContactInfo {
                name: "王五".to_string(),
                kind: "private".to_string(),
            },
            text: "带理解的图片和正常附件".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![
                MediaRef {
                    kind: "image".to_string(),
                    media_id: Some("media_img_with_und_01".to_string()),
                    filename: None,
                    understanding: Some("一张白色咖啡杯特写".to_string()),
                    label: None,
                },
                MediaRef {
                    kind: "file".to_string(),
                    media_id: Some("media_file_doc_02".to_string()),
                    filename: Some("产品手册.pdf".to_string()),
                    understanding: None,
                    label: None,
                },
                MediaRef {
                    kind: "image".to_string(),
                    media_id: None,
                    filename: None,
                    understanding: None,
                    label: None,
                },
            ],
        };
        let claim = ClaimInfo {
            claim_token: "tok_claim_mr".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(),
        };

        let result = turn_v1(&turn, &claim, "c_wangwu_05", &lookup);
        assert_json_matches_snapshot(&result, "turn_v1_media_refs.json");
    }

    #[test]
    fn failed_attachment_merges_with_media_ref_by_label() {
        use crate::brain::MediaRef;
        let failed = |label: &str, kind: &str, reason: &str| FailedAttachment {
            label: label.to_string(),
            kind: Some(kind.to_string()),
            reason: Some(reason.to_string()),
            hint: None,
        };
        let media_ref = |label: &str, media_id: Option<&str>| MediaRef {
            kind: "image".to_string(),
            media_id: media_id.map(str::to_string),
            filename: None,
            understanding: None,
            label: Some(label.to_string()),
        };
        let turn = Turn {
            turn_id: "t_merge".to_string(),
            account_id: "wx_acc".to_string(),
            contact: ContactInfo {
                name: "测试客户".to_string(),
                kind: "private".to_string(),
            },
            text: String::new(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![
                failed("图1", "image", "too_large"),
                failed("图2", "image", "download_failed"),
                failed("其他附件", "file", "unsupported"),
            ],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![media_ref("图1", Some("md_a")), media_ref("图2", None)],
        };
        let claim = ClaimInfo {
            claim_token: "tok".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let result = turn_v1(
            &turn,
            &claim,
            "c_x",
            &MockMediaLookup {
                mapping: HashMap::new(),
            },
        );

        // 有媒体 ID 的图片只出现在 messages，附带本机处理失败的原因；不再重复出现在 unavailable。
        assert_eq!(
            result["messages"],
            serde_json::json!([{"kind":"image","media_id":"md_a","message_id":null,"processing_failed":"too_large"}])
        );
        // 没有媒体 ID 的图片只出现一次，并使用真实的失败原因；对不上附件的失败项保持原样。
        assert_eq!(
            result["unavailable"],
            serde_json::json!([
                {"kind":"image","reason":"download_failed"},
                {"kind":"file","reason":"unsupported"}
            ])
        );
    }

    #[test]
    fn turn_v1_media_ref_without_id_is_unavailable() {
        use crate::brain::MediaRef;
        let turn = Turn {
            turn_id: "t_no_id".to_string(),
            account_id: "wx_acc".to_string(),
            contact: ContactInfo {
                name: "Eve".to_string(),
                kind: "private".to_string(),
            },
            text: "test".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: vec![],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: None,
            earlier_images: 0,
            max_attachment_mb: 50,
            snapshot: None,
            media_refs: vec![
                MediaRef {
                    kind: "image".to_string(),
                    media_id: None,
                    filename: None,
                    understanding: None,
                    label: None,
                },
                MediaRef {
                    kind: "file".to_string(),
                    media_id: None,
                    filename: Some("empty.bin".to_string()),
                    understanding: None,
                    label: None,
                },
            ],
        };
        let claim = ClaimInfo {
            claim_token: "tok_no_id".to_string(),
            lease_until: 1700000060,
            deadline_at: 1700000120,
        };
        let lookup = MockMediaLookup {
            mapping: HashMap::new(),
        };

        let result = turn_v1(&turn, &claim, "c_eve", &lookup);
        let messages = result.get("messages").and_then(|v| v.as_array()).unwrap();
        // 只有 1 条文本消息，没有 image 和 file
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].get("kind").and_then(|k| k.as_str()),
            Some("text")
        );

        let unavailable = result
            .get("unavailable")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(
            unavailable,
            &vec![
                serde_json::json!({
                    "kind": "image",
                    "reason": "media_id_unknown"
                }),
                serde_json::json!({
                    "kind": "file",
                    "reason": "media_id_unknown"
                })
            ]
        );
    }
}
