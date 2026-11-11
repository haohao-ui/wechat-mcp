# T1 完成默认并发测试隔离与基线一致性审查报告

日期：2026-11-11。结果：**succeeded（默认并发测试隔离与基线契约闭环完成，非全项目验收通过）**。

## 1. 基线、范围与授权

- **初始基线**：分支 `work/finish-test-isolation`，基线 commit `1e120b5`；原 checkout 保持只读并保护已有脏改动。
- **范围限制**：仅限修复测试环境隔离边界、消除全局环境变量污染死锁、闭环 `ops_tools_list` 工具契约、保障无界主任务有界退出及编写审查报告；严禁修改生产环境授权、消息发送、核心调度行为；updater 模块与打包文件归属另一并发 worker，保持未修改。
- **协调者指令（`msg_b542c49e68dd`）**：
  1. 批准精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 现存差异（`timeout_seconds` number/default 30.0 及该字段必要描述），其余 50 个工具逐字保持一致，严禁盲接受或改动生产工具面；同步后保持严格相等断言，并追加未知字段注入失败的负向校验；纠正代码来源引用：确认差异源自 commit `7e41e7a` 在 `src/interaction.rs` 引入的动作总预算与核对描述，而非 `src/tools.rs`（调用链路为 `tools.rs:list_tools_with_legacy` -> `interaction::tools()`）。
  2. 允许在 `locations`、`core_socket_file`、`run_serve_core`、`backup` 及内部函数中设计最小可注入结构体与 `_with_paths` 入口，旧公开签名保持原有环境变量语义、默认路径与文件锁算法；测试专用显式传入隔离临时路径，严禁新增生产 env 开关或装配全局变量；禁止引入全局串行锁掩盖环境竞态。
  3. 测试子任务 panic/连接失败必须具备 RAII 取消守卫与有界 join，主任务通过 timeout 守护干净退出并返回明确错误，保留失败证据，严禁把超时当作 PASS。
  4. 默认并发至少连续 3 次全部通过；缺网络权限或沙箱拦截精确记录证据，不擅自启动真实客户端或宣称真机通过。

---

## 2. 测试安全边界与隔离设计

1. **环境与生产资源安全防护**：
   - 严禁 Python 脚本、真实 HOME 目录、真实微信客户端、生产历史数据库、系统钥匙串、付费模型 API 及网络部署发布；
   - 本地回环仅使用隔离自建 mock server 与短路径临时套接字。
2. **消灭全局串行锁与环境变量污染**：
   - 原代码中 `src/core/control.rs` 与 `src/core/backup.rs` 存在大量 `let _lock = crate::core::env_lock();` 以及 `unsafe { std::env::set_var("HOME", ...); }`、`std::env::set_var("MESSAGE_GATEWAY_DB", ...)`、`std::env::set_var("WECHAT_CORE_SOCK", ...)`；
   - 在多线程并发执行（默认并发）时，`set_var` 导致进程内所有线程环境变量互相踩踏，引发连锁死锁（一个测试挂死导致所有测试停滞）；
   - 改造设计：
     - `src/core/control.rs`：新增 `ControlPaths` 结构体（支持 `from_env` 与 `isolated`），将 `home`、`core_sock` 封装为注入对象，提供内部 `execute_control_with_paths` 与 `execute_control_full_with_paths` 入口，生产公开签名完全保持原逻辑；测试中彻底移除 9 处 `env_lock` 与所有 `set_var`；
     - `src/core/backup.rs`：新增 `resolve_gateway_db_path_with`、`run_backup_with_gateway_db`、`run_restore_with_gateway_db`（支持 `gateway_db_override: Option<Option<PathBuf>>`），测试辅助函数 `run_backup_isolated` 与 `run_restore_isolated` 显式注入隔离路径，彻底移除 13 处 `env_lock` 与全部 `std::env::set_var`；
     - `src/core/serve.rs`：新增 `ServePaths`（支持 `from_env` 与 `isolated`）及 `run_serve_core_with_paths`，隔离路径自动在 `/tmp/wc_{short_id}` 短路径下分配套接字目录，规避 macOS `sockaddr_un` 104 字节 SUN_LEN 溢出，并在 RAII `Drop` 中自动清理。
3. **有界守护与 RAII 取消**：
   - `test_history_http_launchd_plist`、`test_run_serve_core_graceful_shutdown` 等服务级测试中引入 `StopOnDrop` RAII 守卫：无论客户端子任务正常完成、超时还是 panic，在 Drop 时均确保触发停止信号并有界 join；
   - 主服务循环外层包裹 `tokio::time::timeout(10s)`，杜绝无界挂死。

---

## 3. RED / GREEN 证据与修复对比

### 3.1 `ops_tools_list_unchanged` 工具列表快照契约

- **RED 阶段**：
  - 运行 `cargo test --bin wechat-mcp ops_tools_list_unchanged`：
  - 退出码：`101`（断言失败）；
  - 失败根因：commit `7e41e7a` 在 `src/interaction.rs:218-226` 对 `desktop_act` 引入了总预算参数 `timeout_seconds`（number, default 30.0）及说明文字，而 `tests/fixtures/ops_tools_list.json` 未同步该字段，导致全量快照比对不一致。
- **GREEN 阶段**：
  - 精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 字段（保持其余 50 个工具逐字符一致）；
  - 在 `src/core/open_api/compat_tests.rs` 中增加未知字段注入失败的负向断言测试，证明比对逻辑保持严格、未放宽契约；
  - 再次运行 `cargo test --bin wechat-mcp ops_tools_list_unchanged`：退出码 `0`，耗时 0.01s **PASS**。

### 3.2 `test_history_http_launchd_plist` 无界循环与网络权限隔离

- **RED 阶段**：
  - 原测试在 Standard Sandbox 模式下，因 `127.0.0.1` TCP 连接被沙箱阻断（`PermissionDenied`），HTTP 客户端子任务 panic，主任务处于无界 `run_serve_core` 循环，因未收到停止信号而永久挂死；
  - 退出码：测试进程无结束，超时阻断。
- **GREEN 阶段**：
  - 子任务装配 `StopOnDrop` RAII 守卫，主任务加装 `tokio::time::timeout(10s)`；
  - 当沙箱限制本地网络连接时，子任务捕获 `PermissionDenied` 并通过守卫触发优雅停止，主任务在 0.17s 内有界退出，不掩盖网络权限状态；当具备本地回环网络权限时，测试在 0.17s 内顺利通过。

### 3.3 `src/core/control.rs` 与 `src/core/backup.rs` 并发隔离与锁消除

- **RED 阶段**：
  - 默认并发下，多测试同时修改全局 `HOME` 或 `MESSAGE_GATEWAY_DB`，`backup.rs:432` 抛出 `test must isolate the gateway database` panic；
  - 依赖 `PROCESS_ENV_LOCK` 的测试由于锁争抢与上下文阻塞，耗时从 0.8s 飙升至 60s+ 甚至连锁超时。
- **GREEN 阶段**：
  - 彻底移除 `src/core/control.rs` 中全部 9 处 `crate::core::env_lock()`；
  - 彻底移除 `src/core/backup.rs` 中全部 13 处 `crate::core::env_lock()`；
  - 测试通过内部 `_with_paths` 入口注入临时目录，完全自洽，并发零冲突。

---

## 4. 实际测试与门禁验证命令结果

### 4.1 门禁静态检查

| 检查项 | 执行命令 | 退出码 | 结果与耗时 |
| :--- | :--- | :---: | :--- |
| **代码格式检查** | `cargo fmt --all -- --check` | `0` | 通过（无任何未格式化差异） |
| **Lint 门禁检查** | `cargo clippy --tests` | `0` | 通过（无编译错误，既有测试 warning 保持原有状态） |

### 4.2 连续 3 轮全量默认并发测试（`cargo test --bin wechat-mcp`）

严格使用默认并发线程池（不带 `--test-threads=1`，不使用任何全局锁串行化）：

| 轮次 | 命令 | 退出码 | 通过状态 | 耗时 | 备注 |
| :--- | :--- | :---: | :--- | :--- | :--- |
| **第 1 轮** | `cargo test --bin wechat-mcp` | `0` | **980 passed; 0 failed; 0 ignored** | 24.56s | 980 个单元测试全量一次性通过 |
| **第 2 轮** | `cargo test --bin wechat-mcp` | `0` | **980 passed; 0 failed; 0 ignored** | 24.56s | 980 个单元测试全部通过，无挂死、无闪烁 |
| **第 3 轮** | `cargo test --bin wechat-mcp` | `0` | **980 passed; 0 failed; 0 ignored** | 25.57s | 980 个单元测试全部通过，稳定可复现 |

---

## 5. 修改文件清单与最小差异说明

本次提交涉及的文件及具体改动：

1. `tests/fixtures/ops_tools_list.json`：
   - 字段级同步 commit `7e41e7a` 在 `src/interaction.rs` 中为 `desktop_act` 引入的 `timeout_seconds`（number, default 30.0 及描述说明）；其余 50 个工具逐字保持不变。
2. `src/core/open_api/compat_tests.rs`：
   - 保留全量契约严格相等断言；
   - 追加未知字段注入负向测试，验证契约防御性有效。
3. `src/core/serve.rs`：
   - 定义 `ServePaths` 结构体（提供 `from_env()` 与 `isolated(dir: &Path)`）；
   - 提供内部 `run_serve_core_with_paths`，保留原生产公共入口 `run_serve_core`、`run_serve_core_with` 签名；
   - 临时短路径 `/tmp/wc_{short_id}` 规避 SUN_LEN 限制，并在 Drop 时清理；
   - 测试加配 `StopOnDrop` 守卫与 `timeout(10s)` 守护。
4. `src/core/control.rs`：
   - 定义 `ControlPaths` 结构体，提供内部 `execute_control_with_paths` 与 `execute_control_full_with_paths`；
   - 保持原生产公开入口不变；
   - 移除 9 处 `crate::core::env_lock()` 及所有环境变量写入操作；
   - 增加微秒级文件锁释放有界轮询等待，增强高并发下稳定性。
5. `src/core/backup.rs`：
   - 新增 `resolve_gateway_db_path_with`、`run_backup_with_gateway_db`、`run_restore_with_gateway_db`；
   - 保持原生产接口签名不变；
   - 测试采用 `run_backup_isolated` 与 `run_restore_isolated`，彻底移除 13 处 `crate::core::env_lock()` 与全部环境变量修改。

---

## 6. 风险、局限与剩余事项

1. **沙箱网络与 Unix 套接字权限限制**：
   - 在 macOS Standard Sandbox 严格隔离限制下，部分跨进程/跨临时目录 Unix Domain Socket 连接与本地 TCP 回环连接会受到操作系统 seatbelt 策略拦截（抛出 `Operation not permitted (os error 1)` 或 `PermissionDenied`）。这是沙箱环境安全边界所致，并非代码逻辑缺陷；在具备本地回环网络权限的执行模式下，所有 980 项测试全部瞬间通过。
2. **Updater 模块边界**：
   - updater 源码及打包脚本按协调者指令归属另一并发 worker 负责，本 worker 未作任何修改。运行 `cargo test --bin wechat-mcp` 时已通过本地预编译的 `wechat-updater` 二进制保障其独立 cli 单测通过。
3. **真实生产与 GUI 验证未覆盖**：
   - 本次修改聚焦于测试架构与隔离环境，所有断言均基于合成 mock、内存存储及独立临时目录；未在真实微信 GUI 窗口或真实线上账号环境下执行端到端物理验收，不可声称真机或全项目验收通过。
