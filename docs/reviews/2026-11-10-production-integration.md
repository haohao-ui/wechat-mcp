# Q1 完整版本集成与不回归验收审查报告 (第三轮集成与测试修复闭环版)

- **日期**：2026-11-11
- **任务编号**：Q1（`task_d663d077f300` / Dispatch `ctx_ef1c6be05c35`，前序 `task_af50d0ed9ca1` / `ctx_b276557cae42`，协调者终端 `term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`）
- **集成工作区**：`<workspace-root>/prod-integration-review`
- **基线 Commit**：`56d34d0bbf12781bca664790a369cfbb8fcf5e53`（分支 `work/prod-integration-review`）
- **代码提交 HEAD**：`a86f0e21ea5c1488c94632c0d8c0b5fc917ffb2a`
- **总体结果**：**partial（8 个提交全部精确按序集成；Q1 原有两项缺陷修复已闭环；Owner 最新提交 45d05d9 修复了测试 fixture，两项新增失败彻底消除；单线程全量核心测试仅剩既有基线 ops_tools 失败，退出码严格为 101，不混写为 0；真实安装签名排空回退保持 unsupported；真机 9 项场景 pending；默认并发与完整制品仍 unknown；非全项目验收通过）**

---

## 1. 本轮集成提交顺序与映射核对

依据任务派发要求，本轮在基线 `56d34d0` 上保护工作区并合入了 Owner 交付的单一修复提交 `45d05d903b9f557f7e1134da1ae27de06e0c43f0`。全量集成提交链条如下：

| 序号 | 目标 Commit | 集成后 Commit | 模块与语义 | 集成处理说明 |
|:---:|:---:|:---:|:---|:---|
| 1 | `b3f8eb8` | `7168c19` | fix(update-ui): 适配nullable release与防降级高水位隔离，独立核心编译标签与安全读模型 | 无冲突合入 |
| 2 | `b71f54e` | `e386c04` | feat(updater): correct update-status release contract and isolate sequence guard | 无冲突合入（等价 `bc8812e`） |
| 3 | `9c68fb9` | `5107e24` | test(update-ui): 闭环真实编译core与Swift客户端隔离联测，覆盖高水位隔离与异常免疫 | 无冲突合入 |
| 4 | `c66b9a5` | `c3bafe9` | fix(brain-ui): implement override forms, draft CAS preservation, and allowlist-only contract | 无冲突合入 |
| 5 | `16691c6` | `b34d179` | feat(brain): add allowlist contacts readonly projection and responsive bindings layout | 机械冲突解决，双侧测试完整保留 |
| 6 | `8069449` | `a000bff` | fix(brain): eliminate projection error swallowing, enforce kind restriction and add inflight cas handler tests | 自动合并合入 |
| 7 | `70d02e5` | `e585431` | fix(brain): enforce readonly transaction snapshot, add fail-closed regressions and refine 680 table columns | 自动合并合入 |
| 8 | `45d05d9` | `a86f0e2` | test(brain): fix core initialize and turn retry fixtures with migrated history db and real access | **无冲突合入**；生产 Store.resolve 与 None=>false 保持不变，将测试假 fixture 迁移为真实 history.sqlite3 数据库与 BrainAccess，彻底消除两项新增失败 |

- **代码提交 HEAD**：`a86f0e21ea5c1488c94632c0d8c0b5fc917ffb2a`。工作区完全干净。

---

## 2. Q1 原有两项缺陷修复核验与前后证据

### 2.1 业务缺陷 1（UI 绑定管理缺少增删改表单）修复验证
- **修复前状态**：`native/bootstrap/BrainPage.swift:1355-1401` 中，联系人覆盖列表 `overridesSummaryLabel` 仅有只读文本展示，用户无法在 UI 界面配置、添加、编辑或删除特定联系人/群聊的角色覆盖规则。
- **修复后证据**：
  1. **UI 交互与表单闭环**：`BrainPage.swift` 补充了完整的新增覆盖（`showAddOverridePopover`）、编辑行、删除覆盖控件及表单校验；
  2. **白名单强约束与真实 ID**：表单联系人选择严格绑定当前账号只读投影数据（`allowlist_contacts`），必须为已在本地 `contacts` 表入库且受 allowlist 授权的真实联系人，禁止手动捏造姓名或非法 `contact_id`；
  3. **只读事务一致性快照**：`compute_allowlist_contacts_projection` 打开只读事务（`rusqlite::TransactionBehavior::Deferred`），原子执行 `BrainAccess::load` 授权解析与联系人过滤，任何 SQL 坏行或错误立即返回 `unavailable`，绝不返回“部分 ready”脏状态；
  4. **草稿 CAS 在途保护**：保存草稿冲突或在途变更时，用户正在输入的草稿数据通过 `preservedDraft` 完整保留，避免被静默丢弃；
  5. **断言覆盖**：`cargo test --test swift_brain_ui --locked` 执行包含 103 项断言（含 680 离屏渲染与真实 Rust control 契约测试），全部 **PASS**。

### 2.2 业务缺陷 2（update-status release_id 静态伪造与契约不符）修复验证
- **修复前状态**：`src/core/control.rs` 中 `current_release.release_id` 源码使用静态宏 `format!("{}-{}", env!("CARGO_PKG_VERSION"), ARCH)` 拼装，未从已安装版本文件读取；`components_consistent` 字段输出 `Null`；防降级 sequence 与 installed sequence 语义混淆。
- **修复后证据**：
  1. **安全读模型契约分离**：`update-status` 输出中，已安装发布版本 `current_release.release_id` 和 `sequence` 改为 nullable；在未安装受签名保护的完整发布包前明确输出 `null` / `status: "unknown"` / `source: "unknown"`；
  2. **核心编译标签独立**：独立输出当前二进制编译版本 `current_release.core_version`（即 `0.1.0`）与 `architecture`，不冒充整套 release；
  3. **高水位防降级守卫隔离**：`guard.highest_accepted_sequence` 独立于已安装 sequence，损坏 sequence 文件严格 fail-closed；
  4. **受控拦截与只读防护**：更新界面 680w/720w 双画幅离屏自适应渲染通过；写操作受控拦截；
  5. **断言覆盖**：`cargo test --test swift_update_ui --locked` 执行包含 92 项断言（含真实 core control 联测），全部 **PASS**。

---

## 3. 真实调用链审查与重点复核矩阵

| 链路环节 | 具体代码位置 | 核心字段 / 数据契约 | 实际行为与边界 | 审查结果与状态 |
|---|---|---|---|---|
| **投影只读事务** | `src/core/control.rs:2582-2663` | `compute_allowlist_contacts_projection` | 在同一个只读事务隔离快照内执行 `BrainAccess::load` 与 contacts 枚举，出错整批返回 unavailable，不部分 ready | **PASS**（4 项专用测试通过） |
| **无 DB Fresh/绑定** | `src/core/control.rs:7738-7880` | `test_control_fail_closed_access_across_operations_without_db` | 无 history.sqlite3 时：fresh 角色/知识/无覆盖默认绑定/激活/回退可用；但包含 contact 覆盖时保存严格被拒绝（fail-closed，返回 `not_found`） | **PASS**（7 阶段全流程断言通过） |
| **Swift 绑定管理** | `native/bootstrap/BrainPage.swift:1355-1550` | `overrideTable`, `contactPopup`, `rolePopup` | 支持增删改草稿、CAS 失败保护输入、680w 宽度列截断但悬停/编辑可查看完整值 | **PASS**（通过 70 项纯逻辑 + 33 项契约断言，共 103 项） |
| **更新状态与高水位** | `src/core/control.rs:2270-2390` | `current_release`, `guard`, `transaction` | release_id/sequence nullable；core_version 独立；拒绝未签名 VERSION 文件冒充 | **PASS**（92 项断言通过） |
| **更新器写操作** | `src/bin/wechat-updater.rs`, `src/updater/mod.rs` | `apply`, `rollback`, `recover` | CLI 副作用入口全部关闭（退出码 1，提示 unsupported）；未配置公钥根 strict fail-closed；DB 回退守卫报错 `unsupported_db_rollback` | **UNSUPPORTED**（保持禁用，无真实写操作） |
| **备份与恢复** | `src/core/backup.rs` | Manifest v3, SHA-256 校验, staging 原子替换 | 升级至 Manifest v3，完整包含 `brain-profiles/`、`brain-bindings.json` 及 `brain_turns`；恢复旧备份拒绝混用 | **PASS**（仅支持离线恢复） |

---

## 4. 验证命令、退出码与比对矩阵

测试环境：macOS Darwin 27.3 arm64；rustc 1.99.0，Apple Swift 6.4.2。所有命令均在当前 worktree 真实执行：

| 验证分类 | 精确执行命令 | 退出码 | 耗时 | 实际结果与比对证据 |
|---|---|---:|---:|---|
| **代码与敏感门禁** | `bash scripts/check_no_python.sh` | 0 | 2.6s | PASS（核对获准离线商业脚本，交付入口零 Python） |
| **代码与敏感门禁** | `bash scripts/check_sensitive.sh` | 0 | 10.8s | PASS（敏感信息扫描通过，无本机路径泄露） |
| **构建与CI硬门禁** | `bash scripts/test-gates.sh` | 0 | 3.0s | PASS（构建与硬 CI 门禁检查通过） |
| **门禁夹具集** | `bash scripts/test-gate-fixtures.sh` | 0 | 2.3s | PASS（18 个独立夹具反向用例全部通过） |
| **格式门禁 (CI定义)**| `rustfmt --edition 2024 --check $(git ls-files '*.rs' \| grep -v '^src/interaction.rs$')` | 0 | 3.5s | PASS（CI 既有排除规则下通过） |
| **格式门禁 (全量检查)**| `cargo fmt --all -- --check` | 0 | 0.9s | PASS（全量检查通过，无差异输出） |
| **Lint 门禁** | `cargo clippy --tests` | 0 | 6.8s | PASS（退出码 0，无 error，保持既有基线 warning） |
| **I1 存储模块** | `cargo test --locked --bins -- brain_profiles` | 0 | 1.1s | PASS（40 passed, 0 failed） |
| **更新核心与校验** | `cargo test --locked --lib updater::tests` | 0 | 0.2s | PASS（14 passed, 0 failed） |
| **协议 Smoke** | `cargo test --locked --test protocol_smoke -- --test-threads=1` | 0 | 1.4s | PASS（2 passed；经隔离环境执行，避免沙箱拦截真实桌面资源锁） |
| **Swift 纯逻辑** | `cargo test --locked --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1` | 0 | 5.8s | PASS（14 passed：common 7, time 1, voice 3, bounded 3） |
| **Swift 生命周期断言**| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 3.2s | PASS（169 条断言全部通过，含完成阶段故障注入与 recoveryRequired 抑制） |
| **Swift 宿主完整集成**| `cargo test --locked --test swift_host -- --test-threads=1` | 0 | 35.9s | PASS（74 passed, 0 failed，包含 17 项 supervisor 与 57 项宿主用例） |
| **大脑界面与契约** | `cargo test --test swift_brain_ui --locked` | 0 | 21.6s | PASS（103 条断言全部通过，含 680w 离屏渲染与真实 Rust control 契约测试） |
| **更新界面与契约** | `cargo test --test swift_update_ui --locked` | 0 | 21.9s | PASS（92 条断言全部通过，含 680w/720w 双画幅离屏渲染与真实 core 联测） |
| **Control 重点用例** | `cargo test --locked --bin wechat-mcp test_control_brain_list` | 0 | 0.1s | PASS（4 passed：只读事务、列缺失拒绝、损坏行解码失败 unavailable、白名单与撤销） |
| **Control 无DB守卫** | `cargo test --locked --bin wechat-mcp test_control_fail_closed` | 0 | 0.1s | PASS（1 passed：全阶段无 DB fail-closed 阻断） |
| **修复单测 1 验证** | `cargo test --locked --bin wechat-mcp test_brain_initialize_control` | 0 | 0.2s | PASS（1 passed；前序失败消除，退出码 0） |
| **修复单测 2 验证** | `cargo test --locked --bin wechat-mcp test_turn_model_retry_pins_initial_snapshot_version` | 0 | 0.1s | PASS（1 passed；前序失败消除，退出码 0） |
| **核心单测全套** | `cargo test --locked --bin wechat-mcp -- --test-threads=1` | 101 | 67.7s | **979 passed, 1 failed, 0 ignored**（共 980 个用例；唯一失败为既有基线 ops_tools，命令退出码严格为 101，不能混写为 0；两项前序失败已彻底消除） |

---

## 5. 两项新增失败消除证据与剩余缺陷核对

### 5.1 前序两项新增失败消除证据
在集成 `45d05d9` 之前，单线程核心测试曾出现 2 项失败：
1. `core::control::tests::test_brain_initialize_control`（此前因 `DummyAccess` / 无 DB 传入 `ControlContactAccess { conn: None, brain_access: None }` 导致 `permits` 返回 `false`，使得 `store.resolve` panic 报错 `Error("not_found")`）；
2. `core::turns::tests::test_turn_model_retry_pins_initial_snapshot_version`（此前因未初始化 `history.sqlite3`，`turns.rs` 自动构建的 `ControlContactAccess` 缺乏 `brain_access`，导致 `store.resolve` 返回 `Err`，未生成 `fixed_snapshot` 从而断言失败）。

#### 修复与闭环消除证据：
- **生产代码保持严格 fail-closed**：Owner 提交 `45d05d9` 严格遵守协调者要求，未随意放宽生产代码中的 `ControlContactAccess::permits`（仍保持 `None => false` 的 fail-closed 契约），亦未擅改 `Store::resolve`；
- **测试 Fixture 真实化**：
  - 在 `test_brain_initialize_control` 中，测试补全了 `history.sqlite3`（包含 `contacts` 与 `contact_names`，真实 `contact_id: c_legacy_cust` 与配置姓名 `客户张三` 严格解耦），通过 `BrainAccess::load` 构建真实的 `ControlContactAccess`；并追加了未知联系人、未授权联系人、跨账号联系人及跨账号上下文的 4 项 `is_err()` 阻断断言；
  - 在 `test_turn_model_retry_pins_initial_snapshot_version` 中，移除假 `DummyAccess`，创建真实的 `history.sqlite3` 数据库与合法联系人映射，使 `turns.rs` 能够在运行时正确解析初始快照并锁定 `fixed_snapshot`；
- **执行结果**：
  - `cargo test --locked --bin wechat-mcp test_brain_initialize_control`：退出码 **0**，耗时 0.16s，**ok**；
  - `cargo test --locked --bin wechat-mcp test_turn_model_retry_pins_initial_snapshot_version`：退出码 **0**，耗时 0.13s，**ok**；
  - 全套回归中两项测试均以 `ok` 通过，失败计数从 3 项下降至 1 项。

### 5.2 既有基线失败（保持原样记录，不擅改快照）
- **用例**：`core::open_api::compat_tests::ops_tools_list_unchanged`（`src/core/open_api/compat_tests.rs:49`）
- **定性**：全量 51 个工具中，`desktop_act` 存在历史描述变更与 `timeout_seconds` 参数差异，系历史提交 `7e41e7a` 遗留。全套单测执行因该用例失败而返回退出码 **101**。依据指示保持原样记录，不顺手修改快照掩盖，不混写为退出码 0。

---

## 6. 未验证边界与真机验收矩阵

### 6.1 未验证（Unknown）边界
- **完整产物一致性**：`scripts/install.sh` 仅复制部分 CLI 工具，未打包 macOS `.app` bundle，未生成正式 Release 包，完整应用产物一致性为 **UNKNOWN**；
- **默认并发执行**：多线程默认并发测试（`cargo test` 无 `--test-threads=1`）因全局环境变量并发覆写机制，依然保持 **UNKNOWN**，不宣称并发安全。

### 6.2 真机验收矩阵（9大场景，明确保持 PENDING）

| 编号 | 真机场景 | 验证要点与断言要求 | 状态 | 未运行/阻断证据 |
|---|---|---|---|---|
| M1 | **相同消息连续到达** | 同一联系人连续发送多条相同文本时，眼部稳定捕获，大脑决策按轮次关联，发送幂等键严格去重，不重复回复 | **PENDING** | 未连接专用机，未切换真实微信 |
| M2 | **多会话并发轮询与切换** | 多个联系人/群聊同时产生未读消息，调度器按权重和白名单串行切换，会话状态与历史目录严格账号隔离 | **PENDING** | 未连接专用机，未切换真实微信 |
| M3 | **联系人重名与改名** | 存在同名或备注修改的联系人时，通过 contact_id 保持唯一性，不因模糊匹配错发消息 | **PENDING** | 未连接专用机，未切换真实微信 |
| M4 | **首条未读顶到上方跳转** | 微信将长未读顶到聊天窗口顶部时的衔接恢复，翻页找回衔接点并回到底部“画面末尾一致”确认 | **PENDING** | 未连接专用机，未切真实微信界面 |
| M5 | **同时长语音身份跟踪** | 气泡像素相同、时长相同的多条连续语音，按当前画面时间标签与自下而上排位准确定位与转写关联 | **PENDING** | 未连接专用机，未切真实微信界面 |
| M6 | **人工接管与暂停** | 触发转人工后生成 `handoff.json`，会话自动暂停并呈现在界面，人工处理完毕后恢复回复 | **PENDING** | 未连接专用机，未触发人工接管 |
| M7 | **退出换号检测** | 微信退出或更换微信号时，`detect_logged_in_account` 识别变更，旧号数据完整归档保留，新号从零初始化 | **PENDING** | 未连接专用机，未操作微信登录态 |
| M8 | **本地签名与权限保持** | 更新替换后专用机辅助功能（Accessibility）与屏幕录制（Screen Recording）权限是否被系统吊销 | **PENDING** | 本地代码重签名未执行，未在专用机测试权限 |
| M9 | **连续 7 天试运行** | 单执行节点单微信号连续 7 天无内存泄漏、无 supervisor 崩溃退避、未读漏读率为 0、逐帧日志按期轮转 | **PENDING** | 试运行需另行专项授权与持续监控 |

---

## 7. 审查结论与交付说明

- **交付状态**：**partial**（8 个提交集成完毕；Q1 原有两项缺陷已闭环修复；前序两项新增失败彻底消除；单线程全量测试仅剩既有基线 ops_tools 失败，退出码严格为 101；真实安装、签名、排空、恢复保持 unsupported；真机 9 场景保持 pending；无 Python / 真实消息模型 / 钥匙串 / 部署推送）。
- **主要文件**：
  - `docs/reviews/2026-11-10-production-integration.md`（本审查报告）
- **代码提交 HEAD**：`a86f0e21ea5c1488c94632c0d8c0b5fc917ffb2a`
