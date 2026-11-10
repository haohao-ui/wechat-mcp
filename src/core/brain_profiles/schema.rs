use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SAFE_INT: u64 = 9_007_199_254_740_991;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self("store_unavailable")
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self("invalid_arguments")
    }
}
pub(crate) fn require(ok: bool, code: &'static str) -> Result<()> {
    if ok { Ok(()) } else { Err(Error(code)) }
}
pub(crate) fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
pub fn canonical<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json_canonicalizer::to_vec(v).map_err(|_| Error("invalid_arguments"))
}
pub fn sha256(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        output.push(char::from(HEX[usize::from(b >> 4)]));
        output.push(char::from(HEX[usize::from(b & 15)]));
    }
    output
}
pub fn content_hash<T: Serialize>(v: &T) -> Result<String> {
    let mut v = serde_json::to_value(v)?;
    v.as_object_mut()
        .ok_or(Error("invalid_arguments"))?
        .remove("content_sha256");
    Ok(sha256(&canonical(&v)?))
}
pub(crate) fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(crate) fn account(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256
}
pub(crate) fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn chars(s: &str, max: usize) -> bool {
    s.chars().count() <= max
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleRef {
    pub role_id: String,
    pub revision_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    Private,
    Group,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub contact_id: String,
    pub chat_kind: ChatKind,
    pub role_id: String,
    pub revision_id: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BindingSet {
    #[serde(deserialize_with = "required_option")]
    pub default: Option<RoleRef>,
    pub overrides: Vec<Override>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeRef {
    pub knowledge_id: String,
    pub revision_id: String,
    pub content_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBudget {
    pub max_input_bytes: u64,
    pub knowledge_bytes: u64,
    pub history_bytes: u64,
    pub overflow: String,
}
impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_input_bytes: 49152,
            knowledge_bytes: 16384,
            history_bytes: 16384,
            overflow: "truncate_tail".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Templates {
    #[serde(deserialize_with = "required_option")]
    pub greeting: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub insufficient_information: Option<String>,
    pub unreadable: BTreeMap<String, Option<String>>,
    #[serde(deserialize_with = "required_option")]
    pub unreadable_missing_labels: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub unreadable_many_items: Option<String>,
}
pub const REASONS: [&str; 8] = [
    "unsupported_type",
    "too_large",
    "download_failed",
    "snapshot_changed",
    "extract_failed",
    "empty",
    "transcribe_failed",
    "unknown",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRevision {
    pub schema_version: u32,
    pub owner_account_id: String,
    pub role_id: String,
    pub revision_id: String,
    #[serde(deserialize_with = "required_option")]
    pub parent_revision_id: Option<String>,
    pub created_at: u64,
    pub origin: String,
    pub name: String,
    pub persona: String,
    pub goals: Vec<String>,
    pub tone: String,
    pub business_rules: Vec<String>,
    pub model_ref: String,
    pub knowledge_refs: Vec<KnowledgeRef>,
    pub templates: Templates,
    pub context_budget: ContextBudget,
    pub content_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeRevision {
    pub schema_version: u32,
    pub owner_account_id: String,
    pub knowledge_id: String,
    pub revision_id: String,
    #[serde(deserialize_with = "required_option")]
    pub parent_revision_id: Option<String>,
    pub created_at: u64,
    pub title: String,
    pub text: String,
    pub content_sha256: String,
    pub source_label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleEntry {
    pub owner_account_id: String,
    pub role_id: String,
    pub draft_revision_id: String,
    pub archived: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeEntry {
    pub owner_account_id: String,
    pub knowledge_id: String,
    pub draft_revision_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEntry {
    pub account_id: String,
    pub active: BindingSet,
    pub draft: BindingSet,
    #[serde(deserialize_with = "required_option")]
    pub legacy_fallback: Option<RoleRef>,
    pub execution_mode: ExecutionMode,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationEntry {
    pub id: String,
    pub owner_account_id: String,
    pub source_config_sha256: String,
    pub seed_content_sha256: String,
    pub completed_generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationEntry {
    pub account_id: String,
    pub generation: u64,
    pub active: BindingSet,
    pub at: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WriteResult {
    pub generation: u64,
    #[serde(deserialize_with = "required_option")]
    pub role_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub knowledge_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub revision_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub content_sha256: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub archived: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEntry {
    pub account_id: String,
    pub operation: String,
    pub request_id: String,
    pub request_sha256: String,
    pub result: WriteResult,
    pub at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub schema_version: u32,
    pub generation: u64,
    pub roles: Vec<RoleEntry>,
    pub knowledge: Vec<KnowledgeEntry>,
    pub accounts: Vec<AccountEntry>,
    pub migrations: Vec<MigrationEntry>,
    pub activation_history: Vec<ActivationEntry>,
    pub request_results: Vec<RequestEntry>,
}
impl Default for Root {
    fn default() -> Self {
        Self {
            schema_version: 2,
            generation: 0,
            roles: vec![],
            knowledge: vec![],
            accounts: vec![],
            migrations: vec![],
            activation_history: vec![],
            request_results: vec![],
        }
    }
}

pub(crate) fn template(s: &str) -> Result<()> {
    require(chars(s, 2000), "storage_capacity_exceeded")?;
    let mut tail = s;
    while let Some(i) = tail.find('{') {
        require(!tail[..i].contains('}'), "template_invalid")?;
        let rest = &tail[i + 1..];
        let end = rest.find('}').ok_or(Error("template_invalid"))?;
        require(
            ["label", "labels", "count", "max_mb", "supported_formats"].contains(&&rest[..end]),
            "template_invalid",
        )?;
        tail = &rest[end + 1..];
    }
    require(!tail.contains('}'), "template_invalid")
}
impl RoleRevision {
    pub fn seal(mut self) -> Result<Self> {
        self.content_sha256 = content_hash(&self)?;
        self.validate()?;
        Ok(self)
    }
    pub fn reference(&self) -> RoleRef {
        RoleRef {
            role_id: self.role_id.clone(),
            revision_id: self.revision_id.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        require(self.schema_version == 1, "schema_unsupported")?;
        require(
            account(&self.owner_account_id)
                && id(&self.role_id)
                && id(&self.revision_id)
                && self.parent_revision_id.as_deref().is_none_or(id)
                && self.created_at <= SAFE_INT,
            "invalid_arguments",
        )?;
        require(
            ["user", "legacy_builtin_v1"].contains(&self.origin.as_str())
                && self.model_ref == "config.model_config",
            "invalid_arguments",
        )?;
        require(
            !self.name.is_empty()
                && chars(&self.name, 80)
                && chars(&self.persona, 8000)
                && chars(&self.tone, 2000)
                && self.goals.len() <= 20
                && self.business_rules.len() <= 20
                && self
                    .goals
                    .iter()
                    .chain(&self.business_rules)
                    .all(|x| chars(x, 500))
                && self.knowledge_refs.len() <= 32,
            "storage_capacity_exceeded",
        )?;
        let b = &self.context_budget;
        require(
            (4096..=1_048_576).contains(&b.max_input_bytes)
                && b.knowledge_bytes
                    .checked_add(b.history_bytes)
                    .is_some_and(|n| n <= b.max_input_bytes)
                && b.overflow == "truncate_tail",
            "invalid_arguments",
        )?;
        require(
            self.templates.unreadable.len() == 8
                && REASONS
                    .iter()
                    .all(|r| self.templates.unreadable.contains_key(*r)),
            "invalid_arguments",
        )?;
        for s in self
            .templates
            .unreadable
            .values()
            .chain([
                &self.templates.greeting,
                &self.templates.insufficient_information,
                &self.templates.unreadable_missing_labels,
                &self.templates.unreadable_many_items,
            ])
            .flatten()
        {
            template(s)?;
        }
        for k in &self.knowledge_refs {
            require(
                id(&k.knowledge_id) && id(&k.revision_id) && hex(&k.content_sha256),
                "invalid_arguments",
            )?;
        }
        require(
            serde_json::to_vec(self)?.len() <= 128 * 1024,
            "storage_capacity_exceeded",
        )?;
        require(
            hex(&self.content_sha256) && content_hash(self)? == self.content_sha256,
            "integrity_mismatch",
        )
    }
}
impl KnowledgeRevision {
    pub fn seal(mut self) -> Result<Self> {
        self.content_sha256 = content_hash(&self)?;
        self.validate()?;
        Ok(self)
    }
    pub fn validate(&self) -> Result<()> {
        require(self.schema_version == 1, "schema_unsupported")?;
        require(
            account(&self.owner_account_id)
                && id(&self.knowledge_id)
                && id(&self.revision_id)
                && self.parent_revision_id.as_deref().is_none_or(id)
                && self.created_at <= SAFE_INT,
            "invalid_arguments",
        )?;
        require(
            self.text.len() <= 1024 * 1024
                && chars(&self.title, 500)
                && chars(&self.source_label, 500)
                && serde_json::to_vec(self)?.len() <= 2 * 1024 * 1024,
            "storage_capacity_exceeded",
        )?;
        require(
            hex(&self.content_sha256) && content_hash(self)? == self.content_sha256,
            "integrity_mismatch",
        )
    }
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let v: Value = serde_json::from_slice(bytes)?;
    require(
        v.get("schema_version") == Some(&Value::from(1)),
        "schema_unsupported",
    )?;
    Ok(serde_json::from_value(v)?)
}

pub(crate) fn decode_root<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let v: Value = serde_json::from_slice(bytes)?;
    require(
        v.get("schema_version") == Some(&Value::from(2)),
        "schema_unsupported",
    )?;
    Ok(serde_json::from_value(v)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Legacy,
    Managed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountEntryV1 {
    pub account_id: String,
    pub active: BindingSet,
    pub draft: BindingSet,
    #[serde(deserialize_with = "required_option")]
    pub legacy_fallback: Option<RoleRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootV1 {
    pub schema_version: u32,
    pub generation: u64,
    pub roles: Vec<RoleEntry>,
    pub knowledge: Vec<KnowledgeEntry>,
    pub accounts: Vec<AccountEntryV1>,
    pub migrations: Vec<MigrationEntry>,
    pub activation_history: Vec<ActivationEntry>,
    pub request_results: Vec<RequestEntry>,
}

impl RootV1 {
    pub(crate) fn validate(&self) -> Result<()> {
        require(self.schema_version == 1, "schema_unsupported")?;
        require(self.generation <= SAFE_INT, "invalid_arguments")?;
        require(
            self.accounts.len() <= 16
                && self.roles.len() <= 2048
                && self.knowledge.len() <= 2048
                && self.migrations.len() <= 64
                && self.activation_history.len() <= 1600
                && self.request_results.len() <= 16000,
            "storage_capacity_exceeded",
        )?;
        let mut accounts = std::collections::HashSet::new();
        for a in &self.accounts {
            require(
                account(&a.account_id) && accounts.insert(&a.account_id),
                "invalid_arguments",
            )?;
            a.active.validate()?;
            a.draft.validate()?;
            if let Some(r) = &a.legacy_fallback {
                require(id(&r.role_id) && id(&r.revision_id), "invalid_arguments")?;
            }
        }
        let mut ids = std::collections::HashSet::new();
        for e in &self.roles {
            require(
                accounts.contains(&e.owner_account_id)
                    && id(&e.role_id)
                    && id(&e.draft_revision_id)
                    && ids.insert(&e.role_id),
                "integrity_mismatch",
            )?;
        }
        ids.clear();
        for e in &self.knowledge {
            require(
                accounts.contains(&e.owner_account_id)
                    && id(&e.knowledge_id)
                    && id(&e.draft_revision_id)
                    && ids.insert(&e.knowledge_id),
                "integrity_mismatch",
            )?;
        }
        for a in &self.accounts {
            require(
                self.roles
                    .iter()
                    .filter(|r| r.owner_account_id == a.account_id)
                    .count()
                    <= 128
                    && self
                        .knowledge
                        .iter()
                        .filter(|r| r.owner_account_id == a.account_id)
                        .count()
                        <= 128
                    && self
                        .migrations
                        .iter()
                        .filter(|r| r.owner_account_id == a.account_id)
                        .count()
                        <= 4
                    && self
                        .activation_history
                        .iter()
                        .filter(|r| r.account_id == a.account_id)
                        .count()
                        <= 100
                    && self
                        .request_results
                        .iter()
                        .filter(|r| r.account_id == a.account_id)
                        .count()
                        <= 1000,
                "storage_capacity_exceeded",
            )?;
            for b in [&a.active, &a.draft] {
                self.validate_refs(&a.account_id, b)?;
            }
            if let Some(r) = &a.legacy_fallback {
                self.validate_ref(&a.account_id, r)?;
            }
        }
        let mut seen = std::collections::HashSet::new();
        for m in &self.migrations {
            require(
                accounts.contains(&m.owner_account_id)
                    && !m.id.is_empty()
                    && m.id.len() <= 274
                    && hex(&m.source_config_sha256)
                    && hex(&m.seed_content_sha256)
                    && m.completed_generation <= self.generation
                    && seen.insert((&m.owner_account_id, &m.id)),
                "integrity_mismatch",
            )?;
        }
        let mut seen = std::collections::HashSet::new();
        for h in &self.activation_history {
            require(
                accounts.contains(&h.account_id)
                    && h.generation <= self.generation
                    && h.at <= SAFE_INT
                    && seen.insert((&h.account_id, h.generation)),
                "integrity_mismatch",
            )?;
            h.active.validate()?;
            self.validate_refs(&h.account_id, &h.active)?;
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.request_results {
            require(
                accounts.contains(&r.account_id)
                    && [
                        "brain-save-role",
                        "brain-save-knowledge",
                        "brain-copy-role",
                        "brain-archive-role",
                        "brain-save-bindings",
                        "brain-activate",
                        "brain-rollback",
                    ]
                    .contains(&r.operation.as_str())
                    && uuid::Uuid::parse_str(&r.request_id)
                        .is_ok_and(|u| u.to_string() == r.request_id)
                    && hex(&r.request_sha256)
                    && r.at <= SAFE_INT
                    && r.result.generation <= self.generation
                    && seen.insert((&r.account_id, &r.operation, &r.request_id)),
                "integrity_mismatch",
            )?;
        }
        Ok(())
    }
    fn validate_ref(&self, account: &str, r: &RoleRef) -> Result<()> {
        require(
            self.roles
                .iter()
                .any(|e| e.owner_account_id == account && e.role_id == r.role_id),
            "integrity_mismatch",
        )
    }
    fn validate_refs(&self, account: &str, b: &BindingSet) -> Result<()> {
        if let Some(r) = &b.default {
            self.validate_ref(account, r)?;
        }
        for o in &b.overrides {
            self.validate_ref(
                account,
                &RoleRef {
                    role_id: o.role_id.clone(),
                    revision_id: o.revision_id.clone(),
                },
            )?;
        }
        Ok(())
    }
}

pub(crate) fn decode_v1_root(bytes: &[u8]) -> Result<RootV1> {
    let v: Value = serde_json::from_slice(bytes)?;
    require(
        v.get("schema_version") == Some(&Value::from(1)),
        "schema_unsupported",
    )?;
    Ok(serde_json::from_value(v)?)
}
