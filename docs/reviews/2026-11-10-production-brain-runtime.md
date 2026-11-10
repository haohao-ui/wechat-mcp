# I2 大脑运行接入、外部契约与对话追溯 技术审查报告

- **日期**：2026-11-10
- **任务编号**：I2（`task_9b1305bd5282` / `ctx_6fec15c5f1eb`，前序 `task_6ffc5c6c0fb1`、`task_bdf9c67ded02`）
- **基线 Commit**：`345dde5f73442d5580705ffe8d63d7ca0b296b70`
- **I1 依赖集成**：已无冲突合入 I1 完整增量 commit `0bf4940`（包含 Root schema v2、`ExecutionMode`、`Store::initialize`、原子 activate/rollback）
- **交付分支**：`work/prod-brain-runtime`
- **当前状态**：阶段①、②、③ 及审查追加要求（`execution_mode`、`brain-initialize` 接线、`role_required`、损坏根阻断、模型快照注入、v2 reply 强制必填 `snapshot_id`、备份恢复 Manifest v3）均已完成；并根据协调者最新明确指示的**收敛方案 C**闭环了 `brain-initialize` 部署语义与严格校验。并行与单线程测试均已执行并通过（955 passed, 1 既有基线 failed）。

---

## 1. 任务目标与范围

根据权威方案及协调者要求，完成大脑运行接入、外部契约与对话追溯：
1. **控制接口与快照编译（阶段① & I1 集成接线）**：
   - 合入 I1 增量 `0bf4940`，获得 Root schema v2、`ExecutionMode`（`Legacy` / `Managed`）、原子 `activate` / `rollback`；
   - 实现纯快照组装与预览（`src/brain/snapshot.rs`），系统能力事实规则与客户话术拆分，预算二分截断；
   - 本地控制接口（`src/core/control.rs`）实现 `brain-list` 输出 `execution_mode`；新增 `brain-initialize` 控制操作，支持 `fresh` 与 `legacy_upgrade`，并在维护锁保护下调用 `store.initialize`。
2. **运行接入、模型输入与 Provenance（阶段②）**：
   - 数据库存储升级（migration 5 新增 `brain_turns` 表与索引，记录 snapshot/decision/model/execution）；
   - 快照注入模型：`Turn` 新增 `snapshot: Option<Value>`，内置大脑 `build_prompt` 优先使用已编译的 `snapshot.compiled` 提示词，避免写死客服模板；
   - 调度接入（`src/core/turns.rs`）：
     - 检测到 `brain-bindings.json` 存在时读取 `execution_mode`；
     - 损坏根阻断：根文件损坏（`integrity_mismatch` / `schema_unsupported`）fail-closed 记录 journal 状态 `integrity_mismatch` 并报错，绝不降级为 Legacy；
     - Managed 模式无角色：记录 journal 状态 `role_required` 并安全退出（模型调用 0、发送 0）；
     - Legacy 模式无角色：保留旧链路正常回复；
     - 草稿隔离：仅 active 角色参与调度，草稿不生效；
     - 快照在 `understand_images` 前事务化固定并持久化到 `brain_turns`，失败安全阻断模型与发送调用；发送后更新只读投影，写入失败不重发。
3. **外部契约严格校验与备份恢复（阶段③）**：
   - 外部契约与协议协商（`src/core/open_api/`）：实现 `brain_negotiate` 工具与会话状态升级；协商 v2 后 `next_turn` 返回带快照的 `turn_v2`；
   - `reply` 严格契约：在 v2 模式下**强制必填 `snapshot_id`**（缺失返回 `invalid_arguments`），比对期望快照（不匹配返回 `snapshot_mismatch` 且保持 claim 依然有效）；
   - 备份与恢复（`src/core/backup.rs`）：升级至 Manifest v3，覆盖角色、知识、绑定及 `brain_turns`；验证相对路径白名单、防 symlink、hash/size 校验、staging 原子切换；恢复旧 v2 备份时在已有配置时返回 `restore_incomplete` 拒绝混用。

---

## 2. 既有失败调查与字段级差异根因

在全仓回归中存在 1 项基线既有失败：
- **测试符号**：`core::open_api::compat_tests::ops_tools_list_unchanged`
- **比对结果**：全量 51 个工具中，有且仅有 1 处差异，位于索引 14 的 `desktop_act`。
- **引入 Commit**：`7e41e7a4189d791db55f0f91f872e395d1a9a76f`（提交信息：`wip(interaction): 通用桌面工具进行中的修改与专用机验收记录`）。
- **具体差异**：
  - `description` 文案修改；
  - `inputSchema.properties` 新增 `timeout_seconds` 字段（类型 `number`，`default: 30.0`）。
- **处理策略**：
  - 严格遵守交付约束，不删除工具、不私自更新旧 fixture 掩盖回归，保留既有失败并在交付中客观汇报。

---

## 3. 实现与验证证据（RED / GREEN）

### 3.1 阶段①：控制接口、I1 集成与 assemble_only 快照
- **新增模块**：`src/brain/snapshot.rs`
- **控制分发与接线**：`src/core/control.rs`
  - `brain-list` 输出增加了 `execution_mode`；
  - `brain-initialize` 支持 `installation: fresh | legacy_upgrade`，调用 `store.initialize`。
- **测试记录**：
  - `test_brain_control_list_and_save`：验证角色保存、草稿、预览、版本冲突、跨账号隔离；
  - `test_brain_initialize_control`：
    - 初始未初始化时，`brain-list` 报告 `execution_mode: "legacy"`；
    - 执行 `brain-initialize` fresh 后，`brain-list` 报告 `execution_mode: "managed"`。

### 3.2 阶段②：调度执行模式、模型快照注入与 Provenance
- **模型提示词注入**：
  - `src/brain/builtin.rs`：`build_prompt` 优先使用 `turn.snapshot.compiled`；
  - 测试：`test_build_prompt_prefers_compiled_snapshot` 验证有快照时使用快照提示词，无快照时走默认客服人设。
- **执行模式接线与安全阻断**：
  - `src/core/turns.rs`：
    - `test_fresh_managed_mode_no_role_records_role_required`：Fresh Managed 模式下无角色时，安全退出，journal 记录 `role_required`，模型调用 0 次，发送调用 0 次；
    - `test_broken_root_blocks_and_records_integrity_mismatch`：根文件损坏时，fail-closed 阻断并报错，journal 记录 `integrity_mismatch`，绝不降级为 Legacy 且模型调用 0 次；
    - `test_legacy_mode_no_role_preserves_old_pipeline`：未初始化 Legacy 模式下保留原有链路正常回复；
    - `test_brain_snapshot_provenance_and_failure_injection`：验证快照落库与失败注入阻断。

### 3.3 阶段③：外部契约严格校验与备份恢复
- **Open API 外部契约严格校验**：
  - `src/core/open_api/http.rs`：
    - `test_negotiate_v2_and_capabilities`：协商 v2 与能力集合；
    - `test_v2_reply_requires_snapshot_id_and_validates_mismatch`：
      - v2 下 `reply` 缺失 `snapshot_id` 返回 `invalid_arguments`；
      - `snapshot_id` 不匹配返回 `snapshot_mismatch`，且 claim 保持有效；
      - 使用正确的 `snapshot_id` 成功返回 `accepted: true`。
- **备份与恢复升级**：
  - `src/core/backup.rs`：Manifest v3 结构，备份与恢复完整包含角色与绑定配置；
  - 测试：`test_backup_v3_roundtrip_with_brain`、`test_restore_v2_with_existing_brain_rejected`、`test_restore_symlink_rejected` 全部通过。

### 3.4 审查缺口闭环与收敛方案 C 实施详情

根据协调者审查意见及最新方案 C 指令，针对 4 项关键点进行彻底收敛与验证：

#### 1. `brain-initialize` 的 `upgrade_schema_from` 严格整型检查与防绕过
- **问题根因**：原先使用 `as_u64().map(|v| v as u32)` 会导致超出 u32 的大整数被截断绕过（如 `4294967297` 截断为 `1`），而负数/字符串/浮点数则混淆为 None。
- **修复实现**（`src/core/control.rs`）：
  - 严格检查 `upgrade_schema_from` 字段：缺失时为 `None`；存在时仅当为整数且值精确等于 1 时才为 `Some(1u32)`；其余任何非法类型或范围（`4294967297`、`-1`、`"1"`、`2`、`0`、`1.5`、`[1]` 等）一律拒绝为 `invalid_arguments`（退出码 2）。
  - **测试断言**：在 `test_brain_initialize_control` 中对非法输入断言返回 `invalid_arguments`，且精确断言磁盘上 `brain-bindings.json` 的字节保持原状、完全未变。

#### 2. v2 `reply` 快照严格校验与保留 Claim
- **问题根因**：原实现当 DB 查无记录且内存无 snapshot 时，未能严格区分未知与匹配；DB 读取失败直接降级吞掉。
- **修复实现**（`src/core/open_api/http.rs`）：
  - 严格区分缺失与失败：DB 读失败（如文件损坏、IO 异常）返回 `internal`，绝不无声降级吞掉；DB 查无记录且内存无快照时返回 `snapshot_not_found`；快照不匹配返回 `snapshot_mismatch`。
  - 在拒绝的同时，严格在内存中保留当前 turn 的 Claim 状态与 `claim_token`。
  - **测试断言**：单测 `v2_reply_rejects_missing_snapshot_and_db_failure_preserving_claim` 验证无 snapshot 拒绝（`snapshot_not_found`）和数据库损坏拒绝（`internal`），并断言内存中 pending turn 的 `claimed_by` 和 `claim_token` 保持有效不变。

#### 3. 协调者收敛方案 C：`brain-initialize` 部署语义收敛与配置保护
- **既有调用链事实核查**：
  - 既有部署中，`src/core/turns.rs:260` 明确调用 `Policy::new(account, recipient, "service", None, group)` 硬编码 `"service"`；
  - `src/core/turns.rs:1864` 调度时明确指定 `BrainTurn.mode = "service".to_string()`；
  - `src/brain/builtin.rs:80` 对应判断 `if turn.mode == "service"` 走用户部署的个人微信AI客服助手。
  - 事实证明旧部署在线上真实运行实体全量为客服助手。
- **方案 C 落地实现**：
  - **明确入口限制**：`brain-initialize` 控制入口仅用于迁移当前 `BotService` 部署。因此在执行升级时将 `is_service` 固定为 `true`，忠实保持此入口原有部署语义，绝非未知来源猜测。
  - **彻底撤销 `BotConfig.mode` 扩展**：完全恢复基线 `BotConfig` 字段白名单，严禁扩展公共配置 schema。升级过程中配置文件的原始字节完全不变。
  - **显式拒绝请求参数中的 `mode`**：若请求带有 `mode` 参数（无论传入 `"service"`、`"test"` 还是其它值），明确返回 `invalid_arguments`（退出码 2），避免调用方误以为支持该配置，绝不静默忽略，且断言磁盘根文件字节不变。
  - **底层 Store 接口保持完备**：底层 `Store::initialize_upgrade_v1` 保留 `service: bool` 参数供既有显式调用者使用，不改动底层存储契约，亦不在当前控制入口支持未支持的测试 Bot 迁移。
- **补充的 4 组精确测试断言**（`test_brain_initialize_control`）：
  - ① **旧无 mode 配置升级**：验证 legacy_upgrade 升级成功，升级后原配置文件字节完全未变，active 默认角色人设严格保持为 `用户部署的个人微信AI客服助手`；
  - ② **外部大脑模式保留**：验证 `brain.mode: "external"` 配置升级成功，配置文件字节完全未变，`active.default` 为 null，`legacy_fallback` 角色保留为 `原内置客服`；
  - ③ **请求 mode 显式拒绝**：请求传入 `mode` 参数（`service`、`test`、`other`）均被拒绝（退出码 2，`invalid_arguments`），且断言磁盘根文件完全未变；
  - ④ **Strict Reader 白名单回归**：验证原配置解析器严格拒绝根含有 `mode` 字段的配置。

#### 4. 轮次固定版本机制与模型重试安全
- **问题根因**：轮次模型调用失败重试（`failures < max_retries`）重新 cycle 时，若未保存快照则会重新执行 `store.resolve`，导致如果重试期间有管理员激活新角色 revision，轮次将混用新版本。
- **修复实现**（`src/core/turns.rs`）：
  - 在 `TurnData` 中增加 `fixed_snapshot: Option<Value>` 字段；
  - 在 `run_generation_and_send` 中，若 `pending.fixed_snapshot` 存在，直接复用该快照，跳过 `store.resolve` 与 `snapshot::compile`；
  - 模型调用失败触发重试时，将本次固定的快照存入 `retrying_turn.fixed_snapshot` 并持久化到 `turn.json`；
  - 在 `src/core/scheduler.rs` 的测试构造中对 `fixed_snapshot: None` 进行了机械适配。
  - **测试断言**：单测 `test_turn_model_retry_pins_initial_snapshot_version` 验证模型失败重试时，即使管理员已激活全新的 role-v2，重试 cycle 传入模型的快照版本与 ID 依然严格锁定为初始的 role-v1。

---

## 4. 全量回归与代码质量检查

### 4.1 格式与 Clippy（硬门禁）
- `cargo fmt --check`：**退出码 0**，完全符合 rustfmt 格式规范。
- `cargo clippy --bin wechat-mcp`：**退出码 0**，通过 Clippy 检查无告警。

### 4.2 默认并行单元测试（多线程并发）
- **执行命令**：`CARGO_TARGET_DIR=/tmp/cargo-target cargo test --bin wechat-mcp`
- **退出码**：101（仅限 1 项既有失败）
- **运行结果**：`test result: FAILED. 955 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 24.17s`
- **失败项比对**：
  - 唯一失败项为 `core::open_api::compat_tests::ops_tools_list_unchanged`（由历史 commit `7e41e7a` 桌面工具字段引入，严格保留）。

### 4.3 单线程单元测试（`--test-threads=1`）
- **执行命令**：`CARGO_TARGET_DIR=/tmp/cargo-target cargo test --bin wechat-mcp -- --test-threads=1`
- **退出码**：101（仅限 1 项既有失败）
- **运行结果**：`test result: FAILED. 955 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 65.68s`
- **失败项比对**：与默认并行测试及既有基线完全一致。

---

## 5. 主要修改与新增文件清单

- **新增文件**：
  - `src/brain/snapshot.rs`：纯快照编译与预览函数、系统规则与话术渲染、单测。
  - `src/core/open_api/snapshots/turn_v2.json`：v2 契约 golden snapshot。
  - `docs/reviews/2026-11-10-production-brain-runtime.md`：本技术审查报告。
- **修改文件**（严格遵守原 I2 任务文件预定边界）：
  - `src/brain/mod.rs`：`Turn` 新增 `snapshot: Option<Value>`，注册 `pub mod snapshot;`。
  - `src/brain/builtin.rs`：`build_prompt` 优先使用快照中的 `compiled` 系统输入。
  - `src/brain/model_api.rs`：测试构造补充字段。
  - `src/core/config.rs`：保持基线白名单，零配置扩展。
  - `src/core/control.rs`：接线 `execution_mode` 输出与 `brain-initialize` 控制操作，严格校验整型 `upgrade_schema_from`，请求 mode 显式拒绝，固定 `is_service = true` 迁移当前 BotService 部署语义，补齐 4 组回归测试。
  - `src/core/scheduler.rs`：测试构造补充 `fixed_snapshot: None` 机械适配。
  - `src/core/turns.rs`：接入 `execution_mode`、损坏根 fail-closed 阻断、Managed 无角色记录 `role_required`、快照注入模型、轮次模型重试版本固定（`fixed_snapshot`）及测试。
  - `src/core/history/store.rs`：migration 5 创建 `brain_turns` 表，实现 Provenance CRUD 与单测。
  - `src/core/external_brain.rs`：测试构造补充字段。
  - `src/core/open_api/session.rs`：增加 `api_version` 与 `capabilities`。
  - `src/core/open_api/contract.rs`：实现 `turn_v2`，测试构造补充字段。
  - `src/core/open_api/http.rs`：`brain_negotiate`、`brain_get_turn_detail`、v2 `reply` 强制必填与匹配 `snapshot_id`、DB 失败返回 `internal` 且保留 claim 及测试。
  - `src/core/backup.rs`：Manifest v3 结构、打包 `brain-bindings.json` 与 `brain-profiles`、防 symlink、v2 存在 brain 时 `restore_incomplete` 拒绝。
  - `src/gateway.rs`：测试构造补充字段。

---

## 6. 风险与限制

1. **入口限制**：当前 `brain-initialize` 控制入口仅支持迁移既有 `BotService` 部署（固定为服务语义），不提供测试 Bot 迁移或运行时模式覆盖接口；若调用方传入 `mode` 参数将被显式拒绝。
2. **执行预览首版保持禁用**：`execution: "execute"` 按照契约在当前阶段返回 `preview_execution_disabled`，避免误触外部付费模型。
3. **既有工具列表失败**：`desktop_act` 与既有 fixture 的差异仍保留在代码库中，需待后续工具面任务统一更新 fixture。
4. **真实真机运行**：本任务所有验证均在隔离临时目录与合成数据下完成，未执行真实微信客户端或 GUI 交互。
