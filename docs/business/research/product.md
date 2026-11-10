# wechat-mcp 产品与技术就绪度盘点

> 用途：商业化尽调 / 售卖可行性判断。基线：`HEAD=cb92ce5`（2026-11-09 23:53）。盘点时工作区**不干净**（含另一会话的通用眼手复核 WIP，且 00:05 前后又有 voice/policy 改动落盘），门禁结果是 **2026-11-10 00:00 的工作区快照**；本文把“源码事实”与“该快照的门禁结果”分开陈述。
> 方法：通读 README、`docs/` 关键文档与 `research/` 实机记录，逐项对源码用 grep/read 交叉核对，并在本机实跑 `tools/list`、`tools/call` 与门禁命令。**文档 ≠ 现状**的地方单独标注。

---

## 0. 结论摘要

**这个仓库不是"演示 Demo"，而是一套已接近可用形态的 macOS 个人微信 RPA 底座：单机、单账号、标准窗口下的"读屏事实 + 幂等发送 + 轮次编排 + 本地历史 + 外部大脑协议"都已实现，技术差异化明显；但作为"可对外售卖的产品"，还差 5 类硬缺口。**

1. **迁移未复验**：白名单自动回复 Bot 已从 Python 全量迁到 Rust `serve-core`（Python 源码已删除），但所有"实机验收"证据都来自 Python 时代；Rust 实现只有自动化测试。
2. **对外契约有断点**：51 个 MCP 工具里，7 个 `history_*` 工具**在 `tools/list` 中可见但无法调用**（实测返回"未知工具"）；`outputSchema` 为 0，错误只有自然语言。
3. **交付工程未产品化**：release 未签名/未公证、安装脚本主动摘 `quarantine`、宿主 App 只由 `deploy_host.sh` 本机构建，无公网可分发的带签名宿主安装包。
4. **运行时边界仍是审计红线**：桌面操作/`osascript` 无超时、一个工具 task panic 会退出整个 MCP 服务（`src/main.rs:220-221`）。
5. **实机验收覆盖不全**：群聊、好友申请、通讯录同步、通用眼手、语音、Open API、备份恢复均未完成真机验收。

已实机验收的"最小可信内核"是：**单账号私聊的 OCR 读取 + 核验式发送 + 幂等记录 + 轮次聚合 + 白名单自动回复（Python 版）+ 亮暗主题 880×640 自适应 + 浮层动作可视化**。

---

## 1. 盘点口径

| 项 | 值 |
|---|---|
| 仓库 | `/path/to/wechat-mcp` |
| HEAD | `cb92ce5 fix(voice): decide voice bubbles by width geometry, not OCR characters` |
| 版本 | `Cargo.toml` → `0.1.0`，`edition 2024`，`rust-version 1.88`，工具链锁 `1.99.0`（`rust-toolchain.toml`） |
| 代码量 | Rust `src/` 67,973 行；Swift `native/` 12,725 行（含宿主/worker/通用眼手） |
| 测试 | 18 个测试目标、**实跑 1,104 个用例**（1,089 pass / 15 fail / 1 ignored） |
| 运行形态 | 本机 stdio MCP（`wechat-mcp`）+ 常驻 `serve-core` + 6 页 Swift 宿主 App「微信客服助手」+ launchd |
| 证据口径 | `research/*acceptance*` = 真机记录；`tests/` = 自动化；文档未标注即视为未验收 |

---

## 2. 产品形态与架构

```text
MCP 客户端（Claude Code/Codex/Cursor/…）
  ├─ stdio MCP  ── wechat-mcp（网关：账号隔离、参数校验、幂等 outbox）
  └─ HTTP MCP   ── open_api（内网，令牌，单会话，11 个白名单工具，默认关闭）
        └─ serve-core（Rust 常驻：调度、轮次、历史库、外部大脑账本、通讯录、好友申请、自动备份）
              ├─ SQLite（多库：gateway service / history / 状态 JSON）
              └─ Swift GUI 宿主「微信客服助手」（launchd 常驻、TCC 授权、Unix socket 托管）
                    └─ Swift worker（读屏 OCR、点击、会话切换、通讯录、好友申请）
                          └─ native/action（拟人轨迹）· native/interaction（通用眼手）
```

| 组件 | 位置 | 职责 | 成熟度 |
|---|---|---|---|
| stdio MCP 网关 | `src/main.rs`、`src/gateway.rs`、`src/tools.rs` | 51 工具、并发 64、取消、旧工具兼容 | 已实机（基础收发） |
| 常驻核心 | `src/core/serve.rs`、`scheduler.rs`、`turns.rs` | 轮次、白名单、历史、Open API、备份 | 仅自动化测试 |
| Swift 宿主 App | `native/bootstrap/`（19 个 Swift 文件） | 权限、socket、core 监护、6 页 UI | 旧宿主已实机／新面板未验收 |
| Swift worker | `native/wechat_bubbles.swift`、`native/bot/` | 采集/点击/滚动/媒体/通讯录 | 亮暗 880×640 已实机 |
| 通用眼手 | `src/interaction.rs`、`native/interaction/` | 应用无关 AX+OCR+动作 | **实机阻塞** |
| 外部大脑 | `src/core/open_api/`、`src/core/external_brain.rs` | 轮次认领/回复/终态账本 | 局部实机联调 |

---

## 3. 能力清单：MCP 工具面（运行时实测）

`tools/list` 实测 **51 个工具**（与 `target/debug/wechat-mcp` 握手得到），分组如下：

| 工具组 | 数量 | 代表工具 | 来源 | 成熟度 |
|---|---|---:|---|---|
| 旧版兼容工具 | 5 | `send_wechat_message`、`read_chat_messages`、`wait_for_new_message`、`focus_wechat`、`get_recent_sessions` | `src/tools.rs:39-120`；`MESSAGE_GATEWAY_LEGACY_TOOLS=0` 可关（`:3-16`） | 已实机（单账号） |
| 统一微信工具 | 6 | `list_accounts`、`send_message`、`send_file`、`get_messages`、`list_conversations`、`wait_for_message` | `src/tools.rs:137-184` | 已实机（单账号） |
| 通用桌面眼手 | 5 | `desktop_apps/windows/observe/act/stop` | `src/interaction.rs:197-260`，`native/interaction/` | 仅自动化测试＋实机 blocked |
| CRM / 收件箱 / 监听 | 23 | 客户、跟进、分析、事件租约、监听、`get_send_status` | `src/service_tools.rs`（含 `annotations`） | 仅自动化测试 |
| 文件上传 | 1 | `upload_file`（base64→本机 `~/Documents/微信客服附件/待发送`，≤10MB） | `src/tools.rs:130`；`src/wechat/files.rs:7,84-92` | 仅自动化测试 |
| 公众号文章 | 1 | `fetch_article`（公开网页→Markdown+图片） | `src/article.rs` | 仅自动化测试 |
| 外部大脑 | 3 | `next_turn`、`reply`、`resume_session` | `src/core/brain_tools.rs:25-90`，经 `core.sock` 转发 | 仅自动化测试 |
| 历史查询 | 7 | `history_list_contacts` 等 | `src/core/history/mcp_tools.rs:21-34`；`src/tools.rs:133` | **未接线（见 §3.1）** |

**契约现状（实测）**：51 个工具中 `annotations` 仅 23 个、**`outputSchema` 为 0**；错误返回只有文本 `{"content":[{"text":"…"}],"isError":true}`（`src/gateway.rs:84-86`），无 `code/retryable/side_effect`。与 `docs/reviews/2026-11-08-agent-tool-optimization.md` 的 P1 结论一致，尚未修复。

### 3.1 已确认的对外契约断点：7 个 `history_*` 工具列而不可调

- 工具列表把它们加进来了：`src/tools.rs:133`（`tools.extend(crate::core::history::mcp_tools::tools())`）。
- 但网关分发没有对应分支：`src/gateway.rs:100-135` / `:193-200`（未知工具分支）。
- **本机实测**（标准 stdio 握手后调用）：

```text
tools/call history_list_contacts → isError=true, text="未知工具: history_list_contacts"
tools/call next_turn             → isError=true, text="core_not_running"（预期：未启 core）
```

- 历史查询的生产入口实际是 `serve-core` 内的只读 HTTP（`src/core/serve.rs:318-356`）与 Open API 的白名单（`docs/open-api/v1.md:549-579`）；`history::mcp_tools::run_stdio` 仅被测试调用（`src/core/history/mcp_tools.rs:540,757`）。
- **影响**：文档承诺的"历史库 MCP 查询"（`docs/allowlist-bot.md:184-192`）当前对 Agent 不可用；这是与 2026-11-08 评审同一类 P0（"列而不可调用"）的残留。

---

## 4. 宿主 App「微信客服助手」

| 项 | 内容 | 证据 |
|---|---|---|
| 形态 | 普通 GUI App，`local.wechat-mcp.host`，`~/Applications/微信客服助手.app`，launchd 登录自启 | `native/bootstrap/README.md:14-22` |
| 页面 | **6 页**：总览 / 会话 / 联系人 / MCP / 设置 / 日志 | `native/bootstrap/main.swift:72-77` |
| 菜单栏面板 | 状态灯、暂停/恢复、重启机器人、放回微信窗口、权限请求、日志/附件入口 | `native/bootstrap/StatusPanel.swift:115-340` |
| MCP 页 | 外部大脑模式切换、Open API 地址/令牌复制/轮换/真实握手测试、工具分组（对外接口/运维/通用桌面） | `native/bootstrap/McpPage.swift:99-260` |
| 联系人页 | 同步结果、五分类（含企业微信/公众号/已忽略）、忽略与自动回复开关 | `native/bootstrap/ContactsPage.swift:97-272` |
| core 监护 | 读 `~/.wechat-rpa/core.enabled`，托管 `wechat-mcp serve-core`，与旧 Python 服务冲突检测，SIGTERM→240s→SIGKILL | `native/bootstrap/CoreSupervisor.swift:146-170,304-340` |
| 成熟度 | **旧宿主 `WeChat RPA Test` 已实机**（权限、socket、MCP 自检 34 工具）；**当前 6 页面板与 core 托管无实机验收记录** | `research/dedicated-host-2026-11-04.md:17-45` |

---

## 5. 白名单自动回复 Bot（当前实现 = Rust `serve-core`）

配置统一走 `~/.WeChatRPABot/config.json`（宿主与 CLI 都读这个路径，`native/bootstrap/CoreSupervisor.swift:227`、`src/core/control.rs:530-537`）。

| 能力 | 实现/默认值 | 证据 | 成熟度 |
|---|---|---|---|
| 多私聊白名单 + 排除列表（排除优先） | ≥1 条，≤200 条；精确名称；≤64 字符；≤10 别名；归一化撞名拒绝 | `src/core/config.rs:199-301`；`docs/allowlist-bot.md:116-139` | Python 版单联系人已实机；多联系人/排除列表仅自动化测试 |
| **群聊自动回复** | 配置层允许 `kind=group` 进 allowlist，调度会置 `groups_enabled`；但产品文档仍写"群聊只能排除" | `src/core/config.rs:250-252`；`src/core/scheduler.rs:705`；`docs/allowlist-bot.md:133` | **未验收（口径冲突）** |
| 一轮一回聚合 | 静默 10s / 单轮上限 60s / 重启续计时 | `src/core/config.rs:658-659`；`docs/allowlist-bot.md:51` | 仅自动化测试 |
| 图片 | 尺寸+phash 识别；原图优先、缩略图兜底；分批理解（默认每批 4 张） | `src/core/policy.rs:9-55`；`src/core/config.rs:660` | 部分：文本已实机；图片/文件回复实机**不稳定**（`research/ui-auto-adapt-acceptance-2026-11-05.md:18`） |
| 文件 | 原件保存 + 文本/表格提取（PDF/DOCX/XLSX）；失败记 `media_failed` 并请客户重发 | `src/brain/documents.rs:300-400`；`docs/allowlist-bot.md:53` | 仅自动化测试 |
| 语音消息 | 识别语音气泡→点"转文字"→把转写当客户文本；已有自动化测试 | `src/core/worker_client.rs:967-1031`；`tests/swift_voice.rs`（3 例）；提交 `52fc493`/`cb92ce5` | **仅自动化测试，无实机验收**；文档仍写"语音尚未接入"（`docs/allowlist-bot.md:89`） |
| 限流 | 冷却 15s（0–300）、每小时 20 次（1–100）、轮询 2s（1–60） | `src/core/config.rs:655-657` | 已实机（Python 版参数一致） |
| 会话隔离 | 每会话独立游标/限流/幂等/暂停；单会话异常不影响其他 | `docs/allowlist-bot.md:26,135` | 仅自动化测试 |
| 人工已回复检测 | 屏幕/历史/流水三源判 `already_replied`，不补发 | `docs/allowlist-bot.md:55` | 仅自动化测试 |
| 停止/暂停 | `paused` 需人工复核；停止信号在发送前检查 | `docs/allowlist-bot.md:41-45`；`src/core/turns.rs` | 部分（信号实时性见 §17.2 P1-3） |
| 模型后端 | 默认 `codex-exec`（本机 CLI，已实现，`src/brain/model_api.rs:393,465`）；OpenAI 兼容（DeepSeek 等） | `src/core/config.rs:334-343`；`src/brain/model_api.rs:123-128` | codex 后端曾缺失，**HEAD 已补齐**（2026-11-08 评审 P1 已修） |
| 密钥安全 | `api_key_file` 必须绝对路径/非软链/属主本人/0600；只从 env 或文件读 | `src/core/config.rs:303-325` | 仅自动化测试 |
| 专用机/窗口 | 自动恢复最小化、调回 880×640、挪回屏幕内；未知主题/尺寸拒绝发送 | `docs/allowlist-bot.md:60-64` | 已实机（深/浅色、窗口恢复） |
| 状态浮层 | 「● 微信 Bot · 运行中」独立面板，不拦截鼠标 | `docs/allowlist-bot.md:97-99` | 已实机（浮层） |

**关键结论**：Bot 的**产品行为**（轮次、白名单、图片/文件、限流、状态机）在 Python 时代有 3 条真实模型+真实发送记录（单次验证 1 条 + 常驻 2 条）（`research/bot-acceptance-2026-11-04.md:9-28,35-47`）；**当前 Rust 实现没有任何一篇 `research/*acceptance*`**。Python 源码已删除（`git ls-files '*.py'` 为空，`scripts/check_no_python.sh` 强制禁止回归），验收证据不能直接继承到新代码。

---

## 6. 历史库

| 能力 | 实现 | 证据 | 成熟度 |
|---|---|---|---|
| 存储 | `<state_dir>/history/history.sqlite3`，WAL，0600；附件按内容 SHA-256 寻址 | `src/core/history/store.rs:271,441-445,1035-1049` | 仅自动化测试 |
| 记录内容 | 可见消息（`customer/bot/self_other`）+ capture 来源（live/baseline/rebaseline_gap/ambiguous/imported）+ 附件理解要点 | `docs/allowlist-bot.md:158-165` | 仅自动化测试 |
| 检索 | FTS5（`messages_fts MATCH`）+ 图片/文件理解要点 LIKE 兜底；分页 cursor | `src/core/history/query.rs:749-760,863-874` | 仅自动化测试 |
| 接口 | 只读 HTTP（`/v1/contacts|search|media`，token 0600，≤500 条/页）；Open API 白名单 7 个工具 | `src/core/serve.rs:318-356`；`docs/allowlist-bot.md:184-203` | 仅自动化测试 |
| MCP 接口 | 7 个 `history_*` 工具 | — | **断点（§3.1，列而不可调）** |
| 名称治理 | 别名、改名、合并/反合并、未匹配名称建议 | `docs/allowlist-bot.md:167-182` | 仅自动化测试 |
| 限制 | 只存 Bot 实际看到的内容；时间为观察时间；`self_other` 不区分人工/回显 | `docs/allowlist-bot.md:205` | 设计限制（非缺陷） |

---

## 7. 通讯录同步

| 项 | 内容 | 证据 |
|---|---|---|
| 实现 | 按微信分组（联系人/群/企业微信/公众号）截图识别+点击翻页，checkpoint 断点续传；写 `contact_profiles`；空闲编排全量/增量 | `src/core/contacts/sync.rs:23,145-235,741-1009`；`docs/superpowers/specs/2026-11-08-contacts-sync-design.md` |
| UI | 联系人页 5 分类、忽略/自动回复开关 | `native/bootstrap/ContactsPage.swift:97-272` |
| 成熟度 | **仅自动化测试**；实施计划 Task 7（部署+实机验收）**仍未勾选**：`docs/superpowers/plans/2026-11-08-contacts-sync-and-ui.md:237-243` |

---

## 8. 好友申请处理

| 项 | 内容 | 证据 |
|---|---|---|
| 默认 | **关闭**；`mode=dry_run`；来源默认"扫一扫/搜索"；`match=all`；每日 20；120s 检查 | `src/core/friend_requests.rs:25-48` |
| 规则 | 来源/验证消息包含匹配；按账号/昵称/验证消息/来源摘要去重；`needs_review` 不自动重试 | `docs/features/friend-requests.md:17-19` |
| 安全 | 只点 OCR 定位的昵称/前往验证/许可按钮；dry_run 永不点确认；诊断目录 0700/文件 0600；live 通过后走增量通讯录同步 | `docs/features/friend-requests.md:19-23` |
| 成熟度 | **未完成/未验收**：文档明确"验证对话框尚无真机验收证据；未启动生产机器人，也未操作真实好友申请" | `docs/features/friend-requests.md:25`；`docs/rpa-runtime.md:122`（"群聊和好友操作未验收"） |

---

## 9. 外部大脑 Open API（技术亮点 + 未验收）

| 项 | 内容 | 证据 |
|---|---|---|
| 形态 | `serve-core` 内 Streamable HTTP MCP（`POST/DELETE /mcp`），默认 `127.0.0.1:8770`，默认关闭 | `src/core/open_api/http.rs:35-47`；`docs/open-api/v1.md:9-16` |
| 安全 | 64 位 hex Bearer 令牌（常数时间比对）、Origin 白名单、单活动会话 90s 租期、body ≤1MiB、header ≤16KiB、并发 64 | `src/core/open_api/http.rs:49-80`；`docs/open-api/v1.md:9-16,625-633` |
| 工具面 | 11 个白名单工具：`next_turn`、`reply`、`get_turn_status`、`resume_session` + 7 个历史查询 | `docs/open-api/v1.md:510-579` |
| 协议语义 | 认领租约/`claim_token`、终态账本（accepted/expired/revoked）、同参重放幂等（text SHA-256）、`accepted ≠ 已发送`、发送状态四分 | `docs/open-api/v1.md:583-621`；`src/core/open_api/ledger.rs`、`turn_tests.rs`（25 例） |
| 测试 | `open_api` 模块 80 个用例（http 23、turn 25、contract 12、history 9、access 6…） | `src/core/open_api/*.rs` |
| 成熟度 | **局部实机联调、无正式验收**：提交 `d57abc2` 记录"实机验收的客户端因此漏掉已领取的轮次"并据此修 `next_turn` 返回结构；计划中的"专用机实机验收"步骤（`docs/superpowers/plans/2026-11-09-external-brain-open-api.md:319-323`）无验收记录 | `git log` |

---

## 10. 备份与恢复

| 能力 | 状态 | 证据 |
|---|---|---|
| 在线备份（SQLite backup API，源库 READONLY + 5s busy_timeout，128 页步进） | 已实现 | `src/core/backup.rs:171-248` |
| manifest（每库 sha256、附件文件数/字节数） | 已实现 | `src/core/backup.rs:484-509,603-632` |
| 附件目录 `history/media` 纳入备份 | **已实现**（2026-11-09 审计 D-4 之后修复） | `src/core/backup.rs:484-490` |
| `restore` 子命令（校验 manifest/sha256 → 抢 `core.lock` → 换库） | 已实现 | `src/main.rs:138-183`；`src/core/backup.rs:603+` |
| `--out` 目录保护 / 自动备份轮转 | 已实现（只认程序备份目录、prune 保留 N 份） | `src/core/backup.rs:314-351` |
| 成熟度 | **仅自动化测试**；无"备份→删源→恢复→附件抽查"实机演练记录 | — |

---

## 11. 通用眼手（`desktop_*`，5 工具）

| 项 | 内容 | 证据 |
|---|---|---|
| 能力 | 应用无关：列应用/窗口、AX+OCR 观察（单次凭证）、focus/click/right_click/double_click/drag/scroll/key/type_text、拟人轨迹、输入读回与预期文字验证分离、可取消 | `src/interaction.rs:197-260`；`docs/interaction/desktop-tools.md:5-60` |
| 安全 | 观察绑定 PID/窗口/几何/AX 身份，OCR 目标要求同文同位置唯一并做局部像素校验；共用 `desktop.lock`；观察一次性消费 | `docs/interaction/desktop-tools.md:47-60` |
| 实机状态 | **blocked**：专用机 click 返回 `window_occluded`（系统 Dock 窗口覆盖全屏），真实点击/输入/读回未验收；部署机 MCP 工具列表里甚至还没有 `desktop_*` | `docs/interaction/dedicated-acceptance-2026-11-09.md:4,13-21` |
| 成熟度 | **未完成/未验收**（单测+Swift 合成测试 + 编译通过） | `docs/interaction/desktop-tools.md:68-89` |

---

## 12. 其它能力

| 能力 | 说明 | 证据 | 成熟度 |
|---|---|---|---|
| 公众号文章 | 只读网页抓取（curl），标题/公众号/时间/摘要/封面/Markdown，图片按文件头识别落盘 | `src/article.rs`；`README.md:152-158` | 仅自动化测试 |
| 操作浮层 | `wechat_overlay`：定位角标、青色指针、橙色点击光圈、截图前隐藏 | `docs/rpa-runtime.md:125-140` | **已实机（单屏）**：`research/overlay-acceptance-2026-11-04.md` |
| 事件收件箱 | 持久事件、租约、续租、ack/release，`wait_for_events` 不占桌面锁 | `src/listeners.rs`；`src/service_tools.rs` | 仅自动化测试（曾有 Python 桥接单事件实机闭环，桥接已删除） |
| frame dump + replay | 帧级 OCR JSONL 落盘（50MB/日、0600）+ `wechat-mcp replay` 离线回放回归 | `src/core/worker_client.rs:65-129,510-549`；`src/core/replay.rs:30,219-253` | 工具能力，仅自动化测试 |
| 桌面诊断 | `--desktop-diagnostics` 只读输出权限/AX/窗口状态 | `src/main.rs:61-65`；`docs/rpa-runtime.md:30-35` | 已实机 |
| 一键安装 | `install.sh`：下载通用二进制+sha256→安装→生成 gateway.json→装 2 个 Skill→自动识别并注册 8 类 Agent | `README.md:49-89` | 脚本存在；私有仓库需 GitHub Token；安装脚本隔离测试曾失败（`docs/rpa-runtime.md:122`） |
| 多平台/多账号 | 微信桌面仅单 GUI 会话单账号；第二桌面账号启动即失败；WhatsApp/TikTok/Windows 未实现 | `README.md:40-46`；`src/accounts.rs` | **未实现** |

---

## 13. 成熟度分级汇总

> 分级规则：**已实机验收**=有 `research/*acceptance*` 或等价实机记录；**仅自动化测试**=有 `tests/`/单测覆盖但无实机；**未完成或未验收**=能力存在但关键链路断/阻塞/明确未验收。

### 13.1 已实机验收

| 能力 | 证据 | 边界 |
|---|---|---|
| MCP 桌面微信私聊读取 + 核验式发送 + 幂等状态 | `research/dedicated-host-2026-11-04.md:41-62` | 单账号、单联系人、legacy 驱动、单样本耗时 |
| 白名单 Bot 真实模型自动回复（**Python 版**） | `research/bot-acceptance-2026-11-04.md:9-28,35-47` | 代码已删除；Python 版 |
| 亮/暗主题 + 880×640 + 窗口最小化恢复（Bot 视觉链路） | `research/ui-auto-adapt-acceptance-2026-11-05.md:10-18` | 图片/文件连续性不稳定；Retina 未跑完整收发 |
| 操作浮层（单屏） | `research/overlay-acceptance-2026-11-04.md:9-40` | 双屏/缩放未验收 |
| 专用机 GUI 宿主 + TCC 权限恢复 + socket 托管 | `research/dedicated-host-2026-11-04.md:17-45` | 旧宿主 `WeChat RPA Test` |

### 13.2 仅自动化测试

Rust Bot core（轮次/限流/隔离/历史写库）、CRM/收件箱/监听、通讯录同步、历史库与 HTTP、备份恢复、Open API（80 例 + 局部联调）、外部大脑 MCP 3 工具、`upload_file`、`fetch_article`、语音转写、frame dump/replay、6 页宿主 App（`swift_host` 74 例等）。

### 13.3 未完成 / 未验收

| 项 | 原因 | 证据 |
|---|---|---|
| 历史库 MCP 工具 | 列而不可调 | §3.1 实测 |
| 通用眼手 `desktop_*` | 实机被遮挡判定阻塞、部署机未装 | `docs/interaction/dedicated-acceptance-2026-11-09.md` |
| 好友申请生产使用 | 验证对话框无真机证据，默认关闭 | `docs/features/friend-requests.md:25` |
| 群聊自动回复 | 文档口径不支持、无群成员/方向识别验收 | `docs/allowlist-bot.md:133`；`docs/rpa-runtime.md:122` |
| 多账号 / 多平台 | 未实现 | `README.md:40-46` |
| 声音输入/语音回复 | 只有入站转写，回复仍为文本 | `docs/allowlist-bot.md:89`；`src/core/worker_client.rs:967-1031` |
| 官方 API/webhook/送达回执/完整历史 | 未接入，能力声明均为 false | `README.md:170-173,198` |

---

## 14. 交付硬前提（必须写进售前物料）

| 前提 | 具体要求 | 证据 |
|---|---|---|
| 操作系统 | macOS 12+（宿主/构建 deployment target 12.0）；原生采集需 14+；macOS 26+ Vision 冷启动约 30s 并可回退 Tesseract | `native/bootstrap/README.md:29`；`.github/workflows/ci.yml:18`；`docs/allowlist-bot.md:60`；`README.md:179-185` |
| 硬件 | **一台只挂微信的专用 Mac**；单 GUI 登录会话；SSH 进程拿不到 TCC，必须由 GUI 宿主托管 | `docs/allowlist-bot.md:64`；`native/bootstrap/README.md:23` |
| 权限 | 辅助功能 + 屏幕录制（+ AppleScript 自动化），需对**实际运行的那个 App bundle**授权；宿主更新后可能需重新授权 | `README.md:177`；`research/dedicated-host-2026-11-04.md:33-39` |
| 微信 | **4.1.15** 已标定；亮/暗主题；窗口 **880×640 逻辑点**；1×/2×；未知主题/尺寸/缩放拒绝自动发送 | `docs/allowlist-bot.md:60`；`README.md:203` |
| 账号 | **单账号**；`identity_verified:false`；手动切换微信登录后需人工重核绑定 | `README.md:44` |
| 进程模型 | 一个宿主只接纳一个 MCP 连接；同一 DB/账号只允许一个监听负责人；桌面锁串行（进程内 + 跨进程 flock） | `native/bootstrap/README.md:57`；`README.md:34`；`src/resource_lock.rs:52` |
| 数据 | 本地 SQLite（**未加密**）；state 目录 0700、socket 0600；客户数据不出机（Open API 仅回环/内网） | `docs/service-workflow.md:83`；`src/core/external_brain.rs:866-883` |
| 网络 | Open API 默认关闭，开启后仅回环/内网，Bearer 令牌；外部大脑需可达该地址 | `docs/open-api/v1.md:9-16` |
| 构建 | Rust 1.99.0（锁定）+ Swift 5.7+（macOS 12 目标） | `rust-toolchain.toml`；`native/bootstrap/README.md:27-40` |
| 分发 | 仓库私有，一键安装需 GitHub Token；release 无签名/公证 | `README.md:50-58`；`.github/workflows/release.yml:44-67`；`scripts/install.sh:1001` |

---

## 15. 已确认缺口（如实列出）

| 缺口 | 现状 | 商业影响 |
|---|---|---|
| 单账号 | 一个微信 GUI 会话 = 一个账号；第二个桌面账号启动即失败 | 不能做"多客服号矩阵/多店铺" |
| 群聊 | 文档口径"只能排除"；代码允许 group 配置但无验收、无群成员/方向识别 | 不能承诺社群运营 |
| 语音输入/语音回复 | 仅入站语音→转文字；不支持发语音/语音唤醒 | 与"AI 电话/语音客服"无关 |
| 无平台级 ID/方向/回执 | OCR 文字、`coverage:visible_only`、`incoming_verified:false`、`delivery:unknown` | 不能承诺"消息必达/完整历史/流失分析" |
| 无官方事件源 | 无 webhook/Provider，后台观察是界面启发式 | 可靠性上限受 UI 限制 |
| 无知识库/订单/退款 | 明确无业务系统；模型只能生成文本 | 需按行业二次集成 |
| 多平台 | WhatsApp/TikTok/远程 worker 未实现 | 只能卖 macOS 个人微信场景 |
| 加密/合规 | 本地 SQLite 明文；无审计日志导出、无数据留存策略 | 企业客户需额外合规方案 |
| 性能数据 | 无 P50/P95 基准，实机耗时为单样本（发送 5.98s、列表 6.47s） | 不能承诺 SLA |
| 分发 | 无签名/公证安装包；宿主 App 只本机构建 | 客户安装摩擦大、Gatekeeper 拦截 |
| 文档漂移 | README"部署三个文件"vs release 实际 9 个可执行文件（+skills/layout/示例配置）；`docs/allowlist-bot.md:89` 仍写语音未接入；`research/overlay-acceptance-2026-11-04.md:35` 仍给已删除 `.py` 入口；`docs/rpa-runtime.md:99,101,122` 的测试数字仍为 85/95/101 | 售前材料不可直接复用 |

---

## 16. 门禁与验证现状（本机实测）

命令与结果（**2026-11-10 00:00 快照**，工作区含未提交改动；之后仓库又被其他会话改动，复跑数字可能变化）：

| 门禁 | 结果 | 说明 |
|---|---|---|
| `cargo fmt --all -- --check` | **PASS**（exit 0，无输出） | 2026-11-09 审计时 51 文件 diff，已修复 |
| `cargo clippy --locked -q` | **PASS**（exit 0；0 error；大量 `style/pedantic` warning） | 工程把 style 降为 warn（`Cargo.toml [lints.clippy]`）；`--all-targets -D warnings` 仍会红 |
| `cargo test --locked --no-fail-fast`（timeout 900s） | **FAIL**：1,089 pass / 15 fail / 1 ignored；3 个目标失败（`--bin wechat-mcp`、`--test protocol_smoke`、`--test swift_host`）；耗时 1m27s | 见下失败归因 |
| `swiftc native/click/*.swift native/common/*.swift native/action/*.swift` | **PASS**（产出 614KB 二进制） | 审计 P0-1（漏编 `Input.swift`）已修：`scripts/build.sh:7`、`ci.yml:74` 已用 glob |
| `bash scripts/build.sh` / release / CI | **未运行**（耗时与远端不可控） | CI 配置已修：锁 1.99.0、去 Python、shellcheck 与 test 解耦、tag 守卫（`.github/workflows/ci.yml:44-90`、`release.yml:19-27`）；但**最近一次远端 CI 是否绿无证据** |

### 16.1 15 个失败归因

| 失败 | 数量 | 归因 | 证据 |
|---|---:|---|---|
| `core::open_api::compat_tests::ops_tools_list_unchanged` | 1 | **真实漂移**：未提交的 `src/interaction.rs` 把 `desktop_act` 描述/参数改回旧文案，与 `tests/fixtures/ops_tools_list.json` 不一致（实测仅 `desktop_act` 两个字段 diff；HEAD 版本与 fixture 一致） | 本机 diff 复现 |
| `gateway::tests::failure_injected_at_click_records_not_sent`、`legacy_schema_version_2_…`、`core::serve::tests::test_serve_core_auto_backup_on_idle` | 3 | **测试不隔离**：`ResourceLock`/备份写真实 `$HOME/Library/Application Support`，本会话沙箱报 `Operation not permitted` | `src/resource_lock.rs:52`；日志原文 |
| `protocol_smoke`（2 例） | 2 | 同上：`Isolated UI operation did not acquire the desktop lock` | 日志原文 |
| `swift_host`（9 例） | 9 | 同上：ConfigForm 写 `~/.wechat-rpa/tmp`（`FileManager.homeDirectoryForCurrentUser`）被沙箱拒绝，`host_selftest` 返回 `ok:false` | `native/bootstrap/ConfigForm.swift:340-348` |

> 即：**14/15 是审计已列的 R-5"测试不 hermetic"**（可写 HOME 的机器/CI 上大概率通过），**1/15 是工作区未提交改动未同步 golden**。这两项都不代表业务功能回归，但都意味着"当前不能在任意机器上复现全绿门禁"。

---

## 17. 售卖阻断项与工作量估算

> 基准：**一个熟悉该代码库的 Rust 工程师**（不含真机操作员、不含等待 Apple 审核/客户配合的时间）。区间含自测与回归。

### 17.1 P0（不修完不能对外卖）

| # | 阻断项 | 为什么阻断 | 证据 | 人日 |
|---|---|---|---|---:|
| P0-1 | Rust `serve-core` 自动回复端到端**实机复验**（收发/轮次/重启恢复/失败暂停） | 现有验收证据属已删除的 Python 实现 | §5、§13 | 3–5 |
| P0-2 | 7 个 `history_*` MCP 工具接线 + 契约测试 | 文档承诺的查询能力当前不可用 | §3.1 | 1–2 |
| P0-3 | 测试可移植性：state dir/锁/临时目录注入，修 14 个环境失败 | 门禁无法在客户/CI 机器复现 | §16.1 | 2–3 |
| P0-4 | 运行时边界：桌面操作/`osascript`/请求级超时 + task panic 隔离 | 一个卡死/panic 会让整台机器自动化停摆 | `src/main.rs:220-221`；`src/wechat.rs:262-282`（无超时）；审计 R-1/R-2 | 3–5 |
| P0-5 | 通用眼手实机阻塞修复（遮挡判定区分系统窗口）+ 部署更新 | `desktop_*` 是核心差异化，现 blocked 且未部署 | `docs/interaction/dedicated-acceptance-2026-11-09.md:13-21` | 3–5 |
| P0-6 | 签名/公证 + 安装校验 + 宿主 App 分发包装 | 未签名 + 摘 quarantine 在客户机上不可交付 | `release.yml`（无 codesign）；`scripts/install.sh:1001` | 3–5 |
| P0-7 | 交付硬前提自检/拒绝/引导（微信版本、窗口尺寸、单账号、权限、数据目录） | 减少"装不上/答不了"的售前售后成本 | §14 | 2–3 |
| P0-8 | 好友申请、通讯录同步、语音三项真机验收矩阵 | 功能已写但零实机证据，卖点不可承诺 | §7、§8、§5 | 5–8 |
| | **P0 合计** | | | **22–36** |

### 17.2 P1（影响规模化交付与信任）

| # | 事项 | 证据 | 人日 |
|---|---|---|---:|
| P1-1 | 错误契约：`code/retryable/side_effect/send_id` + 全量 `outputSchema` + `annotations` 补齐 | 实测 0 outputSchema | 4–6 |
| P1-2 | 结构化可观测性（tracing：tool/op/耗时/error_type，不含正文） | 审计 C-2 | 2–3 |
| P1-3 | SIGTERM/CancellationToken 贯穿到发送前门禁 | 审计 R-4；2026-11-08 评审 #3 | 2–3 |
| P1-4 | core 同步 SQLite 调用改 `spawn_blocking`/DB actor | 审计 R-3（`src/core` 当前 0 处 `spawn_blocking`） | 3–5 |
| P1-5 | Open API 正式验收（含多客户端互斥、重启不重发） | §9 | 2–3 |
| P1-6 | 备份→删源→恢复→附件抽查演练 + 保留策略 | §10 | 1–2 |
| P1-7 | 长白名单分批探测（worker `names≤50` 与 config 200 会话不匹配） | 2026-11-08 评审疑点 D | 1–2 |
| P1-8 | 远端 CI 跑通 + `cargo audit` + release 产物一致性校验 | 审计 B-1/B-4 | 1–2 |
| P1-9 | 文档一致性清理（README 产物清单、语音状态、已删除入口、测试数字） | §15 | 1–2 |
| | **P1 合计** | | **17–28** |

### 17.3 P2（打磨与架构）

| # | 事项 | 人日 |
|---|---|---:|
| P2-1 | `control.rs` 巨函数拆分（1,250 行，公开签名不变） | 5–8 |
| P2-2 | history 工具面/MCP 双写收敛、IPC 客户端统一 | 5–10 |
| P2-3 | 性能基准 P50/P95 + 亮暗/缩放/双屏/长稳实机矩阵 | 3–5 |
| P2-4 | UI/交互打磨（状态面板、引导、日志检索） | 3–5 |
| P2-5 | 多账号 worker 预研（独立设备/OS 会话） | 8–15（路线性，不计入短周期） |
| | **P2 合计（不含多账号）** | **16–28** |

### 17.4 总工作量

| 范围 | 人日 |
|---|---:|
| P0（可卖最小闭环） | **22–36** |
| P0+P1（可规模化交付） | **39–64** |
| P0+P1+P2（工程化收尾，不含多账号） | **55–92** |

> 换算：1 名熟悉代码库的 Rust 工程师约 **1.5–2.5 个月**完成 P0+P1；若同时补齐 P2 架构收敛，约 **3–4.5 个月**。真机验收需另配测试机与操作员，且每轮验收受微信登录/权限状态影响。

---

## 18. 技术差异化与可复用资产

| # | 差异化 | 说明 | 证据 | 售卖价值 |
|---|---|---|---|---|
| D-1 | **应用无关的 macOS 眼手底座** | AX 控件 + Vision OCR 双源观察；单次观察凭证；动作前重新核对 PID/窗口/几何/AX 身份；OCR 目标做局部像素校验；拟人轨迹（贝塞尔+费茨+抖动）；输入读回与预期文字验证分离；与微信流程共用桌面锁 | `native/action/`、`native/interaction/`、`src/interaction.rs:197-260`；`docs/interaction/desktop-tools.md:34-60` | 可扩到任意 macOS 客户端自动化，不止微信；**但目前实机 blocked，不能按"已可用"售卖** |
| D-2 | **双入口 Agent 协议** | stdio MCP（51 工具，2025-06-18/2024-11-05 双版本）+ 内网 HTTP MCP（11 工具）；`next_turn→reply→get_turn_status` 轮次协议带认领租约、claim_token、终态账本、同参重放幂等（text SHA-256）、`accepted≠已发送` | `src/main.rs:381-420`；`src/core/open_api/`；`docs/open-api/v1.md` | 可直接接 DeepSeek/自建/Codex 等任意大脑，是"卖服务"而非"卖脚本"的关键 |
| D-3 | **轮次编排与视觉连续性算法** | 静默窗口聚批（10s/60s）；双帧一致 + 锚点 + 重复计数；`gap/ambiguous` 只跳过不猜；人工已回复三源拦截；phash 图片去重；90pt 滚动拼接；发送后回显复核 | `src/core/policy.rs:9-55,211-350`；`docs/allowlist-bot.md:51-64,113` | 决定"能否在不稳定 UI 上少误发/少漏回"，是同类的真门槛 |
| D-4 | **本地优先数据底座** | SQLite WAL + FTS5 + 内容寻址附件（SHA-256）+ 媒体理解要点 + 别名/合并 + manifest/sha256 备份 + `restore` | `src/core/history/store.rs:271,1035-1049`；`src/core/history/query.rs:749-760`；`src/core/backup.rs:484-632` | 客户数据不出机，适合对数据位置敏感的中小商家 |
| D-5 | **可复现的测试资产** | 1,104 用例、1:1 测试比（审计口径）、FakeWorker/FakeMcpServer、`with_test_send_fault` 故障注入、frame dump + `replay` 离线回放、Swift 自测 fixture | `tests/`（18 target）；`src/core/replay.rs`；`docs/reviews/2026-11-09-deep-audit.md:40-45` | 交付风险低、可做"验收脚本随货" |
| D-6 | **部署/运维形态** | launchd 常驻 + 自愈退避（5/15/60s，1h 5 次上限）；宿主 core 监护；一键安装自动识别 8 类 Agent、装 2 个 Skill；私有 Unix socket（0700/0600，对端 UID 校验） | `native/bootstrap/Supervisor.swift`；`README.md:49-89`；`native/bootstrap/README.md:20-24` | 面向"非技术商家"的最后一公里 |
| D-7 | **本地模型/本地执行** | 默认 `codex-exec`（本机 ChatGPT 登录，无需把密钥放微信机）；OpenAI 兼容可接 DeepSeek；密钥 0600 文件/环境变量 | `src/brain/model_api.rs:393-509`；`docs/allowlist-bot.md:71-95` | 无需自建推理集群即可交付 |

---

## 19. 复现与复核入口

```bash
# 工具面（51 tools / outputSchema=0 / annotations=23）
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"a","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
 '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"history_list_contacts","arguments":{}}}' \
 | HOME="$PWD/target/fakehome" ./target/debug/wechat-mcp | jq -c 'select(.id==2).result.tools|length'

# 门禁
cargo fmt --all -- --check
cargo clippy --locked -q
cargo test --locked --no-fail-fast        # 本机 1,089 pass / 15 fail，归因见 §16.1

# 原生点击工具编译（审计 P0-1 回归）
swiftc native/click/*.swift native/common/*.swift native/action/*.swift -o /tmp/wc
```

---

## 20. 一句话给商业化决策

**技术底子是"能做产品"的：眼手、轮次协议、幂等发送、本地历史、外部大脑接口都已是工程实现而非概念；但当前能签合同的形态只能是"单机单账号私聊 AI 客服（含历史与 Open API）定制交付"，且必须先花约 3–5 周补齐 P0（Rust 化实机复验、历史工具接线、运行时边界、签名分发、三项真机验收）。多账号、群运营、语音客服、Windows/多平台均不在当前能力边界内，不应写进承诺。**
