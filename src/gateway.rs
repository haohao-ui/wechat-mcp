use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    accounts::{AccountRegistry, GatewayConfig},
    adapters::{Operation, WechatDesktop},
    crm,
    listeners::Listeners,
    mcp::make_tool_result,
    storage::Store,
};

pub struct Gateway {
    pub accounts: AccountRegistry,
    pub store: Arc<Store>,
    pub listeners: Listeners,
    pub legacy_tools: bool,
    pub(crate) desktop_tools: crate::interaction::DesktopTools,
}

impl Gateway {
    pub fn from_config(config: GatewayConfig, store: Arc<Store>) -> Result<Self> {
        let mut accounts = AccountRegistry::default();
        let desktop = Arc::new(Mutex::new(()));
        if config.accounts.is_empty() {
            bail!("accounts 不能为空");
        }
        for account in config.accounts {
            match (account.platform.as_str(), account.adapter.as_str()) {
                ("wechat", "wechat_desktop") => {
                    accounts.register(Arc::new(WechatDesktop::new(
                        account.account_id,
                        desktop.clone(),
                    )))?;
                }
                _ => bail!("尚未实现适配器: {}/{}", account.platform, account.adapter),
            }
        }
        Ok(Self {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: true,
            desktop_tools: crate::interaction::DesktopTools::default(),
        })
    }

    pub fn load() -> Result<Self> {
        let config = match std::env::var_os("MESSAGE_GATEWAY_CONFIG") {
            Some(path) => {
                let data = std::fs::read(path).context("读取 MESSAGE_GATEWAY_CONFIG 失败")?;
                serde_json::from_slice(&data).context("账号配置格式错误")?
            }
            None => GatewayConfig::default(),
        };
        let path = match std::env::var_os("MESSAGE_GATEWAY_DB") {
            Some(path) => std::path::PathBuf::from(path),
            None => std::path::PathBuf::from(
                std::env::var_os("HOME")
                    .context("需要 HOME 或 MESSAGE_GATEWAY_DB 设置数据库路径")?,
            )
            .join("Library/Application Support/message-gateway/service.sqlite3"),
        };
        let mut gateway = Self::from_config(config, Store::open(&path)?)?;
        gateway.legacy_tools = crate::tools::legacy_tools_enabled();
        Ok(gateway)
    }

    pub async fn call(&self, name: &str, args: Value) -> Value {
        match self.execute(name, args).await {
            Ok(mut data) => {
                let screenshot = if crate::interaction::is_tool(name) {
                    data.as_object_mut()
                        .and_then(|obj| obj.remove("screenshot_base64"))
                        .and_then(|v| v.as_str().map(str::to_owned))
                } else {
                    None
                };
                let mut result = make_tool_result(&data.to_string(), false);
                result["structuredContent"] = data;
                if let Some(png) = screenshot {
                    result["content"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"type":"image","mimeType":"image/png","data":png}));
                }
                result
            }
            Err(error) => make_tool_result(&format!("{error:#}"), true),
        }
    }

    async fn execute(&self, name: &str, args: Value) -> Result<Value> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("arguments 必须是对象"))?;
        if crate::interaction::is_tool(name) {
            return self.desktop_tools.execute(name, args).await;
        }
        if name == "list_accounts" {
            if !object.is_empty() {
                bail!("list_accounts 不接受参数");
            }
            return Ok(json!({"accounts":self.accounts.list()}));
        }
        if name == "upload_file" {
            for field in object.keys() {
                if !["name", "content_base64"].contains(&field.as_str()) {
                    bail!("不支持的参数: {field}");
                }
            }
            let file_name = required_string(&args, "name")?;
            let content = required_string(&args, "content_base64")?;
            let root = crate::wechat::files::default_upload_root()
                .context("需要 HOME 才能保存上传文件")?;
            let path = crate::wechat::files::store_upload(file_name, content, &root)?;
            let size = std::fs::metadata(&path)?.len();
            return Ok(json!({"path": path, "name": file_name, "size": size}));
        }
        if name == crate::article::TOOL {
            return crate::article::execute(args).await;
        }
        if crate::core::brain_tools::is_brain_tool(name) {
            return crate::core::brain_tools::execute(name, args).await;
        }
        if crm::is_tool(name)
            || crate::listeners::is_tool(name)
            || name == "capture_customer_context"
        {
            let platform = required_string(&args, "platform")?;
            let account_id = required_string(&args, "account_id")?;
            let adapter = self.accounts.resolve(platform, Some(account_id))?;
            let key = adapter.account().key.clone();
            if crm::is_tool(name) {
                return crm::execute(self.store.clone(), key, name, args).await;
            }
            if name == "capture_customer_context" {
                for field in object.keys() {
                    if !["platform", "account_id", "customer_id"].contains(&field.as_str()) {
                        bail!("不支持的参数: {field}");
                    }
                }
                let customer = required_string(&args, "customer_id")?.to_owned();
                let account = key.clone();
                let customer_id = customer.clone();
                let recipient: String = self.store.run(move |conn| {
                    crate::storage::require_customer(conn,&account,&customer_id)?;
                    let recipient: Option<String> = conn.query_row("SELECT recipient FROM customers WHERE platform=?1 AND account_id=?2 AND customer_id=?3",rusqlite::params![account.platform,account.account_id,customer_id],|row|row.get(0))?;
                    recipient.context("客户档案没有 recipient")
                }).await?;
                let output = adapter
                    .execute(Operation::ReadMessages {
                        recipient: Some(recipient),
                    })
                    .await?;
                if output["account"] != json!(key) {
                    bail!("适配器返回不同账号，拒绝存储");
                }
                return self.store.capture(key, customer, output).await;
            }
            return self
                .listeners
                .execute(adapter, self.store.clone(), name, args)
                .await;
        }
        let legacy = crate::tools::is_legacy_tool(name);
        if legacy && !self.legacy_tools {
            if let Some(unified) = crate::tools::legacy_tool_replacement(name) {
                bail!("旧工具 {name} 已禁用，请改用统一工具 {unified}");
            } else {
                bail!("旧工具 {name} 已禁用，无统一替代工具");
            }
        }
        let (kind, allowed): (&str, &[&str]) = match name {
            "send_message" | "send_wechat_message" => (
                "send",
                &["recipient", "message", "idempotency_key", "customer_id"],
            ),
            "send_file" => (
                "send_file",
                &["recipient", "path", "idempotency_key", "customer_id"],
            ),
            "list_conversations" | "get_recent_sessions" => ("list", &[]),
            "get_messages" | "read_chat_messages" => ("read", &["recipient"]),
            "wait_for_message" | "wait_for_new_message" => {
                ("wait", &["recipient", "timeout_seconds"])
            }
            "focus_wechat" => ("focus", &[]),
            _ => bail!("未知工具: {name}"),
        };
        for field in object.keys() {
            if field != "account_id"
                && !(field == "platform" && !legacy)
                && !allowed.contains(&field.as_str())
            {
                bail!("不支持的参数: {field}");
            }
        }
        let platform = if legacy {
            "wechat"
        } else {
            required_string(&args, "platform")?
        };
        let account_id = optional_string(&args, "account_id")?;
        if !legacy && account_id.is_none() {
            bail!("必须提供 account_id");
        }
        let adapter = self.accounts.resolve(platform, account_id)?;
        if kind == "wait"
            && self
                .store
                .listener_enabled(adapter.account().key.clone())
                .await?
        {
            bail!("后台监听已启用，请使用 wait_for_events / claim_events，避免长轮询占用桌面");
        }
        let operation = match kind {
            "focus" => Operation::Focus,
            "list" => Operation::ListConversations,
            "read" => Operation::ReadMessages {
                recipient: optional_string(&args, "recipient")?.map(str::to_owned),
            },
            "send" => Operation::SendMessage {
                recipient: required_string(&args, "recipient")?.to_owned(),
                message: required_string(&args, "message")?.to_owned(),
            },
            "send_file" => Operation::SendFile {
                recipient: required_string(&args, "recipient")?.to_owned(),
                // Validated before the outbox entry and before any desktop action.
                file: crate::wechat::files::prepare_send_file(
                    required_string(&args, "path")?,
                    &crate::wechat::files::default_send_roots(),
                )?,
            },
            "wait" => {
                let timeout = match args.get("timeout_seconds") {
                    None => 60,
                    Some(value) => value
                        .as_u64()
                        .filter(|v| (1..=300).contains(v))
                        .ok_or_else(|| anyhow::anyhow!("timeout_seconds 必须是 1–300 的整数"))?,
                };
                Operation::WaitForMessage {
                    recipient: optional_string(&args, "recipient")?.map(str::to_owned),
                    timeout_seconds: timeout,
                }
            }
            _ => unreachable!("static operation mapping"),
        };
        let outbox = match &operation {
            Operation::SendMessage { recipient, message } => {
                Some((recipient.clone(), message.clone()))
            }
            Operation::SendFile { recipient, file } => {
                Some((recipient.clone(), file.outbox_text()))
            }
            _ => None,
        };
        if let Some((recipient, message)) = outbox {
            if kind == "send_file" && legacy {
                bail!("send_file 只能通过统一工具调用");
            }
            let request_key = if legacy {
                optional_string(&args, "idempotency_key")?
                    .map(str::to_owned)
                    .unwrap_or_else(crate::storage::new_id)
            } else {
                required_string(&args, "idempotency_key")?.to_owned()
            };
            if request_key.trim().is_empty() || request_key.len() > 256 {
                bail!("idempotency_key 必须为 1–256 字节");
            }
            let customer = optional_string(&args, "customer_id")?.map(str::to_owned);
            let key = adapter.account().key.clone();
            let (dispatch, mut receipt) = self
                .store
                .begin_send(
                    key.clone(),
                    request_key,
                    recipient.clone(),
                    message.clone(),
                    customer,
                )
                .await?;
            if !dispatch {
                receipt["replayed"] = json!(true);
                return Ok(json!({"account":key,"data":receipt}));
            }
            let send_id = receipt["send_id"]
                .as_str()
                .context("发送记录缺少 send_id")?
                .to_owned();
            match adapter.execute(operation).await {
                Ok(output) => {
                    if output["account"] != json!(key) {
                        self.store
                            .finish_send(
                                key,
                                send_id,
                                json!({"error":"适配器返回了不同账号，发送结果未知"}),
                                "unknown",
                            )
                            .await?;
                        bail!("适配器返回不同账号，已记录为未知发送结果");
                    }
                    let status = if output["data"]["status"] == "triggered" {
                        "triggered"
                    } else {
                        "unknown"
                    };
                    let receipt = self
                        .store
                        .finish_send(key.clone(), send_id, output, status)
                        .await?;
                    return Ok(json!({"account":key,"data":receipt}));
                }
                Err(error) => {
                    let is_not_sent =
                        if let Some(send_err) = error.downcast_ref::<crate::wechat::SendError>() {
                            matches!(send_err, crate::wechat::SendError::NotSent(_))
                        } else if let Some(send_err) = error
                            .root_cause()
                            .downcast_ref::<crate::wechat::SendError>()
                        {
                            matches!(send_err, crate::wechat::SendError::NotSent(_))
                        } else {
                            format!("{error:#}").contains("未触发发送")
                        };

                    if is_not_sent {
                        let err_text = format!("{error:#}");
                        let marked_err = if err_text.contains("未触发发送") {
                            err_text
                        } else {
                            format!("未触发发送: {err_text}")
                        };
                        self.store
                            .finish_send(
                                key,
                                send_id.clone(),
                                json!({"error": marked_err, "delivery": "not_sent"}),
                                "unknown",
                            )
                            .await?;
                        bail!("发送尝试 {send_id} 未触发发送: {marked_err}");
                    } else {
                        self.store
                            .finish_send(
                                key,
                                send_id.clone(),
                                json!({"error":format!("{error:#}"),"delivery":"unknown"}),
                                "unknown",
                            )
                            .await?;
                        bail!(
                            "发送尝试 {send_id} 结果未知，已持久记录；请使用 get_send_status 核对。错误: {error:#}"
                        );
                    }
                }
            }
        }
        adapter.execute(operation).await
    }
}

fn optional_string<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>> {
    args.get(name)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{name} 必须是字符串"))
        })
        .transpose()
}

fn required_string<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    optional_string(args, name)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("必须提供非空字符串 {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounts::{AccountDescriptor, AccountKey, Capabilities},
        adapters::{AdapterFuture, MessageAdapter},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingAdapter {
        account: AccountDescriptor,
        calls: AtomicUsize,
    }

    impl RecordingAdapter {
        fn new(platform: &str, account_id: &str, binding: &str) -> Arc<Self> {
            Arc::new(Self {
                account: AccountDescriptor {
                    key: AccountKey {
                        platform: platform.into(),
                        account_id: account_id.into(),
                    },
                    adapter: "test".into(),
                    binding_id: binding.into(),
                    identity_verified: true,
                    capabilities: Capabilities {
                        send_text: true,
                        visible_conversations: true,
                        visible_messages: true,
                        full_history: false,
                        delivery_receipts: false,
                        background_events: false,
                        background_observations: true,
                        source: "test".into(),
                    },
                },
                calls: AtomicUsize::new(0),
            })
        }
    }

    impl MessageAdapter for RecordingAdapter {
        fn account(&self) -> &AccountDescriptor {
            &self.account
        }
        fn execute(&self, _: Operation) -> AdapterFuture<'_> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"account":self.account.key}))
            })
        }
    }

    #[tokio::test]
    async fn routes_same_platform_accounts_without_crossing_bindings() {
        let first = RecordingAdapter::new("whatsapp", "work", "api:1");
        let second = RecordingAdapter::new("whatsapp", "personal", "api:2");
        let mut accounts = AccountRegistry::default();
        accounts.register(first.clone()).unwrap();
        accounts.register(second.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: true,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };
        let response = gateway.call("send_message", json!({"platform":"whatsapp","account_id":"personal","recipient":"r","message":"hello","idempotency_key":"routing-test"})).await;
        assert_eq!(response["isError"], false);
        assert_eq!(
            response["structuredContent"]["account"]["account_id"],
            "personal"
        );
        assert_eq!(first.calls.load(Ordering::SeqCst), 0);
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);
        let response = gateway.call("send_message", json!({"platform":"whatsapp","account_id":"missing","recipient":"r","message":"hello"})).await;
        assert_eq!(response["isError"], true);
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn account_names_are_scoped_to_platform() {
        let mut registry = AccountRegistry::default();
        registry
            .register(RecordingAdapter::new("wechat", "work", "a"))
            .unwrap();
        registry
            .register(RecordingAdapter::new("whatsapp", "work", "b"))
            .unwrap();
        assert_eq!(
            registry
                .resolve("wechat", Some("work"))
                .unwrap()
                .account()
                .key
                .platform,
            "wechat"
        );
        assert_eq!(
            registry
                .resolve("whatsapp", Some("work"))
                .unwrap()
                .account()
                .key
                .platform,
            "whatsapp"
        );
    }

    #[test]
    fn duplicate_keys_and_transport_aliases_are_rejected() {
        let mut registry = AccountRegistry::default();
        registry
            .register(RecordingAdapter::new("wechat", "work", "desktop:1"))
            .unwrap();
        assert!(
            registry
                .register(RecordingAdapter::new("wechat", "work", "desktop:2"))
                .is_err()
        );
        assert!(
            registry
                .register(RecordingAdapter::new("wechat", "other", "desktop:1"))
                .is_err()
        );
    }

    #[tokio::test]
    async fn ambiguous_legacy_account_does_not_execute() {
        let first = RecordingAdapter::new("wechat", "a", "a");
        let second = RecordingAdapter::new("wechat", "b", "b");
        let mut accounts = AccountRegistry::default();
        accounts.register(first.clone()).unwrap();
        accounts.register(second.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: true,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };
        let result = gateway
            .call(
                "send_wechat_message",
                json!({"recipient":"r","message":"m"}),
            )
            .await;
        assert_eq!(result["isError"], true);
        assert_eq!(first.calls.load(Ordering::SeqCst), 0);
        assert_eq!(second.calls.load(Ordering::SeqCst), 0);
        let result = gateway
            .call(
                "send_wechat_message",
                json!({"account_id":"b","recipient":"r","message":"m"}),
            )
            .await;
        assert_eq!(result["isError"], false);
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn send_file_checks_the_path_before_any_desktop_action() {
        let adapter = RecordingAdapter::new("wechat", "local", "local");
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };
        for args in [
            json!({"platform":"wechat","account_id":"local","recipient":"r","path":"Desktop/a.pdf","idempotency_key":"k1"}),
            json!({"platform":"wechat","account_id":"local","recipient":"r","path":"/etc/hosts","idempotency_key":"k2"}),
            json!({"platform":"wechat","account_id":"local","recipient":"r","path":"/etc/hosts"}),
            json!({"platform":"wechat","account_id":"local","recipient":"r","path":"/etc/hosts","idempotency_key":"k3","message":"m"}),
        ] {
            assert_eq!(gateway.call("send_file", args).await["isError"], true);
        }
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
        let tools = crate::tools::list_tools_with_legacy(false);
        let tool = tools["tools"]
            .as_array()
            .or(tools.as_array())
            .unwrap()
            .iter()
            .find(|t| t["name"] == "send_file")
            .expect("send_file listed")
            .clone();
        assert_eq!(
            tool["inputSchema"]["required"],
            json!([
                "platform",
                "account_id",
                "recipient",
                "path",
                "idempotency_key"
            ])
        );
    }

    #[tokio::test]
    async fn upload_file_rejects_bad_input_and_is_listed() {
        let gateway = Gateway {
            accounts: AccountRegistry::default(),
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };
        for args in [
            json!({"name":"../a.pdf","content_base64":"aGk="}),
            json!({"name":"a.pdf","content_base64":"not base64!"}),
            json!({"name":"a.pdf"}),
            json!({"name":"a.pdf","content_base64":"aGk=","path":"/tmp/x"}),
        ] {
            assert_eq!(gateway.call("upload_file", args).await["isError"], true);
        }
        let tools = crate::tools::list_tools_with_legacy(false);
        let tool = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "upload_file")
            .expect("upload_file listed")
            .clone();
        assert_eq!(
            tool["inputSchema"]["required"],
            json!(["name", "content_base64"])
        );
    }

    #[tokio::test]
    async fn invalid_arguments_never_reach_an_adapter() {
        let adapter = RecordingAdapter::new("wechat", "local", "local");
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: true,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };
        for args in [
            json!({"platform":"wechat","recipient":"r","message":"m"}),
            json!({"platform":"wechat","account_id":"local","recipient":3,"message":"m"}),
            json!({"platform":"wechat","account_id":"local","recipient":"r","message":"m","typo":true}),
            json!({"platform":"wechat","account_id":null,"recipient":"r","message":"m"}),
        ] {
            assert_eq!(gateway.call("send_message", args).await["isError"], true);
        }
        for timeout in [json!(0), json!(-1), json!(301), json!(1.5), json!("60")] {
            let result = gateway
                .call(
                    "wait_for_message",
                    json!({"platform":"wechat","account_id":"local","timeout_seconds":timeout}),
                )
                .await;
            assert_eq!(result["isError"], true);
        }
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn desktop_config_cannot_claim_multiple_accounts_or_unimplemented_platforms() {
        let config = serde_json::from_value(json!({"accounts":[
            {"platform":"wechat","account_id":"a","adapter":"wechat_desktop"},
            {"platform":"wechat","account_id":"b","adapter":"wechat_desktop"},
        ]}))
        .unwrap();
        assert!(Gateway::from_config(config, Store::memory()).is_err());
        let config = serde_json::from_value(json!({"accounts":[
            {"platform":"whatsapp","account_id":"a","adapter":"cloud_api"},
        ]}))
        .unwrap();
        assert!(Gateway::from_config(config, Store::memory()).is_err());
        assert!(Gateway::from_config(GatewayConfig { accounts: vec![] }, Store::memory()).is_err());
    }

    #[tokio::test]
    async fn desktop_capabilities_do_not_claim_delivery_or_history() {
        let gateway = Gateway::from_config(GatewayConfig::default(), Store::memory()).unwrap();
        let result = gateway.call("list_accounts", json!({})).await;
        let account = &result["structuredContent"]["accounts"][0];
        assert_eq!(account["identity_verified"], false);
        assert_eq!(account["capabilities"]["full_history"], false);
        assert_eq!(account["capabilities"]["delivery_receipts"], false);
        assert_eq!(account["capabilities"]["background_events"], false);
    }

    #[tokio::test]
    async fn legacy_tools_disabled_returns_error_with_hint() {
        let adapter = RecordingAdapter::new("wechat", "local", "local");
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store: Store::memory(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        for (tool, expected_unified) in [
            ("send_wechat_message", Some("send_message")),
            ("get_recent_sessions", Some("list_conversations")),
            ("read_chat_messages", Some("get_messages")),
            ("wait_for_new_message", Some("wait_for_message")),
            ("focus_wechat", None),
        ] {
            let res = gateway.call(tool, json!({})).await;
            assert_eq!(
                res["isError"], true,
                "tool {tool} should return isError: true when legacy disabled"
            );
            let text = res["content"][0]["text"].as_str().unwrap();
            if let Some(unified) = expected_unified {
                assert!(
                    text.contains(unified),
                    "error message for {tool} should mention {unified}, got: {text}"
                );
            } else {
                assert!(
                    text.contains("无统一替代工具"),
                    "error message for {tool} should state no unified replacement, got: {text}"
                );
            }
        }
    }

    #[tokio::test]
    async fn failure_injected_at_click_records_not_sent() {
        let store = Store::memory();
        let desktop = Arc::new(tokio::sync::Mutex::new(()));
        let adapter = Arc::new(crate::adapters::WechatDesktop::new("work".into(), desktop));
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter).unwrap();
        let gateway = Gateway {
            accounts,
            store: store.clone(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        crate::wechat::with_test_send_fault(
            crate::wechat::SendFault::Click(Some(
                "accessibility_permission_missing_for_executing_process",
            )),
            async {
                let response = gateway
                    .call(
                        "send_message",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "recipient": "test_user",
                            "message": "hello",
                            "idempotency_key": "fail-click-1"
                        }),
                    )
                    .await;
                assert_eq!(response["isError"], true);
                let err_text = response["content"][0]["text"].as_str().unwrap();
                assert!(
                    err_text.contains("未触发发送"),
                    "error text must contain marker: {err_text}"
                );
                assert!(
                    err_text.contains("accessibility_permission_missing_for_executing_process")
                );

                let check = gateway
                    .call(
                        "get_send_status",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "idempotency_key": "fail-click-1"
                        }),
                    )
                    .await;
                assert_eq!(check["isError"], false);
                let send_record = &check["structuredContent"]["data"];
                assert_eq!(send_record["status"], "unknown");
                assert_eq!(send_record["delivery"], "not_sent");
            },
        )
        .await;
    }

    #[tokio::test]
    async fn legacy_schema_version_2_records_not_sent_without_check_violation() {
        let db_path = std::env::temp_dir().join(format!("legacy_v2_{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&db_path).unwrap();

        // 验证数据库是由 001+002 初始化且处于 user_version 2
        let version: i64 = store
            .run(|conn| Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(version, 2);

        let desktop = Arc::new(tokio::sync::Mutex::new(()));
        let adapter = Arc::new(crate::adapters::WechatDesktop::new("work".into(), desktop));
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter).unwrap();
        let gateway = Gateway {
            accounts,
            store: store.clone(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        crate::wechat::with_test_send_fault(
            crate::wechat::SendFault::Click(Some(
                "accessibility_permission_missing_for_executing_process",
            )),
            async {
                let response = gateway
                    .call(
                        "send_message",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "recipient": "test_user",
                            "message": "hello",
                            "idempotency_key": "legacy-not-sent-1"
                        }),
                    )
                    .await;
                assert_eq!(response["isError"], true);
                let err_text = response["content"][0]["text"].as_str().unwrap();
                assert!(err_text.contains("未触发发送"));

                // 验证 get_send_status 报告 delivery 为 not_sent，且未触发任何 CHECK 约束错误
                let check = gateway
                    .call(
                        "get_send_status",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "idempotency_key": "legacy-not-sent-1"
                        }),
                    )
                    .await;
                assert_eq!(check["isError"], false);
                let send_record = &check["structuredContent"]["data"];
                assert_eq!(send_record["status"], "unknown");
                assert_eq!(send_record["delivery"], "not_sent");
            },
        )
        .await;

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn failure_injected_after_enter_records_unknown() {
        let store = Store::memory();
        let desktop = Arc::new(tokio::sync::Mutex::new(()));
        let adapter = Arc::new(crate::adapters::WechatDesktop::new("work".into(), desktop));
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter).unwrap();
        let gateway = Gateway {
            accounts,
            store: store.clone(),
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        crate::wechat::with_test_send_fault(
            crate::wechat::SendFault::AfterEnter(Some("applescript execution failed after enter")),
            async {
                let response = gateway
                    .call(
                        "send_message",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "recipient": "test_user",
                            "message": "hello",
                            "idempotency_key": "fail-enter-1"
                        }),
                    )
                    .await;
                assert_eq!(response["isError"], true);
                let err_text = response["content"][0]["text"].as_str().unwrap();
                assert!(
                    err_text.contains("结果未知"),
                    "error text must contain 结果未知: {err_text}"
                );
                assert!(!err_text.contains("未触发发送"));

                let check = gateway
                    .call(
                        "get_send_status",
                        json!({
                            "platform": "wechat",
                            "account_id": "work",
                            "idempotency_key": "fail-enter-1"
                        }),
                    )
                    .await;
                assert_eq!(check["isError"], false);
                let send_record = &check["structuredContent"]["data"];
                assert_eq!(send_record["status"], "unknown");
                assert_eq!(send_record["delivery"], "unknown");
            },
        )
        .await;
    }

    struct SequenceAdapter {
        account: AccountDescriptor,
        calls: AtomicUsize,
        responses: std::sync::Mutex<Vec<Result<Value, String>>>,
    }

    impl SequenceAdapter {
        fn new(
            platform: &str,
            account_id: &str,
            responses: Vec<Result<Value, String>>,
        ) -> Arc<Self> {
            Arc::new(Self {
                account: AccountDescriptor {
                    key: AccountKey {
                        platform: platform.into(),
                        account_id: account_id.into(),
                    },
                    adapter: "test".into(),
                    binding_id: "test:seq".into(),
                    identity_verified: true,
                    capabilities: Capabilities {
                        send_text: true,
                        visible_conversations: true,
                        visible_messages: true,
                        full_history: false,
                        delivery_receipts: false,
                        background_events: false,
                        background_observations: true,
                        source: "test".into(),
                    },
                },
                calls: AtomicUsize::new(0),
                responses: std::sync::Mutex::new(responses),
            })
        }
    }

    impl MessageAdapter for SequenceAdapter {
        fn account(&self) -> &AccountDescriptor {
            &self.account
        }
        fn execute(&self, _: Operation) -> AdapterFuture<'_> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let mut guard = self.responses.lock().unwrap();
                if guard.is_empty() {
                    Ok(json!({"account": self.account.key, "data": {"status": "triggered"}}))
                } else {
                    let resp = guard.remove(0);
                    match resp {
                        Ok(v) => Ok(v),
                        Err(err_msg) => {
                            if err_msg.contains("未触发发送") {
                                Err(
                                    crate::wechat::SendError::NotSent(anyhow::anyhow!("{err_msg}"))
                                        .into(),
                                )
                            } else {
                                Err(anyhow::anyhow!("{err_msg}"))
                            }
                        }
                    }
                }
            })
        }
    }

    #[tokio::test]
    async fn not_sent_then_same_key_again_dispatches_twice() {
        let store = Store::memory();
        let adapter = SequenceAdapter::new(
            "wechat",
            "work",
            vec![
                Err("未触发发送: click failed".into()),
                Ok(
                    json!({"account": {"platform": "wechat", "account_id": "work"}, "data": {"status": "triggered"}}),
                ),
            ],
        );
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        // 第 1 次发送：失败，记录 not_sent
        let resp1 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-retry-dispatch"
                }),
            )
            .await;
        assert_eq!(resp1["isError"], true);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);

        // 第 2 次相同 key：重新 dispatch，调用 adapter 第二次
        let resp2 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-retry-dispatch"
                }),
            )
            .await;
        assert_eq!(resp2["isError"], false);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
        assert_ne!(resp2["structuredContent"]["data"]["replayed"], true);
        assert_eq!(resp2["structuredContent"]["data"]["status"], "triggered");
    }

    #[tokio::test]
    async fn unknown_without_not_sent_delivery_then_same_key_again_replays() {
        let store = Store::memory();
        let adapter = SequenceAdapter::new("wechat", "work", vec![Err("timeout error".into())]);
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        // 第 1 次发送：失败，记录 unknown (delivery 不是 not_sent)
        let resp1 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-unknown-replay"
                }),
            )
            .await;
        assert_eq!(resp1["isError"], true);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);

        // 第 2 次相同 key：replay，不再调用 adapter
        let resp2 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-unknown-replay"
                }),
            )
            .await;
        assert_eq!(resp2["isError"], false);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(resp2["structuredContent"]["data"]["replayed"], true);
    }

    #[tokio::test]
    async fn triggered_then_same_key_again_replays() {
        let store = Store::memory();
        let adapter = SequenceAdapter::new(
            "wechat",
            "work",
            vec![Ok(
                json!({"account": {"platform": "wechat", "account_id": "work"}, "data": {"status": "triggered"}}),
            )],
        );
        let mut accounts = AccountRegistry::default();
        accounts.register(adapter.clone()).unwrap();
        let gateway = Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        // 第 1 次发送：成功 triggered
        let resp1 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-triggered-replay"
                }),
            )
            .await;
        assert_eq!(resp1["isError"], false);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);

        // 第 2 次相同 key：replay，不再调用 adapter
        let resp2 = gateway
            .call(
                "send_message",
                json!({
                    "platform": "wechat",
                    "account_id": "work",
                    "recipient": "user1",
                    "message": "hello",
                    "idempotency_key": "key-triggered-replay"
                }),
            )
            .await;
        assert_eq!(resp2["isError"], false);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(resp2["structuredContent"]["data"]["replayed"], true);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn brain_tools_dispatched_via_gateway() {
        let _guard = crate::core::env_lock();
        let prev_env = std::env::var("WECHAT_CORE_SOCK").ok();
        let missing = format!("/tmp/nonexistent_core_{}.sock", uuid::Uuid::new_v4());
        unsafe {
            std::env::set_var("WECHAT_CORE_SOCK", &missing);
        }

        let store = Store::memory();
        let accounts = AccountRegistry::default();
        let gateway = Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        let resp = gateway.call("next_turn", json!({"wait_seconds": 0})).await;
        // 未接入网关时会返回 "未知工具: next_turn"
        // 接入网关后由于 core.sock 未启动应返回 "core_not_running"
        let err_text = resp["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            !err_text.contains("未知工具"),
            "brain tools must be dispatched by gateway, but got: {err_text}"
        );
        assert!(
            err_text.contains("core_not_running"),
            "expected core_not_running but got: {err_text}"
        );

        if let Some(prev) = prev_env {
            unsafe {
                std::env::set_var("WECHAT_CORE_SOCK", prev);
            }
        } else {
            unsafe {
                std::env::remove_var("WECHAT_CORE_SOCK");
            }
        }
    }

    async fn mcp_handle_request(
        gateway: &Gateway,
        request: Value,
        structured_content: bool,
    ) -> Value {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "tools/call" => {
                let params = request.get("params").and_then(Value::as_object).unwrap();
                let name = params.get("name").and_then(Value::as_str).unwrap();
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let mut res = gateway.call(name, args).await;
                if !structured_content {
                    if let Some(obj) = res.as_object_mut() {
                        obj.remove("structuredContent");
                    }
                }
                res
            }
            _ => panic!("unsupported method in test"),
        };
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_mcp_handle_request_through_gateway_to_core_sock_contract_and_mutex() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::core::env_lock();

        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let td = std::env::temp_dir().join(format!("gw_contract_{pid}_{nanos}"));
        std::fs::create_dir_all(&td).unwrap();
        std::fs::set_permissions(&td, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = td.join("core.sock");

        // 真实绑定 core.sock
        let listener = crate::core::external_brain::bind_core_socket(&sock_path).unwrap();

        // 实例化真实的 ExternalBrain 并启动 socket server
        let brain_cfg = crate::core::config::BrainConfig {
            mode: crate::core::config::BrainMode::External,
            external: crate::core::config::ExternalBrainConfig {
                timeout_seconds: 30,
                timeout_action: crate::core::config::TimeoutAction::Skip,
                queue_minutes: 30,
            },
        };
        let model_cfg = crate::core::config::ModelConfig {
            backend: "codex-exec".to_string(),
            base_url: None,
            model: None,
            api_key_file: None,
            api_key_env: None,
            supports_images: None,
            timeout_seconds: None,
            json_output: None,
        };
        let brain = crate::core::external_brain::ExternalBrain::new_with_socket(
            &brain_cfg,
            &model_cfg,
            td.clone(),
            sock_path.clone(),
        );

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let server_handle = crate::core::external_brain::start_socket_server(
            brain.clone(),
            listener,
            Some(stop_rx),
        );

        // 设置环境变量 WECHAT_CORE_SOCK 供 Gateway / brain_tools 发现
        let prev_env = std::env::var("WECHAT_CORE_SOCK").ok();
        unsafe {
            std::env::set_var("WECHAT_CORE_SOCK", &sock_path);
        }

        // 构建真实的 Gateway
        let store = Store::memory();
        let accounts = AccountRegistry::default();
        let gateway = Gateway {
            accounts,
            store,
            listeners: Listeners::default(),
            legacy_tools: false,
            desktop_tools: crate::interaction::DesktopTools::default(),
        };

        // 生产一轮待回复消息
        let b_fn = brain.brain_fn();
        let sample_turn = crate::brain::Turn {
            turn_id: "turn-gw-1".to_string(),
            account_id: "acc-gw".to_string(),
            contact: crate::brain::ContactInfo {
                name: "午夜咖啡".to_string(),
                kind: "private".to_string(),
            },
            text: "来自客户的问题".to_string(),
            mode: "service".to_string(),
            chat_kind: "private".to_string(),
            context: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            notes: Vec::new(),
            failed: Vec::new(),
            image_count: Some(0),
            earlier_images: 0,
            max_attachment_mb: 50,
            media_refs: Vec::new(),
        };

        let brain_handle = tokio::spawn(async move { b_fn(&sample_turn).await });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 1. 客户端 1：通过 MCP handle_request 经 Gateway 调用 next_turn
        let req1 = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "next_turn",
                "arguments": { "wait_seconds": 1 }
            }
        });
        let resp1 = mcp_handle_request(&gateway, req1, true).await;
        assert_eq!(resp1["result"]["isError"], false);
        let turn_data = &resp1["result"]["structuredContent"]["turn"];
        assert_eq!(turn_data["turn_id"], "turn-gw-1");
        assert_eq!(turn_data["contact"]["name"], "午夜咖啡");
        assert_eq!(turn_data["text"], "来自客户的问题");
        let claim_token = turn_data["claim_token"]
            .as_str()
            .expect("必须包含 claim_token")
            .to_string();
        assert!(!claim_token.is_empty());
        assert!(turn_data["deadline_at"].as_u64().is_some());

        // 2. 客户端 2：在客户端 1 单次调用断开后，连续发起 next_turn，验证互斥！
        let req2 = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "next_turn",
                "arguments": { "wait_seconds": 0 }
            }
        });
        let resp2 = mcp_handle_request(&gateway, req2, true).await;
        assert_eq!(resp2["result"]["isError"], false);
        assert_eq!(
            resp2["result"]["structuredContent"]["turn"],
            Value::Null,
            "轮次已被客户端 1 认领，客户端 2 连续 next_turn 必须互斥返回 Null"
        );

        // 3. 客户端 2：尝试用错误的 claim_token 抢答 reply，应当被拒绝
        let fake_reply_req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "reply",
                "arguments": {
                    "turn_id": "turn-gw-1",
                    "action": "send",
                    "text": "恶意抢答",
                    "claim_token": "bad_token_xyz"
                }
            }
        });
        let resp_fake = mcp_handle_request(&gateway, fake_reply_req, true).await;
        assert_eq!(resp_fake["result"]["isError"], true);
        assert!(
            resp_fake["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("invalid_claim_token")
        );

        // 4. 客户端 1：用正确的 claim_token 通过 MCP handle_request 经 Gateway 回复
        let reply_req = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "reply",
                "arguments": {
                    "turn_id": "turn-gw-1",
                    "action": "send",
                    "text": "网关契约测试成功回复",
                    "claim_token": claim_token
                }
            }
        });
        let resp_reply = mcp_handle_request(&gateway, reply_req, true).await;
        assert_eq!(resp_reply["result"]["isError"], false);
        assert_eq!(resp_reply["result"]["structuredContent"]["accepted"], true);

        // 检查后台 ExternalBrain 收到的回复与元数据
        let brain_reply = brain_handle.await.unwrap().unwrap();
        assert_eq!(brain_reply.reply, "网关契约测试成功回复");
        assert_eq!(
            brain_reply.metadata.get("action").and_then(Value::as_str),
            Some("send")
        );

        // 5. 再次 reply 应该被拒绝（已回答）
        let dup_reply = mcp_handle_request(
            &gateway,
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "reply",
                    "arguments": {
                        "turn_id": "turn-gw-1",
                        "action": "skip"
                    }
                }
            }),
            true,
        )
        .await;
        assert_eq!(dup_reply["result"]["isError"], true);

        // 6. 测试 resume_session 工具通过 Gateway 正常执行
        let session_dir = td.join("sessions").join("session_1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let handoff_file = session_dir.join("handoff.json");
        std::fs::write(&handoff_file, json!({"recipient": "午夜咖啡"}).to_string()).unwrap();
        assert!(handoff_file.exists());

        let resume_req = json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "resume_session",
                "arguments": {
                    "recipient": "午夜咖啡"
                }
            }
        });
        let resp_resume = mcp_handle_request(&gateway, resume_req, true).await;
        assert_eq!(resp_resume["result"]["isError"], false);
        assert_eq!(
            resp_resume["result"]["structuredContent"]["resumed"],
            "午夜咖啡"
        );
        assert!(
            !handoff_file.exists(),
            "handoff.json 必须已被清理以恢复自动回复"
        );

        // 清理资源
        let _ = stop_tx.send(true);
        server_handle.abort();
        if let Some(prev) = prev_env {
            unsafe {
                std::env::set_var("WECHAT_CORE_SOCK", prev);
            }
        } else {
            unsafe {
                std::env::remove_var("WECHAT_CORE_SOCK");
            }
        }
        let _ = std::fs::remove_dir_all(&td);
    }
}
