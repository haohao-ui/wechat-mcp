# I2 大脑运行接入、外部契约与对话追溯 技术审查报告

- **日期**：2026-11-10
- **任务编号**：I2（`task_bdf9c67ded02` / `ctx_66eb7c8e57b3`）
- **基线 Commit**：`345dde5f73442d5580705ffe8d63d7ca0b296b70`
- **交付分支**：`work/prod-brain-runtime`
- **状态**：阶段①、②、③ 实现与测试全部完成，本地代码规范检查通过。

---

## 1. 任务目标与范围

根据权威方案及任务书，完成大脑运行接入、外部契约与对话追溯：
1. **控制接口与快照编译（阶段①）**：
   - 实现纯快照组装与预览（`src/brain/snapshot.rs`），系统能力事实规则与客户话术拆分，预算二分截断；
   - 本地控制接口（`src/core/control.rs`）分发 `brain-*` 操作，支持权限隔离与原子写。
2. **运行接入与 Provenance（阶段②）**：
   - 数据库存储升级（migration 5 新增 `brain_turns` 表与索引，记录 snapshot/decision/model/execution）；
   - 调度接入（`src/core/turns.rs`）：在 `understand_images` 之前事务化固定快照，失败安全阻断模型与发送调用；模型决策落库，发送后更新只读投影；发送失败不重发。
3. **外部兼容与备份恢复（阶段③）**：
   - 外部契约与协议协商（`src/core/open_api/`）：实现 `brain_negotiate` 工具与会话状态升级；协商 v2 后 `next_turn` 返回带快照的 `turn_v2`；`reply` 校验 `snapshot_id`（不匹配返回 `snapshot_mismatch` 并保持 claim）；新增 `brain_get_turn_detail`；
   - 备份与恢复（`src/core/backup.rs`）：升级至 Manifest v3，覆盖角色、知识、绑定及 `brain_turns`；验证相对路径白名单、防 symlink、hash/size 校验、staging 原子切换；恢复旧 v2 备份时在已有配置时返回 `restore_incomplete` 拒绝混用。

---

## 2. 既有失败调查与字段级差异根因

在任务初始阶段，全仓回归中存在 1 项既有失败：
- 测试符号：`core::open_api::compat_tests::ops_tools_list_unchanged`
- 比对结果：全量 51 个工具中，有且仅有 1 处差异，位于索引 14 的 `desktop_act`。
- 引入 Commit：`7e41e7a4189d791db55f0f91f872e395d1a9a76f`（提交信息：`wip(interaction): 通用桌面工具进行中的修改与专用机验收记录`）。
- 具体差异：
  - `description` 文案修改；
  - `inputSchema.properties` 新增 `timeout_seconds` 字段（类型 `number`，`default: 30.0`，带有详细描述）。
  - 由于该 commit 仅修改了 `src/tools.rs` 中工具的注册定义，未同步更新 `tests/fixtures/ops_tools_list.json`，导致 snapshot 比对测试失败。
- 处理策略：
  - 严格遵守交付约束，不删除工具、不私自更新旧 fixture 掩盖回归，保留既有失败并在交付中客观汇报。

---

## 3. 实现与验证证据（RED / GREEN）

### 3.1 阶段①：控制接口与 assemble_only 快照
- **新增模块**：`src/brain/snapshot.rs`
- **控制分发**：`src/core/control.rs` 扩展 `execute_brain_control`
- **测试记录**：
  - `brain::snapshot::tests`（5 项纯逻辑测试）：
    - `snapshot_compile_preserves_contract_order`：PASS
    - `snapshot_compile_overflow_truncates_knowledge_cleanly`：PASS
    - `snapshot_preview_budget_fails_instead_of_truncating_current_message`：PASS
    - `snapshot_preview_assemble_only_rejects_forbidden_fields`：PASS
    - `snapshot_preview_execute_disabled`：PASS
  - `test_brain_control_list_and_save`（端到端控制测试）：
    - RED：未接入时控制操作报 `unknown control action`；
    - GREEN：完整覆盖角色草稿保存、知识保存、预览、跨账号隔离拒绝、版本冲突 current_generation 返回。

### 3.2 阶段②：运行接入与 Provenance
- **历史存储升级**：`src/core/history/store.rs`
  - migration 5：创建 `brain_turns` 表及 `brain_turns_contact_time` 索引；
  - 实现 `record_brain_turn_snapshot`, `update_brain_turn_decision`, `update_brain_turn_execution`, `get_brain_turn_detail`。
- **运行链路**：`src/core/turns.rs`
  - 在 `understand_images` 前固定快照；
  - 快照持久化失败安全阻断（不调模型、不发消息）；
  - 发送后更新 execution 投影，写入失败不重发。
- **测试记录**：
  - `test_brain_turns_provenance_lifecycle`（store 测试）：PASS
  - `test_brain_snapshot_provenance_and_failure_injection`（turns 失败注入与端到端测试）：
    - RED：初期因 store 权限和放行逻辑导致未命中预期分支；
    - GREEN：验证快照写入 `brain_turns`；人为 DROP 表注入写入失败，断言模型调用为 0、发送调用为 0。
  - 原有 113 项 turns 测试全部回归通过。

### 3.3 阶段③：外部契约协商与备份恢复
- **Open API 外部契约**：
  - `src/core/open_api/session.rs`：`Session` 增加 `api_version` 与 `capabilities`；
  - `src/core/open_api/contract.rs`：实现 `turn_v2`，新增 `snapshots/turn_v2.json` 快照测试；
  - `src/core/open_api/http.rs`：
    - `open_tools()` 增加 `brain_negotiate` 与 `brain_get_turn_detail`；
    - `next_turn` 在 v2 时分发 `turn_v2`（附带快照）；
    - `reply` 在 v2 时若传入 `snapshot_id` 校验匹配性，不匹配报 `snapshot_mismatch`；
    - `valid_args` 增加对 `array` 类型支持。
  - **测试**：`core::open_api::http::tests`（25 项）：全部 PASS。
- **备份恢复升级**：
  - `src/core/backup.rs`：
    - `BackupManifest` 升级支持 v3，新增 `brain`（`root_generation`, `files`）、`config_snapshot`；
    - `run_backup` 自动打包 `brain-bindings.json` 和 `brain-profiles/` 目录；
    - `run_restore` 校验相对路径、禁止 symlink、sha256/大小一致性校验；
    - 恢复旧 v2 备份若目标已有角色配置返回 `restore_incomplete` 拒绝混用。
  - **测试**：`core::backup::tests`（20 项）：全部 PASS（包含新增的 `test_backup_v3_roundtrip_with_brain`, `test_restore_v2_with_existing_brain_rejected`, `test_restore_symlink_rejected` 及原有 17 项回归）。

---

## 4. 全量回归与代码质量检查

### 4.1 格式与 Clippy
- `cargo fmt --check`：退出码 0，代码格式完全规范。
- `cargo clippy --bin wechat-mcp`：退出码 0，无任何 error 阻断。

### 4.2 全量单元测试（单线程 `--test-threads=1`）
- **执行命令**：`cargo test --bin wechat-mcp -- --test-threads=1`
- **运行结果**：`940 passed; 1 failed; 0 ignored; finished in 67.06s`
- **失败项比对**：
  - 唯一失败项为 `core::open_api::compat_tests::ops_tools_list_unchanged`；
  - 与派发基线完全一致，未引入任何新失败。

---

## 5. 主要修改与新增文件清单

- **新增文件**：
  - `src/brain/snapshot.rs`：纯快照编译与预览函数、系统规则与话术渲染、单测。
  - `src/core/open_api/snapshots/turn_v2.json`：v2 契约 golden snapshot。
  - `docs/reviews/2026-11-10-production-brain-runtime.md`：本技术审查报告。
- **修改文件**：
  - `src/brain/mod.rs`：注册 `pub mod snapshot;`。
  - `src/core/control.rs`：分发 `brain-*` 操作，集成快照与存储，权限隔离。
  - `src/core/history/store.rs`：migration 5 创建 `brain_turns` 表，实现 CRUD 与单测。
  - `src/core/turns.rs`：运行接入、理解前固定快照、生命周期投影、失败注入测试。
  - `src/core/open_api/session.rs`：增加 `api_version` 与 `capabilities`。
  - `src/core/open_api/contract.rs`：新增 `turn_v2` 函数与 golden snapshot 测试。
  - `src/core/open_api/http.rs`：`brain_negotiate` 工具与协商分发、`brain_get_turn_detail`、`reply` 校验。
  - `src/core/backup.rs`：Manifest v3 结构、角色与知识文件打包、路径与 symlink 安全校验、staging 原子切换及 v2 兼容拒绝。

---

## 6. 风险与限制

1. **执行预览首版保持禁用**：`execution: "execute"` 按照契约在当前阶段返回 `preview_execution_disabled`，避免误触外部付费模型。
2. **既有工具列表失败**：`desktop_act` 与既有 fixture 的差异仍保留在代码库中，需待工具面统一对齐或由 Q1 任务决定更新 fixture。
3. **真实真机运行**：本任务所有验证均在隔离临时目录与合成数据下完成，未执行真实微信客户端或 GUI 交互。
