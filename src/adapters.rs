use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    accounts::{AccountDescriptor, AccountKey, Capabilities},
    desktop::actor::DesktopActor,
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
    SendFile {
        recipient: String,
        file: wechat::files::FileToSend,
    },
    WaitForMessage {
        recipient: Option<String>,
        timeout_seconds: u64,
    },
}

impl Operation {
    /// Desktop locks are process-wide. A hung OCR/AppleScript/helper call must
    /// not hold that lock forever, otherwise every later request queues behind it.
    fn timeout_budget(&self) -> Duration {
        match self {
            Self::Focus => Duration::from_secs(60),
            Self::ListConversations | Self::ReadMessages { .. } => Duration::from_secs(120),
            Self::SendMessage { .. } | Self::SendFile { .. } => Duration::from_secs(180),
            Self::WaitForMessage {
                timeout_seconds, ..
            } => Duration::from_secs(timeout_seconds.saturating_add(60)),
        }
    }

    fn timeout_message(&self, budget: Duration) -> String {
        let seconds = budget.as_secs();
        match self {
            Self::SendMessage { .. } | Self::SendFile { .. } => format!(
                "桌面发送操作超时（{seconds} 秒），结果未知；桌面锁已释放，不要盲目重发，请先核对发送状态与微信界面"
            ),
            Self::WaitForMessage { .. } => format!(
                "等待消息超时（{seconds} 秒），桌面锁已释放；请检查微信是否被弹窗或权限对话框阻塞"
            ),
            Self::Focus => format!(
                "聚焦微信超时（{seconds} 秒），桌面锁已释放；请检查辅助功能/自动化权限或系统弹窗"
            ),
            Self::ListConversations | Self::ReadMessages { .. } => format!(
                "读取微信界面超时（{seconds} 秒），桌面锁已释放；请检查微信是否被弹窗或权限对话框阻塞"
            ),
        }
    }
}

/// Runs one desktop operation under a hard deadline. Dropping the future also
/// drops the desktop lease and any `kill_on_drop` helper child it owns.
async fn bounded<T>(
    budget: Duration,
    message: String,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(budget, future).await {
        Ok(result) => result,
        Err(_) => bail!("{message}"),
    }
}

pub trait MessageAdapter: Send + Sync {
    fn account(&self) -> &AccountDescriptor;
    fn execute(&self, operation: Operation) -> AdapterFuture<'_>;
}

pub struct WechatDesktop {
    account: AccountDescriptor,
    desktop: DesktopActor,
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
            desktop: DesktopActor::new(desktop),
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
            let _lease = self.desktop.acquire().await?;
            let budget = operation.timeout_budget();
            let message = operation.timeout_message(budget);
            let data = bounded(budget, message, async move {
                let data = match operation {
                Operation::Focus => {
                    let (x, y, width, height) = wechat::focus_wechat().await?;
                    json!({"status":"executed", "window":{"x":x,"y":y,"width":width,"height":height}})
                }
                Operation::ListConversations => {
                    let read = wechat::read_session_observation().await?;
                    let conversations: Vec<_> = read.sessions.into_iter().map(|s| json!({
                        "display_name":s.name, "stable_id":null, "time_text":s.time, "preview":s.preview,
                    })).collect();
                    let mut data = json!({"conversations":conversations,"coverage":"visible_only","source":"desktop_ocr"});
                    if let Some(observation) = read.observation {
                        data["capture"] = json!({"driver":"native_stream","source":"desktop_visual",
                            "epoch":observation.epoch,"sequence":observation.sequence,
                            "captured_at_ms":observation.captured_at_ms,"ocr_cached":observation.ocr_cached});
                    }
                    data
                }
                Operation::ReadMessages { recipient } => {
                    let read = wechat::read_chat_observation(recipient.as_deref()).await?;
                    let mut data = json!({"recipient":recipient,"text_lines":read.text_lines,"coverage":"visible_only",
                        "source":"desktop_ocr","message_ids_available":false,"direction":"unknown"});
                    if let Some(observation) = read.observation {
                        data["observation"] = json!({"kind":"ocr_text_lines","source":"desktop_visual",
                            "incoming_verified":false,"snapshot":observation});
                    }
                    data
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
                Operation::SendFile { recipient, file } => {
                    wechat::send_file(&recipient, &file).await?;
                    json!({"recipient":recipient,"status":"triggered","delivery":"unknown",
                        "file":{"name":file.name,"size":file.size},
                        "recipient_verified":true,"recipient_verification":"chat_title_ocr",
                        "file_verification":"input_ocr_filename",
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
                Ok::<Value, anyhow::Error>(data)
            })
            .await?;
            Ok(
                json!({"account":self.account.key,"identity_verified":self.account.identity_verified,"data":data}),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn desktop_operation_timeouts_are_bounded() {
        assert_eq!(Operation::Focus.timeout_budget(), Duration::from_secs(60));
        assert_eq!(
            Operation::ListConversations.timeout_budget(),
            Duration::from_secs(120)
        );
        assert_eq!(
            Operation::ReadMessages { recipient: None }.timeout_budget(),
            Duration::from_secs(120)
        );
        assert_eq!(
            Operation::WaitForMessage {
                recipient: None,
                timeout_seconds: 300,
            }
            .timeout_budget(),
            Duration::from_secs(360)
        );
        let send = Operation::SendMessage {
            recipient: "someone".into(),
            message: "hello".into(),
        };
        assert_eq!(send.timeout_budget(), Duration::from_secs(180));
        assert!(
            send.timeout_message(Duration::from_secs(180))
                .contains("不要盲目重发")
        );
    }

    #[tokio::test]
    async fn bounded_timeout_drops_future_and_keeps_message() {
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let result = bounded(
            Duration::from_millis(10),
            "读取微信界面超时".to_string(),
            async move {
                let _flag = flag;
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok::<(), anyhow::Error>(())
            },
        )
        .await;
        let error = result.expect_err("future must be cancelled");
        assert!(error.to_string().contains("读取微信界面超时"));
        assert!(
            dropped.load(Ordering::SeqCst),
            "dropping the timed-out future must drop its resources"
        );
    }
}
