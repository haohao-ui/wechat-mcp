use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    accounts::{AccountDescriptor, AccountKey, Capabilities},
    wechat,
};

pub type AdapterFuture<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

pub enum Operation {
    Focus,
    ListConversations,
    ReadMessages {
        recipient: Option<String>,
    },
    SendMessage {
        recipient: String,
        message: String,
    },
    WaitForMessage {
        recipient: Option<String>,
        timeout_seconds: u64,
    },
}

pub trait MessageAdapter: Send + Sync {
    fn account(&self) -> &AccountDescriptor;
    fn execute(&self, operation: Operation) -> AdapterFuture<'_>;
}

pub struct WechatDesktop {
    account: AccountDescriptor,
    desktop: Arc<Mutex<()>>,
}

impl WechatDesktop {
    pub fn new(account_id: String, desktop: Arc<Mutex<()>>) -> Self {
        Self {
            account: AccountDescriptor {
                key: AccountKey {
                    platform: "wechat".into(),
                    account_id,
                },
                adapter: "wechat_desktop".into(),
                binding_id: "local-desktop:WeChat".into(),
                identity_verified: false,
                capabilities: Capabilities {
                    send_text: true,
                    visible_conversations: true,
                    visible_messages: true,
                    full_history: false,
                    delivery_receipts: false,
                    background_events: false,
                    background_observations: true,
                    source: "desktop_ocr".into(),
                },
            },
            desktop,
        }
    }
}

impl MessageAdapter for WechatDesktop {
    fn account(&self) -> &AccountDescriptor {
        &self.account
    }

    fn execute(&self, operation: Operation) -> AdapterFuture<'_> {
        Box::pin(async move {
            // UI focus, clipboard and screenshot files are shared across all requests.
            let _guard = self.desktop.lock().await;
            let _process_guard = crate::resource_lock::ResourceLock::desktop().await?;
            let data = match operation {
                Operation::Focus => {
                    let (x, y, width, height) = wechat::focus_wechat().await?;
                    json!({"status":"executed", "window":{"x":x,"y":y,"width":width,"height":height}})
                }
                Operation::ListConversations => {
                    let sessions = wechat::get_recent_sessions().await?;
                    let conversations: Vec<_> = sessions.into_iter().map(|s| json!({
                        "display_name":s.name, "stable_id":null, "time_text":s.time, "preview":s.preview,
                    })).collect();
                    json!({"conversations":conversations,"coverage":"visible_only","source":"desktop_ocr"})
                }
                Operation::ReadMessages { recipient } => {
                    let lines = wechat::read_chat_messages(recipient.as_deref()).await?;
                    json!({"recipient":recipient,"text_lines":lines,"coverage":"visible_only",
                        "source":"desktop_ocr","message_ids_available":false,"direction":"unknown"})
                }
                Operation::SendMessage { recipient, message } => {
                    if recipient.trim().is_empty() || message.trim().is_empty() {
                        bail!("recipient/message 不能为空");
                    }
                    wechat::send_message(&recipient, &message).await?;
                    json!({"recipient":recipient,"status":"triggered","delivery":"unknown",
                        "recipient_verified":true,"recipient_verification":"chat_title_ocr",
                        "recipient_verification_note":"标题 OCR 已匹配显示名称，仍不是微信 ID 级核验"})
                }
                Operation::WaitForMessage {
                    recipient,
                    timeout_seconds,
                } => {
                    let observation =
                        wechat::wait_for_new_message(recipient.as_deref(), timeout_seconds).await?;
                    json!({"recipient":recipient,"status":if observation.is_some() {"change_observed"} else {"timeout"},
                        "observation":observation,"source":"desktop_ocr","incoming_verified":false})
                }
            };
            Ok(
                json!({"account":self.account.key,"identity_verified":self.account.identity_verified,"data":data}),
            )
        })
    }
}
