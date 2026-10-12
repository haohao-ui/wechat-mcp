# WeChat MCP / 多平台消息网关

Rust 实现的 MCP stdio 服务。面向 Agent 自动化客服、客户管理、跟进与分析。核心按 `(platform, account_id)` 路由到消息适配器；当前实际接入的是 **macOS 桌面微信**，通过 AppleScript、鼠标和本地 Apple Vision OCR 操作可见界面。

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

协议请求可以并行执行，`ping`、账号查询不再等待微信长轮询完成。微信 UI 操作通过进程内互斥锁和当前用户共享的操作系统文件锁串行执行，避免焦点、剪贴板、截图相互干扰。长轮询持有桌面锁，其他 UI 操作需要等待它结束或取消。

会话列表、读取当前会话和全局等待请求支持 `notifications/cancelled`，子进程在任务退出时终止。涉及联系人搜索的读取/等待，以及发送和激活操作不支持中途取消，以避免截断有副作用的 UI 操作。持久收件箱 `wait_for_events` 支持取消且不占桌面锁。客户端关闭输入时停止当前后台任务并取消可取消请求，保留监听启用配置供重启恢复；已接受的有副作用操作会完成后退出。每个连接最多接受 64 个在途请求。

## 同平台多账号

账号唯一键是 `(platform, account_id)`；不同平台可以使用相同的 `account_id`。统一工具必须显式提供这两个字段。微信旧工具允许省略 `account_id`，但仅在微信账号唯一时自动选择；出现多个账号会拒绝猜选。

每个适配器绑定一个真实账号连接，`binding_id` 在网关内唯一，禁止用两个名字包装同一连接。新增 API 适配器时，每个账号必须有独立凭据、消息游标、会话标识和限流状态。消息、历史与发送状态的存储键必须包含平台和账号，不能仅使用联系人昵称或平台消息 ID。

**当前微信桌面适配器只控制本机同一 GUI 会话中的 `WeChat` 进程，不能同时管理两个微信登录账号。** 配置第二个桌面账号会在启动时明确失败。账号名是配置绑定标签，不是读取或验证过的微信身份；返回 `identity_verified: false`。用户手动切换微信登录账号后，需要重新核对绑定，当前程序无法自动识别这种切换。

真正的微信多账号需要独立设备/操作系统会话以及独立 worker。未来 worker 协议应返回已验证的账号身份，网关按绑定路由，不通过切换同一窗口实现并行账号。WhatsApp、TikTok 和远程 worker 适配器尚未实现，配置它们会报错。

## 构建

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

MCP 客户端配置示例（按实际安装位置修改）：

```json
{
  "mcpServers": {
    "wechat": {
      "command": "/Users/example/tools/code/Rust/wechat-mcp/target/release/wechat-mcp",
      "env": {
        "MESSAGE_GATEWAY_CONFIG": "/Users/example/tools/code/Rust/wechat-mcp/gateway.example.json"
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

客服工具另提供客户创建/更新/查询、交互记录、上下文快照、事件领取/续租/确认、跟进任务、带证据分析和统计。后台观察通过 `start_listener` / `stop_listener` / `get_listener_status` 管理，启用状态及间隔持久保存、重启恢复；同一数据库的账号监听只允许一个负责人，事件通过 `wait_for_events` / `claim_events` 消费。详见工作流文档。

原有 `send_wechat_message`、`focus_wechat`、`get_recent_sessions`、`read_chat_messages`、`wait_for_new_message` 工具名称保留，可传入可选的 `account_id`。返回内容改为统一 JSON，调用方应按结果字段判断状态，不能依赖旧自然语言文案。

结果包含账号归属及 `data`，以文本 JSON 返回；协商 MCP 2025-06-18 时还提供 `structuredContent`（同时兼容 2024-11-05）。微信适配器明确报告：

- 发送：outbox 状态为 `attempting`、`triggered` 或 `unknown`，`delivery: unknown`；实际适配器结果仍标明 `recipient_verified: false`。
- 读取：`coverage: visible_only`、`source: desktop_ocr`，无可靠消息 ID 或收发方向。
- 监听：`change_observed` 表示界面变化，`incoming_verified: false`。
- 能力：完整历史、送达回执、已验证后台消息事件均为 `false`；`background_observations: true` 表示支持后台界面观察。

## 权限与验证

宿主需要 macOS 辅助功能、自动化权限；截图读取需要相应的屏幕录制权限。微信应已启动并登录。

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 tests/protocol_smoke.py target/release/wechat-mcp
python3 tests/service_smoke.py target/release/wechat-mcp
```

单元测试覆盖账号隔离、并行领取、租约过期、重启恢复、发送幂等、任务生命周期和分析证据边界。协议及客服工作流测试运行真实 MCP 可执行文件并使用模拟数据，不会读取或发送真实微信消息。

## 后续扩展边界

持久收件箱和后台界面观察已实现；它们不能弥补 OCR 的消息身份、方向和历史缺失。宿主 Agent 需要保持消费/调度循环，当前没有主动唤醒已停止 Agent 的连接。下一步可在这一底座接入官方平台事件与回执、独立账号 worker，以及宿主调度桥接。真实微信多账号、WhatsApp 和 TikTok 接入仍未实现。
