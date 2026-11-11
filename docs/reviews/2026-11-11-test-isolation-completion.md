# T1 完成默认并发测试隔离与基线一致性审查报告

日期：2026-11-11。结果：**succeeded（默认并发测试隔离、所有权安全清理与基线契约闭环完成，非全项目验收通过）**。

## 1. 基线、范围与协调者授权

- **初始基线**：分支 `work/finish-test-isolation`，基线 commit `1e120b5`；原 checkout 保持只读并保护已有脏改动。
- **范围限制**：仅限修复测试环境隔离边界、安全清理边界、消除 `control.rs` 与 `backup.rs` 中的全局环境变量污染死锁、闭环 `ops_tools_list` 工具契约、保障无界主任务有界退出及编写审查报告；严禁修改生产环境授权、消息发送、核心调度行为；updater 模块源码与打包文件归属另一并发 worker，保持未修改。
- **协调者指令（`msg_b542c49e68dd` 及本轮任务补充指令）**：
  1. 批准精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 现存差异（`timeout_seconds` number/default 30.0 及该字段必要描述），其余 50 个工具逐字保持一致，严禁盲接受或改动生产工具面；同步后保持严格相等断言，并追加未知字段注入失败的负向校验；纠正代码来源引用：确认差异源自 commit `7e41e7a` 在 `src/interaction.rs` 引入的动作总预算与核对描述，而非 `src/tools.rs`（调用链路为 `tools.rs:list_tools_with_legacy` -> `interaction::tools()`）。
  2. 修复 `ServePaths` 清理边界：不得按路径前缀字符串推断所有权；`from_env` 永不清理目录；测试 `isolated` 必须使用私有 owned temp guard 独立持有只自己 `create_dir` 的目录；`Clone` 不得提前删除共享目录；创建目录/设置权限错误不吞；`Drop` 仅清理自有对象，替换/symlink 失败时安全防御不删外部目标。
  3. 核对 `ControlPaths.core_socket_file` 与原 `external_brain::default_socket_path` 在 `HOME` 缺失、`HOME` 为空值、`WECHAT_CORE_SOCK` 覆盖等边界完全等价，补充回归测试。
  4. 报告中全局锁消除必须严格限定实际文件与测试范围（`src/core/control.rs` 与 `src/core/backup.rs`），不得泛化称其他模块无 env 依赖。
  5. updater 依赖的预编译二进制必须在验证前通过 `cargo build --locked --bin wechat-updater` 构建当前 worktree 的最新产物，且测试时显式指向该产物，不能跑其他 checkout 的旧包，不修改 updater 源码。
  6. 默认并发至少连续 3 次全部通过；缺网络权限或沙箱拦截精确记录证据，`PermissionDenied` 必须仍为测试失败，不把沙箱拦截泛化为无逻辑缺陷，不擅自启动真实客户端或宣称真机通过。

---

## 2. 架构改造与所有权安全清理设计

### 2.1 `ServePaths` 所有权与 RAII 安全清理（消灭临时目录泄漏与误删风险）

- **原有缺陷分析**：
  - 原实现尝试使用 `self.core_sock.starts_with("/tmp/wc_")` 判断是否删除父目录；但 Rust 的 `Path::starts_with` 是基于路径组件（Component）匹配，`/tmp/wc_uuid` 的组件为 `["/", "tmp", "wc_uuid"]`，无法匹配组件 `wc_`，导致测试临时套接字目录实际上从未被删除（资源泄漏）；
  - 若生产环境或外部自定义 `WECHAT_CORE_SOCK` 恰好配置在类似前缀目录下，按前缀判断存在严重误删非自有目录的风险；
  - 原 `ServePaths` 派生了 `Clone`，若多个副本持有相同路径并在各自的 Drop 中尝试删除，会导致先 Drop 的副本提前删除仍在使用的共享目录；
  - 目录创建和权限设置原使用了静默忽略 `let _ = fs::create_dir_all(...)`，吞没了潜在的 IO / 权限错误。
- **重构设计**：
  1. **私有 Owned 守卫**：定义私有结构体 `OwnedSocketDir { dir: PathBuf }`；
  2. **所有权隔离与 Clone 安全**：`ServePaths` 内持有 `temp_dir_guard: Option<std::sync::Arc<OwnedSocketDir>>`；
     - `ServePaths::from_env()`：`temp_dir_guard` 设为 `None`，在任何情况下析构均永不清理任何目录；
     - `ServePaths::try_isolated(dir: &Path) -> Result<Self>`：原子性 `create_dir` 新临时套接字目录，显式检查并向上传播 `create_dir` 与 `set_permissions` 的错误；成功后由 `Some(Arc::new(OwnedSocketDir { dir }))` 独立持有；
     - `Clone` 时仅增加 `Arc` 引用计数，保证只要有任何一个副本存活，共享目录就不会被清理；只有所有副本均 Drop 后才触发清理；
  3. **符号链接与外部目录安全防御**：
     - `OwnedSocketDir::drop` 在清理前使用 `fs::symlink_metadata` 检查文件类型；
     - 若目录被外部恶意或意外替换为符号链接（symlink），则仅通过 `fs::remove_file` 删除符号链接本身，绝不递归调用 `remove_dir_all`，确保外部目标目录和文件绝对不被误删。

### 2.2 `ControlPaths` 与 `external_brain::default_socket_path` 边界等价性核对

原 `crate::core::external_brain::default_socket_path` 的核心逻辑为：
```rust
if let Ok(path) = std::env::var("WECHAT_CORE_SOCK") {
    return PathBuf::from(path);
}
let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
home.join(".wechat-rpa/core.sock")
```
`ControlPaths::from_env().core_socket_file()` 的实现为：
```rust
pub fn from_env() -> Self {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
    let core_sock = std::env::var("WECHAT_CORE_SOCK").ok().map(PathBuf::from);
    Self { home, core_sock }
}
pub fn core_socket_file(&self) -> PathBuf {
    if let Some(sock) = &self.core_sock {
        return sock.clone();
    }
    self.home.join(".wechat-rpa/core.sock")
}
```
两者在以下所有边界完全等价：
1. **`WECHAT_CORE_SOCK` 存在**：均直接返回环境变量指定的路径；
2. **`WECHAT_CORE_SOCK` 缺失，`HOME` 有效**：均返回 `$HOME/.wechat-rpa/core.sock`；
3. **`WECHAT_CORE_SOCK` 缺失，`HOME` 为空值 `""`**：均返回 `.wechat-rpa/core.sock`；
4. **`WECHAT_CORE_SOCK` 缺失，`HOME` 缺失**：均回退到 `/`，返回 `/.wechat-rpa/core.sock`。

已在 `src/core/control.rs` 中增加了 `test_control_paths_core_socket_file_equivalence_with_default_socket_path` 覆盖上述所有边界断言。

### 2.3 全局环境锁消除范围限定

明确限定本任务消除全局锁和 `std::env::set_var` 的具体范围：
- **`src/core/control.rs`**：消除了 9 处 `crate::core::env_lock()`，消除了测试中对 `HOME` 与 `WECHAT_CORE_SOCK` 的覆写；
- **`src/core/backup.rs`**：消除了 13 处 `crate::core::env_lock()`，测试改用 `run_backup_isolated` 与 `run_restore_isolated` 显式注入，消除了测试中对 `MESSAGE_GATEWAY_DB` 与 `HOME` 的覆写；
- **其他模块范围说明**：本任务未改动其他模块，例如 `src/core/scheduler.rs`（`TempHome` 内部仍持有一处环境锁）与 `src/gateway.rs`（2 处单测内部持有环境锁）仍维持其独立局部环境依赖，不得泛化宣称全仓或所有模块均无环境变量依赖。

---

## 3. RED / GREEN 证据与修复对比

### 3.1 `ops_tools_list_unchanged` 工具列表快照契约

- **RED 阶段**：
  - 执行：`cargo test --bin wechat-mcp ops_tools_list_unchanged`
  - 退出码：`101`（断言失败）；
  - 证据：差异源于 commit `7e41e7a` 在 `src/interaction.rs:218-226`（注册链路 `tools.rs:list_tools_with_legacy` -> `interaction::tools()`）为 `desktop_act` 引入的 `timeout_seconds`（number, default 30.0 及操作总预算描述），而旧 fixture 缺失该字段导致 pretty JSON 比较失败。
- **GREEN 阶段**：
  - 精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 字段（保持其余 50 个工具逐字符一致）；
  - 在 `src/core/open_api/compat_tests.rs` 中增加未知字段注入失败的负向断言测试；
  - 再次执行：退出码 `0`，耗时 0.01s **PASS**。

### 3.2 `test_history_http_launchd_plist` 无界循环与网络权限隔离

- **RED 阶段**：
  - 原测试在 Standard Sandbox 模式下，因 `127.0.0.1` 本地网络受限，客户端子任务连接失败，主服务处于无界轮询循环导致永久挂死。
- **GREEN 阶段**：
  - 子任务装配 `StopOnDrop` RAII 守卫，主任务加装 `tokio::time::timeout(10s)`；
  - 在受限沙箱中能够迅速在 0.17s 优雅退出并暴露 `PermissionDenied`，在具备本地回环网络权限时瞬间通过。

### 3.3 `ServePaths` 清理边界与生命周期安全

- **RED 阶段**：
  - 原 `ServePaths::drop` 使用 `self.core_sock.starts_with("/tmp/wc_")`，因组件不匹配导致返回值恒为 `false`，临时套接字目录未被删除；且在多 `Clone` 场景下缺乏生命周期引用管理。
- **GREEN 阶段**：
  - 引入 `temp_dir_guard: Option<Arc<OwnedSocketDir>>` 与 `try_isolated`；
  - 新增 5 项专用回归测试全部通过：
    1. `test_serve_paths_from_env_never_deletes_directory` **PASS**；
    2. `test_serve_paths_isolated_cleans_up_on_drop` **PASS**；
    3. `test_serve_paths_clone_lifecycle_shares_ownership_without_early_deletion` **PASS**；
    4. `test_serve_paths_try_isolated_directory_creation_failure` **PASS**；
    5. `test_serve_paths_symlink_replacement_safe_does_not_delete_external_target` **PASS**。

---

## 4. 实际准备命令、环境与测试验证结果

### 4.1 准备命令与环境变量导出

为了确保 `updater::tests` 严格绑定当前 worktree 的最新产物，避免使用旧二进制假绿：
1. 构建最新二进制：`cargo build --locked --bin wechat-updater`（生成 `target/debug/wechat-updater`，时间戳确保为当前构建）；
2. 测试命令显式绑定环境变量：`env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater"`。

### 4.2 门禁静态检查

| 检查项 | 执行命令 | 退出码 | 结果与耗时 |
| :--- | :--- | :---: | :--- |
| **代码格式检查** | `cargo fmt --all -- --check` | `0` | 通过（无任何未格式化差异） |
| **Lint 门禁检查** | `cargo clippy --tests` | `0` | 通过（无编译错误） |

### 4.3 连续 3 轮全量默认并发测试（`cargo test --bin wechat-mcp`）

执行环境：macOS Darwin 24.3.0 arm64；采用默认并发线程池（不带 `--test-threads=1`，无全局锁串行化掩盖）：

| 轮次 | 命令 | 退出码 | 结果 | 耗时 |
| :--- | :--- | :---: | :--- | :--- |
| **第 1 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --bin wechat-mcp` | `0` | **986 passed; 0 failed; 0 ignored** | 24.41s |
| **第 2 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --bin wechat-mcp` | `0` | **986 passed; 0 failed; 0 ignored** | 24.39s |
| **第 3 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --bin wechat-mcp` | `0` | **986 passed; 0 failed; 0 ignored** | 24.44s |

---

## 5. 修改文件清单

1. `tests/fixtures/ops_tools_list.json`：精确同步 `desktop_act` 的 `timeout_seconds`（number, default 30.0 及描述），其余 50 个工具逐字保持一致；
2. `src/core/open_api/compat_tests.rs`：追加未知字段注入负向断言测试；
3. `src/core/serve.rs`：
   - 引入 `OwnedSocketDir` RAII 守卫与 `ServePaths`（持有 `Option<Arc<OwnedSocketDir>>`）；
   - 提供 `try_isolated`（显式传播创建/权限错误）与 `isolated`；
   - 移除不安全的字符串前缀 Drop 判断，实现符号链接安全防御；
   - 新增 5 项清理边界与生命周期回归测试；
4. `src/core/control.rs`：
   - 引入 `ControlPaths` 结构体及 `execute_control_with_paths` / `execute_control_full_with_paths`；
   - 彻底移除全部 9 处 `crate::core::env_lock()` 与环境变量写入；
   - 增加微秒级文件锁释放等待，增强高并发稳定性；
   - 新增 `test_control_paths_core_socket_file_equivalence_with_default_socket_path` 回归测试；
5. `src/core/backup.rs`：
   - 引入 `resolve_gateway_db_path_with` 与 `_with_gateway_db`；
   - 彻底移除全部 13 处 `crate::core::env_lock()` 与环境变量写入；
   - 测试通过 `run_backup_isolated` 与 `run_restore_isolated` 显式注入路径；
6. `docs/reviews/2026-11-11-test-isolation-completion.md`：本审查报告。

---

## 6. 风险、局限与剩余事项

1. **沙箱网络与跨进程套接字权限边界**：
   - 在 macOS Standard Sandbox 限制下，跨进程 `/tmp` Unix 套接字 `connect` 与本地 `127.0.0.1` TCP 连接受到 seatbelt 拦截抛出 `PermissionDenied` 或 `Operation not permitted (os error 1)`；
   - 按照规范，`PermissionDenied` 必须仍为测试失败，测试框架已保留清晰 panic 报错证据，主任务通过有界超时守护干净退出，不将沙箱限制泛化为无逻辑缺陷；
   - 在具备本地回环网络权限的执行模式下，全部 986 项测试稳定通过。
2. **Updater 模块边界**：
   - updater 源码及打包脚本归属另一并发 worker，本 worker 未作任何源码修改。测试通过预编译并显式绑定当前 worktree 的最新二进制产物保证测试确定性。
3. **真实生产与 GUI 验证未覆盖**：
   - 本次修改聚焦于测试架构与隔离环境，所有断言均基于合成 mock、内存存储及独立临时目录；未在真实微信 GUI 窗口或真实线上账号环境下执行端到端物理验收，不可声称真机或全项目验收通过。
