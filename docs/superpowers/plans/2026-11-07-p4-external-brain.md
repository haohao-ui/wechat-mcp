# P4 外部大脑 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:**
- 打开开关后，每一轮的回复内容由外部大脑提供。外部大脑通过 MCP 工具 `next_turn` 取消息，通过 `reply` 回复。
- 安全检查一律由服务执行：允许名单、限流、冷却、发送前核对、幂等。

**Architecture:**
- `serve-core`（P2）把每一轮消息交给 `Brain`：
  - `brain.mode = builtin`：调用内置大脑，即今天的行为；
  - `brain.mode = external`：调用 `ExternalBrain`。它把这一轮放进待处理表，等外部回复或超时。
- 外部 MCP 客户端连接 `worker.sock` 后，由助手为每个连接启动一个 `wechat-mcp` 进程。这个进程里的 `next_turn`、`reply`、`resume_session` 工具，通过本机套接字 `~/.wechat-rpa/core.sock` 转发给 `serve-core`。
- 不改数据库结构。转人工的状态写在各会话目录下的 `handoff.json` 里。

**Tech Stack:** Rust（tokio、serde）、Swift（助手面板）、Python（只改一处：配置里允许出现 `brain` 这一项）。

**Spec:** `docs/superpowers/specs/2026-11-07-rust-core-pluggable-brain-design.md` 第 4 节、第 8 节（全局开关、超时默认 `skip`、v1 只做 MCP）。

## Global Constraints

- 配置项 `brain`：`{"mode": "builtin" | "external", "external": {"timeout_seconds": 1..3600（默认 120）, "timeout_action": "skip" | "builtin" | "handoff"（默认 "skip"）, "queue_minutes": 1..1440（默认 30）}}`。
  - 整项省略，等同于 `{"mode": "builtin"}`。
  - 出现其他字段一律拒绝，与 `load_config` 的严格风格一致。
- Python 机器人（P3 切换前仍在运行）必须能接受 `brain` 这一项并忽略它，否则写入配置后 Python 会因为不认识的字段而崩溃。
- `core.sock`：
  - 权限 0600，父目录 `~/.wechat-rpa` 必须是 0700，只允许本机同一个用户连接；
  - 协议是按行分隔的 JSON。请求格式为 `{"id", "op", ...}`，响应格式为 `{"id", "ok", "result" | "error"}`。
- `reply` 只能针对服务交出去、还没结束的 `turn_id`：
  - 接收人始终是这一轮的联系人，请求里任何"接收人"字段一律忽略，或者直接报错；
  - 同一个 `turn_id` 只认第一个到达的 `reply`，后面再来的返回 `already_answered`。
- 一轮消息交给大脑之后，**所有发送都走 T6 原来的路径**，复用发送前检查和幂等逻辑，不允许另开一条发送路径。
- 错误文本和日志里不出现回复正文和客户消息正文。
- 不部署，不 push。真机验收由协调者执行。

## Review Focus
1. 外部大脑永远不回复：每一轮都必须在 `timeout_seconds` 内按 `timeout_action` 结束，不能一直挂着，挡住这个会话后面的消息。
2. `serve-core` 重启后，没有回复的轮次：按超时处理（写入日志文件），不能丢失，也不能重复交给外部大脑。
3. `next_turn` 的长轮询连接断开（MCP 客户端退出）：这一轮回到待处理队列，允许另一个客户端取走，不能丢失。
4. 转人工的会话：服务继续记录消息，但不再交给任何大脑，也不自动回复，直到执行 `resume_session`。
5. 切换开关时，正在等待外部回复的轮次按原来的模式完成，下一轮才用新的模式。

---

### Task 1: 配置与 ExternalBrain（Rust core）
**Files:** Modify `src/core/config.rs`（加 `brain` 配置项）；Create `src/core/external_brain.rs`；Modify `src/core/serve.rs`（按模式选择大脑，启动 `core.sock` 服务）；Modify `src/core/mod.rs`；Modify `scripts/wechat_bot/service.py`（只在 `allowed` 里加 `'brain'`，并补一个测试）

**Interfaces（Produces）：**
- `core.sock` 操作：
  - `{"op":"next_turn","wait_seconds":0..60}`：返回 `{"turn": Turn | null}`。`Turn` 使用 `src/brain/mod.rs` 的同名结构，`turn_id` 等于这一轮的幂等键，另外附带 `deadline_at`（Unix 秒）。
  - `{"op":"reply","turn_id","action":"send"|"skip"|"handoff","text"?}`：返回 `{"accepted":true}`，或者错误 `unknown_turn`、`already_answered`、`invalid_text`。`text` 的校验规则和内置大脑的 `validate_reply` 相同。
  - `{"op":"resume_session","recipient"}`：解除转人工状态。
  - `{"op":"status"}`：返回 `{"mode", "pending", "last_poll_at"}`。
- 一轮被取走但还没回复时，如果取走它的连接断开了，就把这一轮放回队列。
- `health.json` 增加 `brain: {mode, pending, last_poll_at}`。

**Tests：**
- 配置：默认值、取值范围、拒绝未知字段；
- 用一个假的外部客户端走完 `send`、`skip`、`handoff` 三种回复；
- 超时后，按 `timeout_action` 的三种取值分别处理；
- 第一个 `reply` 生效，第二个返回 `already_answered`；
- `reply` 里带了别的接收人，这个字段会被忽略，消息仍然发给这一轮的联系人；
- 长轮询连接断开后，这一轮回到队列；
- 重启后，待处理的轮次按超时处理；
- `core.sock` 的权限；
- Python 的 `load_config` 能接受 `brain` 这一项。

**Commit：** `feat(core): 外部大脑——brain.mode 开关、core.sock 协议、超时与转人工`

### Task 2: MCP 工具（按连接启动的 wechat-mcp 进程）
**Files:** `src/tools.rs`（加入工具列表）、`src/service_tools.rs` 或新建 `src/core/brain_tools.rs`（转发到 `core.sock`）

**Interfaces（Consumes）：** Task 1 的 `core.sock` 协议（以本计划为准，两个任务并行开发）。
**Produces：** 新增三个 MCP 工具：
- `next_turn(wait_seconds)`；
- `reply(turn_id, action, text?)`；
- `resume_session(recipient)`。

它们的参数和返回值与 `core.sock` 一一对应。`core.sock` 不存在时，返回错误 `core_not_running`。原有工具的名称、参数、返回值一律不改。

**Tests：**
- 用一个假的 `core.sock` 服务，验证三个工具的转发和错误映射；
- `tools/list` 里包含这三个工具，原有工具不变；
- `core.sock` 不存在时返回 `core_not_running`。

**Commit：** `feat(mcp): next_turn / reply / resume_session 工具，转发至 serve-core`

### Task 3: 助手面板开关
**Files:** `native/bootstrap/{ConfigForm,StatusPanel,StatusModel}.swift`、`tests/test_host_*.py`、`tests/host_selftest/main.swift`

**内容：**
- 配置表单加一项"回复由：内置大脑 / 外部大脑"，保存流程和现有一样，要经过 `config-check`；
- 选择外部大脑时，可以设置超时秒数和超时后的处理方式；
- 状态面板：
  - mode 为 external，且 `last_poll_at` 超过 60 秒没有更新：显示黄色"外部大脑未连接"；
  - 有待处理的轮次时显示数量；
  - 转人工的会话列出名称，并提供"恢复自动回复"按钮。按钮执行 `wechat-mcp control resume-session <name>`，需要 T8b 的 control 子命令配合；如果这个子命令还不存在，就由本任务在 `src/core/control.rs` 里补上。

**Tests：** 表单读写 `brain` 配置项，并保留其余配置项不变；状态判定的纯函数测试。
**Commit：** `feat(host): 大脑开关与外部大脑状态显示`

### Task 4: 真机验收（协调者）
- 切换到 Rust 核心以后（P3），在测试群用一个示例外部大脑完成验收。示例外部大脑是一个最小的 MCP 客户端脚本，循环调用 `next_turn` 再 `reply`。验收项：
  - `send`；
  - `skip`；
  - `handoff`，然后 `resume_session`；
  - 超时；
  - 外部大脑指定别的接收人，被拒绝或忽略；
  - 切换开关。
