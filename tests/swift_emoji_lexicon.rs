//! Emoji Lexicon format and content tests.
//! Migrated from `tests/test_emoji_lexicon.py`.

use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn load_lexicon() -> Value {
    let path = repo_root()
        .join("native")
        .join("layout")
        .join("emoji-lexicon.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("invalid json in emoji-lexicon.json")
}

fn is_valid_emoji_name(s: &str) -> bool {
    if !s.starts_with('[') || !s.ends_with(']') || s.len() < 3 {
        return false;
    }
    let inner = &s[1..s.len() - 1];
    !inner.is_empty() && !inner.contains('[') && !inner.contains(']')
}

fn is_valid_hex_hash(s: &str) -> bool {
    s.len() == 16
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn test_header() {
    let doc = load_lexicon();
    assert_eq!(doc.get("schema").and_then(|v| v.as_i64()), Some(1));
    let version = doc.get("wechat_version").and_then(|v| v.as_str());
    assert!(version.is_some() && !version.unwrap().is_empty());
}

#[test]
fn test_every_entry_is_complete() {
    let doc = load_lexicon();
    let entries = doc
        .get("entries")
        .and_then(|v| v.as_object())
        .expect("entries must be an object");

    let expected_keys: HashSet<&str> = ["templates", "meaning", "chat_uses", "service_hint"]
        .into_iter()
        .collect();

    for (name, entry) in entries {
        assert!(
            is_valid_emoji_name(name),
            "emoji name '{}' does not match pattern ^\\[[^\\[\\]]+\\]$",
            name
        );
        let entry_obj = entry
            .as_object()
            .unwrap_or_else(|| panic!("entry {} is not an object", name));

        assert!(
            entry_obj.get("templates").map_or(false, |v| v.is_object()),
            "entry {} missing templates object",
            name
        );

        let meaning = entry_obj
            .get("meaning")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            !meaning.trim().is_empty(),
            "entry {} meaning is empty",
            name
        );

        let chat_uses = entry_obj.get("chat_uses").and_then(|v| v.as_array());
        assert!(
            chat_uses.is_some(),
            "entry {} chat_uses is not an array",
            name
        );
        let chat_uses = chat_uses.unwrap();
        assert!(!chat_uses.is_empty(), "entry {} chat_uses is empty", name);
        for u in chat_uses {
            let s = u.as_str().unwrap_or("");
            assert!(
                !s.trim().is_empty(),
                "entry {} chat_use element is empty",
                name
            );
        }

        let service_hint = entry_obj
            .get("service_hint")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            !service_hint.trim().is_empty(),
            "entry {} service_hint is empty",
            name
        );

        let actual_keys: HashSet<&str> = entry_obj.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            actual_keys, expected_keys,
            "entry {} has mismatching keys",
            name
        );
    }
}

#[test]
fn test_easily_misread_emoji_carry_their_wechat_sense() {
    let doc = load_lexicon();
    let entries = doc
        .get("entries")
        .and_then(|v| v.as_object())
        .expect("entries must be an object");

    let smile = entries.get("[微笑]").expect("[微笑] not found");
    let smile_uses: String = smile["chat_uses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        smile_uses.contains("不满"),
        "微笑 chat_uses should contain 不满"
    );
    assert!(
        smile["service_hint"]
            .as_str()
            .unwrap()
            .contains("不要当作满意"),
        "微笑 service_hint should contain 不要当作满意"
    );

    let doge = entries.get("[旺柴]").expect("[旺柴] not found");
    assert!(
        doge["service_hint"].as_str().unwrap().contains("玩笑"),
        "旺柴 service_hint should contain 玩笑"
    );

    let question = entries.get("[疑问]").expect("[疑问] not found");
    assert!(
        question["service_hint"]
            .as_str()
            .unwrap()
            .contains("必须回复"),
        "疑问 service_hint should contain 必须回复"
    );

    let redpacket = entries.get("[红包]").expect("[红包] not found");
    assert!(
        redpacket["meaning"]
            .as_str()
            .unwrap()
            .contains("不是真红包"),
        "红包 meaning should contain 不是真红包"
    );
}

#[test]
fn test_templates_are_perceptual_hashes_when_present() {
    let doc = load_lexicon();
    let entries = doc
        .get("entries")
        .and_then(|v| v.as_object())
        .expect("entries must be an object");

    for (name, entry) in entries {
        let templates = entry["templates"].as_object().unwrap();
        for (scale, value) in templates {
            assert!(
                scale == "1x" || scale == "2x",
                "entry {} invalid scale {}",
                name,
                scale
            );
            let hash_str = value.as_str().unwrap_or("");
            assert!(
                is_valid_hex_hash(hash_str),
                "entry {} scale {} hash '{}' is not a 16-hex perceptual hash",
                name,
                scale,
                hash_str
            );
        }
    }
}
