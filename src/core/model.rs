use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bubble {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub direction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_pending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Frame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<Value>,
    #[serde(default)]
    pub title_lines: Vec<String>,
    #[serde(default)]
    pub bubbles: Vec<Bubble>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_message_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipped_bubble: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_cached: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_bubble_roundtrip_preserves_unknown_fields() {
        let raw = json!({
            "text": "[图片]",
            "digest": "image:50:40",
            "direction": "inbound_visual",
            "kind": "image",
            "phash": "0000000000000000",
            "media_id": "aaaaaaaa",
            "custom_metadata_key": "some_extra_val"
        });
        let bubble: Bubble = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(bubble.text, "[图片]");
        assert_eq!(bubble.kind.as_deref(), Some("image"));
        assert_eq!(bubble.extra.get("custom_metadata_key"), Some(&json!("some_extra_val")));

        let serialized = serde_json::to_value(&bubble).unwrap();
        assert_eq!(serialized.get("custom_metadata_key"), Some(&json!("some_extra_val")));
    }

    #[test]
    fn test_frame_roundtrip_preserves_unknown_fields() {
        let raw = json!({
            "profile": "wechat-4-dark",
            "scale": 1,
            "title_lines": ["午夜咖啡"],
            "bubbles": [{
                "text": "你好",
                "digest": "d1",
                "direction": "inbound_visual"
            }],
            "future_screen_check": {"score": 99}
        });
        let frame: Frame = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(frame.profile.as_deref(), Some("wechat-4-dark"));
        assert_eq!(frame.title_lines, vec!["午夜咖啡"]);
        assert_eq!(frame.bubbles.len(), 1);
        assert_eq!(frame.bubbles[0].text, "你好");
        assert_eq!(frame.extra.get("future_screen_check"), Some(&json!({"score": 99})));

        let serialized = serde_json::to_value(&frame).unwrap();
        assert_eq!(serialized.get("future_screen_check"), Some(&json!({"score": 99})));
    }
}
