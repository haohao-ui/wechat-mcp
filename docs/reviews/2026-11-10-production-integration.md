# Q1 完整版本集成与不回归验收审查报告 (修订版)

- **日期**：2026-11-10
- **任务编号**：Q1（`task_1636f08c7d3d` / Dispatch `ctx_cd10eea526e5`，前序 `task_cb6ec0a8660b` / `ctx_07b244bfc36e`）
- **集成工作区**：`<workspace-root>/prod-integration-review`
- **基线 Commit**：`2a55f45431491d5a20318395941b4ebd4b6216c5`（分支 `work/prod-integration-review`）
- **前序提交**：`5249818`
- **总体结果**：**partial（局部集成审查与单线程指定专项回归通过；真实部署、真机矩阵与更新写操作保持 pending / unsupported；产物一致性与默认并发测试未验证；非全项目或生产验收通过）**

---

## 1. 集成状态与依赖 Commit 核对

本集成分支已按 commit 集成并核对了前置依赖的审查交付提交：
- **Q0**（门禁与基线）：`1ba20e2`（基线门禁、零 Python 门禁、硬 CI 断言及 18 项门禁夹具修复）
- **I1**（角色修订存储）：`0bf4940`（包含 Root schema v2、`ExecutionMode`、`Store::initialize`、原子 activate/rollback、RFC 8785 规范化 JSON 与 40 项单测）
- **I2**（大脑运行接入）：`d9dbea0`（快照组装编译、模型提示词快照注入、`brain_turns` 追溯落库、外部协议协商与 v2 严格必填快照校验、备份 Manifest v3）
- **I3**（大脑配置界面）：`9fc0cdf`（6 个大脑 Tab 与会话追溯页实现、离屏渲染修复与自适应布局）
- **I4**（本地维护适配）：`bc4cd11`（前置 `b3c1365`，本地维护标志持久化、完成阶段故障锁存 `recoveryRequired`、169 项断言与 17 项 supervisor 集成通过）
- **I5**（更新核心与基础校验）：`d001a4e`（合并 commit `d7355e9`，签名封套与清单规范化校验、安全暂存审计、硬上限、防降级、配置强校验、禁用真实写操作）
- **I6**（更新界面与宿主集成）：`2a55f45`（680w/720w 双画幅自适应布局、读模型标签自动折行、只读状态异步刷新、恢复指引如实呈现）

工作区干净，无未解决的代码冲突。

---

## 2. 格式门禁与 Python 检查范围实事求是核对

### 2.1 格式门禁现状与全量比对
- **CI 现有命令边界**：`.github/workflows/ci.yml:56-58` 中明确记录：
  ```yaml
  # 暂不检查 src/interaction.rs，避免与进行中的通用桌面交互会话冲突；该会话提交后恢复为 cargo fmt --all -- --check
  - name: cargo fmt
    run: rustfmt --edition 2024 --check $(git ls-files '*.rs' | grep -v '^src/interaction.rs$')
  ```
  前序报告仅执行了上述 CI 现有命令并声称“完整硬门禁通过”，该表述不严谨，排除了 `src/interaction.rs`。
- **全仓无排除实际执行**：
  - 命令 1：`cargo fmt --all -- --check`，实际退出码 **0**，耗时 0.9s，无任何格式化差异输出；
  - 命令 2：`rustfmt --edition 2024 --check $(git ls-files '*.rs')`（包含 `src/interaction.rs`），实际退出码 **0**，耗时 3.2s，无格式差异；
  - 基线比对：与 `32bb12e` 派发基线一致，未引入格式违规，但由于 CI 配置文件尚未恢复为全仓检查，该豁免项属于 CI 历史遗留状态。

### 2.2 Python 门禁范围现状
- `scripts/check_no_python.sh` 在当前工作区实际扫描结果为通过（退出码 0）；
- **精确文件核对**：全仓仅保留获准的 3 个离线商业报告脚本（`docs/business/build_model.py`、`docs/business/build_compliance_model.py`、`docs/business/render_report.py`）；
- 构建入口（`scripts/build.sh`）、安装入口（`scripts/install.sh`）、CI 及产品源码（`src/`、`native/`）均未包含或引用任何 Python 脚本。不泛化宣称“全仓绝对无 Python”，定位为“运行时与交付入口零 Python 依赖”。

---

## 3. 测试隔离、历史挂起与并发假设定性

### 3.1 单一回环受限用例精确证据
- **用例标识**：`core::serve::tests::test_history_http_launchd_plist`
- **源码位置**：`src/core/serve.rs:878-950`（测试函数）及 `src/core/serve.rs:318-350`（服务启动逻辑）
- **单例隔离执行证据**：
  - **沙箱隔离模式（`BypassSandbox: false`，网络受限）**：
    - 执行命令：`cargo test --locked --bin wechat-mcp core::serve::tests::test_history_http_launchd_plist`
    - 现象：进程在输出 `test core::serve::tests::test_history_http_launchd_plist ...` 后无限挂起（耗时超 25 分钟无响应）；
    - 原因链路：沙箱拦截了对本地 `127.0.0.1:port` TCP 端口的连接；测试客户端子协程（`http_client_task`）重试 50 次失败后触发 `assert!(connected)` panic；由于 panic 发生在 `tx.send(true)` 发送停止信号之前，主协程 `run_serve_core` 永远未收到退出信号，在调度轮询 `loop` 中死锁；
  - **回环网络授权模式（`BypassSandbox: true`）**：
    - 执行命令同上；
    - 实际结果：退出码 **0**，耗时 **0.16s**，输出 `test result: ok. 1 passed; 0 failed`；
    - 结论：该用例在网络隔离环境下的挂起已确凿归因为测试子协程 panic 阻断停止信号引发的死循环。

### 3.2 并发执行卡住的机制假设与未验证声明
- **推测机制（假设）**：静态审查发现 `src/core/control.rs`、`src/core/serve.rs`、`src/core/backup.rs` 等源码中存在 40+ 处 `unsafe { std::env::set_var("HOME", ...) }`、`WECHAT_CORE_SOCK`、`MESSAGE_GATEWAY_DB` 对全局环境变量的覆写。推测在多线程默认并发执行时存在竞态冲突与临时路径污染。
- **未验证与边界声明**：
  - 单线程（`--test-threads=1`）测试通过仅能证明串行环境下隔离有效，**不能作为上述推测为并发卡死唯一根因的确凿证据**；
  - 默认多线程并发（`cargo test --locked` 无 `--test-threads=1`）目前**依然未验证、未归因**；
  - 历史在 I5 阶段由 worker 手动终止的全量测试挂起与本次单测中发现的既有失败（`ops_tools_list_unchanged`）是两类独立性质的问题，不得混淆，亦不得宣称“全量不回归已验证”。

---

## 4. 真实调用链审查证据表

深入审查静态路径、UI 输入、Control 字段、Store 存储、运行时调度、追溯落库及备份全链路，形成如下证据与风险矩阵：

| 链路环节 | 具体代码位置 | 核心字段 / 数据契约 | 实际行为与边界 | 风险与审查发现（缺陷退 Owner） |
|---|---|---|---|---|
| **UI 角色管理** | `native/bootstrap/BrainPage.swift:1597-1640` | `role_id`, `name`, `system_prompt`, `rules`, `templates` | 通过 `BrainClient` 异步调用 `brain-save-role`、`brain-activate` 等，处理 CAS generation 与界面刷新 | 仅管理角色与草稿，不触碰系统底层硬约束 |
| **UI 绑定管理** | `native/bootstrap/BrainPage.swift:1355-1401, 1565-1595` | `default_role_id`, `overrides` 列表 | `defaultRolePopup` 提供账号默认角色选择；但 `overridesSummaryLabel` **仅为只读文本 summary 展示** | **【业务缺陷 1】**：UI 缺少添加、编辑、删除特定联系人/群聊角色覆盖的表单交互，用户在界面无法配置覆盖规则（建议退 I3 owner 补齐交互控件） |
| **Control 接口** | `src/core/control.rs:2000-2220` | `account_id`, `expected_generation`, `request_id`, `installation` | 显式校验账号归属；维护锁保护下执行 `brain-initialize`（`fresh` / `legacy_upgrade`）；严格 CAS 防覆盖 | 请求去重缓存仅保存在当前运行实例内存，跨重启不持久化 |
| **Store 存储层** | `src/core/brain_profiles/{store,schema,migrate}.rs` | `root-schema-v2.json`, `ExecutionMode::Legacy / Managed` | 原子写入，FD 锚定路径防 symlink；LegacyUpgrade 迁出显式默认角色；损坏根 strictly fail-closed | 账号存储配额（1GB/128MB）目前采用算法校验，无自动清理孤立修订策略 |
| **运行时接入** | `src/core/turns.rs:1805-1815` | `Turn::snapshot`, `ExecutionMode` | `Managed` 模式无角色：journal 记录 `role_required` 并安全退出（模型调用 0、发送 0）；根损坏阻断 `integrity_mismatch` | **【风险关注】**：在外部大脑模式（OpenAPI）下，若第三方客户端未协商 v2（走 v1），可能绕过快照比对直接提交回复 |
| **提示词注入** | `src/brain/builtin.rs:35-70` | `turn.snapshot.compiled` | `build_prompt` 优先使用编译后快照系统提示词；系统事实规则（unreadable 等）由代码硬拼接在后 | 保证了客户话术可配置而系统能力契约不被用户篡改 |
| **对话追溯 (Provenance)** | `src/core/open_api/http.rs:684-860`, `brain_turns` 表 | `snapshot_id`, `snapshot_json`, `decision_json`, `model_json`, `execution_json` | 每轮次落库 `brain_turns`；v2 协商后强制必填 `snapshot_id`；不匹配返回 `snapshot_mismatch` 且 claim 保留有效 | 追溯数据库写入失败时不重发消息，确保发送幂等安全 |
| **更新状态查询** | `src/core/control.rs:2272-2278` | `current_release.release_id`, `sequence`, `components_consistent` | `update-status` 输出当前版本元数据与候选版本 journal | **【业务缺陷 2】**：`release_id` 源码中使用静态宏 `format!("{}-{}", env!("CARGO_PKG_VERSION"), ARCH)` 拼装，**非从已安装版本文件（如 VERSION）动态读取**；`components_consistent` 字段输出 `Null`（建议退 I5/I6 owner 完善） |
| **更新界面呈现** | `native/bootstrap/UpdatePage.swift:15-80, 200-240` | `currentReleaseLabel`, `recoveryGuidance` | 680w/720w 双画幅宽度自适应换行；恢复指引如实呈现“维护标志已记录，存活与数据完整性未知” | 写操作按钮（下载/安装重启/延期）在 UI 明确标注为“未启用”，点击受控拦截弹窗 |
| **更新器写操作** | `src/bin/wechat-updater.rs`, `src/updater/mod.rs` | `apply`, `rollback`, `recover` | CLI 副作用入口全部关闭（退出码 1，提示 unsupported）；未配置公钥根 strict fail-closed；DB 回退守卫报错 `unsupported_db_rollback` | 真实写操作、真实 codesign 重签名与核心排空完全处于禁用状态 |
| **备份与恢复** | `src/core/backup.rs` | Manifest v3, SHA-256 校验, staging 原子替换 | 升级至 Manifest v3，完整包含 `brain-profiles/`、`brain-bindings.json` 及 `brain_turns`；恢复旧备份拒绝混用 | 仅支持完整离线恢复，不支持在线热恢复 |

---

## 5. 验证命令、退出码与比对矩阵（单线程及指定专项覆盖）

测试环境：macOS Darwin 27.3 arm64；rustc 1.99.0，Apple Swift 6.4.2。以下命令均在当前 worktree 真实执行：

| 验证分类 | 精确执行命令 | 退出码 | 耗时 | 实际结果与比对证据 |
|---|---|---:|---:|---|
| **代码与敏感门禁** | `bash scripts/check_no_python.sh` | 0 | 2.8s | PASS（核对 3 个旧允许资料脚本，运行时零 Python） |
| **代码与敏感门禁** | `bash scripts/check_sensitive.sh` | 0 | 11.2s | PASS（敏感信息扫描通过，无本机路径泄露） |
| **构建与CI硬门禁** | `bash scripts/test-gates.sh` | 0 | 3.1s | PASS（构建、硬 CI 门禁检查通过） |
| **门禁夹具集** | `bash scripts/test-gate-fixtures.sh` | 0 | 2.4s | PASS（18 个独立夹具反向用例全部通过） |
| **格式门禁 (CI定义)**| `rustfmt --edition 2024 --check $(git ls-files '*.rs' \| grep -v '^src/interaction.rs$')` | 0 | 3.6s | PASS（CI 当前排除规则下通过） |
| **格式门禁 (全量检查)**| `cargo fmt --all -- --check` | 0 | 0.9s | PASS（无排除全量检查通过，退出码 0，无差异） |
| **Lint 门禁** | `cargo clippy --all-targets --locked` | 0 | 27.9s | PASS（退出码 0，无 error，保持既有基线 warning） |
| **核心单测集** | `cargo test --locked --bin wechat-mcp -- --test-threads=1` | 101 | 66.84s | **970 passed, 1 failed, 0 ignored**；在单线程覆盖下无新增失败；唯一失败为既有基线 `core::open_api::compat_tests::ops_tools_list_unchanged` |
| **I1 存储模块** | `cargo test --locked --bins -- brain_profiles` | 0 | 1.4s | **40 passed, 0 failed**（校对准确数量：包含 33 项基准 + 7 项 schema2/execution_mode 增量测试） |
| **更新核心与校验** | `cargo test --locked --lib updater::tests` | 0 | 21.6s | PASS（14 passed：清单签名、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、日志原子持久化、重复JSON键拒绝、CLI无副作用） |
| **更新器可执行文件**| `cargo test --locked --bin wechat-updater` | 0 | 1.3s | 退出码 0（输出 `0 passed; 0 failed`；**明确声明：仅代表该二进制可测试目标编译通过，不计为功能测试**） |
| **协议 Smoke** | `cargo test --locked --test protocol_smoke -- --test-threads=1` | 0 | 1.42s | PASS（2 passed：v2024_11_05 与 v2025_06_18） |
| **Swift 纯逻辑** | `cargo test --locked --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1` | 0 | 5.8s | PASS（14 passed：common 7, time 1, voice 3, bounded 3） |
| **Swift 生命周期断言**| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 3.2s | PASS（169 条断言全部通过，含完成阶段故障注入与 recoveryRequired 抑制） |
| **Supervisor 集成** | `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 9.3s | PASS（17 passed：排除主动停止与正常退出，5次崩溃退避上限保持） |
| **大脑界面与离屏** | `cargo test --test swift_brain_ui --locked` | 0 | 20.4s | PASS（1 passed：6 个大脑 Tab 与会话页离屏渲染，无裁切） |
| **更新界面与自适应** | `cargo test --test swift_update_ui --locked` | 0 | 19.5s | PASS（1 passed：41 项断言，含 680w/720w 双画幅离屏渲染、不越界验证与只读守卫） |

---

## 6. 产物一致性核对与 未验证（Unknown）边界

### 6.1 产物一致性静态核对
- 命令 `target/release/wechat-updater --version` 仅证明该单一二进制产物输出 `wechat-updater 0.1.0`，**绝不能以此推断完整应用产物一致**；
- 静态核对构建与安装脚本：
  - `scripts/build.sh` 编译了 CLI 工具（`wechat-mcp`, `wechat-updater`, `wechat_ocr`, `wechat_click`, `wechat_window`, `desktop_helper`, `wechat_bubbles`, `wechat_runtime`, `wechat_overlay`, `wechat_bot_worker`）；
  - `scripts/install.sh` 中的 `TOOLS` 数组仅包含 `wechat-mcp`, `wechat-updater`, `wechat_ocr`, `wechat_click`, `wechat_window`，缺少部分 helper；且未打包宿主 macOS App 包（`.app` bundle）；
  - 发布清单与正式 Release 包未生成；未执行真实本地安装验证；
- **明确结论**：**完整应用产物一致性当前为 UNKNOWN（未验证）**。

### 6.2 既有业务失败
- `core::open_api::compat_tests::ops_tools_list_unchanged`（`src/core/open_api/compat_tests.rs:49`）失败：在全量 51 个工具中，仅 `desktop_act` 存在描述修改与新增 `timeout_seconds` 参数差异，系历史提交 `7e41e7a` 遗留，保持原样记录，不私自改快照掩盖。在已执行单线程及专项覆盖范围内，前后比对无新增失败。

---

## 7. 真机验收矩阵（9大场景，明确保持 PENDING）

真机验证涉及外部微信客户端、隐私权限与真实网络消息，未经用户另行授权禁止执行。以下 9 大场景矩阵明确标记为 **PENDING**：

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

## 8. 审查结论与后续行动建议

- **交付状态**：**partial**（局部集成审查与单线程专项回归通过；更新写操作 unsupported；完整产物一致性与默认并发测试 unknown；真机矩阵 pending；非全项目验收通过）。
- **主要文件**：
  - `docs/reviews/2026-11-10-production-integration.md`（本审查报告修订版）
- **行动建议（退对应 Owner）**：
  1. **退 I3 owner**：在 `BrainPage.swift` 中为特定联系人覆盖规则补充新增、编辑、删除表单交互控件，消除目前仅能只读 summary 展示的缺陷；
  2. **退 I5/I6 owner**：在 `src/core/control.rs` 中将 `update-status` 的 `current_release.release_id` 调整为从实际已安装版本文件读取，避免仅依赖静态编译宏；
  3. **测试基础设施后续优化**：在 `core::serve::tests::test_history_http_launchd_plist` 中为客户端任务补充 `panic` 捕获兜底，确保在任何网络隔离或异常情况下均能正常发送停止通知，避免主调度死锁挂起。
