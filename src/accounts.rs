use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::adapters::MessageAdapter;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountKey {
    pub platform: String,
    pub account_id: String,
}

impl AccountKey {
    pub fn validate(&self) -> Result<()> {
        for value in [&self.platform, &self.account_id] {
            if value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
            {
                bail!("platform/account_id 必须是 1–128 字节的字母、数字或 _-.: 字符");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountDescriptor {
    pub key: AccountKey,
    pub adapter: String,
    pub binding_id: String,
    pub identity_verified: bool,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub send_text: bool,
    pub visible_conversations: bool,
    pub visible_messages: bool,
    pub full_history: bool,
    pub delivery_receipts: bool,
    pub background_events: bool,
    pub background_observations: bool,
    pub source: String,
}

#[derive(Default)]
pub struct AccountRegistry {
    adapters: BTreeMap<AccountKey, Arc<dyn MessageAdapter>>,
}

impl AccountRegistry {
    pub fn register(&mut self, adapter: Arc<dyn MessageAdapter>) -> Result<()> {
        let account = adapter.account();
        account.key.validate()?;
        if self.adapters.contains_key(&account.key) {
            bail!(
                "账号重复: {}/{}",
                account.key.platform,
                account.key.account_id
            );
        }
        // Two aliases of the same authenticated transport are not two accounts.
        if self
            .adapters
            .values()
            .any(|existing| existing.account().binding_id == account.binding_id)
        {
            bail!(
                "绑定 {} 已被占用，不能把同一个会话注册为多个账号",
                account.binding_id
            );
        }
        self.adapters.insert(account.key.clone(), adapter);
        Ok(())
    }

    pub fn list(&self) -> Vec<&AccountDescriptor> {
        self.adapters
            .values()
            .map(|adapter| adapter.account())
            .collect()
    }

    pub fn resolve(
        &self,
        platform: &str,
        account_id: Option<&str>,
    ) -> Result<Arc<dyn MessageAdapter>> {
        if let Some(account_id) = account_id {
            let key = AccountKey {
                platform: platform.into(),
                account_id: account_id.into(),
            };
            key.validate()?;
            return self
                .adapters
                .get(&key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("未注册账号: {platform}/{account_id}"));
        }
        let mut matching = self
            .adapters
            .iter()
            .filter(|(key, _)| key.platform == platform);
        let first = matching
            .next()
            .ok_or_else(|| anyhow::anyhow!("平台 {platform} 没有已注册账号"))?;
        if matching.next().is_some() {
            bail!("平台 {platform} 有多个账号，必须明确提供 account_id");
        }
        Ok(first.1.clone())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub platform: String,
    pub account_id: String,
    pub adapter: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub accounts: Vec<AccountConfig>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            accounts: vec![AccountConfig {
                platform: "wechat".into(),
                account_id: "local".into(),
                adapter: "wechat_desktop".into(),
            }],
        }
    }
}
