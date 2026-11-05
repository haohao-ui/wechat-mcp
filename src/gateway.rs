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
            Ok(data) => {
                let mut result = make_tool_result(&data.to_string(), false);
                result["structuredContent"] = data;
                result
            }
            Err(error) => make_tool_result(&format!("{error:#}"), true),
        }
    }

    async fn execute(&self, name: &str, args: Value) -> Result<Value> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("arguments 必须是对象"))?;
        if name == "list_accounts" {
            if !object.is_empty() {
                bail!("list_accounts 不接受参数");
            }
            return Ok(json!({"accounts":self.accounts.list()}));
        }
        if name == crate::article::TOOL {
            return crate::article::execute(args).await;
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
            Operation::SendMessage { recipient, message } => Some((recipient.clone(), message.clone())),
            Operation::SendFile { recipient, file } => Some((recipient.clone(), file.outbox_text())),
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
        let tool = tools["tools"].as_array().or(tools.as_array()).unwrap().iter().find(|t| t["name"] == "send_file").expect("send_file listed").clone();
        assert_eq!(tool["inputSchema"]["required"], json!(["platform","account_id","recipient","path","idempotency_key"]));
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
}
