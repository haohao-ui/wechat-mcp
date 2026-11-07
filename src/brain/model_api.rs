use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::core::config::ModelConfig;

const O_NONBLOCK: i32 = 0x0004;
const O_NOFOLLOW: i32 = 0x0100;

#[derive(Debug)]
pub struct ModelOutputError(pub String);

impl std::fmt::Display for ModelOutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ModelOutputError {}

pub fn validate_reply(value: &str) -> Result<String> {
    if value.trim().is_empty() || value.chars().count() > 200 {
        bail!("model reply must be 1..200 characters");
    }
    if value.chars().any(|c| (c as u32) < 32 && c != '\n') {
        bail!("reply contains control characters");
    }
    Ok(value.to_string())
}

#[derive(Debug, PartialEq)]
struct ParsedUrlParts {
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
}

fn parse_base_url(raw: &str) -> Result<ParsedUrlParts> {
    if raw.contains('@') || raw.contains('?') || raw.contains('#') {
        bail!("model base_url requires HTTPS, or local HTTP, without credentials/query");
    }

    let (scheme, rest) = if let Some(stripped) = raw.strip_prefix("https://") {
        ("https", stripped)
    } else if let Some(stripped) = raw.strip_prefix("http://") {
        ("http", stripped)
    } else {
        bail!("model base_url requires HTTPS, or local HTTP, without credentials/query");
    };

    let (host_port, path_part) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };

    if host_port.is_empty() {
        bail!("model base_url requires HTTPS, or local HTTP, without credentials/query");
    }

    let (host, port) = if host_port.starts_with('[') {
        if let Some(end_bracket) = host_port.find(']') {
            let h = &host_port[1..end_bracket];
            let after = &host_port[end_bracket + 1..];
            let p = if let Some(port_str) = after.strip_prefix(':') {
                Some(port_str.parse::<u16>().map_err(|_| anyhow::anyhow!("invalid port"))?)
            } else {
                None
            };
            (h, p)
        } else {
            bail!("invalid ipv6 host");
        }
    } else if let Some(colon) = host_port.rfind(':') {
        let h = &host_port[..colon];
        let p = host_port[colon + 1..]
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("invalid port"))?;
        (h, Some(p))
    } else {
        (host_port, None)
    };

    let is_local = host == "localhost" || host == "127.0.0.1" || host == "::1";
    let valid_scheme = scheme == "https" || (scheme == "http" && is_local);

    if !valid_scheme {
        bail!("model base_url requires HTTPS, or local HTTP, without credentials/query");
    }

    Ok(ParsedUrlParts {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
        path: path_part.to_string(),
    })
}

pub fn extract_host_from_base_url(raw: &str) -> String {
    parse_base_url(raw).map(|p| p.host).unwrap_or_default()
}

pub fn validate_config(config: &Value) -> Result<()> {
    let obj = config.as_object().context("model_config must be an object")?;
    let backend = obj
        .get("backend")
        .and_then(|v| v.as_str())
        .unwrap_or("codex-exec");

    if backend == "codex-exec" {
        for key in obj.keys() {
            if key != "backend" {
                bail!("unsupported codex model setting");
            }
        }
        return Ok(());
    }

    const ALLOWED: &[&str] = &[
        "backend",
        "base_url",
        "model",
        "api_key_file",
        "api_key_env",
        "supports_images",
        "timeout_seconds",
        "json_output",
    ];

    if backend != "openai-compatible" {
        bail!("unsupported model backend or setting");
    }
    for key in obj.keys() {
        if !ALLOWED.contains(&key.as_str()) {
            bail!("unsupported model backend or setting");
        }
    }

    let base_url_str = obj
        .get("base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    parse_base_url(base_url_str)?;

    let model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .unwrap_or("");
    if model.is_empty() {
        bail!("explicit model name required");
    }

    if let Some(akf) = obj.get("api_key_file") {
        let s = akf.as_str().context("absolute api_key_file required")?;
        if !Path::new(s).is_absolute() {
            bail!("absolute api_key_file required");
        }
    }

    if let Some(ake) = obj.get("api_key_env") {
        let s = ake.as_str().context("invalid api_key_env name")?;
        let is_valid_name = !s.is_empty()
            && s.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !is_valid_name {
            bail!("invalid api_key_env name");
        }
    }

    for key in &["supports_images", "json_output"] {
        if let Some(val) = obj.get(*key) {
            if !val.is_boolean() {
                bail!("invalid {}", key);
            }
        }
    }

    if let Some(to) = obj.get("timeout_seconds") {
        if !to.is_i64() && !to.is_u64() {
            bail!("model timeout must be 1..180 seconds");
        }
        let n = to.as_i64().unwrap_or(0);
        if !(1..=180).contains(&n) {
            bail!("model timeout must be 1..180 seconds");
        }
    }

    Ok(())
}

unsafe extern "C" {
    fn getuid() -> u32;
}

pub fn read_key(path: &Path) -> Result<String> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)?;

    let meta = file.metadata()?;
    let mode = meta.permissions().mode();
    let is_reg = meta.file_type().is_file();
    let uid = meta.uid();
    let current_uid = unsafe { getuid() };

    if !is_reg || uid != current_uid || (mode & 0o777) != 0o600 {
        bail!("api_key_file must be an owned regular file with mode 600");
    }

    let mut buf = [0u8; 4097];
    let n = file.read(&mut buf)?;
    let content = std::str::from_utf8(&buf[..n])
        .context("invalid API key file")?
        .trim();

    if content.is_empty() || content.len() > 4096 || content.chars().any(|c| c.is_whitespace()) {
        bail!("invalid API key file");
    }

    Ok(content.to_string())
}

pub fn resolve_key(config: &ModelConfig) -> Result<String> {
    if let Some(ref path_str) = config.api_key_file {
        return read_key(Path::new(path_str));
    }

    let base_url_str = config.base_url.as_deref().unwrap_or("");
    let host = extract_host_from_base_url(base_url_str);

    let default_env = if host == "api.deepseek.com" {
        "DEEPSEEK_API_KEY"
    } else {
        "OPENAI_API_KEY"
    };

    let env_name = config
        .api_key_env
        .as_deref()
        .unwrap_or(default_env);

    let key = std::env::var(env_name).unwrap_or_default();
    let trimmed = key.trim();

    if trimmed.is_empty() {
        bail!("model API key environment variable is missing: {}", env_name);
    }
    if trimmed.len() > 4096 || trimmed.chars().any(|c| c.is_whitespace()) {
        bail!("invalid model API key environment variable: {}", env_name);
    }

    Ok(trimmed.to_string())
}

pub async fn request_completion(config: &ModelConfig, payload: &Value) -> Result<Value> {
    let key = resolve_key(config)?;
    let base_url = config.base_url.as_deref().unwrap_or("").trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    let timeout_secs = config.timeout_seconds.unwrap_or(60);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;

    let mut response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", key))
        .header("Content-Type", "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!("model connection failed; not automatically retried: {}", e)
        })?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "model HTTP error {}; not automatically retried",
            status.as_u16()
        );
    }

    let max_bytes = 2 * 1024 * 1024;
    let mut total_bytes = 0usize;
    let mut body = Vec::new();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("model connection failed; not automatically retried: {}", e))?
    {
        total_bytes += chunk.len();
        if total_bytes > max_bytes {
            bail!("model response too large");
        }
        body.extend_from_slice(&chunk);
    }

    let result: Value = serde_json::from_slice(&body)?;
    Ok(result)
}

pub fn parse_json_object(content: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(content) {
        if v.is_object() {
            return Some(v);
        }
    }
    let start = content.find('{')?;
    let sub = &content[start..];
    let mut stream = serde_json::Deserializer::from_str(sub).into_iter::<Value>();
    if let Some(Ok(v)) = stream.next() {
        if v.is_object() {
            return Some(v);
        }
    }
    None
}

const SYSTEM_REPLY: &str =
    "只输出JSON对象，严格使用 {\"reply\":\"回复正文\"}，只允许reply一个字段，值为非空字符串。不要输出Markdown或其他字段。";
const SYSTEM_NOTES: &str =
    "只输出JSON对象，严格使用 {\"notes\":[\"第1张图片的要点\", \"...\"]}，只允许notes一个字段，数组长度等于图片数。不要输出Markdown或其他字段。";

fn encode_base64(data: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(CHARSET[((n >> 18) & 63) as usize] as char);
        out.push(CHARSET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARSET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARSET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub async fn generate_api<F>(
    prompt: &str,
    images: &[PathBuf],
    config: &ModelConfig,
    metadata: Option<&mut HashMap<String, Value>>,
    field: &str,
    check: F,
) -> Result<Value>
where
    F: Fn(&Value) -> Result<Value> + Send + Sync,
{
    generate_api_with_requester(prompt, images, config, metadata, field, check, |cfg, payload| {
        Box::pin(async move { request_completion(&cfg, &payload).await })
    })
    .await
}

pub async fn generate_api_with_requester<F, R>(
    prompt: &str,
    images: &[PathBuf],
    config: &ModelConfig,
    metadata: Option<&mut HashMap<String, Value>>,
    field: &str,
    check: F,
    requester: R,
) -> Result<Value>
where
    F: Fn(&Value) -> Result<Value> + Send + Sync,
    R: Fn(ModelConfig, Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>,
{
    let config_val = serde_json::to_value(config)?;
    validate_config(&config_val)?;

    if !images.is_empty() && !config.supports_images.unwrap_or(false) {
        bail!("configured model has no enabled image support");
    }

    let mut user_content = Vec::new();
    user_content.push(json!({"type": "text", "text": prompt}));

    let mut total_image_bytes = 0usize;
    for img_path in images {
        if !img_path.is_file() {
            bail!("invalid image or image exceeds 10MiB");
        }
        let mut file = File::open(img_path)?;
        let meta = file.metadata()?;
        if meta.len() > 10 * 1024 * 1024 {
            bail!("invalid image or image exceeds 10MiB");
        }
        let mut data = Vec::with_capacity(meta.len() as usize);
        file.read_to_end(&mut data)?;

        let mime = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if data.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else {
            bail!("unsupported image bytes");
        };

        total_image_bytes += data.len();
        if total_image_bytes > 30 * 1024 * 1024 {
            bail!("combined images exceed 30MiB");
        }

        let b64 = encode_base64(&data);
        user_content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", mime, b64)
            }
        }));
    }

    let system_content = if field == "reply" {
        SYSTEM_REPLY.to_string()
    } else if field == "notes" {
        SYSTEM_NOTES.to_string()
    } else {
        format!(
            "只输出JSON对象，只允许{}一个字段。不要输出Markdown或其他字段。",
            field
        )
    };

    let user_msg = if images.is_empty() {
        json!({"role": "user", "content": prompt})
    } else {
        json!({"role": "user", "content": user_content})
    };

    let mut payload = json!({
        "model": config.model.as_deref().unwrap_or(""),
        "messages": [
            {"role": "system", "content": system_content},
            user_msg
        ],
        "stream": false,
        "max_tokens": 1024
    });

    if config.json_output.unwrap_or(true) {
        payload["response_format"] = json!({"type": "json_object"});
    }

    let began = Instant::now();
    let mut last_content = String::new();
    let mut parsed_obj = None;
    let mut last_result = Value::Null;

    for _attempt in 0..2 {
        let result = requester(config.clone(), payload.clone()).await?;
        last_result = result.clone();
        let choices = result.get("choices").and_then(|c| c.as_array());

        let (choice, finish_reason) = match choices {
            Some(arr) if arr.len() == 1 => {
                let c = &arr[0];
                let fr = c.get("finish_reason").and_then(|v| v.as_str());
                (c, fr)
            }
            _ => {
                let fr = choices
                    .and_then(|a| a.first())
                    .and_then(|c| c.get("finish_reason"))
                    .and_then(|v| v.as_str());
                let fr_display = fr.map_or("None".to_string(), |s| s.to_string());
                return Err(ModelOutputError(format!(
                    "model response missing or incomplete (finish_reason={})",
                    fr_display
                ))
                .into());
            }
        };

        if finish_reason != Some("stop") {
            let fr_display = finish_reason.map_or("None".to_string(), |s| s.to_string());
            return Err(ModelOutputError(format!(
                "model response missing or incomplete (finish_reason={})",
                fr_display
            ))
            .into());
        }

        let message = choice.get("message");
        let has_tool_calls = message
            .and_then(|m| m.get("tool_calls"))
            .map_or(false, |tc| !tc.is_null());
        let content_str = message.and_then(|m| m.get("content")).and_then(|c| c.as_str());

        if has_tool_calls || content_str.is_none() {
            return Err(ModelOutputError("expected model text without tool calls".to_string()).into());
        }

        let content = content_str.unwrap();
        last_content = content.to_string();

        if let Some(parsed) = parse_json_object(content) {
            parsed_obj = Some(parsed);
            break;
        }
    }

    let parsed = match parsed_obj {
        Some(p) => p,
        None => {
            return Err(ModelOutputError(format!(
                "model returned malformed JSON twice (length={}, has_brace={})",
                last_content.chars().count(),
                last_content.contains('{')
            ))
            .into());
        }
    };

    let map = parsed.as_object().context("parsed is not object")?;
    if map.len() != 1 || !map.contains_key(field) {
        return Err(ModelOutputError(format!("expected JSON with {} only", field)).into());
    }

    let checked_reply = match check(&map[field]) {
        Ok(v) => v,
        Err(err) => {
            return Err(ModelOutputError(err.to_string()).into());
        }
    };

    if let Some(meta) = metadata {
        meta.insert("backend".to_string(), json!("openai-compatible"));
        let host = extract_host_from_base_url(config.base_url.as_deref().unwrap_or(""));
        meta.insert("provider".to_string(), json!(host));
        meta.insert("requested_model".to_string(), json!(config.model));
        meta.insert("model".to_string(), last_result.get("model").cloned().unwrap_or(Value::Null));
        meta.insert("seconds".to_string(), json!(began.elapsed().as_secs_f64()));
        meta.insert("usage".to_string(), last_result.get("usage").cloned().unwrap_or(Value::Null));
    }

    Ok(checked_reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            let p = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&p);
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn with_vars<I, K, V, F, R>(vars: I, f: F) -> R
    where
        I: IntoIterator<Item = (K, Option<V>)>,
        K: AsRef<str>,
        V: AsRef<str>,
        F: FnOnce() -> R,
    {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();

        let var_list: Vec<(String, Option<String>)> = vars
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_string(), v.map(|s| s.as_ref().to_string())))
            .collect();

        let old: Vec<(String, Option<String>)> = var_list
            .iter()
            .map(|(k, _)| (k.clone(), std::env::var(k).ok()))
            .collect();

        for (k, v) in &var_list {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }

        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        for (k, old_val) in old {
            match old_val {
                Some(v) => unsafe { std::env::set_var(&k, v) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }

        match res {
            Ok(r) => r,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    struct TestFixture {
        temp_dir: TempDir,
        key_path: PathBuf,
        config: ModelConfig,
        result: Value,
    }

    impl TestFixture {
        fn new() -> Self {
            let temp_dir = TempDir::new("model-api-test");
            let key_path = temp_dir.path().join("key");
            fs::write(&key_path, "fixture-key").unwrap();
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();

            let config = ModelConfig {
                backend: "openai-compatible".to_string(),
                base_url: Some("https://api.deepseek.com".to_string()),
                model: Some("fixture-model".to_string()),
                api_key_file: Some(key_path.to_str().unwrap().to_string()),
                api_key_env: None,
                supports_images: None,
                timeout_seconds: None,
                json_output: None,
            };

            let result = json!({
                "model": "fixture-actual",
                "choices": [{
                    "finish_reason": "stop",
                    "message": {
                        "content": json!({"reply": "你好，我是AI客服助手。"}).to_string()
                    }
                }]
            });

            Self {
                temp_dir,
                key_path,
                config,
                result,
            }
        }
    }

    #[tokio::test]
    async fn test_bot_model_api_dispatch_and_model_metadata() {
        let f = TestFixture::new();
        let recorded_payload = Arc::new(tokio::sync::Mutex::new(Value::Null));
        let rec = recorded_payload.clone();
        let result_clone = f.result.clone();

        let mut meta = HashMap::new();
        let check = |val: &Value| {
            let s = val.as_str().unwrap();
            validate_reply(s).map(|s| Value::String(s))
        };

        let reply = generate_api_with_requester(
            "你好",
            &[],
            &f.config,
            Some(&mut meta),
            "reply",
            check,
            move |_cfg, payload| {
                let rec = rec.clone();
                let res = result_clone.clone();
                Box::pin(async move {
                    *rec.lock().await = payload;
                    Ok(res)
                })
            },
        )
        .await
        .unwrap();

        assert!(reply.as_str().unwrap().contains("AI"));
        let p = recorded_payload.lock().await.clone();
        assert_eq!(p["model"], "fixture-model");
        assert_eq!(p["messages"][0]["role"], "system");
        assert!(p["messages"][0]["content"].as_str().unwrap().contains("{\"reply\":"));
        assert_eq!(p["stream"], false);
        assert!(!p.to_string().contains("fixture-key"));
        assert_eq!(meta.get("model").unwrap(), &json!("fixture-actual"));
        assert!(!meta.contains_key("api_key_file"));
    }

    #[tokio::test]
    async fn test_bot_model_api_earlier_images_are_not_counted_as_this_turn() {
        let f = TestFixture::new();
        let recorded_payload = Arc::new(tokio::sync::Mutex::new(Value::Null));
        let rec = recorded_payload.clone();
        let result_clone = f.result.clone();

        let turn = crate::brain::Turn {
            turn_id: "t1".to_string(),
            account_id: "acc".to_string(),
            contact: crate::brain::ContactInfo {
                name: "午夜咖啡".to_string(),
                kind: "group".to_string(),
            },
            text: "午夜咖啡：[图片]".to_string(),
            mode: "service".to_string(),
            chat_kind: "group".to_string(),
            context: vec![crate::brain::ContextItem {
                role: "customer".to_string(),
                text: "[图片]".to_string(),
                media: Some(crate::brain::ContextMedia {
                    kind: Some("image".to_string()),
                    understanding: Some("一张截图".to_string()),
                    ..Default::default()
                }),
            }],
            images: vec![],
            files: vec![],
            notes: vec![],
            failed: vec![],
            image_count: Some(1),
            earlier_images: 0,
            max_attachment_mb: 50,
        };

        let prompt = crate::brain::builtin::build_prompt(&turn).unwrap();
        let _ = generate_api_with_requester(
            &prompt,
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, payload| {
                let rec = rec.clone();
                let res = result_clone.clone();
                Box::pin(async move {
                    *rec.lock().await = payload;
                    Ok(res)
                })
            },
        )
        .await
        .unwrap();

        let p = recorded_payload.lock().await.clone();
        let prompt_str = p["messages"].to_string();
        assert!(prompt_str.contains("recent_context里的图片和文件是之前发的"));
        assert!(prompt_str.contains("本轮收到的图片数量以image_count为准"));
        assert!(prompt_str.contains("earlier_images"));
        assert!(prompt_str.contains("image_count"));
    }

    #[tokio::test]
    async fn test_bot_model_api_generate_api_custom_field() {
        let f = TestFixture::new();
        let notes_res = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": json!({"notes": ["一张收据"]}).to_string()
                }
            }]
        });

        let check = |val: &Value| {
            if !val.is_array() {
                bail!("notes must be a list");
            }
            Ok(val.clone())
        };

        let recorded_payload = Arc::new(tokio::sync::Mutex::new(Value::Null));
        let rec = recorded_payload.clone();
        let res_clone = notes_res.clone();

        let val = generate_api_with_requester(
            "看图",
            &[],
            &f.config,
            None,
            "notes",
            check,
            move |_cfg, payload| {
                let rec = rec.clone();
                let res = res_clone.clone();
                Box::pin(async move {
                    *rec.lock().await = payload;
                    Ok(res)
                })
            },
        )
        .await
        .unwrap();

        assert_eq!(val, json!(["一张收据"]));
        let p = recorded_payload.lock().await.clone();
        assert!(p["messages"][0]["content"].as_str().unwrap().contains("{\"notes\":"));

        let res_default = f.result.clone();
        let err = generate_api_with_requester(
            "看图",
            &[],
            &f.config,
            None,
            "notes",
            check,
            move |_cfg, _payload| {
                let res = res_default.clone();
                Box::pin(async move { Ok(res) })
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("expected JSON with notes only"));
    }

    #[tokio::test]
    async fn test_bot_model_api_malformed_output_is_a_model_output_error_with_shape_only() {
        let f = TestFixture::new();
        let bad = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "好的，这是回复"
                }
            }]
        });

        let err = generate_api_with_requester(
            "hi",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                let bad = bad.clone();
                Box::pin(async move { Ok(bad) })
            },
        )
        .await
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("length=7"), "expected length=7 in {}", msg);
        assert!(!msg.contains("好的"), "chat body should never leak into error");
    }

    #[tokio::test]
    async fn test_bot_model_api_actual_image_bytes_and_capability_gate() {
        let f = TestFixture::new();
        let img_path = f.temp_dir.path().join("image.png");
        fs::write(&img_path, b"\x89PNG\r\n\x1a\nfixture").unwrap();

        let err = generate_api_with_requester(
            "test",
            &[img_path.clone()],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| Box::pin(async move { unreachable!() }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("image support"));

        let mut cfg2 = f.config.clone();
        cfg2.supports_images = Some(true);
        let recorded = Arc::new(tokio::sync::Mutex::new(Value::Null));
        let rec = recorded.clone();
        let res_clone = f.result.clone();

        generate_api_with_requester(
            "test",
            &[img_path],
            &cfg2,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, payload| {
                let rec = rec.clone();
                let res = res_clone.clone();
                Box::pin(async move {
                    *rec.lock().await = payload;
                    Ok(res)
                })
            },
        )
        .await
        .unwrap();

        let p = recorded.lock().await.clone();
        let content = &p["messages"][1]["content"];
        assert!(content[1]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn test_bot_model_api_incomplete_output_never_accepted() {
        let f = TestFixture::new();
        for reason in [Some("length"), Some("tool_calls"), None] {
            let res = json!({
                "model": "fixture-actual",
                "choices": [{
                    "finish_reason": reason,
                    "message": {
                        "content": "{}"
                    }
                }]
            });
            let err = generate_api_with_requester(
                "test",
                &[],
                &f.config,
                None,
                "reply",
                |v| Ok(v.clone()),
                move |_cfg, _payload| {
                    let res = res.clone();
                    Box::pin(async move { Ok(res) })
                },
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("model response missing or incomplete"));
        }
    }

    #[tokio::test]
    async fn test_bot_model_api_invalid_reply_rejected() {
        let f = TestFixture::new();
        let res = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": json!({"reply": "ok", "recipient": "other"}).to_string()
                }
            }]
        });
        let err = generate_api_with_requester(
            "test",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                let res = res.clone();
                Box::pin(async move { Ok(res) })
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("expected JSON with reply only"));
    }

    #[test]
    fn test_bot_model_api_key_file_ownership_mode_and_symlink() {
        let f = TestFixture::new();
        assert_eq!(read_key(&f.key_path).unwrap(), "fixture-key");

        fs::set_permissions(&f.key_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_key(&f.key_path).is_err());

        fs::set_permissions(&f.key_path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = f.temp_dir.path().join("link");
        std::os::unix::fs::symlink(&f.key_path, &link).unwrap();
        assert!(read_key(&link).is_err());
    }

    #[test]
    fn test_bot_model_api_config_transport_and_fields() {
        let f = TestFixture::new();
        let base = serde_json::to_value(&f.config).unwrap();
        validate_config(&base).unwrap();

        let mut local = base.clone();
        local["base_url"] = json!("http://localhost:8000/v1");
        validate_config(&local).unwrap();

        let bad_deltas = [
            json!({"base_url": "http://api.deepseek.com"}),
            json!({"base_url": "https://user:pass@example.com"}),
            json!({"api_key": "secret"}),
            json!({"timeout_seconds": 0}),
            json!({"supports_images": "yes"}),
        ];

        for delta in bad_deltas {
            let mut bad = base.clone();
            for (k, v) in delta.as_object().unwrap() {
                bad[k] = v.clone();
            }
            assert!(validate_config(&bad).is_err(), "expected error for delta {:?}", delta);
        }
    }

    #[test]
    fn test_bot_model_api_deepseek_defaults_to_environment_key() {
        let f = TestFixture::new();
        let mut cfg = f.config.clone();
        cfg.api_key_file = None;

        with_vars([("DEEPSEEK_API_KEY", Some("fixture-env-key"))], || {
            assert_eq!(resolve_key(&cfg).unwrap(), "fixture-env-key");
        });
    }

    #[test]
    fn test_bot_model_api_custom_environment_and_other_provider_default() {
        let f = TestFixture::new();
        let mut cfg = f.config.clone();
        cfg.api_key_file = None;
        cfg.api_key_env = Some("OTHER_MODEL_KEY".to_string());

        let mut cfg2 = f.config.clone();
        cfg2.api_key_file = None;
        cfg2.base_url = Some("https://example.com/v1".to_string());

        with_vars(
            [
                ("OTHER_MODEL_KEY", Some("fixture-custom")),
                ("OPENAI_API_KEY", Some("fixture-openai")),
            ],
            || {
                assert_eq!(resolve_key(&cfg).unwrap(), "fixture-custom");
                assert_eq!(resolve_key(&cfg2).unwrap(), "fixture-openai");
            },
        );
    }

    #[test]
    fn test_bot_model_api_missing_or_invalid_environment_key_fails_without_exposing_value() {
        let f = TestFixture::new();
        let mut cfg = f.config.clone();
        cfg.api_key_file = None;

        with_vars([("DEEPSEEK_API_KEY", None::<&str>)], || {
            let err = resolve_key(&cfg).unwrap_err();
            assert!(err.to_string().contains("missing"));
        });

        with_vars([("DEEPSEEK_API_KEY", Some("secret invalid key"))], || {
            let err = resolve_key(&cfg).unwrap_err();
            assert!(!err.to_string().contains("secret invalid key"));
        });

        let base = serde_json::to_value(&cfg).unwrap();
        for name in [json!(""), json!("BAD-NAME"), json!("key=value"), Value::Null] {
            let mut bad = base.clone();
            bad["api_key_env"] = name;
            assert!(validate_config(&bad).is_err());
        }
    }

    #[test]
    fn test_bot_model_api_explicit_file_has_priority_over_environment() {
        let f = TestFixture::new();
        with_vars([("DEEPSEEK_API_KEY", Some("fixture-env-key"))], || {
            assert_eq!(resolve_key(&f.config).unwrap(), "fixture-key");
        });
    }

    #[test]
    fn test_bot_model_api_redirect_never_forwards_key() {
        let policy = reqwest::redirect::Policy::none();
        assert!(format!("{:?}", policy).to_lowercase().contains("none"));
    }

    #[tokio::test]
    async fn test_bot_model_api_http_transport_sends_auth_and_json() {
        let f = TestFixture::new();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let res_clone = f.result.clone();
        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let req_str = String::from_utf8_lossy(&buf[..n]);

            assert!(req_str.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(req_str.to_lowercase().contains("authorization: bearer fixture-key"));

            let body_str = serde_json::to_string(&res_clone).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_str.len(),
                body_str
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let mut cfg = f.config.clone();
        cfg.base_url = Some(format!("http://127.0.0.1:{}/v1", port));

        let res = request_completion(&cfg, &json!({"model": "fixture", "messages": []}))
            .await
            .unwrap();
        assert_eq!(res["model"], "fixture-actual");
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_bot_model_api_json_object_wrapped_in_text_is_extracted() {
        let f = TestFixture::new();
        let wrapped_res = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "好的：{\"reply\":\"收到两个表情～\"} 以上"
                }
            }]
        });

        let val = generate_api_with_requester(
            "test",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                let res = wrapped_res.clone();
                Box::pin(async move { Ok(res) })
            },
        )
        .await
        .unwrap();

        assert_eq!(val, json!("收到两个表情～"));
    }

    #[tokio::test]
    async fn test_bot_model_api_malformed_model_json_is_requested_again_once() {
        let f = TestFixture::new();
        let bad = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "{\"reply\":\"图上是\"可爱\"的表情\"}"
                }
            }]
        });
        let ok = f.result.clone();
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let val = generate_api_with_requester(
            "test",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                let count = c.fetch_add(1, Ordering::SeqCst);
                let bad = bad.clone();
                let ok = ok.clone();
                Box::pin(async move {
                    if count == 0 {
                        Ok(bad)
                    } else {
                        Ok(ok)
                    }
                })
            },
        )
        .await
        .unwrap();

        assert!(val.as_str().unwrap().contains("AI"));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_bot_model_api_malformed_model_json_twice_fails() {
        let f = TestFixture::new();
        let bad = json!({
            "model": "fixture-actual",
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "{\"reply\":\"图上是\"可爱\"的表情\"}"
                }
            }]
        });
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let err = generate_api_with_requester(
            "test",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                c.fetch_add(1, Ordering::SeqCst);
                let bad = bad.clone();
                Box::pin(async move { Ok(bad) })
            },
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("malformed JSON twice"));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_bot_model_api_no_automatic_retry() {
        let f = TestFixture::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let err = generate_api_with_requester(
            "test",
            &[],
            &f.config,
            None,
            "reply",
            |v| Ok(v.clone()),
            move |_cfg, _payload| {
                c.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    bail!("HTTP 429");
                })
            },
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("HTTP 429"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_bot_model_api_response_too_large_stops_reading() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let chunk_size = 64 * 1024; // 64KB per chunk
        let total_size = 3 * 1024 * 1024; // 3MB total
        let total_chunks = total_size / chunk_size;

        let bytes_sent = Arc::new(AtomicUsize::new(0));
        let bs = bytes_sent.clone();

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req_buf = vec![0u8; 4096];
            let _ = socket.read(&mut req_buf).await.unwrap();

            let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
            if socket.write_all(header.as_bytes()).await.is_err() {
                return;
            }

            let dummy_chunk = vec![b'a'; chunk_size];
            for _ in 0..total_chunks {
                let chunk_header = format!("{:X}\r\n", chunk_size);
                if socket.write_all(chunk_header.as_bytes()).await.is_err() {
                    break;
                }
                if socket.write_all(&dummy_chunk).await.is_err() {
                    break;
                }
                if socket.write_all(b"\r\n").await.is_err() {
                    break;
                }
                bs.fetch_add(chunk_size, Ordering::SeqCst);
                // Allow reader task to read chunk and drop connection when limit exceeded
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }
        });

        let f = TestFixture::new();
        let mut cfg = f.config.clone();
        cfg.base_url = Some(format!("http://127.0.0.1:{}", port));

        let err = request_completion(&cfg, &json!({"model": "test", "messages": []}))
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("model response too large"),
            "expected 'model response too large', got: {}",
            err
        );

        let sent = bytes_sent.load(Ordering::SeqCst);
        let max_allowed = 2 * 1024 * 1024 + chunk_size * 2;
        assert!(
            sent <= max_allowed,
            "expected bytes sent to not exceed limit + chunks, but sent {} bytes (total 3MB is {})",
            sent,
            total_size
        );
        assert!(
            sent < total_size,
            "server should not have streamed all 3MB; sent {}",
            sent
        );

        server_task.abort();
    }
}
