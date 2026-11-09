# 外部大脑对外开放接口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. 本计划按 Orca 派发：每个任务在独立 worktree 中完成，完成后用 Orca 报告；**不要自行合并到 `feat/wechat-bot-history`，不要部署**。

**Goal:** 在 serve-core 中提供只含大脑能力的内网 Streamable HTTP 接口（MCP），服务端强制权限、认领、幂等与终态规则，运维入口 `worker.sock` 行为不变。

**Architecture:** 新增模块 `src/core/open_api/`：`access`（权限上下文 BrainAccess）、`history`（受限历史查询）、`ledger`（持久终态记录）、`contract`（对外 v1 数据结构）、`http`/`session`（HTTP 入口与会话）。ExternalBrain 增加"严格"方法供对外入口使用，原有 `dispatch_request` 路径保持不变。

**Tech Stack:** Rust 2024（rustc 1.88+）、tokio、rusqlite、serde_json、sha2；Swift 5.7（macOS 12）用于助手面板。**不新增 crate 依赖。**

**Spec:** `docs/superpowers/specs/2026-11-09-external-brain-open-api-design.md`（第二版，已确认）。每个任务的实现者必须先读 spec 第 2、5、6、7、8、9 节。

## Global Constraints

- 运维入口零影响：`worker.sock` 上 `tools/list` 的完整输出、以及现有 `reply` / `next_turn` / `resume_session` / `history_*` 在运维入口上的行为不变。Task 1 建立的兼容快照测试必须在每个任务结束时通过。
- 不修改现有公共函数的签名或语义；需要新行为时**新增**函数（例如 `search_messages_in`），旧函数保持不动。
- 不新增 crate 依赖；HTTP 用 `tokio::net::TcpListener` 加手写的最小 HTTP/1.1 解析。
- 对外接口默认关闭：`open_api.enabled = false`。
- 默认值：认领租期 60 秒；会话租期 90 秒；终态记录上限 1000 条 / 7 天；请求头上限 16 KiB；请求体上限 1 MiB；监听端口 8770；绑定地址 `127.0.0.1`。
- 被拒绝的读取一律返回 `not_found`，不返回 `not_allowed`。
- 私有文件（令牌、终态记录）权限 0600，所在目录 0700；写入用"写临时文件再 rename"。
- 时间：内部用 Unix 秒（u64）；对外 JSON 一律 RFC 3339 UTC（例：`1700000000` → `"2023-11-14T22:13:20Z"`）。
- Swift 目标 macOS 12：禁止使用 macOS 13/14 才有的 API；`Process` 的输出管道必须先读完再 `waitUntilExit`；不得阻塞主线程。
- 验证命令：`cargo test --locked`（全部通过）；不新增 `cargo clippy --locked --all-targets` 警告。
- 提交信息用中文，遵循仓库现有风格（`feat(open-api): …`），末尾加 `Co-Authored-By` 行（按派发说明给出的行）。

## Review Focus

1. **合并过的联系人**：允许名单里的联系人被合并到另一个联系人（`contacts.merged_into`）或反过来，用子 ID 或根 ID 查询，权限判断必须一致（按根判断）。→ Task 1 测试 `permit_follows_merge_family`。
2. **同一媒体被两个联系人的消息引用**（一个允许、一个不允许）：通过允许的那条消息可以读到；没有任何消息引用的媒体一律拒绝。→ Task 1 测试 `media_shared_between_allowed_and_forbidden`、`media_without_message_denied`。
3. **截止时间到达的同时调用 `reply`**：只能产生一条终态记录，调度器只收到一个回复。→ Task 2 测试 `reply_racing_deadline_yields_single_outcome`。
4. **慢速、超大、畸形的 HTTP 请求**：没有 Content-Length、2 MiB 请求体、请求头过大、半包后断开，都要返回 400 / 413 或直接断开，而且不能阻塞同一会话中正在长轮询的 `next_turn` 和其他请求。→ Task 4 测试 `malformed_requests_do_not_block_long_poll`。
5. **认领期间切换微信账号**：`reply` 返回 `turn_revoked`，不产生发送。→ Task 2 测试 `account_switch_revokes_claim`。

---

## 派发与依赖

| 任务 | 执行者 | 依赖 | 可并行 |
|---|---|---|---|
| Task 1 权限上下文与受限历史 | **codex** | — | 与 Task 3 并行 |
| Task 2 轮次状态机与终态记录 | **codex** | Task 1 | — |
| Task 3 对外 v1 数据契约 | **agy** | —（使用 Task 1 的接口签名，先按签名写，集成时对齐） | 与 Task 1 并行 |
| Task 4 HTTP 入口与会话 | **codex** | Task 1、2、3 | — |
| Task 5 对外文档与示例大脑 | **agy** | Task 4 | 与 Task 6 并行 |
| Task 6 助手面板与 mcp-info | **agy** | Task 4 | 与 Task 5 并行 |
| 集成、全分支评审、专用机实机验收 | 协调者（Claude） | 全部 | — |

所有任务都从 `feat/wechat-bot-history` 上包含本计划的那个提交开出分支，分支名 `work/open-api-t<N>`。后一个任务开始前，协调者先把前置任务合进集成分支，再从集成分支开出。

## 共享接口（各任务按此签名实现，不得更改）

```rust
// src/core/open_api/mod.rs
pub mod access; pub mod history; pub mod ledger; pub mod contract; pub mod http; pub mod session;

#[derive(Debug, Clone, PartialEq)]
pub enum OpenApiError {            // 工具业务错误（spec 9.2 第三层）
    TurnNotFound, TurnExpired, TurnRevoked, AlreadyAnswered { action: String },
    InvalidClaimToken, NotFound, InvalidText, InvalidArguments(String),
    RateLimited, BrainModeNotExternal, Internal(String),
}
impl OpenApiError { pub fn code(&self) -> &'static str; }   // 返回 spec 9.2 的 snake_case 代码

// src/core/open_api/access.rs（Task 1）
pub struct BrainAccess { /* 私有字段 */ }
impl BrainAccess {
    pub fn load(config: &crate::core::config::BotConfig, conn: &rusqlite::Connection) -> anyhow::Result<Self>;
    pub fn account_id(&self) -> &str;
    pub fn permit(&self, contact_id: &str) -> bool;
    pub fn allowed_ids(&self) -> &std::collections::HashSet<String>;  // 已减去忽略名单，含合并家族成员
    pub fn contact_for_turn(&self, name: &str, kind: &str) -> Option<String>; // 当前账号下唯一且被允许的 contact_id
    pub fn unresolved(&self) -> &[String];                              // 解析不唯一或找不到的名单项
}

// src/core/open_api/contract.rs（Task 3）
pub struct ClaimInfo { pub claim_token: String, pub lease_until: u64, pub deadline_at: u64 }
pub trait MediaLookup { fn media_id_for_path(&self, path: &std::path::Path) -> Option<String>; }
pub fn turn_v1(turn: &crate::brain::Turn, claim: &ClaimInfo, contact_id: &str, media: &dyn MediaLookup) -> serde_json::Value;
pub fn rfc3339(secs: u64) -> String;

// src/core/open_api/ledger.rs（Task 2）
pub struct TerminalRecord { pub turn_id: String, pub outcome: String /* accepted|expired|revoked */,
    pub action: Option<String>, pub text: Option<String>, pub text_sha256: Option<String>, pub at: u64 }
pub struct TurnLedger { /* 私有 */ }
impl TurnLedger {
    pub fn open(state_dir: &std::path::Path) -> anyhow::Result<Self>;  // 文件：<state_dir>/open-api/turns.json
    pub fn get(&self, turn_id: &str) -> Option<&TerminalRecord>;
    pub fn record(&mut self, rec: TerminalRecord) -> anyhow::Result<()>; // 原子写；裁剪到 1000 条 / 7 天
}

// ExternalBrain 新增方法（Task 2）
pub struct ClaimedTurn { pub turn: crate::brain::Turn, pub contact_id: String, pub claim: ClaimInfo }
pub struct StrictReply { pub turn_id: String, pub claim_token: String, pub action: String, pub text: Option<String> }
pub struct ReplyOutcome { pub accepted: bool, pub duplicate: bool }
impl ExternalBrain {
    pub async fn claim_next(&self, session_id: &str, wait: std::time::Duration, access: &BrainAccess) -> Result<Option<ClaimedTurn>, OpenApiError>;
    pub async fn reply_strict(&self, req: StrictReply, access: &BrainAccess) -> Result<ReplyOutcome, OpenApiError>;
    pub async fn turn_status(&self, turn_id: &str) -> Result<serde_json::Value, OpenApiError>;
    pub async fn renew_session(&self, session_id: &str);    // 续期该会话持有的所有认领
    pub async fn release_session(&self, session_id: &str);  // 该会话的认领全部回到 queued
}
```

---

### Task 1：权限上下文 BrainAccess 与受限历史查询（codex）

**Files:**
- Create: `src/core/open_api/mod.rs`（只放 `OpenApiError` 和 `pub mod`；其余子模块由各自任务创建，本任务先建空文件 `ledger.rs`、`contract.rs`、`http.rs`、`session.rs`，只写一行模块注释）
- Create: `src/core/open_api/access.rs`、`src/core/open_api/history.rs`
- Modify: `src/core/mod.rs`（加 `pub mod open_api;`）
- Modify: `src/core/history/query.rs`：**只新增**两个函数，不改现有函数
- Create: `tests/fixtures/ops_tools_list.json`（运维入口快照）
- Test: 写在 `access.rs`、`history.rs` 的 `#[cfg(test)] mod tests`，以及 `src/core/open_api/compat_tests.rs`

**Interfaces:**
- Consumes: `BotConfig { account_id, allowlist, ignorelist, state_dir }`（`src/core/config.rs:94`）、`AllowlistEntry` 的 `recipient` / `aliases` / `kind`、`crate::core::history::names::name_key`、`crate::core::wechat_account::effective_account_id(state_dir, configured)`、`HistoryQuery`（`src/core/history/query.rs:120`）。
- Produces: 共享接口中的 `BrainAccess`；`RestrictedHistory`；`HistoryQuery::search_messages_in`、`HistoryQuery::media_owners`。

已核实的事实（写进实现，不要另行假设）：
- 历史库：`<state_dir>/history/history.sqlite3`。表：`contacts(contact_id, account_id, kind, display_name, merged_into)`、`contact_names(contact_id, account_id, raw_name, name_key)`、`messages(message_id, contact_id, account_id, …, media_id)`、`media(media_id, …, path)`。**`media` 没有归属字段**，归属只能经 `messages.media_id` 反查。
- 现状允许名单只在 `list_contacts`（query.rs:330）和 `get_contact`（query.rs:416）生效；`get_history`、`get_recent_messages`、`search_messages`、`get_message`、`get_media` 都不过滤（spec F9）。
- 现状 `open_query_from_config`（mcp_tools.rs:241）按 `name_key` 全库匹配，没带 `account_id`（F7）。**不要修改它**：它是运维入口的行为。
- 合并：`contacts.merged_into` 指向根；查询使用 `_family(contact_id)` 取整个家族。

- [ ] **Step 1：建立运维兼容快照（先做，作为基线）**
  测试 `ops_tools_list_unchanged`：把 `crate::tools::list_tools_with_legacy(true)` 序列化（键排序）后，与 `tests/fixtures/ops_tools_list.json` 逐字比较。快照用**本任务开始时**的代码生成并提交。测试 `ops_history_behavior_unchanged`：用一个小型测试库，记录 `mcp_tools::call_tool` 对 7 个历史工具的输出，与内嵌期望一致（现状不过滤的行为也要锁住，证明我们没改运维行为）。
- [ ] **Step 2：写 BrainAccess 的失败测试**（在 `access.rs`）
  - `permit_requires_current_account`：账号 A、B 各有同名联系人"测试群"；当前账号为 B 时，只允许 B 的 ID。
  - `ignore_wins_over_allow`：同一联系人同时在两个名单里 → `permit == false`。
  - `ambiguous_name_is_unresolved`：当前账号下有两个不同的根都叫"小美" → 两者都不允许，`unresolved()` 包含"小美"。
  - `permit_follows_merge_family`：允许的根 R 有被合并的子 C；`permit(C)` 和 `permit(R)` 都为 true；被忽略的根的子也为 false。
  - `contact_for_turn_matches_kind`：同名私聊与群，按 `kind` 选对。
- [ ] **Step 3：实现 `BrainAccess::load`**
  算法：`account = effective_account_id(&config.state_dir, &config.account_id)`；对每个名单项的 `recipient` 和 `aliases` 求 `name_key`，在 `contact_names WHERE account_id=? AND name_key=?` 中取 contact_id，再求根（沿 `merged_into` 直到 NULL）；同一项去重后，根的个数正好为 1 才算解析成功，否则记入 `unresolved`；如果项带 `kind`，只取 `contacts.kind` 相同的。`allowed = 允许的根的全部家族成员 − 被忽略的根的全部家族成员`。
- [ ] **Step 4：运行 Step 2 的测试，确认通过**
  Run: `cargo test --locked open_api::access`　Expected: 全部 PASS
- [ ] **Step 5：写 RestrictedHistory 的失败测试**（在 `history.rs`）
  - `restricted_get_history_denies_forbidden_contact` → `not_found`
  - `restricted_get_message_denies_other_account` → `not_found`
  - `media_without_message_denied` → `not_found`
  - `media_shared_between_allowed_and_forbidden` → 可以读到
  - `media_of_ignored_contact_denied` → `not_found`
  - `search_filters_before_paging`：库中前 20 条命中都属于禁止的联系人、第 21 条属于允许的联系人，`limit=5` 时第一页就返回允许的那条，`next_cursor` 正确
  - `resume_contact_must_be_permitted`：`RestrictedHistory::check_contact("禁止的ID")` 返回 `Err(QueryError)` 且 `code == "not_found"`（给 Task 4 的 `resume_session` 使用）
  - `restricted_tools_list_has_seven_history_tools`
- [ ] **Step 6：实现**
  - `HistoryQuery::search_messages_in(&self, q: &str, contacts: &HashSet<String>, since: Option<i64>, until: Option<i64>, cursor: Option<&str>, limit: Option<i64>) -> Result<Value, QueryError>`：和 `search_messages` 相同，但把 `m.contact_id IN (…)` 写进 SQL 的 WHERE。集合为空时直接返回空页。
  - `HistoryQuery::media_owners(&self, media_id: &str) -> Result<Vec<(String, String, String)>, QueryError>`：返回 `(message_id, contact_id, account_id)`，来自 `SELECT … FROM messages WHERE media_id=?`。
  - `pub struct RestrictedHistory { query: HistoryQuery, access: BrainAccess }`，方法 `new(query, access)`、`call_tool(&self, name: &str, args: &Value) -> Result<Value, QueryError>`、`check_contact(&self, contact_id: &str) -> Result<(), QueryError>`、`tools() -> Vec<Value>`（和 `mcp_tools::tools()` 中 7 个历史工具的 schema 相同）。规则按 spec 第 5 节的表：联系人类工具用 `query.with_allowlist(access.allowed_ids().clone())`；带 `contact_id` 的工具先 `permit`；`get_message` 读出后检查 `contact_id` 和 `account_id`；`get_media` 先 `media_owners`，至少一条通过才调用 `get_media`；搜索不带 `contact_id` 时用 `search_messages_in(allowed_ids)`，带 `contact_id` 时先 `permit` 再用单元素集合。所有拒绝都返回 `QueryError::not_found(...)`。
- [ ] **Step 7：运行全部测试**
  Run: `cargo test --locked`　Expected: 全部 PASS，包括 `ops_tools_list_unchanged`、`ops_history_behavior_unchanged`
- [ ] **Step 8：提交**
  `git add src/core/open_api src/core/mod.rs src/core/history/query.rs tests/fixtures/ops_tools_list.json`
  `git commit -m "feat(open-api): 大脑权限上下文（当前账号、允许/忽略名单按联系人 ID、合并家族）与受限历史查询（媒体经由消息授权、搜索先过滤后分页）；运维入口兼容快照"`

---

### Task 2：轮次状态机、认领租期与持久终态记录（codex）

**Files:**
- Create: `src/core/open_api/ledger.rs`
- Modify: `src/core/external_brain.rs`：新增共享接口中的 5 个方法；`PendingTurn` 增加字段 `claim_session: Option<String>`、`lease_until: u64`；`ExternalBrain` 增加 `ledger: Mutex<TurnLedger>`
- Modify: `handle_turn`（external_brain.rs:114）：入队前查终态记录；超时分支写 `expired`
- Test: 两个文件内的 `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 1 的 `BrainAccess`（`permit`、`account_id`、`contact_for_turn`）、`RestrictedHistory::check_contact`；Task 3 的 `ClaimInfo`（只用结构体，Task 3 未合入时先在 `contract.rs` 按共享签名放这一个结构体）。
- Produces: 共享接口中的 `TurnLedger`、`ClaimedTurn`、`StrictReply`、`ReplyOutcome` 以及 5 个 ExternalBrain 方法。

已核实的事实：
- 现状 `handle_reply`（external_brain.rs:322）接受后立即 `pending.remove`，`answered` 检查走不到；入口不查 `deadline_at`；token 只有双方都有时才比较（F2、F3）。**不要改 `handle_reply` 和 `handle_next_turn`**：它们服务 `core.sock`，即运维入口。
- `BrainReply { reply: String, metadata: HashMap<String, Value> }`，动作放在 `metadata["action"]`（`send` / `skip` / `handoff`）。
- `turn_id` 等于调度器的批次键；发送日志位于调度器会话目录下，文件名 `send-{turn_id}.json`，`status` 取值：`attempting`、`preparing_media`、`generating`、`generated`、`triggered`、`not_sent`、`skipped_by_rule`、`skipped_by_brain`、`stopped_before_send`（turns.rs:1094-1466）。会话目录在 `<state_dir>/sessions/*/`，实现时遍历这些子目录查找该文件。
- 现有启动逻辑会把 `preparing_media/generating/generated` 改成 `not_sent`（turns.rs:284），本任务不改它。

- [ ] **Step 1：写 TurnLedger 的失败测试**：`ledger_roundtrip_atomic`（写入后重新 open 能读到；目录权限 0700、文件权限 0600）、`ledger_prunes_by_count_and_age`（插入 1001 条只保留最新 1000 条；超过 7 天的被删除）、`ledger_corrupt_file_starts_empty_and_preserves_backup`（文件损坏时改名为 `turns.json.corrupt-<ts>`，从空开始，并记录 warn 日志）。
- [ ] **Step 2：实现 `TurnLedger`**：文件内容 `{"version":1,"terminal":[TerminalRecord…]}`；`text_sha256` 为回复文本 UTF-8 的 SHA-256 十六进制。
- [ ] **Step 3：写状态机的失败测试**（用测试用的 `BrainAccess` 构造器 `BrainAccess::for_test(account, allowed: &[&str])`，由本任务在 `access.rs` 中以 `#[cfg(test)] pub fn` 新增）：
  - `claim_requires_external_mode`：`builtin` 模式 → `BrainModeNotExternal`
  - `reply_requires_claim`：未认领的轮次 → `InvalidClaimToken`
  - `reply_wrong_token`、`reply_after_deadline` → `InvalidClaimToken`、`TurnExpired`
  - `account_switch_revokes_claim`：认领后 `access.account_id` 变了 → `TurnRevoked`；调度器收到 `skip`（`metadata.reason = "revoked"`）；终态记录为 `revoked`
  - `permit_lost_revokes_claim`：认领后联系人不再允许 → `TurnRevoked`
  - `duplicate_reply_same_content`：同一回复两次 → 第二次 `ReplyOutcome { accepted: true, duplicate: true }`；调度器只收到一次
  - `conflicting_second_reply`：动作或文本不同 → `AlreadyAnswered { action: "send" }`
  - `invalid_text_keeps_claim`：文本不合法 → `InvalidText`，之后用正确文本仍可回复
  - `claim_lease_expiry_requeues`：60 秒未续期 → 回到 `queued`，旧 token → `InvalidClaimToken`（测试中把租期参数调成 1 秒，租期必须是 `ExternalBrain` 上可在测试中设置的字段）
  - `release_session_requeues_claims`
  - `reply_racing_deadline_yields_single_outcome`：在截止时刻并发调用 `reply` 与超时分支 100 次，每次只有一条终态记录，调度器只收到一个结果
  - `restart_replays_accepted_reply`：接受回复后重建 `ExternalBrain`（同一 state_dir），调度器再次交来同一 `turn_id` → `handle_turn` 立即返回记录中的动作和文本，不入队
  - `restart_with_expired_record`、`restart_with_unclaimed_turn_requeues`
  - `turn_status_reports_send_status`：分别准备 `triggered`、`not_sent`、`generating`、无日志四种 `send-{id}.json`，返回 `send.status` 依次为 `triggered`、`not_sent`、`pending`、`unknown`
- [ ] **Step 4：实现**
  - `claim_next`：模式不是 `external` → `BrainModeNotExternal`。先把租期过期的认领放回 `queued`；按 `order` 找第一个 `queued`、未过 deadline、`access.contact_for_turn(name, kind)` 能解析且 `permit` 的轮次；生成 32 字节随机 hex token（用 `uuid::Uuid::new_v4()` 两次拼接即可）；`lease_until = now + 60`；没有就按 `notify_new_turn` 等到 `wait`（上限 60 秒）。解析不到联系人的轮次不交出去，保留到超时，并记一次 warn 日志（同一轮只记一次）。
  - `reply_strict`：在同一把 `state` 锁里按 spec 6.2 的顺序检查；先查终态记录处理重复和冲突（spec 6.3）；通过后写 `accepted` 终态记录，再 `remove` 并通过 `reply_sender` 发给调度器。终态记录写失败 → `Internal`，并且**不**发给调度器。
  - `handle_turn`：入队前 `ledger.get(turn_id)`：`accepted` → 直接返回 `BrainReply`（文本和动作取自记录）；`expired` / `revoked` → 返回 `skip`；超时分支写 `expired` 记录（与 `reply_strict` 争用时，以先拿到锁、先写记录的为准）。
  - `turn_status`：活动表中 → `{"state":"queued"|"claimed"}`；终态 → `{"state": outcome, "action": …, "send": {"status": …}}`；都没有 → `TurnNotFound`。
- [ ] **Step 5：运行测试**　Run: `cargo test --locked`　Expected: 全部 PASS（含 Task 1 的兼容快照）
- [ ] **Step 6：提交**
  `git commit -m "feat(open-api): 大脑轮次严格认领与回复（token 必填、截止时间、换号/移出名单即作废）、持久终态记录与重复回复幂等、重启后同一轮次交还原回复、轮次状态查询"`

---

### Task 3：对外 v1 数据契约（agy）

**Files:**
- Create/Modify: `src/core/open_api/contract.rs`
- Create: `src/core/open_api/snapshots/turn_v1_text.json`、`turn_v1_images.json`、`turn_v1_group_context.json`
- Test: `contract.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::brain::Turn`（`src/brain/mod.rs:52`；`turns.rs` 里的 `BrainTurn` 只是私有别名，外部模块必须用 `crate::brain::Turn`：`turn_id`、`account_id`、`contact: ContactInfo{name, kind}`、`text`、`chat_kind`、`context: Vec<ContextItem{role, text, media: Option<ContextMedia{kind, filename, understanding, path}>}>`、`images: Vec<PathBuf>`、`files: Vec<Value>`、`failed: Vec<FailedAttachment>`）。
- Produces: 共享接口中的 `ClaimInfo`、`MediaLookup`、`turn_v1`、`rfc3339`；以及 `pub struct HistoryMediaLookup<'a> { pub conn: &'a rusqlite::Connection, pub media_root: PathBuf }`，实现 `MediaLookup`。

已核实的事实：
- 历史媒体文件位于 `<state_dir>/history/media/<media.path>`（`media.path` 是相对路径，见 query.rs:757）。`HistoryMediaLookup` 的算法：`canonicalize(path)`，必须以 `canonicalize(media_root)` 开头，去掉前缀得到相对路径，再执行 `SELECT media_id FROM media WHERE path=?`。不在 media_root 下的路径 → `None`。
- `files` 中每项是 JSON 对象，至少含 `filename`；有时带 `path`、`media_id`（turns.rs:103、1377）。有 `media_id` 就直接用；否则按 `path` 查找。

- [ ] **Step 1：写失败测试**
  - `rfc3339_known_values`：`rfc3339(0) == "1970-01-01T00:00:00Z"`、`rfc3339(1700000000) == "2023-11-14T22:13:20Z"`、`rfc3339(951782400) == "2000-02-29T00:00:00Z"`（闰日）
  - `turn_v1_snapshot_text`、`turn_v1_snapshot_images`、`turn_v1_snapshot_group_context`：输出与 snapshots 目录中的 JSON 完全一致（键排序后比较）
  - `turn_v1_never_contains_local_paths`：输出序列化后不包含 `"/Users/"`、`state_dir` 的字符串，也不包含任何以 `/` 开头的字符串值
  - `unresolvable_image_goes_to_unavailable`：图片路径不在 media_root 下 → 不出现在 `messages` 中，出现在 `unavailable: [{"kind":"image","reason":"media_not_in_history"}]`
  - `context_roles_mapped`：`user` → `customer`，`assistant` → `assistant`，其他 → 丢弃该项
- [ ] **Step 2：实现 `turn_v1`**：输出结构严格按 spec 第 8 节，另加 `unavailable` 数组（可以为空）。`messages` 的构造：`text` 非空 → 一条 `{"kind":"text","text":…,"message_id":null,"source":"desktop_ocr"}`；每张图片 → `{"kind":"image","media_id":…,"message_id":null}`；每个文件 → `{"kind":"file","media_id":…,"filename":…,"message_id":null}`；`failed` 中的每项 → `unavailable` 中的 `{"kind":…, "reason":…}`。`observed_at` 本期不输出（内部没有可靠来源），**不得编造**。`contact` 用参数 `contact_id` 加 `turn.contact.name`、`turn.contact.kind`。`deadline_at`、`claim.lease_expires_at` 用 `rfc3339`。`rfc3339` 用整数日历算法实现（不引入 chrono）。
- [ ] **Step 3：运行测试**　Run: `cargo test --locked open_api::contract`　Expected: PASS
- [ ] **Step 4：提交**　`git commit -m "feat(open-api): 对外轮次数据 v1（稳定联系人 ID、只给媒体 ID 不给本机路径、取不到的附件进 unavailable、RFC 3339 时间）与快照测试"`

---

### Task 4：Streamable HTTP 入口与会话（codex）

**Files:**
- Create/Modify: `src/core/open_api/http.rs`、`src/core/open_api/session.rs`
- Modify: `src/core/config.rs`：`BotConfig` 新增 `pub open_api: OpenApiConfig`（缺省时取默认值；未知字段报错，与现有配置校验风格一致）
- Modify: `src/core/serve.rs`：在 `run_core_for_account` 中、`start_socket_server` 之后，`open_api.enabled` 为真时启动 HTTP 服务，并在核心退出时一同停止
- Modify: `src/core/history/serve.rs:47`：把 `constant_time_eq` 改为 `pub(crate)` 以便复用（只改可见性）
- Test: `http.rs` 内 `#[cfg(test)]`，使用真实 TCP（`127.0.0.1:0`）

**Interfaces:**
- Consumes: Task 1 `BrainAccess::load`、`RestrictedHistory`；Task 2 的 5 个 ExternalBrain 方法；Task 3 `turn_v1`、`HistoryMediaLookup`；`crate::mcp::make_tool_result(text, is_error)`（src/mcp.rs:56）。
- Produces: `pub struct OpenApiConfig { pub enabled: bool, pub bind: String, pub port: u16 }`（默认 `false`、`"127.0.0.1"`、`8770`）；`pub struct OpenApiServer`，含 `start(cfg, brain: Arc<ExternalBrain>, config: Arc<BotConfig>) -> anyhow::Result<Self>`、`local_addr()`、`shutdown()`；`pub fn token_path(state_dir) -> PathBuf`（`<state_dir>/open-api/token`）和 `pub fn ensure_token(state_dir) -> io::Result<String>`（不存在就生成 64 位 hex）；`pub fn rotate_token(state_dir) -> io::Result<String>`；`pub async fn self_check(cfg: &OpenApiConfig, state_dir: &Path) -> serde_json::Value`，返回 `{"ok":bool,"busy":bool,"tools":[名字…],"error":字符串或null}`，给 Task 6 使用；`pub fn open_tool_names() -> Vec<&'static str>`。

协议规则（按 MCP 2025-06-18 Streamable HTTP，只实现 JSON 响应，不实现 SSE）：
- 路由：`POST /mcp`、`DELETE /mcp`；`GET /mcp` → `405`；其他路径 → `404`。每个连接只处理一个请求，响应带 `Connection: close`。
- 解析：必须有 `Content-Length`，否则 `411`；请求头超过 16 KiB → `431`；请求体超过 1 MiB → `413`；10 秒内没读完请求 → 断开。每个连接一个 tokio 任务，任何一个慢连接都不能阻塞其他连接。
- 认证：`Authorization: Bearer <token>`，常数时间比较，失败 → `401`。存在 `Origin` 时只接受 `http://127.0.0.1:<port>`、`http://localhost:<port>`、`http://<bind>:<port>`，否则 `403`。每次请求都从文件读取令牌（便于令牌轮换立即生效）；令牌变化时终止当前会话并调用 `release_session`。
- 会话：`initialize` 成功后在响应头返回 `Mcp-Session-Id`（uuid）。已有活动会话 → `409`。之后的请求必须带这个头，否则 → `400`；会话不存在或已过期 → `404`。任何请求都会续期（90 秒），并调用 `renew_session`。`DELETE /mcp` → 结束会话、`release_session`，返回 `204`。后台任务每 5 秒检查一次过期会话。
- JSON-RPC：通知（没有 `id`）→ `202`、空正文；`initialize` 的协议版本协商与 `src/main.rs` 中的 `negotiate_protocol_version` 一致；`tools/list` 只返回白名单：`next_turn`、`reply`、`get_turn_status`、`resume_session` 加 7 个历史工具；未知方法或工具 → `-32601`；参数不合法 → `-32602`。
- 工具结果：成功 → `make_tool_result(json文本, false)` 加 `structuredContent`；`OpenApiError` → `isError: true`、`structuredContent: {"error":{"code":…, "message":…}}`。
- 每个请求开始时 `BrainAccess::load`（只读打开 `<state_dir>/history/history.sqlite3`）。`next_turn` 返回 `turn_v1(...)`。`resume_session(contact_id)` 先用 `RestrictedHistory::check_contact`，再按联系人名字调用现有的恢复逻辑（`handle_resume_session` 只认名字，从 `contacts.display_name` 取名字）。
- 监听：`bind` 解析为 IP；是未指定地址（`0.0.0.0`、`::`）就拒绝启动，并报 `open_api.bind must be a specific address`。

- [ ] **Step 1：写失败测试**
  `rejects_missing_or_wrong_token`（401，ExternalBrain 中没有任何认领）、`rejects_cross_origin`（403）、`second_initialize_conflicts`（409）、`session_expires_and_frees_slot`（租期调成 1 秒）、`delete_ends_session_and_requeues`、`tools_list_is_whitelist_only`（11 个，且不含 `send_message`、`desktop_observe`）、`unknown_tool_is_method_not_found`（-32601，网关没被调用）、`notification_returns_202`、`next_turn_returns_v1_without_paths`、`long_poll_and_reply_in_parallel`（同一会话：一个 `next_turn(wait=30)` 挂起时，另一个请求 1 秒内完成）、`malformed_requests_do_not_block_long_poll`（无 Content-Length → 411；2 MiB → 413；请求头 20 KiB → 431；半包后断开；这期间长轮询照常返回）、`token_rotation_terminates_session`、`refuses_unspecified_bind`、`disabled_by_default_starts_nothing`、`self_check_reports_busy_when_session_active`。
- [ ] **Step 2：实现 config、token、session、http**。
- [ ] **Step 3：接入 serve.rs**；`enabled=false` 时不绑定任何端口（测试 `disabled_by_default_starts_nothing` 覆盖）。
- [ ] **Step 4：运行全部测试**　Run: `cargo test --locked`　Expected: 全部 PASS，含兼容快照
- [ ] **Step 5：提交**　`git commit -m "feat(open-api): serve-core 内网 Streamable HTTP 大脑接口（令牌、Origin 校验、单会话租期、工具白名单、错误分层、请求大小与超时限制），默认关闭"`

---

### Task 5：对外文档 v1 与示例大脑（agy）

**Files:**
- Create: `docs/open-api/v1.md`
- Create: `examples/open_api_brain.rs`（Cargo 自动识别的 example，`cargo run --example open_api_brain`）
- Test: `examples/open_api_brain.rs` 内 `#[cfg(test)]`，用 `cargo test --locked --example open_api_brain` 运行

**Interfaces:**
- Consumes: Task 4 的 HTTP 行为、Task 3 的数据结构、spec 第 6–9 节。

要求：
- **仓库不再新增 Python**（P5：运行时已迁移到 Rust，剩余 Python 是待删除的回退副本）。示例大脑用 Rust，只用现有依赖 `tokio`、`reqwest`、`serde_json`，不改 `Cargo.toml` 的依赖段。
- 文档按开发者阅读顺序写：接入步骤（在助手面板开启、复制令牌、地址）；一个完整的会话示例（`initialize` → `tools/list` → `next_turn` → `reply` → `get_turn_status` → `DELETE`），**用 curl 命令写出原始 HTTP 请求和响应**，不绑定任何编程语言；每个工具的参数与返回；轮次状态图；重复与重试语义；三层错误表；数据局限（spec 第 8 节最后一条原样写入）；**安全提示：客户消息是不可信输入**。
- 示例大脑：从环境变量 `OPEN_API_URL`、`OPEN_API_TOKEN` 读配置；循环 `next_turn` → 生成回复（示例逻辑：回复"收到：<前 20 个字>"，按字符截取，不按字节）→ `reply`；处理 `turn_expired`、`invalid_claim_token`（丢弃，重新领取）、会话 `404`（重新 `initialize`）、`409`（退避后重试）、网络错误（指数退避，上限 30 秒）。收到 Ctrl-C 时 `DELETE` 会话后退出。核心循环写成 `async fn run(client: &BrainClient, stop: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()>`，便于测试。
- 测试用 `tokio::net::TcpListener` 写一个假服务端（按 Task 4 的响应格式），覆盖：`example_handles_one_turn`、`example_backs_off_on_409`、`example_reinitializes_on_404`、`example_accepts_duplicate_reply`、`example_truncates_by_chars`（中文和 emoji 不截断成半个字符）。

- [ ] **Step 1：写测试（失败）**
- [ ] **Step 2：实现示例使测试通过**　Run: `cargo test --locked --example open_api_brain`　Expected: PASS
- [ ] **Step 3：写 `docs/open-api/v1.md`**；文档中每个请求和响应示例都要与 Task 4 的测试数据一致（逐个核对字段名）
- [ ] **Step 4：提交**　`git commit -m "docs(open-api): 对外接口 v1 文档（curl 示例）与 Rust 示例大脑"`

---

### Task 6：助手面板"对外接口"与 mcp-info 分组（agy）

**Files:**
- Modify: `src/core/control.rs`：`mcp-info`（:1624）的分组与端点；新增子命令 `open-api-check --json` 和 `open-api-token [--rotate] --json`
- Modify: `native/bootstrap/McpPage.swift`
- Test: `src/core/control.rs` 现有测试 `test_control_mcp_info_lists_tool_groups`（:3933）扩展；Swift 侧编译通过

**Interfaces:**
- Consumes: Task 4 的 `open_tool_names()`、`self_check`、`ensure_token`、`rotate_token`、`OpenApiConfig`。
- Produces（JSON 契约，固定）：
  - `mcp-info` 的 `tool_groups`：`[{"title":"对外接口","tools":[…open_tool_names]},{"title":"运维","tools":[…其余]},{"title":"通用桌面","tools":["desktop_apps","desktop_windows","desktop_observe","desktop_act","desktop_stop"]}]`。运维组不再包含通用桌面工具。
  - `mcp-info` 的 `endpoints` 新增 `{"id":"open_api","label":"对外接口","value":"http://<bind>:<port>/mcp","ok":<self_check.ok>,"detail":"已关闭"|"已有大脑连接中"|"可连接"|<错误>}`。
  - `open-api-check --json` → `self_check` 的输出原样打印；退出码 0。
  - `open-api-token --json` → `{"token":"…"}`；加 `--rotate` 时轮换后打印新令牌。

要求：
- MCP 页"连接信息"中显示对外接口行；新增"对外接口"区：开关状态（只读，提示到设置页修改 `open_api.enabled`）、地址、令牌（"复制"、"重新生成"按钮，重新生成前弹确认，说明会断开当前大脑）、"测试连接"按钮调用 `open-api-check`。`busy=true` 时显示"已有大脑连接中"（橙色），不当作失败。
- "复制配置"按钮改为两份：
  1. 运维 SSH：`ssh <NSUserName()>@<本机第一个非回环 IPv4> nc -U <NSHomeDirectory()>/.wechat-rpa/worker.sock`；IPv4 用 `getifaddrs` 获取，取不到时用 `<本机地址>` 占位并提示；
  2. 对外接口：`{"url":"http://<地址>:<端口>/mcp","headers":{"Authorization":"Bearer <令牌>"}}`。
  删除现在的 `"command": "~/.wechat-rpa/bin/wechat-mcp"` 模板。
- 所有命令调用都放在后台队列，通过现有的 `runControlCommand(executable:arguments:timeout:)`（McpPage.swift:411 的用法）；**先读完管道再等待进程结束**；不使用 macOS 13 以上的 API。
- 离线时的默认分组（McpPage.swift:512）改成与 `mcp-info` 相同的三组标题。

- [ ] **Step 1：扩展 `test_control_mcp_info_lists_tool_groups`**：断言三组标题、通用桌面五个工具只出现在"通用桌面"组、对外接口组正好是 `open_tool_names()`。新增 `test_open_api_token_rotate_changes_token`、`test_open_api_check_disabled_reports_closed`。
- [ ] **Step 2：实现 control.rs 部分，运行 `cargo test --locked`，通过。**
- [ ] **Step 3：实现 McpPage.swift**；编译：`swiftc -O -target x86_64-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift -o /tmp/wechat-cs-host-check`，必须成功且没有新增警告。
- [ ] **Step 4：提交**　`git commit -m "feat(host,open-api): MCP 页对外接口区（地址、令牌复制/轮换、真实握手测试、占用时提示）；工具分为对外接口/运维/通用桌面；SSH 配置按本机实际用户与地址生成"`

---

## 集成与验收（协调者）

1. 按依赖顺序合并到集成分支，每次合并后运行 `cargo test --locked`。
2. 全分支评审，重点看 Review Focus 的五条，以及 spec 的反目标。
3. 专用机实机验收（spec 第 13 节）：`brain.mode=external`、`open_api.enabled=true`、`bind` 填本机内网地址；在开发机上运行 `examples/open_api_brain/brain.py`，用测试群发一条消息，跑通 `next_turn` → `reply(send)` → `get_turn_status` 返回 `send.status = triggered`；再验证重启 serve-core 后重复回复不重发。验收结束后恢复 `brain.mode=builtin`（除非用户另有决定）。
4. Webhook（spec 第 10 节）不在本计划内，另写计划。
