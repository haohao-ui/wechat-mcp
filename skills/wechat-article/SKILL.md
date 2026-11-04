---
name: wechat-article
description: 通过公开网页读取、保存或总结 mp.weixin.qq.com 微信公众号文章及正文图片。用于文章链接，不操作个人微信聊天、群聊或好友；桌面操作使用 wechat-operator。
---

# 微信公众号文章

用 wechat MCP 服务的 `fetch_article` 工具获取公众号文章的标题、公众号、作者、发布时间、Markdown 正文和图片。它只请求 mp.weixin.qq.com 的公开网页，不操作微信客户端，不需要 `platform` / `account_id`，也不需要 macOS 辅助功能或录屏权限。

## 何时使用

- 消息中出现 `https://mp.weixin.qq.com/s/...` 或 `https://mp.weixin.qq.com/s?__biz=...` 链接。
- 用户要"看看这篇公众号文章""抓下来""存成 Markdown""连图一起下载""总结/翻译/拆解这篇文章"。

个人微信聊天、群聊、好友和监听任务转用 `wechat-operator`。本 Skill 不覆盖视频号/小程序、需登录的付费内容，不能从抓取结果推断阅读数、点赞数或评论。

## 调用方式

工具名在不同客户端里可能带前缀，例如 `mcp__wechat__fetch_article`。

| 参数 | 必填 | 说明 |
|---|---|---|
| `url` | 是 | 文章链接。`http://` 会被升级为 `https://`，`#rd` 等锚点会被去掉 |
| `output_dir` | 否 | **绝对路径**。写入 `article.md` 和 `images/01.jpg…`，Markdown 改为引用本地图片；同名文件会被覆盖 |
| `include_markdown` | 否 | 默认 `true`。只需要落盘时设为 `false`，避免全文占用上下文 |

按用户意图选择参数：

1. **只想读/总结/问答**：只传 `url`，直接使用返回的 `markdown`。
2. **要保存或要图片**：传 `output_dir`。用户没指定目录时，用当前工作目录下的 `wechat-articles/<标题的安全文件名>/`，先转成绝对路径；`include_markdown` 设为 `false`，再按需读取 `saved.markdown_path`。
3. **批量链接**：逐篇串行调用，每篇间隔 1–2 秒，避免触发微信环境验证。

## 返回字段

`title`、`account_name`（公众号）、`author`（可能与公众号相同）、`publish_time`（北京时间 `YYYY-MM-DD HH:MM`）、`digest`、`cover_image`、`text_length`、`image_count`、`images[]`（`url`，下载成功时还有 `local_path`）、`markdown`、`saved`（`markdown_path`、`images_dir`）、`image_errors`（部分图片失败时出现）。

`markdown` 开头已包含 `# 标题` 和一行公众号/发布时间/原文链接的引用块，转贴时不要重复拼接这些信息。

## 结果处理

- 回复用户时说明标题、公众号、发布时间，以及保存位置（如有）。不要把全文原样倒进对话，除非用户要求全文。
- 图片说明通常是紧跟在图片后、以 `▲` 开头的段落。
- 需要知道图片内容时，用当前环境可用的识图能力查看 `images/` 下的文件，不要凭图注猜测。
- 存在 `image_errors` 时如实告知哪几张失败；Markdown 中失败的图片仍是远程链接，可以直接下载（不需要 Referer）。
- 正文中出现的日期（如"2026-10-09原创"）是作者自己写的，以 `publish_time` 为准。

## 错误处理

| 错误信息 | 含义 | 处理 |
|---|---|---|
| `触发微信环境验证` | 请求过频或 IP 受限 | 停止批量请求，等几分钟再试；仍失败请用户在浏览器中打开 |
| `页面没有文章正文…：该内容已被发布者删除` 等 | 文章已删除、违规或链接错误 | 把原因转告用户，不要重试 |
| `文章正文为空` | 纯图片消息、视频或不支持的页面类型 | 告知用户；可只返回标题和封面 |
| `只支持 mp.weixin.qq.com` | 不是公众号文章链接 | 请用户提供原始文章链接（短链要先在浏览器中打开拿到原链接） |
| 找不到 `fetch_article` 工具 | MCP 未安装或版本过旧 | 使用下方后备方案，并提示用户重新安装 wechat-mcp |

## 后备方案（MCP 不可用时）

```bash
curl -sL -A "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36" "<文章链接>" -o article.html
```

- 标题：`<meta property="og:title" content="...">`
- 公众号：脚本变量 `var nickname = htmlDecode("...")`
- 发布时间：`var ct = "<Unix 秒>"`，按北京时间 (UTC+8) 换算
- 正文：`<div id="js_content">` 内部；图片地址在 `<img>` 的 `data-src` 属性（不是 `src`）

## 边界

文章版权归原作者。用于个人阅读、摘要、研究可以；用户要求整篇转载发布时，提醒其注明出处并取得授权。
