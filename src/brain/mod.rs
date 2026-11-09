pub mod builtin;
pub mod documents;
pub mod model_api;
pub mod understanding;

use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ContactInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ContextMedia {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub understanding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ContextItem {
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<ContextMedia>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FailedAttachment {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MediaRef {
    pub kind: String,
    pub media_id: Option<String>,
    pub filename: Option<String>,
    pub understanding: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub contact: ContactInfo,
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_chat_kind")]
    pub chat_kind: String,
    #[serde(default)]
    pub context: Vec<ContextItem>,
    #[serde(default)]
    pub images: Vec<PathBuf>,
    #[serde(default)]
    pub files: Vec<Value>,
    #[serde(default)]
    pub notes: Vec<Value>,
    #[serde(default)]
    pub failed: Vec<FailedAttachment>,
    #[serde(default)]
    pub image_count: Option<usize>,
    #[serde(default)]
    pub earlier_images: usize,
    #[serde(default = "default_max_attachment_mb")]
    pub max_attachment_mb: u64,
    #[serde(skip)]
    pub media_refs: Vec<MediaRef>,
}

fn default_mode() -> String {
    "service".to_string()
}

fn default_chat_kind() -> String {
    "private".to_string()
}

fn default_max_attachment_mb() -> u64 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainReply {
    pub reply: String,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

pub trait Brain: Send + Sync {
    fn reply(&self, turn: &Turn) -> impl std::future::Future<Output = Result<BrainReply>> + Send;
}

pub use builtin::BuiltinBrain;
