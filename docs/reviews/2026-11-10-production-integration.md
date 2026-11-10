# Q1 完整版本集成与不回归验收审查报告

- **日期**：2026-11-10
- **任务编号**：Q1（`task_cb6ec0a8660b` / Dispatch `ctx_07b244bfc36e`）
- **集成工作区**：`<workspace-root>/prod-integration-review`
- **基线 Commit**：`2a55f45431491d5a20318395941b4ebd4b6216c5`（分支 `work/prod-integration-review`）
- **总体结果**：**partial（局部集成审查与无新增失败回归通过；真实部署、真机矩阵与更新写操作保持 pending / unsupported；非全项目或生产验收通过）**

---

## 1. 集成状态与依赖 Commit 核对

本集成分支完整包含了全部前置依赖的审查交付提交：
- **Q0**（门禁与基线）：`1ba20e2`（基线门禁、零 Python 门禁、硬 CI 断言及 18 项门禁夹具修复）
- **I1**（角色修订存储）：`0bf4940`（包含 Root schema v2、`ExecutionMode`、`Store::initialize`、原子 activate/rollback、RFC 8785 规范化 JSON 与 33 项单测）
- **I2**（大脑运行接入）：`d9dbea0`（快照组装编译、模型提示词快照注入、`brain_turns` 追溯落库、外部协议协商与 v2 严格必填快照校验、备份 Manifest v3）
- **I3**（大脑配置界面）：`9fc0cdf`（6 个大脑 Tab 与会话追溯页实现、离屏渲染修复与自适应布局）
- **I4**（本地维护适配）：`bc4cd11`（前置 `b3c1365`，本地维护标志持久化、完成阶段故障锁存 `recoveryRequired`、169 项断言与 17 项 supervisor 集成通过）
- **I5**（更新核心与基础校验）：`d001a4e`（合并 commit `d7355e9`，签名封套与清单规范化校验、安全暂存审计、硬上限、防降级、配置强校验、禁用真实写操作）
- **I6**（更新界面与宿主集成）：`2a55f45`（680w/720w 双画幅自适应布局、读模型标签自动折行、只读状态异步刷新、恢复指引如实呈现）

工作区干净，无未解决的代码冲突。

---

## 2. 历史全量测试挂起与隔离根因调查

协调者特别要求归因“此前 `cargo test --locked` 被终止的隔离问题”，本次审查进行了精准定位与隔离验证，查明了双重根因：

### 2.1 阻塞挂起用例定位：`core::serve::tests::test_history_http_launchd_plist`
1. **现象**：在标准沙箱模式（`BypassSandbox: false`）下单线程运行 `cargo test --bin wechat-mcp -- --test-threads=1` 时，测试在执行到第 594 个用例 `core::serve::tests::test_history_http_launchd_plist` 时永久阻塞（挂起超过 25 分钟无输出）。
2. **隔离原因**：
   - 该测试通过 `get_free_port()` 探测可用端口，并在 `run_serve_core` 中启动 `HttpServer` 监听 `127.0.0.1:port`；
   - 标准沙箱（`BypassSandbox: false`）默认具有完整工作区读写权限，但**严格禁止网络访问（no network access）**，包括对本地回环 TCP 端口的连接；
   - 测试中的客户端子协程 `http_client_task` 尝试 `TcpStream::connect(("127.0.0.1", port))` 被沙箱阻止，在 50 次重试超时后抛出 `assert!(connected)` panic；
   - **代码死循环缺陷**：子协程 panic 发生在 `tx.send(true)` 停止信号之前，导致主协程 `run_serve_core` 永远无法接收到停止信号，在调度轮询 `loop` 中无限循环；
3. **对照验证证据**：
   - 授予本地回环网络权限（`BypassSandbox: true`）单独运行该测试：`test core::serve::tests::test_history_http_launchd_plist ... ok`，**耗时 0.16s 瞬间通过**；
   - 证明该挂起纯属沙箱网络拦截与测试代码中缺少 panic 保护的超时停止机制所致，非业务逻辑死锁。

### 2.2 并发执行卡住根因：进程级全局环境变量竞态
- 代码中多处测试（`src/core/control.rs`、`src/core/serve.rs`、`src/core/backup.rs` 等）直接使用 `unsafe { std::env::set_var("HOME", ...) }`、`WECHAT_CORE_SOCK`、`MESSAGE_GATEWAY_DB` 修改全局环境变量；
- 默认多线程并发运行测试时，环境变量在各个并发用例间产生竞态覆写，导致 `WorkerClient` 或控制接口连入错误 socket 或读取不存在的临时目录，产生无限等待与重试；
- 验证结论：测试必须采用 `--test-threads=1` 隔离执行，消除环境变量并发污染。

---

## 3. 完整验证命令、退出码与证据矩阵

测试环境：macOS Darwin 27.3 arm64；rustc 1.99.0，Apple Swift 6.4.2。全部命令均真实执行，无跳过或伪造：

| 验证分类 | 精确执行命令 | 退出码 | 耗时 | 实际结果与比对证据 |
|---|---|---:|---:|---|
| **代码与敏感门禁** | `bash scripts/check_no_python.sh` | 0 | 2.8s | PASS（仅允许 3 个离线资料脚本，运行时零 Python） |
| **代码与敏感门禁** | `bash scripts/check_sensitive.sh` | 0 | 11.2s | PASS（敏感信息扫描通过） |
| **构建与CI硬门禁** | `bash scripts/test-gates.sh` | 0 | 3.1s | PASS（构建、硬 CI、无 continue-on-error 门禁检查通过） |
| **门禁夹具集** | `bash scripts/test-gate-fixtures.sh` | 0 | 2.4s | PASS（18 个独立夹具反向用例全部通过） |
| **格式门禁** | `rustfmt --edition 2024 --check $(git ls-files '*.rs' \| grep -v '^src/interaction.rs$')` | 0 | 3.6s | PASS（代码格式一致，退出码 0） |
| **Lint 门禁** | `cargo clippy --all-targets --locked` | 0 | 27.9s | PASS（退出码 0，无 error，保持既有基线 warning） |
| **核心单测集** | `cargo test --locked --bin wechat-mcp -- --test-threads=1` | 101 | 66.84s | **970 passed, 1 failed, 0 ignored**；**零新增失败**！唯一失败为既有基线 `core::open_api::compat_tests::ops_tools_list_unchanged`（详见第 4 节） |
| **更新器单测集** | `cargo test --locked --bin wechat-updater` | 0 | 1.3s | PASS（0 tests，可执行文件编译通过） |
| **更新核心与校验** | `cargo test --locked --lib updater::tests` | 0 | 21.6s | PASS（14 passed：覆盖清单签名、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、日志原子持久化、重复JSON键拒绝、CLI无副作用） |
| **协议 Smoke** | `cargo test --locked --test protocol_smoke -- --test-threads=1` | 0 | 1.42s | PASS（2 passed：v2024_11_05 与 v2025_06_18） |
| **Swift 纯逻辑** | `cargo test --locked --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1` | 0 | 5.8s | PASS（14 passed：common 7, time 1, voice 3, bounded 3） |
| **Swift 生命周期断言**| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 3.2s | PASS（169 条断言全部通过，含完成阶段故障注入与 recoveryRequired 抑制） |
| **Supervisor 集成** | `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 9.3s | PASS（17 passed：排除主动停止与正常退出，5次崩溃退避上限保持） |
| **大脑界面与离屏** | `cargo test --test swift_brain_ui --locked` | 0 | 20.4s | PASS（1 passed：6 个大脑 Tab 与会话页离屏渲染，无裁切） |
| **更新界面与自适应** | `cargo test --test swift_update_ui --locked` | 0 | 19.5s | PASS（1 passed：41 项断言，含 680w/720w 双画幅离屏渲染、不越界验证与只读守卫） |
| **构建产物一致性** | `target/release/wechat-updater --version` | 0 | 0.05s | 输出 `wechat-updater 0.1.0`，与 `scripts/build.sh` 和 `scripts/install.sh` 声明一致 |

---

## 4. 既有失败比对与不回归证据

全仓回归中存在且仅存在 1 项既有失败：
- **用例**：`core::open_api::compat_tests::ops_tools_list_unchanged`
- **位置**：`src/core/open_api/compat_tests.rs:49`
- **比对结果**：与 Q0、I2 审查记录完全一致。在全量 51 个工具定义中，仅在第 14 号工具 `desktop_act` 存在差异：
  - `description` 文案修改；
  - `inputSchema.properties` 新增 `timeout_seconds` 字段（`number`，`default: 30.0`）。
- **来源**：历史提交 `7e41e7a`（`wip(interaction): 通用桌面工具进行中的修改与专用机验收记录`）。
- **合规处理**：严格按交付约束保留既有失败，不删除工具、不私自更新旧快照掩盖变更。除此项外，**无任何新增失败**。

---

## 5. 本地基础交付范围与 Unsupported 项声明

根据协调者审核意见，当前版本为**本地基础校验与界面只读交付**，以下能力严格保持禁用并声明为 `unsupported`：
1. **真实更新写操作未启用**：
   - `wechat-updater apply` / `rollback` / `recover` 均无副作用退出并返回 `unsupported`（退出码 1）；
   - 更新界面第二行写操作按钮（下载、安装重启、延期）均明确标记为“未启用”，点击弹窗提示受控管理，不执行任何写操作；
2. **生产信任根公钥未配置**：未配置公钥根时严格 fail closed（输出 `unverified_trust_root`），杜绝外部测试公钥绕过；
3. **真实本机代码签名未执行**：未访问专用机钥匙串，未执行本地自签名证书重签名；
4. **真实核心排空未接入**：真实进程优雅停止、在途轮次排空与跨进程唯一互斥更新锁尚未完成；
5. **数据库自动回退禁用**：`restore_database_backup_guarded` 严格报错 `unsupported_db_rollback`，防止新业务数据被旧快照静默覆盖。

---

## 6. 真机验收矩阵（9大场景，全部标为 PENDING）

真机运行涉及外部微信应用、隐私权限与真实消息收发，未经用户另行授权不得执行。以下真机矩阵已制定并明确保持 **PENDING** 状态：

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

## 7. 审查结论与交付成果

- **交付状态**：**partial**（本地集成代码与回归测试完备，无新增失败；更新写操作与真机矩阵保持 unsupported / pending；非全项目验收通过）。
- **主要文件**：
  - `docs/reviews/2026-11-10-production-integration.md`（本审查报告）
- **风险与未验证项**：
  1. 生产环境需配置官方 Ed25519 根公钥，更新分发源与下载通道需安全隔离；
  2. 真实专用机本地代码重签名对 macOS TCC 权限的影响需在真机测试阶段实测；
  3. `test_history_http_launchd_plist` 需在后续重构中为子协程补充 panic 兜底机制，防止无网络环境下挂起。
