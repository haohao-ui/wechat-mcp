//! 微信公众号文章抓取：公开网页 → 元数据 + Markdown，可选下载正文图片到本地。
//! 只请求 mp.weixin.qq.com 公开页面，不操作微信客户端，也不需要桌面锁。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::{Value, json};
use tokio::process::Command;

pub const TOOL: &str = "fetch_article";
const CHROME_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
const WECHAT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) MicroMessenger/3.8.8(0x13080810) MacWechat/3.8.8(0x13080810) NetType/WIFI";
const MAX_IMAGES: usize = 300;

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "抓取微信公众号文章（mp.weixin.qq.com）的标题、公众号、作者、发布时间和 Markdown 正文；提供 output_dir 时下载正文图片并写入 article.md。只读公开网页，不操作微信客户端，无需 platform/account_id",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {"type": "string", "minLength": 1, "description": "文章链接，例如 https://mp.weixin.qq.com/s/xxxx"},
                "output_dir": {"type": "string", "minLength": 1, "description": "可选，绝对路径。写入 article.md 与 images/，Markdown 改为引用本地图片；同名文件会被覆盖"},
                "include_markdown": {"type": "boolean", "description": "结果是否包含 Markdown 全文，默认 true；只需保存到本地时设为 false 以节省上下文"}
            },
            "required": ["url"],
            "additionalProperties": false
        }
    })
}

pub async fn execute(args: Value) -> Result<Value> {
    let object = args.as_object().context("arguments 必须是对象")?;
    for field in object.keys() {
        if !["url", "output_dir", "include_markdown"].contains(&field.as_str()) {
            bail!("不支持的参数: {field}");
        }
    }
    let url = normalize_url(
        args.get("url")
            .and_then(Value::as_str)
            .context("必须提供 url")?,
    )?;
    let output_dir = match args.get("output_dir") {
        None | Some(Value::Null) => None,
        Some(Value::String(dir)) if Path::new(dir).is_absolute() => Some(PathBuf::from(dir)),
        Some(Value::String(_)) => bail!("output_dir 必须是绝对路径"),
        Some(_) => bail!("output_dir 必须是字符串"),
    };
    let include_markdown = match args.get("include_markdown") {
        None => true,
        Some(value) => value.as_bool().context("include_markdown 必须是布尔值")?,
    };

    let html = fetch_html(&url, 30).await?;
    let article = parse(&html)?;
    let mut markdown = article.document(&url);
    let mut images: Vec<Value> = article.images.iter().map(|u| json!({"url": u})).collect();
    let mut image_errors = Vec::new();
    let mut saved = Value::Null;

    if let Some(dir) = output_dir {
        let image_dir = dir.join("images");
        std::fs::create_dir_all(&image_dir)
            .with_context(|| format!("创建目录失败: {}", image_dir.display()))?;
        let mut local: HashMap<&str, String> = HashMap::new();
        for (index, image_url) in article.images.iter().enumerate() {
            if index >= MAX_IMAGES {
                image_errors.push(json!({"url": image_url, "error": format!("超过 {MAX_IMAGES} 张上限，保留远程链接")}));
                continue;
            }
            if let Some(path) = local.get(image_url.as_str()) {
                images[index]["local_path"] = json!(path);
                continue;
            }
            match download_image(image_url, &image_dir, index + 1).await {
                Ok(name) => {
                    let relative = format!("images/{name}");
                    markdown =
                        markdown.replace(&format!("]({image_url})"), &format!("]({relative})"));
                    images[index]["local_path"] = json!(relative);
                    local.insert(image_url, relative);
                }
                Err(error) => {
                    image_errors.push(json!({"url": image_url, "error": format!("{error:#}")}))
                }
            }
        }
        let markdown_path = dir.join("article.md");
        std::fs::write(&markdown_path, &markdown)
            .with_context(|| format!("写入失败: {}", markdown_path.display()))?;
        saved = json!({
            "markdown_path": markdown_path,
            "images_dir": image_dir,
        });
    }

    let mut result = json!({
        "url": url,
        "title": article.title,
        "account_name": article.account_name,
        "author": article.author,
        "publish_time": article.publish_time,
        "digest": article.digest,
        "cover_image": article.cover,
        "text_length": article.body.chars().filter(|c| !c.is_whitespace()).count(),
        "image_count": images.len(),
        "images": images,
        "source": "public_web_page",
    });
    if !saved.is_null() {
        result["saved"] = saved;
    }
    if !image_errors.is_empty() {
        result["image_errors"] = json!(image_errors);
    }
    if include_markdown {
        result["markdown"] = json!(markdown);
    }
    Ok(result)
}

fn normalize_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .context("url 必须以 https:// 开头")?;
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if &rest[..host_end] != "mp.weixin.qq.com" {
        bail!("只支持 mp.weixin.qq.com 的公众号文章链接");
    }
    if rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("url 不能包含空白或控制字符");
    }
    let rest = rest.split('#').next().unwrap_or_default();
    Ok(format!("https://{rest}"))
}

async fn fetch_html(url: &str, timeout_seconds: u32) -> Result<String> {
    // 1. 尝试以现代 Chrome 标头与内存 CookieJar 请求（支持 302 重定向自动携带 wxtokenkey）
    let chrome_html = match curl_page(url, CHROME_UA, timeout_seconds).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => String::new(),
    };

    if !chrome_html.is_empty() && parse(&chrome_html).is_ok() {
        return Ok(chrome_html);
    }

    // 2. 若未解析成功或命中限制，尝试以微信客户端 UA 请求
    let wechat_html = match curl_page(url, WECHAT_UA, timeout_seconds).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => String::new(),
    };

    if !wechat_html.is_empty() && parse(&wechat_html).is_ok() {
        return Ok(wechat_html);
    }

    // 3. 尝试调用本机 Google Chrome 无头渲染兜底（若可用）
    if let Ok(browser_html) = fetch_via_chrome(url).await
        && parse(&browser_html).is_ok()
    {
        return Ok(browser_html);
    }

    // 若均未能成功解析，返回最有诊断价值的 HTML（供 parse 抛出明确原因）
    if !chrome_html.is_empty() {
        Ok(chrome_html)
    } else if !wechat_html.is_empty() {
        Ok(wechat_html)
    } else {
        let bytes = curl(url, None, timeout_seconds).await?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }
}

async fn fetch_via_chrome(url: &str) -> Result<String> {
    let chrome_paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/usr/local/bin/google-chrome",
    ];
    let Some(bin) = chrome_paths.iter().find(|p| Path::new(p).exists()) else {
        bail!("未找到 Chrome 可执行文件");
    };

    let user_data_dir = std::env::temp_dir().join(format!("wechat_chrome_{}", std::process::id()));
    let mut command = Command::new(bin);
    command
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            &format!("--user-data-dir={}", user_data_dir.display()),
            "--dump-dom",
            url,
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);

    let output = command.output().await?;
    let _ = std::fs::remove_dir_all(&user_data_dir);

    if !output.status.success() {
        bail!("Chrome 执行失败");
    }
    let html = String::from_utf8_lossy(&output.stdout).to_string();
    if html.is_empty() {
        bail!("Chrome 返回内容为空");
    }
    Ok(html)
}

async fn curl_page(url: &str, user_agent: &str, timeout_seconds: u32) -> Result<Vec<u8>> {
    let mut command = Command::new("curl");
    command
        .args([
            "-sS",
            "-L",
            "--fail",
            "--compressed",
            "--max-filesize",
            "50000000",
            "-b",
            "",
        ])
        .args(["--max-time", &timeout_seconds.to_string()])
        .args([
            "-A",
            user_agent,
            "-e",
            "https://mp.weixin.qq.com/",
            "-H",
            "Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
            "-H",
            "Accept-Language: zh-CN,zh;q=0.9,en;q=0.8",
            "-H",
            "Sec-Ch-Ua: \"Not/A)Brand\";v=\"8\", \"Chromium\";v=\"126\", \"Google Chrome\";v=\"126\"",
            "-H",
            "Sec-Ch-Ua-Mobile: ?0",
            "-H",
            "Sec-Ch-Ua-Platform: \"macOS\"",
            "-H",
            "Sec-Fetch-Dest: document",
            "-H",
            "Sec-Fetch-Mode: navigate",
            "-H",
            "Sec-Fetch-Site: none",
            "-H",
            "Sec-Fetch-User: ?1",
            "-H",
            "Upgrade-Insecure-Requests: 1",
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = command.arg(url).output().await.context("无法执行 curl")?;
    if !output.status.success() {
        bail!(
            "下载失败 {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

async fn curl(url: &str, output: Option<&Path>, timeout_seconds: u32) -> Result<Vec<u8>> {
    let mut command = Command::new("curl");
    command
        .args([
            "-sS",
            "-L",
            "--fail",
            "--compressed",
            "--max-filesize",
            "50000000",
            "-b",
            "",
        ])
        .args(["--max-time", &timeout_seconds.to_string()])
        .args(["-A", CHROME_UA, "-e", "https://mp.weixin.qq.com/"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(path) = output {
        command.arg("-o").arg(path);
    }
    let output = command.arg(url).output().await.context("无法执行 curl")?;
    if !output.status.success() {
        bail!(
            "下载失败 {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

async fn download_image(url: &str, dir: &Path, index: usize) -> Result<String> {
    let partial = dir.join(format!("{index:02}.part"));
    let result = async {
        curl(url, Some(&partial), 60).await?;
        let mut head = [0u8; 16];
        let read = std::io::Read::read(&mut std::fs::File::open(&partial)?, &mut head)?;
        let extension = image_extension(&head[..read]).context("下载内容不是图片")?;
        let name = format!("{index:02}.{extension}");
        std::fs::rename(&partial, dir.join(&name))?;
        Ok(name)
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn image_extension(head: &[u8]) -> Option<&'static str> {
    match head {
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("webp"),
        _ => None,
    }
}

#[derive(Debug)]
struct Article {
    title: String,
    account_name: Option<String>,
    author: Option<String>,
    publish_time: Option<String>,
    digest: Option<String>,
    cover: Option<String>,
    body: String,
    images: Vec<String>,
}

impl Article {
    fn document(&self, url: &str) -> String {
        let mut meta = Vec::new();
        if let Some(name) = &self.account_name {
            meta.push(format!("公众号：{name}"));
        }
        if let Some(author) = self
            .author
            .as_ref()
            .filter(|a| Some(*a) != self.account_name.as_ref())
        {
            meta.push(format!("作者：{author}"));
        }
        if let Some(time) = &self.publish_time {
            meta.push(format!("发布时间：{time}"));
        }
        let mut document = format!("# {}\n\n", self.title);
        if !meta.is_empty() {
            document.push_str(&format!("> {}\n", meta.join(" ｜ ")));
        }
        document.push_str(&format!("> 原文：{url}\n\n{}\n", self.body));
        document
    }
}

fn parse(html: &str) -> Result<Article> {
    let document = Html::parse_document(html);
    let content_selectors = [
        "#js_content",
        "#img-content .rich_media_content",
        ".rich_media_content",
        "#js_article",
    ];
    let content = content_selectors.iter().find_map(|sel| {
        let selector = Selector::parse(sel).ok()?;
        document.select(&selector).next()
    });

    let Some(content) = content else {
        bail!("{}", unavailable_reason(&document));
    };
    let mut markdown = Markdown::default();
    markdown.children(content);
    markdown.flush();
    if markdown.out.is_empty() {
        bail!("文章正文为空（可能是纯图片/视频消息，或页面结构不受支持）");
    }

    let title = meta(&document, "og:title")
        .or_else(|| select_text(&document, "#activity-name"))
        .or_else(|| js_var(html, "msg_title"))
        .context("未找到文章标题")?;
    let publish_time = js_var(html, "ct")
        .and_then(|ct| ct.parse::<i64>().ok())
        .map(format_beijing_time)
        .or_else(|| js_var(html, "createTime"));
    Ok(Article {
        title,
        account_name: js_var(html, "nickname").or_else(|| select_text(&document, "#js_name")),
        author: js_var(html, "author").or_else(|| meta(&document, "og:article:author")),
        publish_time,
        digest: meta(&document, "og:description").or_else(|| js_var(html, "msg_desc")),
        cover: meta(&document, "og:image").or_else(|| js_var(html, "msg_cdn_url")),
        body: markdown.out,
        images: markdown.images,
    })
}

fn unavailable_reason(document: &Html) -> String {
    let err_selector =
        Selector::parse(".weui-msg__title, .weui-msg__desc").expect("static selector");
    let err_text = document
        .select(&err_selector)
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");

    let title_selector = Selector::parse("title").expect("static selector");
    let title_text = document
        .select(&title_selector)
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");

    let combined = if err_text.is_empty() {
        title_text.clone()
    } else {
        format!("{err_text} ({title_text})")
    };

    if err_text.contains("环境异常")
        || err_text.contains("验证")
        || err_text.contains("微信客户端打开")
        || title_text.contains("环境异常")
    {
        return format!(
            "触发微信环境验证（请求过频或 IP 受限），请稍后重试或在浏览器中打开：{combined}"
        );
    }
    format!("页面没有文章正文（可能已删除、违规、需登录，或为不支持的消息类型）：{combined}")
}

fn meta(document: &Html, property: &str) -> Option<String> {
    let selector = Selector::parse(&format!(r#"meta[property="{property}"]"#)).ok()?;
    document
        .select(&selector)
        .find_map(|e| e.value().attr("content"))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn select_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let text = document
        .select(&selector)
        .next()?
        .text()
        .collect::<String>();
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// 读取页面脚本中的 `var name = "..."`、`'...'` 或 `htmlDecode("...")` 字面量。
fn js_var(html: &str, name: &str) -> Option<String> {
    let needle = format!("var {name}");
    let mut from = 0;
    while let Some(found) = html[from..].find(&needle) {
        let start = from + found + needle.len();
        from = start;
        let rest = html[start..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("htmlDecode(").unwrap_or(rest);
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let mut value = String::new();
        let mut chars = rest[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('x') => {
                        let hex: String = chars.by_ref().take(2).collect();
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    Some('n') => value.push('\n'),
                    Some(other) => value.push(other),
                    None => break,
                },
                c if c == quote => {
                    let value = decode_entities(value.trim());
                    return Some(value).filter(|v| !v.is_empty());
                }
                c => value.push(c),
            }
        }
        return None;
    }
    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Unix 秒 → 北京时间 `YYYY-MM-DD HH:MM`（公历换算，避免引入时间库）。
fn format_beijing_time(timestamp: i64) -> String {
    let seconds = timestamp + 8 * 3600;
    let (days, rem) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

fn image_src(element: ElementRef) -> Option<String> {
    let raw = element
        .value()
        .attr("data-src")
        .or_else(|| element.value().attr("src"))?
        .trim();
    let url = if let Some(rest) = raw.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = raw.strip_prefix("http://") {
        format!("https://{rest}")
    } else if raw.starts_with("https://") {
        raw.to_owned()
    } else {
        return None;
    };
    Some(url)
}

/// 把正文 DOM 转成 Markdown，并按出现顺序收集图片地址。
#[derive(Default)]
struct Markdown {
    out: String,
    line: String,
    images: Vec<String>,
}

impl Markdown {
    fn flush(&mut self) {
        let text = self
            .line
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        self.line.clear();
        self.block(&text);
    }

    fn block(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
        }
        self.out.push_str(text);
    }

    fn children(&mut self, element: ElementRef) {
        for child in element.children() {
            match child.value() {
                Node::Text(text) => self.text(text),
                Node::Element(_) => self.element(ElementRef::wrap(child).expect("element node")),
                _ => {}
            }
        }
    }

    fn text(&mut self, text: &str) {
        let mut previous_space = self.line.is_empty() || self.line.ends_with([' ', '\n']);
        for c in text.chars() {
            if c.is_whitespace() {
                if !previous_space {
                    self.line.push(' ');
                    previous_space = true;
                }
            } else {
                self.line.push(c);
                previous_space = false;
            }
        }
    }

    /// 渲染子节点为一段行内文本，不影响当前行。
    fn inline(&mut self, element: ElementRef) -> String {
        let saved = std::mem::take(&mut self.line);
        self.children(element);
        std::mem::replace(&mut self.line, saved)
    }

    fn wrap(&mut self, element: ElementRef, marker: &str) {
        let inner = self.inline(element);
        let trimmed = inner.trim();
        if trimmed.is_empty() || trimmed.contains('\n') || trimmed.starts_with(marker) {
            self.line.push_str(&inner);
            return;
        }
        if inner.starts_with(' ') && !self.line.ends_with(' ') {
            self.line.push(' ');
        }
        self.line.push_str(&format!("{marker}{trimmed}{marker}"));
        if inner.ends_with(' ') {
            self.line.push(' ');
        }
    }

    fn sub(&mut self, element: ElementRef) -> String {
        let mut sub = Markdown::default();
        sub.children(element);
        sub.flush();
        self.images.append(&mut sub.images);
        sub.out
    }

    fn element(&mut self, element: ElementRef) {
        let name = element.value().name();
        match name {
            "script" | "style" | "svg" | "iframe" | "noscript" | "mpvoice" | "mpvideo" => {}
            _ if name.starts_with("mp-") => {}
            "br" => self.line.push('\n'),
            "img" => {
                if let Some(src) = image_src(element) {
                    self.flush();
                    self.block(&format!("![]({src})"));
                    self.images.push(src);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                let level = usize::from(name.as_bytes()[1] - b'0');
                let inner = self.inline(element);
                let text = inner.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    self.block(&format!("{} {text}", "#".repeat(level)));
                }
            }
            "strong" | "b" => self.wrap(element, "**"),
            "em" | "i" => self.wrap(element, "*"),
            "del" | "s" => self.wrap(element, "~~"),
            "code" => self.wrap(element, "`"),
            "a" => {
                let inner = self.inline(element);
                let text = inner.trim();
                match element
                    .value()
                    .attr("href")
                    .filter(|h| h.starts_with("http"))
                {
                    Some(href) if !text.is_empty() && text != href => {
                        self.line.push_str(&format!("[{text}]({href})"));
                    }
                    Some(href) if text.is_empty() => self.line.push_str(href),
                    _ => self.line.push_str(&inner),
                }
            }
            "pre" => {
                self.flush();
                let mut code = String::new();
                code_text(element, &mut code);
                let code = code.trim_matches('\n');
                if !code.is_empty() {
                    self.block(&format!("```\n{code}\n```"));
                }
            }
            "blockquote" => {
                self.flush();
                let quoted = self
                    .sub(element)
                    .lines()
                    .map(|l| {
                        if l.is_empty() {
                            ">".to_owned()
                        } else {
                            format!("> {l}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.block(&quoted);
            }
            "ul" | "ol" => {
                self.flush();
                let mut items = Vec::new();
                for child in element.child_elements() {
                    let content = if child.value().name() == "li" {
                        self.sub(child)
                    } else {
                        self.element(child);
                        continue;
                    };
                    if content.is_empty() {
                        continue;
                    }
                    let marker = if name == "ol" {
                        format!("{}.", items.len() + 1)
                    } else {
                        "-".into()
                    };
                    let indent = " ".repeat(marker.chars().count() + 1);
                    let item = content
                        .lines()
                        .filter(|l| !l.is_empty())
                        .enumerate()
                        .map(|(i, l)| {
                            if i == 0 {
                                format!("{marker} {l}")
                            } else {
                                format!("{indent}{l}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    items.push(item);
                }
                self.block(&items.join("\n"));
            }
            "table" => {
                self.flush();
                let row_selector = Selector::parse("tr").expect("static selector");
                let mut rows = Vec::new();
                for row in element.select(&row_selector) {
                    let cells: Vec<String> = row
                        .child_elements()
                        .filter(|c| matches!(c.value().name(), "td" | "th"))
                        .map(|cell| {
                            let text = self.sub(cell).replace('\n', " ").replace('|', "\\|");
                            text.trim().to_owned()
                        })
                        .collect();
                    if !cells.is_empty() {
                        rows.push(format!("| {} |", cells.join(" | ")));
                        if rows.len() == 1 {
                            rows.push(format!("|{}", " --- |".repeat(cells.len())));
                        }
                    }
                }
                self.block(&rows.join("\n"));
            }
            "hr" => {
                self.flush();
                self.block("---");
            }
            "p" | "section" | "div" | "figure" | "figcaption" | "article" | "header" | "footer"
            | "li" | "tr" => {
                self.flush();
                self.children(element);
                self.flush();
            }
            _ => self.children(element),
        }
    }
}

fn code_text(element: ElementRef, out: &mut String) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => out.push_str(&text.replace('\u{a0}', " ")),
            Node::Element(e) if e.name() == "br" => out.push('\n'),
            Node::Element(e) => {
                let block = matches!(e.name(), "code" | "p" | "div" | "section");
                code_text(ElementRef::wrap(child).expect("element node"), out);
                if block && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head>
<meta property="og:title" content="标题 &amp; 测试" />
<meta property="og:description" content="摘要" />
<meta property="og:image" content="https://mmbiz.qpic.cn/cover/0?wx_fmt=jpeg" />
<script>var ct=et(r,G); var nickname = htmlDecode("某公众号"); var author = "张三";
var ct = "1791601320"; var msg_title = 'x\x26y'.html(false);</script></head>
<body><div id="js_content" style="visibility: hidden;">
<section><h2><span leaf="">小标题</span></h2>
<p><strong><span>第一，</span></strong><span>&nbsp;正文<em>强调</em>。</span></p>
<p><br></p>
<p><img data-src="http://mmbiz.qpic.cn/a/640?from=appmsg&amp;wx_fmt=png" /></p>
<p>▲ 图注</p>
<ul><li>甲</li><li><p>乙</p></li></ul>
<ol><li>一</li><li>二</li></ol>
<blockquote><p>引用1</p><p>引用2</p></blockquote>
<pre><code>fn main() {</code><code>    run();</code><code>}</code></pre>
<p>见 <a href="https://example.com">链接</a></p>
<table><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>
<mp-common-profile data-nickname="x"></mp-common-profile>
<script>ignored()</script>
</section></div></body></html>"#;

    #[test]
    fn parses_metadata_and_markdown() {
        let article = parse(PAGE).unwrap();
        assert_eq!(article.title, "标题 & 测试");
        assert_eq!(article.account_name.as_deref(), Some("某公众号"));
        assert_eq!(article.author.as_deref(), Some("张三"));
        assert_eq!(article.publish_time.as_deref(), Some("2026-10-10 11:02"));
        assert_eq!(article.digest.as_deref(), Some("摘要"));
        assert_eq!(
            article.images,
            ["https://mmbiz.qpic.cn/a/640?from=appmsg&wx_fmt=png"]
        );
        assert_eq!(
            article.body,
            "## 小标题\n\n**第一，** 正文*强调*。\n\n\
             ![](https://mmbiz.qpic.cn/a/640?from=appmsg&wx_fmt=png)\n\n▲ 图注\n\n\
             - 甲\n- 乙\n\n1. 一\n2. 二\n\n> 引用1\n>\n> 引用2\n\n\
             ```\nfn main() {\n    run();\n}\n```\n\n见 [链接](https://example.com)\n\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |"
        );
        let document = article.document("https://mp.weixin.qq.com/s/abc");
        assert!(document.starts_with(
            "# 标题 & 测试\n\n> 公众号：某公众号 ｜ 作者：张三 ｜ 发布时间：2026-10-10 11:02\n> 原文：https://mp.weixin.qq.com/s/abc\n\n## 小标题"
        ));
    }

    #[test]
    fn js_var_skips_non_literals_and_decodes_escapes() {
        assert_eq!(js_var(PAGE, "ct").as_deref(), Some("1791601320"));
        assert_eq!(js_var(PAGE, "msg_title").as_deref(), Some("x&y"));
        assert_eq!(js_var(PAGE, "missing"), None);
        assert_eq!(js_var("var nick = \"no\"", "nickname"), None);
    }

    #[test]
    fn reports_blocked_or_missing_pages() {
        let blocked = parse(
            "<html><head><title>微信公众平台</title></head><body><p class=\"weui-msg__title\">环境异常</p></body></html>",
        );
        assert!(blocked.unwrap_err().to_string().contains("环境验证"));
        let deleted = parse(
            "<html><body><p class=\"weui-msg__title\">该内容已被发布者删除</p></body></html>",
        );
        assert!(
            deleted
                .unwrap_err()
                .to_string()
                .contains("该内容已被发布者删除")
        );
        let empty = parse("<html><body><div id=\"js_content\"> </div></body></html>");
        assert!(empty.unwrap_err().to_string().contains("正文为空"));
    }

    #[test]
    fn normalizes_only_wechat_article_urls() {
        assert_eq!(
            normalize_url(" http://mp.weixin.qq.com/s/abc#rd ").unwrap(),
            "https://mp.weixin.qq.com/s/abc"
        );
        assert_eq!(
            normalize_url("https://mp.weixin.qq.com/s?__biz=x&mid=1").unwrap(),
            "https://mp.weixin.qq.com/s?__biz=x&mid=1"
        );
        for bad in [
            "https://mp.weixin.qq.com.evil.com/s/a",
            "https://evil.com/mp.weixin.qq.com/s/a",
            "ftp://mp.weixin.qq.com/s/a",
            "mp.weixin.qq.com/s/a",
            "https://mp.weixin.qq.com/s/a b",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn formats_beijing_time_and_sniffs_images() {
        assert_eq!(format_beijing_time(0), "1970-01-01 08:00");
        assert_eq!(format_beijing_time(951_782_400), "2000-02-29 08:00");
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8"), Some("webp"));
        assert_eq!(image_extension(b"<html>"), None);
    }

    #[tokio::test]
    async fn rejects_invalid_arguments_before_network() {
        for (args, message) in [
            (json!({}), "必须提供 url"),
            (json!({"url": "https://example.com"}), "mp.weixin.qq.com"),
            (
                json!({"url": "https://mp.weixin.qq.com/s/a", "output_dir": "rel"}),
                "绝对路径",
            ),
            (
                json!({"url": "https://mp.weixin.qq.com/s/a", "extra": 1}),
                "不支持的参数",
            ),
            (
                json!({"url": "https://mp.weixin.qq.com/s/a", "include_markdown": "no"}),
                "布尔值",
            ),
        ] {
            let error = execute(args).await.unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        }
    }
}
