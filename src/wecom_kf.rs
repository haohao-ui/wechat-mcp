use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{sync::Mutex, time::Instant};

use crate::{
    accounts::{AccountDescriptor, AccountKey, Capabilities, WecomKfConfig},
    adapters::{AdapterFuture, MessageAdapter, Operation},
    storage::{Store, now},
};

/// 只暴露错误码；HTTP URL、响应正文和 errmsg 都可能回显凭据。
#[derive(Debug)]
pub struct ProviderError {
    pub code: Option<i64>,
    pub definitely_unsent: bool,
    description: &'static str,
}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "微信客服：{}", self.description)?;
        if let Some(code) = self.code {
            write!(f, "（错误码 {code}）")?;
        }
        Ok(())
    }
}
impl std::error::Error for ProviderError {}
impl ProviderError {
    fn local(description: &'static str) -> anyhow::Error {
        Self {
            code: None,
            definitely_unsent: true,
            description,
        }
        .into()
    }
    fn api(code: i64) -> Self {
        Self {
            code: Some(code),
            // -1 是平台繁忙；重复 ID 也不能证明之前的发送未发生。
            // 按文档假设，待联调核对：95033 只表示重复，不能作为送达证据。
            definitely_unsent: code > 0 && code != 95033,
            description: "接口拒绝请求",
        }
    }
    pub fn rate_limited(&self) -> bool {
        matches!(self.code, Some(45009 | 45011))
    }
}

struct CachedToken {
    value: String,
    refresh_at: Instant,
}

pub struct WecomKf {
    account: AccountDescriptor,
    corp_id: String,
    open_kfid: String,
    secret: String,
    base: String,
    client: reqwest::Client,
    token: Mutex<Option<CachedToken>>,
    store: Arc<Store>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEvent {
    pub msgid: String,
    pub occurred_at: i64,
    pub verified: bool,
    pub payload: Value,
}

#[derive(Serialize, Deserialize)]
pub struct SyncPage {
    pub messages: Vec<ProviderEvent>,
    pub next_cursor: String,
    pub has_more: bool,
}

impl WecomKf {
    pub fn from_config(
        account_id: String,
        config: WecomKfConfig,
        store: Arc<Store>,
    ) -> Result<Self> {
        // 不把环境变量名或值拼入错误，避免误配置时泄露 secret。
        if config.secret_env.is_empty()
            || !config
                .secret_env
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            bail!("微信客服 secret_env 必须是环境变量名");
        }
        let secret = std::env::var(&config.secret_env)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .context("微信客服凭据环境变量缺失或为空")?;
        Self::new(account_id, config, secret, store)
    }

    fn new(
        account_id: String,
        config: WecomKfConfig,
        secret: String,
        store: Arc<Store>,
    ) -> Result<Self> {
        if config.corp_id.trim().is_empty() || config.open_kfid.trim().is_empty() {
            bail!("微信客服 corp_id/open_kfid 不能为空");
        }
        let base = config
            .base_url
            .unwrap_or_else(|| "https://qyapi.weixin.qq.com".into());
        let url = reqwest::Url::parse(&base)
            .map_err(|_| anyhow::anyhow!("微信客服 base_url 格式错误"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
        {
            bail!(
                "微信客服 base_url 需要 HTTPS；仅本地 mock 允许 HTTP，且不能包含凭据、查询或片段"
            );
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow::anyhow!("微信客服 HTTP 客户端初始化失败"))?;
        Ok(Self {
            account: AccountDescriptor {
                key: AccountKey {
                    platform: "wechat_kf".into(),
                    account_id,
                },
                adapter: "wecom_kf".into(),
                binding_id: format!("wecom_kf:{}:{}", config.corp_id, config.open_kfid),
                identity_verified: true,
                capabilities: Capabilities {
                    send_text: true,
                    visible_conversations: false,
                    visible_messages: false,
                    full_history: false,
                    delivery_receipts: false,
                    background_events: true,
                    background_observations: false,
                    source: "wecom_kf_api".into(),
                },
            },
            corp_id: config.corp_id,
            open_kfid: config.open_kfid,
            secret,
            base: base.trim_end_matches('/').into(),
            client,
            token: Mutex::new(None),
            store,
        })
    }

    async fn response(request: reqwest::RequestBuilder) -> Result<Value> {
        // 不保留 reqwest/serde 的原始错误链，其中可能有带 access_token 的 URL 或响应内容。
        let response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("微信客服网络请求失败，结果无法确认"))?;
        if !response.status().is_success() {
            bail!("微信客服 HTTP 请求失败，结果无法确认");
        }
        response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("微信客服响应格式错误，结果无法确认"))
    }

    async fn access_token(&self, invalid: Option<&str>) -> Result<String> {
        let mut cache = self.token.lock().await;
        if let Some(token) = cache.as_ref()
            && Instant::now() < token.refresh_at
            && invalid != Some(token.value.as_str())
        {
            return Ok(token.value.clone());
        }
        // 持锁刷新；同一个旧 token 的并发失败只会使首个请求刷新。
        let response = Self::response(
            self.client
                .get(format!("{}/cgi-bin/gettoken", self.base))
                .query(&[("corpid", &self.corp_id), ("corpsecret", &self.secret)]),
        )
        .await?;
        let code = errcode(&response)?;
        if code != 0 {
            return Err(ProviderError::api(code).into());
        }
        let value = response["access_token"]
            .as_str()
            .filter(|v| !v.is_empty())
            .context("微信客服响应缺少凭据")?
            .to_owned();
        let expiry = response["expires_in"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 86400)
            .context("微信客服凭据有效期无效")?;
        *cache = Some(CachedToken {
            value: value.clone(),
            refresh_at: Instant::now() + Duration::from_secs(expiry.saturating_sub(300)),
        });
        Ok(value)
    }

    async fn post(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut token = self.access_token(None).await?;
        for attempt in 0..2 {
            let output = Self::response(
                self.client
                    .post(format!("{}/cgi-bin/kf/{endpoint}", self.base))
                    .query(&[("access_token", &token)])
                    .json(&body),
            )
            .await?;
            let code = errcode(&output)?;
            if code == 0 {
                if contains_credential(&output, &self.secret)
                    || contains_credential(&output, &token)
                {
                    bail!("微信客服响应回显了凭据，已拒绝保存；结果无法确认");
                }
                return Ok(output);
            }
            // 按文档假设，待联调核对：token 失效响应未产生发送副作用，可用同一 msgid 重试。
            if attempt == 0 && matches!(code, 40014 | 42001 | 40001) {
                token = self.access_token(Some(&token)).await?;
                continue;
            }
            return Err(ProviderError::api(code).into());
        }
        unreachable!("重试次数有界")
    }

    async fn sync(&self, cursor: Option<String>) -> Result<Value> {
        let mut body = json!({"limit":1000,"open_kfid":self.open_kfid});
        if let Some(cursor) = cursor.as_ref() {
            body["cursor"] = json!(cursor);
        }
        // 按文档假设，待联调核对：不带回调 token 轮询的实际配额及限流码。
        let output = self.post("sync_msg", body).await?;
        let next_cursor = output["next_cursor"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 64)
            .context("微信客服同步游标无效")?
            .to_owned();
        let has_more = match output["has_more"].as_u64() {
            Some(0) => false,
            Some(1) => true,
            _ => bail!("微信客服 has_more 无效"),
        };
        if has_more && cursor.as_deref() == Some(&next_cursor) {
            bail!("微信客服分页游标未推进，拒绝重复请求");
        }
        let messages = output["msg_list"]
            .as_array()
            .context("微信客服消息列表无效")?
            .iter()
            .map(|m| normalize_message(m, &self.open_kfid))
            .collect::<Result<Vec<_>>>()?;
        Ok(serde_json::to_value(SyncPage {
            messages,
            next_cursor,
            has_more,
        })?)
    }

    async fn send(
        &self,
        recipient: String,
        message: String,
        idempotency: Option<String>,
    ) -> Result<Value> {
        if recipient.trim().is_empty() || message.trim().is_empty() || message.len() > 2048 {
            return Err(ProviderError::local(
                "发送目标/正文不能为空，正文最多 2048 字节",
            ));
        }
        // 按文档假设，待联调核对：48 小时边界采用严格小于，5 条及会话状态由平台裁决。
        let latest = self
            .store
            .latest_provider_inbound(self.account.key.clone(), recipient.clone())
            .await
            .map_err(|_| ProviderError::local("读取客户回复窗口失败，未发送"))?;
        if !within_reply_window(latest, now()) {
            return Err(ProviderError::local(
                "客户未发送消息或已超过 48 小时回复窗口",
            ));
        }
        let mut body = json!({"touser":recipient,"open_kfid":self.open_kfid,"msgtype":"text","text":{"content":message}});
        if let Some(key) = idempotency {
            body["msgid"] = json!(platform_msgid(&key));
        }
        let output = self.post("send_msg", body).await?;
        let msgid = output["msgid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("微信客服发送响应缺少消息 ID，结果无法确认")?;
        Ok(
            json!({"status":"accepted_by_platform","delivery":"unknown","platform_msgid":msgid,"recipient":recipient}),
        )
    }
}

impl MessageAdapter for WecomKf {
    fn account(&self) -> &AccountDescriptor {
        &self.account
    }
    fn execute(&self, operation: Operation) -> AdapterFuture<'_> {
        Box::pin(async move {
            let data = match operation {
                Operation::SyncEvents { cursor } => self.sync(cursor).await?,
                Operation::SendMessage {
                    recipient,
                    message,
                    idempotency,
                } => self.send(recipient, message, idempotency).await?,
                _ => bail!("微信客服 API 不支持桌面观察或完整历史，请通过监听收件箱消费消息"),
            };
            Ok(json!({"account":self.account.key,"identity_verified":true,"data":data}))
        })
    }
}

fn contains_credential(value: &Value, credential: &str) -> bool {
    match value {
        Value::String(s) => s.contains(credential),
        Value::Array(items) => items.iter().any(|v| contains_credential(v, credential)),
        Value::Object(items) => items
            .iter()
            .any(|(k, v)| k.contains(credential) || contains_credential(v, credential)),
        _ => false,
    }
}

fn errcode(value: &Value) -> Result<i64> {
    value["errcode"]
        .as_i64()
        .context("微信客服响应缺少错误码，结果无法确认")
}

pub fn within_reply_window(latest: Option<i64>, current: i64) -> bool {
    latest.is_some_and(|time| time <= current && current - time < 48 * 3600)
}

pub fn platform_msgid(key: &str) -> String {
    if !key.is_empty()
        && key.len() <= 32
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return key.into();
    }
    // SHA-256 截取 128 位，稳定且满足平台 32 字节字符限制。
    format!("{:x}", Sha256::digest(key.as_bytes()))[..32].into()
}

pub fn normalize_message(message: &Value, open_kfid: &str) -> Result<ProviderEvent> {
    let msgid = message["msgid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("微信客服消息缺少 ID")?
        .to_owned();
    let occurred_at = message["send_time"]
        .as_i64()
        .filter(|t| *t >= 0)
        .context("微信客服消息时间无效")?;
    let origin = message["origin"].as_u64().context("微信客服消息来源无效")?;
    let msgtype = message["msgtype"]
        .as_str()
        .context("微信客服消息类型无效")?;
    // 官方文档允许 event 不含顶层 open_kfid/external_userid，身份位于 event 对象内。
    let kfid = message["open_kfid"]
        .as_str()
        .or_else(|| message["event"]["open_kfid"].as_str());
    if kfid.is_some_and(|id| id != open_kfid) || (kfid.is_none() && msgtype != "event") {
        bail!("微信客服消息属于不同账号或缺少账号");
    }
    let user = message["external_userid"]
        .as_str()
        .or_else(|| message["event"]["external_userid"].as_str());
    let direction = match origin {
        3 if msgtype != "event" => "inbound",
        5 if msgtype != "event" => "outbound",
        _ => "system",
    };
    if direction == "inbound" && user.is_none_or(str::is_empty) {
        bail!("微信客服客户消息缺少客户 ID");
    }
    let mut payload = json!({"msgid":msgid,"open_kfid":open_kfid,"external_userid":user,"msgtype":msgtype,"direction":direction,"origin":origin});
    if let Some(text) = message["text"]["content"].as_str() {
        payload["text"] = json!(text);
    }
    if let Some(media_id) = message[msgtype]["media_id"].as_str() {
        payload["media_id"] = json!(media_id);
    }
    if msgtype == "event" {
        payload["event"] = message["event"].clone();
    }
    Ok(ProviderEvent {
        msgid,
        occurred_at,
        verified: matches!(origin, 3..=5),
        payload,
    })
}

#[cfg(test)]
mod tests;
