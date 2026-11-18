//! Read-only model availability probe. Never exposes transport diagnostics or response bodies.
use crate::brain::model_api::{read_key, resolve_key};
use crate::core::config::BotConfig;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

fn result(category: &str) -> Value {
    json!({"ok": category == "ok", "category": category})
}

fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn codex_available() -> bool {
    let bin = std::env::var("WECHAT_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    if bin.contains('/') {
        return executable(Path::new(&bin));
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| executable(&path.join(&bin))))
}

fn status_category(status: u16) -> &'static str {
    match status {
        200..=299 => "ok",
        401 | 403 => "auth_failed",
        404 => "not_found",
        429 => "rate_limited",
        _ => "http_error",
    }
}

fn response_category(body: &Value, model: &str) -> &'static str {
    let Some(data) = body.get("data").and_then(Value::as_array) else {
        return "bad_response";
    };
    if !data
        .iter()
        .all(|entry| entry.get("id").and_then(Value::as_str).is_some())
    {
        return "bad_response";
    }
    if data.iter().any(|entry| entry["id"].as_str() == Some(model)) {
        "ok"
    } else {
        "not_found"
    }
}

pub async fn probe(path: &Path) -> Value {
    // Inspect key-file validity separately so it has a stable category even when config loading rejects it.
    let raw: Value = match std::fs::read(path)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
    {
        Some(value) => value,
        None => return result("invalid_config"),
    };
    if raw["model_config"]["backend"] == "openai-compatible" {
        if let Some(file) = raw["model_config"]["api_key_file"].as_str() {
            if !Path::new(file).is_absolute() || read_key(Path::new(file)).is_err() {
                return result("key_file_invalid");
            }
        }
    }
    let config = match BotConfig::load(path) {
        Ok(config) => config,
        Err(_) => return result("invalid_config"),
    };
    let model = &config.model_config;
    if model.backend == "codex-exec" {
        return result(if codex_available() { "ok" } else { "not_found" });
    }
    let key = match resolve_key(model) {
        Ok(key) => key,
        Err(_) => {
            return result(if model.api_key_file.is_some() {
                "key_file_invalid"
            } else {
                "key_missing"
            });
        }
    };
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(model.timeout_seconds.unwrap_or(60)))
        .build()
    {
        Ok(client) => client,
        Err(_) => return result("connect_failed"),
    };
    let start = Instant::now();
    let url = format!(
        "{}/models",
        model
            .base_url
            .as_deref()
            .unwrap_or_default()
            .trim_end_matches('/')
    );
    let response = client.get(url).bearer_auth(key).send().await;
    let mut output = match response {
        Err(error) => result(if error.is_timeout() {
            "timeout"
        } else {
            "connect_failed"
        }),
        Ok(mut response) => {
            let status = response.status().as_u16();
            let category = status_category(status);
            let mut output = if category != "ok" {
                result(category)
            } else {
                // Bound memory use and never include supplier-provided strings in the output.
                let mut bytes = Vec::new();
                let category = loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= 1024 * 1024 => {
                            bytes.extend_from_slice(&chunk)
                        }
                        Ok(Some(_)) => break "bad_response",
                        Ok(None) => {
                            break serde_json::from_slice::<Value>(&bytes)
                                .map(|body| {
                                    response_category(
                                        &body,
                                        model.model.as_deref().unwrap_or_default(),
                                    )
                                })
                                .unwrap_or("bad_response");
                        }
                        Err(error) => {
                            break if error.is_timeout() {
                                "timeout"
                            } else {
                                "bad_response"
                            };
                        }
                    }
                };
                result(category)
            };
            output["http_status"] = json!(status);
            output
        }
    };
    output["latency_ms"] = json!(start.elapsed().as_millis());
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(dir: &Path, backend: Value) -> std::path::PathBuf {
        let path = dir.join("config.json");
        std::fs::write(&path, json!({"schema_version":1,"host":"local","account_id":"probe-test","allowlist":[{"recipient":"probe","kind":"private"}],"state_dir":dir.join("state"),"model_config":backend}).to_string()).unwrap();
        path
    }

    #[tokio::test]
    async fn probes_http_without_leaking_key_or_response() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("model-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let key = dir.join("key");
        std::fs::write(&key, "probe-fixture-secret").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        for (status, body, expected) in [
            (200, r#"{"data":[{"id":"wanted"}]}"#, "ok"),
            (200, r#"{"data":[]}"#, "not_found"),
            (200, "supplier-private-error", "bad_response"),
            (401, "supplier-private-error", "auth_failed"),
            (404, "supplier-private-error", "not_found"),
            (429, "supplier-private-error", "rate_limited"),
            (500, "supplier-private-error", "http_error"),
            (302, "supplier-private-error", "http_error"),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                listener.set_nonblocking(true).unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                let (mut stream, _) = loop {
                    if let Ok(pair) = listener.accept() {
                        break pair;
                    }
                    assert!(Instant::now() < deadline, "probe did not connect");
                    std::thread::sleep(Duration::from_millis(10));
                };
                // 这个线程只是假服务端：客户端提前断开（例如 302 后立刻关闭连接）不是被测行为，
                // 不能因为读写错误让整个测试失败。真正的断言仍在下面针对 probe() 的输出。
                let mut request = [0u8; 8192];
                let n = match stream.read(&mut request) {
                    Ok(n) => n,
                    Err(_) => return,
                };
                assert!(
                    String::from_utf8_lossy(&request[..n]).starts_with("GET /v1/models HTTP/1.1")
                );
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
            let path = fixture(
                &dir,
                json!({"backend":"openai-compatible","base_url":format!("http://{addr}/v1"),"model":"wanted","api_key_file":key}),
            );
            let output = probe(&path).await;
            server.join().unwrap();
            assert_eq!(output["category"], expected);
            assert_eq!(output["http_status"], status);
            assert!(!output.to_string().contains("probe-fixture-secret"));
            assert!(!output.to_string().contains("supplier-private-error"));
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let path = fixture(
            &dir,
            json!({"backend":"openai-compatible","base_url":format!("http://{addr}"),"model":"wanted","api_key_file":key}),
        );
        assert_eq!(probe(&path).await["category"], "connect_failed");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(1400));
        });
        let path = fixture(
            &dir,
            json!({"backend":"openai-compatible","base_url":format!("http://{addr}"),"model":"wanted","api_key_file":key,"timeout_seconds":1}),
        );
        assert_eq!(probe(&path).await["category"], "timeout");
        server.join().unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(probe(&path).await["category"], "key_file_invalid");
        let path = fixture(
            &dir,
            json!({"backend":"openai-compatible","base_url":"https://example.com","model":"wanted","api_key_env":format!("PROBE_MISSING_{}", std::process::id())}),
        );
        assert_eq!(probe(&path).await["category"], "key_missing");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn codex_override_checks_executable_without_running_it() {
        let _guard = crate::brain::model_api::CODEX_ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("WECHAT_CODEX_BIN");
        let dir = std::env::temp_dir().join(format!("codex-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let bin = dir.join("codex");
        std::fs::write(&bin, "#!/bin/sh\nexit 99\n").unwrap();
        unsafe {
            std::env::set_var("WECHAT_CODEX_BIN", &bin);
        }
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!codex_available());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(codex_available());
        unsafe {
            if let Some(old) = old {
                std::env::set_var("WECHAT_CODEX_BIN", old);
            } else {
                std::env::remove_var("WECHAT_CODEX_BIN");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stable_categories_and_no_supplier_body() {
        for (status, category) in [
            (200, "ok"),
            (401, "auth_failed"),
            (403, "auth_failed"),
            (404, "not_found"),
            (429, "rate_limited"),
            (500, "http_error"),
        ] {
            assert_eq!(status_category(status), category);
        }
        assert_eq!(
            response_category(&json!({"data":[{"id":"wanted"}]}), "wanted"),
            "ok"
        );
        assert_eq!(
            response_category(&json!({"data":[]}), "wanted"),
            "not_found"
        );
        assert_eq!(
            response_category(&json!({"error":"supplier-secret"}), "wanted"),
            "bad_response"
        );
        for category in [
            "ok",
            "auth_failed",
            "not_found",
            "rate_limited",
            "timeout",
            "connect_failed",
            "key_missing",
            "key_file_invalid",
            "invalid_config",
            "http_error",
            "bad_response",
        ] {
            let output = result(category);
            assert_eq!(output["category"], category);
            assert_eq!(output["ok"], category == "ok");
            assert!(!output.to_string().contains("supplier-secret"));
        }
    }
    #[tokio::test]
    async fn invalid_config_is_sanitized() {
        assert_eq!(
            probe(Path::new("/nonexistent/probe-config")).await,
            result("invalid_config")
        );
    }
}
