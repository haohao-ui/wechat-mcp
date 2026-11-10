pub use crate::core::open_api::http::OpenApiConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

unsafe extern "C" {
    fn getuid() -> u32;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowlistEntry {
    pub recipient: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryHttpConfig {
    pub bind: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_output: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrainMode {
    Builtin,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimeoutAction {
    Skip,
    Builtin,
    Handoff,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalBrainConfig {
    pub timeout_seconds: u64,
    pub timeout_action: TimeoutAction,
    pub queue_minutes: u64,
}

impl Default for ExternalBrainConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 120,
            timeout_action: TimeoutAction::Skip,
            queue_minutes: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrainConfig {
    pub mode: BrainMode,
    pub external: ExternalBrainConfig,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            mode: BrainMode::Builtin,
            external: ExternalBrainConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub host: String,
    pub account_id: String,
    pub allowlist: Vec<AllowlistEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignorelist: Option<Vec<AllowlistEntry>>,
    pub state_dir: PathBuf,
    pub poll_seconds: u64,
    pub cooldown_seconds: u64,
    pub max_replies_per_hour: u64,
    /// Accepted for config compatibility only; chats are no longer visited periodically.
    pub idle_check_seconds: u64,
    pub turn_quiet_seconds: u64,
    pub turn_max_seconds: u64,
    pub images_per_model_call: usize,
    pub max_attachment_mb: u64,
    pub context_messages: usize,
    pub manage_window: bool,
    pub capture_backend: String,
    pub history_http: HistoryHttpConfig,
    #[serde(default)]
    pub open_api: OpenApiConfig,
    pub model_config: ModelConfig,
    pub brain: BrainConfig,
    #[serde(default)]
    pub friend_requests: crate::core::friend_requests::Config,
}

const INVISIBLE_CHARS: &[char] = &[
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{2060}', '\u{FEFF}',
];

fn is_invisible(c: char) -> bool {
    INVISIBLE_CHARS.contains(&c)
}

fn is_variation(c: char) -> bool {
    matches!(c, '\u{FE00}'..='\u{FE0F}')
}

fn is_emoji_or_symbol(c: char) -> bool {
    matches!(
        c,
        '\u{1F000}'..='\u{1FAFF}'
            | '\u{2600}'..='\u{27BF}'
            | '\u{2300}'..='\u{23FF}'
            | '\u{2B00}'..='\u{2BFF}'
            | '\u{2000}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{25A0}'..='\u{25FF}'
            | '\u{2900}'..='\u{297F}'
    )
}

pub fn clean_title(raw: &str) -> String {
    raw.chars()
        .filter(|c| !is_invisible(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn nfkc_normalize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            // Full-width ASCII variants 0xFF01..=0xFF5E -> 0x0021..=0x007E
            '\u{FF01}'..='\u{FF5E}' => {
                if let Some(ascii) = char::from_u32(c as u32 - 0xFEE0) {
                    out.push(ascii);
                } else {
                    out.push(c);
                }
            }
            // Ideographic space
            '\u{3000}' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

pub fn name_key(raw: &str) -> String {
    let text = nfkc_normalize(raw);
    let visible: String = text
        .chars()
        .filter(|c| !is_invisible(*c) && !is_variation(*c) && !c.is_whitespace())
        .collect();
    let key: String = visible
        .chars()
        .filter(|c| !is_emoji_or_symbol(*c))
        .collect();
    if key.is_empty() { visible } else { key }
}

pub fn candidate_names(entry: &AllowlistEntry) -> Vec<String> {
    let mut names = vec![entry.recipient.clone()];
    if let Some(aliases) = &entry.aliases {
        names.extend(aliases.clone());
    }
    names
}

fn validate_rules(value: &Value) -> Result<()> {
    let mut seen_keys: HashMap<String, usize> = HashMap::new();

    for list_name in ["allowlist", "ignorelist"] {
        let entries_opt = value.get(list_name);
        if list_name == "allowlist" && entries_opt.is_none() {
            bail!("at least one explicitly allowlisted conversation required");
        }
        let Some(entries_val) = entries_opt else {
            continue;
        };

        let Some(entries) = entries_val.as_array() else {
            bail!(
                "{} must be an array of at most 200 conversations",
                list_name
            );
        };
        if entries.len() > 200 {
            bail!(
                "{} must be an array of at most 200 conversations",
                list_name
            );
        }
        if list_name == "allowlist" && entries.is_empty() {
            bail!("at least one explicitly allowlisted conversation required");
        }

        let mut seen = HashSet::new();

        for (idx, entry) in entries.iter().enumerate() {
            let Some(map) = entry.as_object() else {
                bail!("invalid {} entry", list_name);
            };

            for k in map.keys() {
                if k != "recipient" && k != "kind" && k != "aliases" {
                    bail!("invalid {} entry", list_name);
                }
            }
            if !map.contains_key("recipient") || !map.contains_key("kind") {
                bail!("invalid {} entry", list_name);
            }

            let recipient = map["recipient"]
                .as_str()
                .context(format!("invalid {} entry", list_name))?;
            let kind = map["kind"]
                .as_str()
                .context(format!("invalid {} entry", list_name))?;

            if kind != "private" && kind != "group" {
                bail!("conversation kind must be private or group");
            }

            let mut aliases = Vec::new();
            if let Some(aliases_val) = map.get("aliases") {
                let Some(arr) = aliases_val.as_array() else {
                    bail!("aliases must be an array of at most 10 names");
                };
                if arr.len() > 10 {
                    bail!("aliases must be an array of at most 10 names");
                }
                for a in arr {
                    let Some(a_str) = a.as_str() else {
                        bail!("aliases must be an array of at most 10 names");
                    };
                    aliases.push(a_str.to_string());
                }
            }

            for val in std::iter::once(recipient).chain(aliases.iter().map(|s| s.as_str())) {
                if val.is_empty() || val.trim() != val {
                    bail!("exact conversation name without surrounding whitespace required");
                }
                if val.chars().count() > 64 {
                    bail!("conversation names are limited to 64 characters (native worker limit)");
                }
            }

            let identity = (recipient.to_string(), kind.to_string());
            if !seen.insert(identity) {
                bail!("duplicate {} entry", list_name);
            }

            if list_name == "allowlist" {
                let mut c_names = vec![recipient.to_string()];
                c_names.extend(aliases);
                for n in c_names {
                    let key = name_key(&n);
                    if let Some(existing_idx) = seen_keys.get(&key) {
                        if *existing_idx != idx {
                            bail!("allowlist names collide after normalization");
                        }
                    } else {
                        seen_keys.insert(key, idx);
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_api_key_file(path_str: &str) -> Result<()> {
    let path = Path::new(path_str);
    if !path.is_absolute() {
        bail!("absolute api_key_file required");
    }
    let sym_meta = fs::symlink_metadata(path)
        .with_context(|| format!("api_key_file does not exist: {}", path.display()))?;
    if sym_meta.file_type().is_symlink() {
        bail!("api_key_file must not be a symlink");
    }
    if !sym_meta.is_file() {
        bail!("api_key_file must be a regular file");
    }
    let current_uid = unsafe { getuid() };
    if sym_meta.uid() != current_uid {
        bail!("api_key_file must be owned by the current user");
    }
    let mode = sym_meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        bail!("api_key_file must have mode 0600, found {:o}", mode);
    }
    Ok(())
}

fn validate_model_config(config_val: &Value) -> Result<ModelConfig> {
    let Some(map) = config_val.as_object() else {
        bail!("model_config must be an object");
    };
    let backend = map
        .get("backend")
        .and_then(|v| v.as_str())
        .unwrap_or("codex-exec");

    if backend == "codex-exec" {
        for k in map.keys() {
            if k != "backend" {
                bail!("unsupported codex model setting");
            }
        }
        return Ok(ModelConfig {
            backend: "codex-exec".into(),
            base_url: None,
            model: None,
            api_key_file: None,
            api_key_env: None,
            supports_images: None,
            timeout_seconds: None,
            json_output: None,
        });
    }

    if backend != "openai-compatible" {
        bail!("unsupported model backend or setting");
    }

    let allowed = [
        "backend",
        "base_url",
        "model",
        "api_key_file",
        "api_key_env",
        "supports_images",
        "timeout_seconds",
        "json_output",
    ];
    for k in map.keys() {
        if !allowed.contains(&k.as_str()) {
            bail!("unsupported model backend or setting");
        }
    }

    let base_url_str = map
        .get("base_url")
        .and_then(|v| v.as_str())
        .context("model base_url requires HTTPS, or local HTTP, without credentials/query")?;

    let parsed_url = reqwest::Url::parse(base_url_str).map_err(|_| {
        anyhow::anyhow!("model base_url requires HTTPS, or local HTTP, without credentials/query")
    })?;

    let host_str = parsed_url
        .host_str()
        .context("model base_url requires HTTPS, or local HTTP, without credentials/query")?;
    let is_local = host_str == "localhost" || host_str == "127.0.0.1" || host_str == "::1";
    let scheme = parsed_url.scheme();
    if (scheme != "https" && !(scheme == "http" && is_local))
        || !parsed_url.username().is_empty()
        || parsed_url.password().is_some()
        || parsed_url.query().is_some()
        || parsed_url.fragment().is_some()
    {
        bail!("model base_url requires HTTPS, or local HTTP, without credentials/query");
    }

    let model = map
        .get("model")
        .and_then(|v| v.as_str())
        .context("explicit model name required")?;
    if model.trim().is_empty() {
        bail!("explicit model name required");
    }

    let api_key_file = if let Some(akf) = map.get("api_key_file") {
        let path_str = akf.as_str().context("absolute api_key_file required")?;
        validate_api_key_file(path_str)?;
        Some(path_str.to_string())
    } else {
        None
    };

    let api_key_env = if let Some(ake) = map.get("api_key_env") {
        let env_str = ake.as_str().context("invalid api_key_env name")?;
        let is_ident = !env_str.is_empty()
            && env_str.chars().enumerate().all(|(i, c)| {
                if i == 0 {
                    c.is_ascii_alphabetic() || c == '_'
                } else {
                    c.is_ascii_alphanumeric() || c == '_'
                }
            });
        if !is_ident {
            bail!("invalid api_key_env name");
        }
        Some(env_str.to_string())
    } else {
        None
    };

    let supports_images = if let Some(si) = map.get("supports_images") {
        let b = si.as_bool().context("invalid supports_images")?;
        Some(b)
    } else {
        Some(false)
    };

    let json_output = if let Some(jo) = map.get("json_output") {
        let b = jo.as_bool().context("invalid json_output")?;
        Some(b)
    } else {
        Some(true)
    };

    let timeout_seconds = if let Some(ts) = map.get("timeout_seconds") {
        let num = ts
            .as_u64()
            .context("model timeout must be 1..180 seconds")?;
        if !(1..=180).contains(&num) {
            bail!("model timeout must be 1..180 seconds");
        }
        Some(num)
    } else {
        Some(60)
    };

    Ok(ModelConfig {
        backend: backend.to_string(),
        base_url: Some(base_url_str.to_string()),
        model: Some(model.to_string()),
        api_key_file,
        api_key_env,
        supports_images,
        timeout_seconds,
        json_output,
    })
}

fn validate_brain_config(val: Option<&Value>) -> Result<BrainConfig> {
    let Some(val) = val else {
        return Ok(BrainConfig::default());
    };
    let map = val.as_object().context("brain must be an object")?;
    for k in map.keys() {
        if k != "mode" && k != "external" {
            bail!("unsupported brain config field: {}", k);
        }
    }
    let mode = match map.get("mode") {
        Some(m) => {
            let s = m.as_str().context("brain mode must be string")?;
            match s {
                "builtin" => BrainMode::Builtin,
                "external" => BrainMode::External,
                _ => bail!("invalid brain mode: {}", s),
            }
        }
        None => BrainMode::Builtin,
    };

    let external = match map.get("external") {
        Some(ext_val) => {
            let ext_map = ext_val
                .as_object()
                .context("external brain config must be an object")?;
            for k in ext_map.keys() {
                if !["timeout_seconds", "timeout_action", "queue_minutes"].contains(&k.as_str()) {
                    bail!("unsupported external brain config field: {}", k);
                }
            }
            let timeout_seconds = match ext_map.get("timeout_seconds") {
                Some(v) => {
                    let n = v.as_u64().context("invalid timeout_seconds")?;
                    if !(1..=3600).contains(&n) {
                        bail!("timeout_seconds must be 1..3600");
                    }
                    n
                }
                None => 120,
            };
            let timeout_action = match ext_map.get("timeout_action") {
                Some(v) => {
                    let s = v.as_str().context("invalid timeout_action")?;
                    match s {
                        "skip" => TimeoutAction::Skip,
                        "builtin" => TimeoutAction::Builtin,
                        "handoff" => TimeoutAction::Handoff,
                        _ => bail!("invalid timeout_action: {}", s),
                    }
                }
                None => TimeoutAction::Skip,
            };
            let queue_minutes = match ext_map.get("queue_minutes") {
                Some(v) => {
                    let n = v.as_u64().context("invalid queue_minutes")?;
                    if !(1..=1440).contains(&n) {
                        bail!("queue_minutes must be 1..1440");
                    }
                    n
                }
                None => 30,
            };
            ExternalBrainConfig {
                timeout_seconds,
                timeout_action,
                queue_minutes,
            }
        }
        None => ExternalBrainConfig::default(),
    };

    Ok(BrainConfig { mode, external })
}

impl BotConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let value: Value = serde_json::from_str(&content)
            .with_context(|| format!("invalid JSON in config file {}", path.display()))?;

        let map = value
            .as_object()
            .context("unsupported bot config schema or field")?;

        let allowed = [
            "schema_version",
            "mode",
            "host",
            "account_id",
            "allowlist",
            "state_dir",
            "poll_seconds",
            "cooldown_seconds",
            "max_replies_per_hour",
            "capture_backend",
            "model_config",
            "ignorelist",
            "idle_check_seconds",
            "context_messages",
            "history_http",
            "open_api",
            "manage_window",
            "turn_quiet_seconds",
            "turn_max_seconds",
            "images_per_model_call",
            "max_attachment_mb",
            "brain",
            "friend_requests",
        ];

        for k in map.keys() {
            if !allowed.contains(&k.as_str()) {
                bail!("unsupported bot config schema or field");
            }
        }

        let schema_version = map
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .context("unsupported bot config schema or field")? as u32;
        if schema_version != 1 {
            bail!("unsupported bot config schema or field");
        }

        validate_rules(&value)?;

        let account_id = map
            .get("account_id")
            .and_then(|v| v.as_str())
            .context("explicit account required")?;
        if account_id.is_empty() {
            bail!("explicit account required");
        }

        let host = map
            .get("host")
            .and_then(|v| v.as_str())
            .context("invalid SSH host")?;
        if host != "local" {
            bail!("host must be local (core service runs locally only)");
        }

        let state_dir_str = map
            .get("state_dir")
            .and_then(|v| v.as_str())
            .context("absolute private state_dir required")?;
        let state_dir = PathBuf::from(state_dir_str);
        if !state_dir.is_absolute() {
            bail!("absolute private state_dir required");
        }

        let capture_backend = map
            .get("capture_backend")
            .map(|v| {
                let s = v.as_str().context("invalid capture_backend")?;
                if s != "native" && s != "legacy" {
                    bail!("invalid capture_backend");
                }
                Ok::<String, anyhow::Error>(s.to_string())
            })
            .transpose()?
            .unwrap_or_else(|| "native".to_string());

        fn check_num<T: TryFrom<u64>>(
            map: &serde_json::Map<String, Value>,
            key: &str,
            default: u64,
            low: u64,
            high: u64,
        ) -> Result<T> {
            let num = match map.get(key) {
                Some(v) => {
                    let n = v.as_u64().context(format!("invalid {}", key))?;
                    if n < low || n > high {
                        bail!("invalid {}", key);
                    }
                    n
                }
                None => default,
            };
            T::try_from(num).map_err(|_| anyhow::anyhow!("invalid {}", key))
        }

        let poll_seconds: u64 = check_num(map, "poll_seconds", 2, 1, 60)?;
        let cooldown_seconds: u64 = check_num(map, "cooldown_seconds", 15, 0, 300)?;
        let max_replies_per_hour: u64 = check_num(map, "max_replies_per_hour", 20, 1, 100)?;
        let idle_check_seconds: u64 = check_num(map, "idle_check_seconds", 60, 10, 3600)?;
        let turn_quiet_seconds: u64 = check_num(map, "turn_quiet_seconds", 10, 0, 120)?;
        let turn_max_seconds: u64 = check_num(map, "turn_max_seconds", 60, 10, 600)?;
        let images_per_model_call: usize = check_num(map, "images_per_model_call", 4, 1, 16)?;
        let max_attachment_mb: u64 = check_num(map, "max_attachment_mb", 50, 1, 200)?;
        let context_messages: usize = check_num(map, "context_messages", 5, 0, 100)?;

        let manage_window = match map.get("manage_window") {
            Some(v) => v.as_bool().context("invalid manage_window")?,
            None => true,
        };

        let open_api = match map.get("open_api") {
            Some(value) => serde_json::from_value(value.clone()).context("invalid open_api")?,
            None => OpenApiConfig::default(),
        };

        let history_http = match map.get("history_http") {
            Some(v) => {
                let hmap = v.as_object().context("invalid history_http")?;
                for k in hmap.keys() {
                    if k != "bind" && k != "port" {
                        bail!("invalid history_http");
                    }
                }
                let bind = match hmap.get("bind") {
                    Some(bv) => bv.as_str().context("invalid history_http")?.to_string(),
                    None => "127.0.0.1".to_string(),
                };
                let port = match hmap.get("port") {
                    Some(pv) => {
                        let p = pv.as_u64().context("invalid history_http")?;
                        if !(1..=65535).contains(&p) {
                            bail!("invalid history_http");
                        }
                        p as u16
                    }
                    None => 8765,
                };
                HistoryHttpConfig { bind, port }
            }
            None => HistoryHttpConfig {
                bind: "127.0.0.1".into(),
                port: 8765,
            },
        };

        let model_config_val = map.get("model_config").cloned().unwrap_or_else(|| {
            serde_json::json!({
                "backend": "codex-exec"
            })
        });
        let model_config = validate_model_config(&model_config_val)?;

        let brain = validate_brain_config(map.get("brain"))?;

        let allowlist: Vec<AllowlistEntry> =
            serde_json::from_value(map["allowlist"].clone()).context("invalid allowlist")?;

        let ignorelist: Option<Vec<AllowlistEntry>> = map
            .get("ignorelist")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .context("invalid ignorelist")?;

        let mode = match map.get("mode") {
            Some(v) => {
                let s = v
                    .as_str()
                    .context("invalid mode (must be 'service' or 'test')")?;
                if s != "service" && s != "test" {
                    bail!("invalid mode (must be 'service' or 'test')");
                }
                Some(s.to_string())
            }
            None => None,
        };

        Ok(BotConfig {
            schema_version,
            mode,
            host: host.to_string(),
            account_id: account_id.to_string(),
            allowlist,
            ignorelist,
            state_dir,
            poll_seconds,
            cooldown_seconds,
            max_replies_per_hour,
            idle_check_seconds,
            turn_quiet_seconds,
            turn_max_seconds,
            images_per_model_call,
            max_attachment_mb,
            context_messages,
            manage_window,
            capture_backend,
            history_http,
            open_api,
            model_config,
            friend_requests: serde_json::from_value(
                map.get("friend_requests")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            )
            .context("invalid friend_requests")?,
            brain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("test-bot-cfg-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn base_config(dir: &Path) -> serde_json::Value {
        json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "test-account",
            "allowlist": [{"recipient": "示例联系人甲", "kind": "private"}],
            "state_dir": dir.join("state").to_str().unwrap()
        })
    }

    #[test]
    fn test_config_context_messages_and_http_bounds() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.context_messages, 5);
        assert_eq!(
            loaded.history_http,
            HistoryHttpConfig {
                bind: "127.0.0.1".into(),
                port: 8765
            }
        );
        assert!(loaded.manage_window);

        let invalid_extras = [
            json!({"manage_window": "yes"}),
            json!({"manage_window": 1}),
            json!({"context_messages": -1}),
            json!({"context_messages": 101}),
            json!({"context_messages": "5"}),
            json!({"history_http": {"bind": "127.0.0.1", "port": 0}}),
            json!({"history_http": {"host": "x"}}),
        ];

        for extra in invalid_extras {
            let mut val = base.clone();
            val.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            fs::write(&path, serde_json::to_string(&val).unwrap()).unwrap();
            assert!(
                BotConfig::load(&path).is_err(),
                "Expected error for extra: {:?}",
                extra
            );
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_config_turn_keys_validated() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.turn_quiet_seconds, 10);
        assert_eq!(loaded.turn_max_seconds, 60);
        assert_eq!(loaded.images_per_model_call, 4);

        let invalid_cases = [
            json!({"turn_quiet_seconds": -1}),
            json!({"turn_quiet_seconds": 121}),
            json!({"turn_max_seconds": 5}),
            json!({"turn_max_seconds": "60"}),
            json!({"images_per_model_call": 0}),
            json!({"images_per_model_call": 17}),
        ];

        for extra in invalid_cases {
            let mut val = base.clone();
            val.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            fs::write(&path, serde_json::to_string(&val).unwrap()).unwrap();
            assert!(
                BotConfig::load(&path).is_err(),
                "Expected error for {:?}",
                extra
            );
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_config_rejects_group_and_ambiguous_allowlist() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        let invalid_allowlists = [
            json!([]),
            json!([{"recipient": "群", "kind": "channel"}]),
            json!([
                {"recipient": "示例联系人甲", "kind": "private"},
                {"recipient": "示例联系人甲", "kind": "private"}
            ]),
        ];

        for allowlist in invalid_allowlists {
            let mut val = base.clone();
            val["allowlist"] = allowlist;
            fs::write(&path, serde_json::to_string(&val).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_err());
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_idle_check_seconds_defaults_and_bounds() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        assert_eq!(BotConfig::load(&path).unwrap().idle_check_seconds, 60);

        for val in [json!(5), json!("60"), json!(7200)] {
            let mut cfg = base.clone();
            cfg["idle_check_seconds"] = val;
            fs::write(&path, serde_json::to_string(&cfg).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_err());
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_load_config_accepts_local() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.host, "local");
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_load_config_rejects_non_local_or_invalid_host() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        for bad_host in ["", "wechat-host", "-invalid", "local host", "remote"] {
            let mut val = base.clone();
            val["host"] = json!(bad_host);
            fs::write(&path, serde_json::to_string(&val).unwrap()).unwrap();
            assert!(
                BotConfig::load(&path).is_err(),
                "Expected reject for host: {}",
                bad_host
            );
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_config_transport_and_fields() {
        let td = temp_dir();
        let path = td.join("config.json");
        let key_file = td.join("key");
        fs::write(&key_file, "fixture-key").unwrap();
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();

        let mut base = base_config(&td);
        base["model_config"] = json!({
            "backend": "openai-compatible",
            "base_url": "https://api.deepseek.com",
            "model": "fixture-model",
            "api_key_file": key_file.to_str().unwrap()
        });

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_ok());

        // Local http allowed
        let mut local_http = base.clone();
        local_http["model_config"]["base_url"] = json!("http://localhost:8000/v1");
        fs::write(&path, serde_json::to_string(&local_http).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_ok());

        let invalid_deltas = [
            json!({"base_url": "http://api.deepseek.com"}),
            json!({"base_url": "https://user:pass@example.com"}),
            json!({"api_key": "secret"}),
            json!({"timeout_seconds": 0}),
            json!({"supports_images": "yes"}),
        ];

        for delta in invalid_deltas {
            let mut bad = base.clone();
            bad["model_config"]
                .as_object_mut()
                .unwrap()
                .extend(delta.as_object().unwrap().clone());
            fs::write(&path, serde_json::to_string(&bad).unwrap()).unwrap();
            assert!(
                BotConfig::load(&path).is_err(),
                "Expected reject for delta {:?}",
                delta
            );
        }
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_api_key_file_permissions() {
        let td = temp_dir();
        let path = td.join("config.json");
        let key_file = td.join("key");
        fs::write(&key_file, "fixture-key").unwrap();
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o644)).unwrap();

        let mut base = base_config(&td);
        base["model_config"] = json!({
            "backend": "openai-compatible",
            "base_url": "https://api.deepseek.com",
            "model": "fixture-model",
            "api_key_file": key_file.to_str().unwrap()
        });

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        // Permission 0o644 should be rejected
        assert!(BotConfig::load(&path).is_err());

        // Fix to 0o600
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(BotConfig::load(&path).is_ok());

        // Symlink rejected
        let symlink = td.join("key_symlink");
        std::os::unix::fs::symlink(&key_file, &symlink).unwrap();
        base["model_config"]["api_key_file"] = json!(symlink.to_str().unwrap());
        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());

        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_unknown_fields_rejected() {
        let td = temp_dir();
        let path = td.join("config.json");
        let mut base = base_config(&td);
        base["unknown_setting_x"] = json!(true);

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_state_dir_must_be_absolute() {
        let td = temp_dir();
        let path = td.join("config.json");
        let mut base = base_config(&td);
        base["state_dir"] = json!("relative/state");

        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());
        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_names_normalization_rules() {
        assert_eq!(name_key("🍓小美丨鞋"), name_key("小美丨鞋"));
        assert_eq!(name_key("✨Ａｂｃ ★"), "Abc");
        assert_eq!(name_key("小\u{200B}美\u{FE0F}"), "小美");
        assert_eq!(name_key("🍓🍓"), "🍓🍓");
        assert_eq!(clean_title(" \u{200B}🍓小美 "), "🍓小美");
    }

    #[test]
    fn test_brain_config_defaults_and_validation() {
        let td = temp_dir();
        let path = td.join("config.json");
        let base = base_config(&td);

        // 1. 默认缺省 brain -> builtin
        fs::write(&path, serde_json::to_string(&base).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.brain.mode, BrainMode::Builtin);
        assert_eq!(loaded.brain.external.timeout_seconds, 120);
        assert_eq!(loaded.brain.external.timeout_action, TimeoutAction::Skip);
        assert_eq!(loaded.brain.external.queue_minutes, 30);

        // 2. 正常配置 external
        let mut ext_cfg = base.clone();
        ext_cfg["brain"] = json!({
            "mode": "external",
            "external": {
                "timeout_seconds": 60,
                "timeout_action": "handoff",
                "queue_minutes": 15
            }
        });
        fs::write(&path, serde_json::to_string(&ext_cfg).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.brain.mode, BrainMode::External);
        assert_eq!(loaded.brain.external.timeout_seconds, 60);
        assert_eq!(loaded.brain.external.timeout_action, TimeoutAction::Handoff);
        assert_eq!(loaded.brain.external.queue_minutes, 15);

        // 3. timeout_action: builtin
        let mut builtin_action = base.clone();
        builtin_action["brain"] = json!({
            "mode": "external",
            "external": {
                "timeout_action": "builtin"
            }
        });
        fs::write(&path, serde_json::to_string(&builtin_action).unwrap()).unwrap();
        let loaded = BotConfig::load(&path).unwrap();
        assert_eq!(loaded.brain.external.timeout_action, TimeoutAction::Builtin);

        // 4. 边界值测试
        for valid_sec in [1, 3600] {
            let mut c = base.clone();
            c["brain"] = json!({"mode": "external", "external": {"timeout_seconds": valid_sec}});
            fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_ok());
        }
        for invalid_sec in [0, 3601, -1] {
            let mut c = base.clone();
            c["brain"] = json!({"mode": "external", "external": {"timeout_seconds": invalid_sec}});
            fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_err());
        }

        for valid_qm in [1, 1440] {
            let mut c = base.clone();
            c["brain"] = json!({"mode": "external", "external": {"queue_minutes": valid_qm}});
            fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_ok());
        }
        for invalid_qm in [0, 1441] {
            let mut c = base.clone();
            c["brain"] = json!({"mode": "external", "external": {"queue_minutes": invalid_qm}});
            fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_err());
        }

        // 5. 拒绝非法 action 和非法 mode
        for bad_action in ["unknown", "", "none"] {
            let mut c = base.clone();
            c["brain"] = json!({"mode": "external", "external": {"timeout_action": bad_action}});
            fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
            assert!(BotConfig::load(&path).is_err());
        }
        let mut bad_mode = base.clone();
        bad_mode["brain"] = json!({"mode": "other"});
        fs::write(&path, serde_json::to_string(&bad_mode).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());

        // 6. 拒绝额外未知字段
        let mut unknown_brain_field = base.clone();
        unknown_brain_field["brain"] = json!({"mode": "builtin", "extra": 123});
        fs::write(&path, serde_json::to_string(&unknown_brain_field).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());

        let mut unknown_ext_field = base.clone();
        unknown_ext_field["brain"] =
            json!({"mode": "external", "external": {"unknown_field": true}});
        fs::write(&path, serde_json::to_string(&unknown_ext_field).unwrap()).unwrap();
        assert!(BotConfig::load(&path).is_err());

        let _ = fs::remove_dir_all(td);
    }
}
