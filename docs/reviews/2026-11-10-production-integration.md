# Q1 完整版本集成与不回归验收审查报告 (第二轮集成修订版)

- **日期**：2026-11-11
- **任务编号**：Q1（`task_af50d0ed9ca1` / Dispatch `ctx_b276557cae42`，协调者终端 `term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`）
- **集成工作区**：`<workspace-root>/prod-integration-review`
- **基线 Commit**：`23d6da10c85ee8d98d022b787ae883d65b2e0ae9`（分支 `work/prod-integration-review`）
- **集成后 HEAD**：`e5854316a7f804fb56897bda52cb7ec18b843d1a`
- **总体结果**：**partial（按序精确集成 7 个提交；Q1 原两项缺陷修复核验闭环；单线程专项测试通过；发现 1 项新缺陷退 Owner 且保留既有基线 ops_tools 失败；真实安装签名排空回退保持 unsupported；真机 9 项场景 pending；完整产物与默认并发测试仍为 unknown；非全项目验收通过）**

---

## 1. 本轮集成提交顺序与映射核对

依据任务派发要求，本轮在原 `work/prod-integration-review`（基线 `23d6da1`）上精确按序合入了版本修复与绑定修复两组提交（父基线均已含 `2a55f45` / `9fc0cdf`）：

| 序号 | 目标 Commit | 集成后 Commit | 模块与语义 | 集成处理说明 |
|:---:|:---:|:---:|:---|:---|
| 1 | `b3f8eb8` | `7168c19` | fix(update-ui): 适配nullable release与防降级高水位隔离，独立核心编译标签与安全读模型 | 无冲突，快速合入 |
| 2 | `b71f54e` | `e386c04` | feat(updater): correct update-status release contract and isolate sequence guard | 无冲突（等价 `bc8812e`，未重复 pick） |
| 3 | `9c68fb9` | `5107e24` | test(update-ui): 闭环真实编译core与Swift客户端隔离联测，覆盖高水位隔离与异常免疫 | 无冲突，联测用例合入 |
| 4 | `c66b9a5` | `c3bafe9` | fix(brain-ui): implement override forms, draft CAS preservation, and allowlist-only contract | 无冲突，UI 增删改表单及契约合入 |
| 5 | `16691c6` | `b34d179` | feat(brain): add allowlist contacts readonly projection and responsive bindings layout | **机械冲突**：`src/core/control.rs` 尾部单元测试冲突；按要求完整保留 HEAD 侧 update-status 测试组与 incoming 侧 contacts projection 测试，不丢测试 |
| 6 | `8069449` | `a000bff` | fix(brain): eliminate projection error swallowing, enforce kind restriction and add inflight cas handler tests | 自动合并，合入投影错误不吞咽与 CAS 在途保护 |
| 7 | `70d02e5` | `e585431` | fix(brain): enforce readonly transaction snapshot, add fail-closed regressions and refine 680 table columns | 自动合并，合入同事务只读快照与 680 列宽优化 |

- **新 HEAD**：`e5854316a7f804fb56897bda52cb7ec18b843d1a`。工作区完全干净，无未解决冲突。

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
| **Swift 绑定管理** | `native/bootstrap/BrainPage.swift:1355-1550` | `overrideTable`, `contactPopup`, `rolePopup` | 支持增删改草稿、CAS 失败保护输入、680w 宽度列截断但悬停/编辑可查看完整值 | **PASS**（通过 70 项纯逻辑 + 33 项契约断言） |
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
| **Lint 门禁** | `cargo clippy --tests` | 0 | 8.4s | PASS（退出码 0，无 error，保持既有基线 warning） |
| **I1 存储模块** | `cargo test --locked --bins -- brain_profiles` | 0 | 1.1s | PASS（40 passed, 0 failed） |
| **更新核心与校验** | `cargo test --locked --lib updater::tests` | 0 | 0.2s | PASS（14 passed, 0 failed） |
| **协议 Smoke** | `cargo test --locked --test protocol_smoke -- --test-threads=1` | 0 | 1.4s | PASS（2 passed；经隔离 HOME 环境变量执行，避免沙箱拦截真实桌面资源锁） |
| **Swift 纯逻辑** | `cargo test --locked --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1` | 0 | 5.8s | PASS（14 passed：common 7, time 1, voice 3, bounded 3） |
| **Swift 生命周期断言**| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 3.2s | PASS（169 条断言全部通过，含完成阶段故障注入与 recoveryRequired 抑制） |
| **Swift 宿主完整集成**| `cargo test --locked --test swift_host -- --test-threads=1` | 0 | 35.9s | PASS（74 passed, 0 failed，包含 17 项 supervisor 与 57 项宿主用例） |
| **大脑界面与契约** | `cargo test --test swift_brain_ui --locked` | 0 | 23.8s | PASS（103 条断言全部通过，含 680w 离屏渲染与真实 Rust control 契约测试） |
| **更新界面与契约** | `cargo test --test swift_update_ui --locked` | 0 | 21.9s | PASS（92 条断言全部通过，含 680w/720w 双画幅离屏渲染与真实 core 联测） |
| **Control 重点用例** | `cargo test --locked --bin wechat-mcp test_control_brain_list` | 0 | 0.1s | PASS（4 passed：只读事务、列缺失拒绝、损坏行解码失败 unavailable、白名单与撤销） |
| **核心单测集** | `cargo test --locked --bin wechat-mcp -- --test-threads=1` | 101 | 65.5s | **977 passed, 3 failed, 0 ignored**（失败明细见 5.2 节：1 项既有基线失败 + 2 项因 ControlContactAccess fail-closed 引起的连锁失败） |

---

## 5. 缺陷核对与剩余缺陷定性（退 Owner 修复）

### 5.1 既有基线失败（保持原样记录）
- **用例**：`core::open_api::compat_tests::ops_tools_list_unchanged`（`src/core/open_api/compat_tests.rs:49`）
- **定性**：全量 51 个工具中，`desktop_act` 存在历史描述变更与 `timeout_seconds` 参数差异，系历史提交 `7e41e7a` 遗留，保持原样记录，不擅自修改测试快照。

### 5.2 新发现实际缺陷（退 I3 / 绑定修复 Owner）
在执行单线程全核心回归时，发现以下 2 项新增失败：
1. `core::control::tests::test_brain_initialize_control`（panic at `src/core/control.rs:6902: called Result::unwrap() on an Err value: Error("not_found")`）；
2. `core::turns::tests::test_turn_model_retry_pins_initial_snapshot_version`（panic at `src/core/turns.rs:8761: 必须锁定初始快照 fixed_snapshot`）。

#### 根因分析与调用链证据：
- 在提交 `8069449` 中，为实现“无 DB 时 contact 覆盖严格 fail-closed”，将 `ControlContactAccess::permits`（`src/core/control.rs:2501`）实现调整为：
  ```rust
  impl crate::core::brain_profiles::ContactAccess for ControlContactAccess {
      fn permits(&self, account: &str, contact: &str, _kind: &ChatKind) -> bool {
          if account != self.account_id { return false; }
          if let Some(ba) = &self.brain_access {
              ba.permit(contact)
          } else {
              false // 8069449 由 true 改为 false
          }
      }
  }
  ```
- 然而，`store.resolve(&ctx, contact, kind, access)`（`src/core/brain_profiles/store.rs:405`）的首行检查为：
  ```rust
  require(access.permits(&ctx.account_id, contact, kind), "not_found")?;
  ```
- **连锁反应**：
  1. 在 `test_brain_initialize_control` 中，验证旧无 mode 配置升级后，测试直接调用 `store_legacy.resolve(&ctx_legacy, "c_any", &Private, &ControlContactAccess { conn: None, brain_access: None, .. })` 试图获取默认角色。由于 `brain_access: None` 时 `permits` 直接返回 `false`，导致即使存在默认激活角色，`store.resolve` 也直接被 `not_found` 阻断并报错；
  2. 在 `test_turn_model_retry_pins_initial_snapshot_version` 中，未初始化 `history.sqlite3` 数据库，`turns.rs:1719` 自动构造的 `ControlContactAccess` 的 `brain_access` 为 `None`，导致 `store.resolve` 返回 `Err(not_found)`；在 Legacy 模式下无法装配角色与快照，未生成 `fixed_snapshot`，导致后续模型重试锁定断言失败。
- **行动建议（退 Owner）**：
  请 I3 / 绑定修复 Owner 厘清“联系人覆盖的访问检查（override check）”与“无覆盖时回退默认角色的联系人准入检查”的语义边界，或在 `ControlContactAccess` / `store.resolve` 中针对无覆盖且已激活默认角色的场景提供合理的准入判定，修复上述 2 项测试失败。集成方遵循“不顺手改业务，保持事实报告”原则，不擅自越权修改该授权逻辑。

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

- **交付状态**：**partial**（7 个指定提交集成完毕；Q1 原有两项缺陷已闭环修复；发现 1 项由 fail-closed 调整引发的默认角色解析缺陷，已留存确凿证据并退 Owner；真实安装、签名、排空、恢复保持 unsupported；真机 9 场景保持 pending；无 Python / 真实消息模型 / 钥匙串 / 部署推送）。
- **主要文件**：
  - `docs/reviews/2026-11-10-production-integration.md`（本审查报告）
- **集成后 HEAD**：`e5854316a7f804fb56897bda52cb7ec18b843d1a`
