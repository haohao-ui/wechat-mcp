# 生产就绪检查报告

- 日期：2026-11-12
- 检查基线：`main` / `3e9c0bf`（临时 worktree `prod-check-main`）
- 当前用户工作区：`feat/wechat-bot-history` / `b7e3afb`，落后 main 105 个提交，另有 13 个提交未合入 main
- 检查方式：只读检查 + 隔离环境构建/测试；未部署、未发送微信消息、未调用付费模型、未连接专用机
- 结论：**未通过生产就绪检查（partial）**。功能测试、构建与本地门禁基本可用；格式门禁、任务 panic 隔离与桌面操作超时已在本分支修复。主要阻断项是更新器真实写操作仍未实现、发布/安装签名链路不完整，以及真机验收仍为 partial。

> 本报告是工程与验收证据检查，不是正式生产验收结论，也不代表整个项目验收通过。

## 1. 门禁结果

| 检查项 | 命令 | 结果 |
|---|---|---|
| 构建 | `bash scripts/build.sh` | PASS（退出码 0；Rust release、Swift 原生工具、self-test 均通过） |
| 门禁脚本 | `bash scripts/test-gates.sh` | PASS |
| 敏感信息 | `bash scripts/check_sensitive.sh` | PASS |
| Shell | `shellcheck -S warning scripts/*.sh scripts/hooks/*` | PASS（0 告警） |
| 格式（修复前） | `cargo fmt --all -- --check` | FAIL；`src/core/history/store.rs`、`src/core/turns.rs`，CI 命令下 55 处 diff |
| 格式（本分支） | `cargo fmt --all -- --check` | PASS（本分支已提交格式化修复） |
| Clippy（CI 口径） | `cargo clippy --all-targets --locked` | PASS（退出码 0，但约 1850 条 warning） |
| Clippy（严格口径） | `cargo clippy --all-targets --locked -- -D warnings` | FAIL（退出码 101，约 2000 个 error） |
| 全量测试 | `cargo test --locked --no-fail-fast` | PASS（2250 passed / 0 failed / 3 ignored，隔离环境） |
| 原生自测 | `wechat_window --self-test`、`wechat_overlay --self-test`、`wechat_runtime --self-test` | PASS |

测试环境说明：修复前裸跑沙箱曾出现 15 个失败，均为环境问题而非业务断言：

- 4 个 updater CLI 用例找不到 `target/debug/wechat-updater`（自定义 `CARGO_TARGET_DIR` 导致回退路径失效）；
- 2 个 UI 联测用例找不到 `target/debug/wechat-mcp`（同一原因）；
- 9 个 `swift_host` 配置用例写真实用户目录下的 `~/.wechat-rpa/tmp`，被沙箱拒绝。

本分支已修复 9 个 `swift_host` 用例的用户目录依赖：测试 harness 为每个 Swift 子进程设置 `CFFIXED_USER_HOME` 到临时目录。修复后不再需要外部设置该变量，全量测试在临时 `HOME` 下仍为 2250 通过、0 失败、3 ignored。updater 与 UI 用例也已改为从 `current_exe` 定位 `target/debug`，在自定义 `CARGO_TARGET_DIR` 下同样可运行（需先构建对应二进制）。

main 上的 `core::open_api::compat_tests::ops_tools_list_unchanged` 已通过（fixture 已更新）；旧分支仍是失败状态。

## 2. P0 阻断项

### P0-1 应用更新核心不可用

`src/bin/wechat-updater.rs` 中 `apply / rollback / recover` 仍被硬编码为：

```text
error: unsupported: real update <action> is disabled in this phase (fixture adapters only)
```

更新检查与只读状态页可用，但真实下载、安装、切换、回退、崩溃恢复没有生产实现。`src/updater/rollback.rs` 也明确拒绝自动数据库回滚（`unsupported_db_rollback`）。因此“应用版本更新”这一生产目标尚未达成。

### P0-2 发布与安装的签名链路不完整

- `.github/workflows/release.yml` 没有 `codesign`、`notarytool` 或 `stapler`，GitHub Release 产物未签名/未公证。
- `scripts/install.sh` 在校验 SHA-256 后无条件执行 `xattr -dr com.apple.quarantine`，且 tar 与 sha256 来自同一 Release，不能抵御同源篡改。
- 专用机当前依赖 `deploy_host.sh` 的本地自签名与人工安装；这条路径可工作，但不等同于可分发、可验证的正式发布链路。

### P0-3 真机验收仍为 partial

`docs/reviews/2026-11-12-device-acceptance.md` 记录：

- 已完成：候选构建、签名、安装、配置保留、启动、健康检查、维护/损坏 journal 启动保护；
- 未完成：登录后的 UI/OCR 观察、命名测试联系人发送、真实更新事务、回滚/恢复、辅助功能与录屏权限保持；
- 9 项真机矩阵（相同消息、多会话、重名改名、未读跳转、同时长语音、人工接管、退出换号、权限保持、连续 7 天）仍为 pending；
- 角色条件化回复仍缺新的 `brain_turns` 实测证据，不能把单元测试或 schema 创建当作真实回复已验证。

### P0-4 生产主线与当前工作区分叉

当前工作区有 13 个提交未进入 main，包含：

- `feat(reply): never drop an inbound message silently (P0 pending replies)`；
- 旧版 Excel `.xls` 读取；
- 下载窗口关闭与“下载到…”处理；
- 主题重绘、OCR 漂移后的回复与基线刷新；
- 调度器空会话/预览基线修复。

这些提交在 main 上没有等价 patch。若它们属于生产范围，必须先合并并重新验收，否则生产版本会缺少这些修复。

## 3. P1 风险

### P1-1 桌面操作无操作级超时（本分支已修复）

原实现取得桌面租约后整段执行且无超时，`osascript` 也使用 `wait_with_output()` 无限等待。任一辅助进程或系统弹窗挂起，都会长期占用进程内互斥锁与跨进程 `desktop.lock`。

本分支在 `src/adapters.rs` 用 `bounded()` 包裹整个桌面操作：聚焦 60 秒、读取 120 秒、发送 180 秒、`WaitForMessage` 为用户超时 + 60 秒。超时返回明确错误、释放进程内/跨进程锁；被丢弃的 future 会触发 helper 进程的 `kill_on_drop` 清理。

限制：超时只能止损，不能回滚已经发生的 UI 动作；发送超时的结果未知，调用方必须核对发送状态而不能盲目重发。当前只有取消/超时语义单测，没有在真机上注入“helper 永久挂起”的进程级回归。

### P1-2 任务 panic 会退出整个 MCP 进程（本分支已修复）

原实现把 `JoinSet` 的非取消错误直接向上返回，单个工具任务 panic 会结束整个服务。本分支改用 `join_next_with_id()` 并维护 task id → 请求 id 映射：panic 时记录日志、返回 JSON-RPC `-32603 Internal error`，进程继续服务；非 panic 的 `JoinError` 仍按原路径退出。

新增单元测试 `panicking_task_response_keeps_request_id`，覆盖字符串/数字请求 id 与不可解析 key。限制：没有进程级“注入真实 panic”的端到端测试；panic 可能留下已中毒的锁或未释放的桌面租约，隔离只能避免整个服务退出，不能恢复被破坏的进程内状态。生产上仍需配合超时与重启策略处理中毒锁/租约。

### P1-3 对外契约与可观测性不足

- 工具列表仍为 0 个 `outputSchema`；
- 错误主要是自然语言文本，缺少稳定的错误码、可重试标记和副作用标记；
- gateway / mcp 层几乎没有结构化 `tracing`，故障定位依赖状态文件。

### P1-4 测试对用户目录不封闭（本分支已修复）

`native/bootstrap/ConfigForm.swift` 的 `getSecureTempDirectory()` 使用 `FileManager.homeDirectoryForCurrentUser`，不读取 `HOME`。本分支在 `tests/swift_host.rs`、`swift_brain_ui.rs`、`swift_update_ui.rs` 中为 Swift 子进程设置 `CFFIXED_USER_HOME` 临时目录，测试不再触碰真实 `~/.wechat-rpa`。updater/UI 测试也不再假设固定的 `target/debug` 路径。仍建议生产代码支持显式注入 state/temp 目录，减少对 Foundation 环境变量的隐式依赖。

### P1-5 Supervisor 崩溃退避上限

`native/bootstrap/Supervisor.swift` 在 1 小时内连续崩溃 5 次后停止自动重启。历史上已出现 core 未恢复、需要人工 `launchctl kickstart` 的情况。生产运维必须明确告警与恢复流程，或调整策略。

## 4. 已确认可用、不要回退

- `record_frame` 与 `record_bot_reply` 已事务化，并带故障注入回归；
- 备份已包含 `media/`、`brain-profiles/`、`brain-bindings.json`、SHA-256 与恢复路径，`--out` 覆盖保护已存在；
- 敏感信息扫描、密钥文件 0600、OpenAPI 拒绝 `0.0.0.0`、Socket 权限等安全边界仍在；
- 大脑角色存储、绑定、界面与更新只读页面已进入 main；
- 桌面操作已有硬超时，超时会释放桌面锁并触发 helper 的 `kill_on_drop` 清理；
- 全量测试在隔离环境下 2250 通过、0 失败；
- 原始工具编译与 self-test 通过。

## 5. 未验证 / Unknown

- 专用机真实微信登录、UI、OCR、发送、人工接管与换号场景；
- 真实更新 apply / rollback / recover 与数据恢复；
- 连续 7 天运行、内存/句柄泄漏、逐帧日志轮转；
- 默认并发下的长期稳定性（本次只跑了一轮全量测试）；
- 双架构 Release 产物与安装后 TCC 权限保持；
- gitleaks 历史扫描（本机未运行）。

## 6. 建议处理顺序

1. 合入本分支的格式化修复，恢复 CI 格式门禁。
2. 明确生产基线，逐项评审并合并当前工作区的 13 个提交；或者明确排除并记录理由。
3. 测试 home 隔离已修复；可选补 state/temp 注入与自定义 `CARGO_TARGET_DIR` 兼容，进一步减少环境假设。
4. 桌面操作超时已补；需在真机验证“helper 挂起”与发送超时后的未知态处理，并继续完善 `outputSchema`/结构化日志。
5. 更新能力二选一：完成真实更新核心与签名验证，或明确首版不支持并在 UI/文档中保持一致。
6. 发布链路补签名/公证，安装器在验签通过前不要无条件去隔离。
7. 在最新 main 上重跑 Q1，并完成真机 M1–M9 与 7 天试运行。

## 7. 复现命令

```bash
# 格式与门禁
cargo fmt --all -- --check
bash scripts/test-gates.sh
bash scripts/check_sensitive.sh
shellcheck -S warning scripts/*.sh scripts/hooks/*

# 构建与测试（隔离用户目录；Swift 子进程由测试 harness 设置 CFFIXED_USER_HOME）
export HOME="$(mktemp -d)"
cargo build --release --locked
cargo test --locked --no-fail-fast

# 严格 lint（文档口径；CI 当前未加 -D warnings）
cargo clippy --all-targets --locked -- -D warnings
```

实际退出码：格式修复前 1、修复后 0；`test-gates` 0；敏感信息 0；shellcheck 0；`build.sh` 0；全量测试 0；CI 口径 clippy 0；严格 clippy 101。
