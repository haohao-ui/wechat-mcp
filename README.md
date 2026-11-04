# WeChat MCP / 个人微信 RPA

Rust 实现的个人微信桌面操作 MCP 服务，以 **macOS 个人微信** 的文字收发、会话观察和 Agent 事件接收为主线，通过 AppleScript、鼠标和本地 OCR 操作可见界面。内部按 `(platform, account_id)` 路由，保留客户管理、跟进和分析的辅助能力；这不表示已接入企业微信或其他消息平台。

这类自动化不能保证账号不受限制或封禁。发送操作不等于送达，OCR 文字不等于完整聊天历史。

客服工作流、幂等重试、数据边界和备份恢复见 [service-workflow.md](docs/service-workflow.md)。

## 架构

```text
MCP stdio（并发请求、取消、单一响应写入）
  └─ Gateway（参数校验、统一结果）
      ├─ SQLite（客户、收件箱、跟进、证据分析、发送幂等）
      ├─ Listeners（显式启动后台观察、持久快照、健康状态）
      └─ AccountRegistry（平台 + 账号路由、绑定去重）
          └─ MessageAdapter（独立的账号连接及能力声明）
              └─ WechatDesktop（桌面互斥锁）
                  └─ AppleScript / CoreGraphics / Swift Vision
```

- `src/accounts.rs`：账号键、能力声明、注册表、配置。
- `src/adapters.rs`：适配器接口和微信桌面实现。
- `src/gateway.rs`：统一工具路由、参数校验和结果封装。
- `src/storage.rs`、`migrations/`：数据库、事务快照与发送幂等记录。
- `src/crm.rs`、`src/service_tools.rs`：客户、收件箱租约、跟进、分析与统计。
- `src/listeners.rs`：后台界面观察、健康状态与持久收件箱等待。
- `src/main.rs`、`src/mcp.rs`、`src/tools.rs`：协议层与工具定义。
- `src/wechat.rs`、`src/clipboard.rs`、`native/`：平台实现。
- `src/article.rs`：公众号文章抓取（公开网页 → Markdown + 图片），不经过桌面适配器。
- `skills/wechat-operator/`：个人微信操作主 Skill；按需加载收发、环境和事件参考。
- `skills/wechat-article/`：独立的公众号网页读取附属 Skill。

协议请求可以并行执行，`ping`、账号查询不再等待微信长轮询完成。微信 UI 操作通过进程内互斥锁和当前用户共享的操作系统文件锁串行执行，避免焦点、剪贴板、截图相互干扰。长轮询持有桌面锁，其他 UI 操作需要等待它结束或取消。

会话列表、读取当前会话和全局等待请求支持 `notifications/cancelled`，子进程在任务退出时终止。涉及联系人搜索的读取/等待，以及发送和激活操作不支持中途取消，以避免截断有副作用的 UI 操作。持久收件箱 `wait_for_events` 支持取消且不占桌面锁。客户端关闭输入时停止当前后台任务并取消可取消请求，保留监听启用配置供重启恢复；已接受的有副作用操作会完成后退出。每个连接最多接受 64 个在途请求。

## 同平台多账号

账号唯一键是 `(platform, account_id)`；不同平台可以使用相同的 `account_id`。统一工具必须显式提供这两个字段。微信旧工具允许省略 `account_id`，但仅在微信账号唯一时自动选择；出现多个账号会拒绝猜选。

每个适配器绑定一个真实账号连接，`binding_id` 在网关内唯一，禁止用两个名字包装同一连接。新增 API 适配器时，每个账号必须有独立凭据、消息游标、会话标识和限流状态。消息、历史与发送状态的存储键必须包含平台和账号，不能仅使用联系人昵称或平台消息 ID。

**当前微信桌面适配器只控制本机同一 GUI 会话中的 `WeChat` 进程，不能同时管理两个微信登录账号。** 配置第二个桌面账号会在启动时明确失败。账号名是配置绑定标签，不是读取或验证过的微信身份；返回 `identity_verified: false`。用户手动切换微信登录账号后，需要重新核对绑定，当前程序无法自动识别这种切换。

真正的微信多账号需要独立设备/操作系统会话以及独立 worker。未来 worker 协议应返回已验证的账号身份，网关按绑定路由，不通过切换同一窗口实现并行账号。WhatsApp、TikTok 和远程 worker 适配器尚未实现，配置它们会报错。

## 一键安装

仓库为私有，需要一个有 `repo`（或 Contents: Read）权限的 GitHub Token：

```bash
export GITHUB_TOKEN=ghp_xxx
set -o pipefail
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash
```

脚本会：下载最新 Release 的 macOS 通用二进制并校验 SHA-256（无 Release 时自动从源码构建）→ 安装到 `~/.wechat-mcp/bin` → 生成 `~/.wechat-mcp/gateway.json`（已存在则保留）→ 安装个人微信主 Skill `wechat-operator` 及附属 `wechat-article` → 自动识别当前 Agent（Claude Code / Codex / Gemini CLI / Cursor / Windsurf / Cline / OpenCode / Claude Desktop）并注册 MCP 服务 `wechat`。识别不到时退回注册 Claude Code；用 `--agent all` 可一次配置所有检测到已安装的客户端。安装全程使用绝对路径与 Agent 局部变量，不修改系统的 `PATH`，不污染用户的 Shell 配置文件。

**让 Agent 自动安装**：把 [agent-install.md](docs/agent-install.md) 里的提示词交给任意 Agent。它会执行一条命令：

```bash
set -o pipefail
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --agent auto --json
```

安装、MCP 注册、skill 安装和自检都由脚本完成，Agent 只需汇报 JSON 结果。手动指定客户端时把 `--agent auto` 换成 `--agent codex`、`--agent all` 等；`--agent none` 只安装不注册。

可选变量：`WECHAT_MCP_AGENT=codex`（同 `--agent`）、`WECHAT_MCP_VERSION=v0.1.0` 指定版本、`WECHAT_MCP_FROM_SOURCE=1` 强制源码构建、`WECHAT_MCP_HOME` 修改安装目录。

检查安装是否正常（输出 `"check":"passed"` 即正常）：

```bash
set -o pipefail
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --check --json
```

卸载（移除二进制、skill 和各 Agent 中的 `wechat` 配置，保留配置与数据库）：

```bash
set -o pipefail
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --uninstall --json
```

发布新版本：`git tag v0.1.0 && git push origin v0.1.0`，Release 工作流自动构建并上传安装包。

## 构建

模块化RPA底座、stream模式和只读能力诊断见 [RPA运行说明](docs/rpa-runtime.md)。默认保持旧驱动；新模式须完成本机实机验收后再启用。

需要 macOS、Rust 1.88+（Edition 2024）及可用的 Swift 编译器：

```bash
bash scripts/build.sh
```

构建产物位于 `target/release/`，部署时保持三个文件在同一目录：

```text
wechat-mcp
wechat_ocr
wechat_click
```

Swift 源码在 `native/`，构建不再依赖本机被忽略的 `bin/`。开发环境仍可使用已有 `bin/` 辅助程序。

## 账号配置

不设置环境变量时，默认注册 `wechat/local` 一个桌面绑定。自定义配置参考 `gateway.example.json`：

```json
{
  "accounts": [
    {"platform": "wechat", "account_id": "work", "adapter": "wechat_desktop"}
  ]
}
```

通过 `MESSAGE_GATEWAY_CONFIG` 指定 JSON 文件绝对路径。通过 `MESSAGE_GATEWAY_DB` 指定客服 SQLite 数据库的绝对路径；默认存于用户应用数据目录。配置错误、重复账号、重复绑定或未实现的适配器都会阻止启动。

MCP 客户端配置示例（一键安装的默认位置；`<HOME>` 替换为你的用户主目录绝对路径，MCP 配置不会展开 `~`）：

```json
{
  "mcpServers": {
    "wechat": {
      "command": "<HOME>/.wechat-mcp/bin/wechat-mcp",
      "env": {
        "MESSAGE_GATEWAY_CONFIG": "<HOME>/.wechat-mcp/gateway.json"
      }
    }
  }
}
```

## 工具

| 统一工具 | 参数 | 用途 |
|---|---|---|
| `list_accounts` | 无 | 查看账号绑定与能力边界 |
| `list_conversations` | `platform`, `account_id` | 当前可见会话 |
| `get_messages` | 上述字段，`recipient` 可选 | 当前可见聊天文字 |
| `send_message` | 上述字段，`recipient`, `message`, `idempotency_key`；`customer_id` 可选 | 幂等登记并触发文本发送 |
| `wait_for_message` | 上述字段，`recipient` 可选，`timeout_seconds` 可选 | 等待界面变化；超时为 1–300 秒 |

### 公众号文章

| 工具 | 参数 | 用途 |
|---|---|---|
| `fetch_article` | `url`；`output_dir`、`include_markdown` 可选 | 抓取 mp.weixin.qq.com 文章的标题、公众号、作者、发布时间（北京时间）、摘要、封面和 Markdown 正文 |

只请求公开网页（系统 `curl`），不操作微信客户端、不占桌面锁，也不需要 `platform` / `account_id` 和 macOS 隐私权限。提供绝对路径 `output_dir` 时，正文图片下载到 `images/01.jpg…`（按文件头识别格式，单篇上限 300 张），并写入引用本地图片的 `article.md`；部分图片失败会列在 `image_errors`，对应位置保留远程链接。已删除文章、触发微信环境验证、纯图片/视频消息会返回明确错误。阅读数、点赞、评论不在公开页面中，无法获取。

个人微信操作从 [wechat-operator](skills/wechat-operator/SKILL.md) 进入，按需读取其自包含的收发、环境和事件参考。群聊成员/方向、好友操作和长期常驻等未完成项在主 Skill 中明确标注；公众号文章由独立的 [wechat-article](skills/wechat-article/SKILL.md) 处理。

安装时两个 Skill 及其参考文件一起进入 `~/.wechat-mcp/skills/`，再按客户端分发到 `~/.claude/skills/`、`~/.agents/skills/`（或 `~/.codex/skills/`）；没有 Skill 目录的客户端使用指向相应入口的规则。Claude Desktop 分别打包两个 Skill，不能把多个 Skill 混成一个上传包。安装多个客户端与主动推送到多个 Agent 是不同能力；后者参见 [Agent 桥接说明](docs/agent-bridge.md)。

客服工具另提供客户创建/更新/查询、交互记录、上下文快照、事件领取/续租/确认、跟进任务、带证据分析和统计。后台观察通过 `start_listener` / `stop_listener` / `get_listener_status` 管理，启用状态及间隔持久保存、重启恢复；同一数据库的账号监听只允许一个负责人，事件通过 `wait_for_events` / `claim_events` 消费。详见工作流文档。

原有 `send_wechat_message`、`focus_wechat`、`get_recent_sessions`、`read_chat_messages`、`wait_for_new_message` 工具名称保留，可传入可选的 `account_id`。返回内容改为统一 JSON，调用方应按结果字段判断状态，不能依赖旧自然语言文案。支持通过环境变量 `MESSAGE_GATEWAY_LEGACY_TOOLS=0`（或 `false`/`off`）关闭旧工具：关闭后 `tools/list` 不列出它们，`tools/call` 调用将返回错误并提示改用对应统一工具（`focus_wechat` 无统一替代，一并禁用）。

结果包含账号归属及 `data`，以文本 JSON 返回；协商 MCP 2025-06-18 时还提供 `structuredContent`（同时兼容 2024-11-05）。微信适配器明确报告：

- 发送：outbox 状态为 `attempting`、`triggered` 或 `unknown`，`delivery: unknown`；实际适配器结果仍标明 `recipient_verified: false`。
- 读取：`coverage: visible_only`、`source: desktop_ocr`，无可靠消息 ID 或收发方向。
- 监听：`change_observed` 表示界面变化，`incoming_verified: false`。
- 能力：完整历史、送达回执、已验证后台消息事件均为 `false`；`background_observations: true` 表示支持后台界面观察。

## 权限与验证

宿主需要 macOS 辅助功能、自动化权限；截图读取需要相应的屏幕录制权限。微信应已启动并登录。

macOS 26+ 上 Apple Vision 精确 OCR 可能不可用或超时，`wechat_ocr` 会自动回退到 Tesseract。建议安装：

```bash
brew install tesseract tesseract-lang
```

也可用 `WECHAT_OCR_ENGINE=tesseract` 或 `WECHAT_OCR_ENGINE=vision` 强制指定引擎；`WECHAT_OCR_TESSERACT` 可指定 tesseract 绝对路径。未安装 Tesseract 时仍会尝试 Apple Vision，但可能较慢。

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 tests/protocol_smoke.py target/release/wechat-mcp
python3 tests/service_smoke.py target/release/wechat-mcp
```

单元测试覆盖账号隔离、并行领取、租约过期、重启恢复、发送幂等、任务生命周期和分析证据边界。协议及客服工作流测试运行真实 MCP 可执行文件并使用模拟数据，不会读取或发送真实微信消息。

## 后续扩展边界

持久收件箱和后台界面观察已实现；它们不能弥补 OCR 的消息身份、方向和历史缺失。宿主 Agent 需要保持消费/调度循环，当前没有主动唤醒已停止 Agent 的连接。下一步可在这一底座接入官方平台事件与回执、独立账号 worker，以及宿主调度桥接。真实微信多账号、WhatsApp 和 TikTok 接入仍未实现。


## 白名单 AI 客服 Bot

个人微信可接入本机常驻模型客服。当前受控版本支持多个明确授权的私聊白名单及多群/私聊排除列表，排除优先、会话状态隔离、桌面操作串行；可使用 Codex 或 OpenAI 兼容模型接口，启动前历史不自动回复；群聊自动回复和好友操作关闭。配置、运行状态及停止命令见[Bot运行说明](docs/allowlist-bot.md)，验证证据见[实机记录](research/bot-acceptance-2026-11-04.md)。视觉配置当前仅适配已标定的暗色880×640单倍像素，不是通用生产验收。
