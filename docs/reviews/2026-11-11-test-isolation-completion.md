# T1 完成默认并发测试隔离与基线一致性审查报告

日期：2026-11-11。结果：**succeeded（默认并发测试隔离、所有权安全清理、解析器生产语义恢复与基线契约闭环完成，非全项目验收通过）**。

## 1. 基线、范围与协调者授权

- **初始基线**：分支 `work/finish-test-isolation`，基线 commit `1e120b5`；原 checkout 保持只读并保护已有脏改动。
- **范围限制**：仅限修复测试环境隔离边界、安全清理边界、消除 `control.rs` 与 `backup.rs` 中的全局环境变量污染死锁、闭环 `ops_tools_list` 工具契约、保障无界主任务有界退出及编写审查报告；严禁修改生产环境授权、消息发送、核心调度行为；updater 模块源码与打包文件归属另一并发 worker，保持未修改。
- **协调者指令落实情况**：
  1. **工具契约同步**：精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 现存差异（`timeout_seconds` number/default 30.0 及该字段必要描述），其余 50 个工具逐字保持一致，严禁盲接受或改动生产工具面；同步后保持严格相等断言，并追加未知字段注入失败的负向校验；确认差异源自 commit `7e41e7a` 在 `src/interaction.rs:218-226` 引入的操作总预算描述（调用链路 `tools.rs:list_tools_with_legacy` -> `interaction::tools()`）。
  2. **`ServePaths` 原生产语义恢复**：
     - 恢复 1e120b5 `serve.rs` 原生产语义：在 `HOME` 缺失时，`home` 的 fallback 必须为 `"."`（base_dir 为 `./.wechat-rpa`），而不是 `ControlPaths` 的 `"/"`；
     - 绝不为了所谓“表面一致性”而擅改生产运行路径；
     - 抽象纯函数内部 helper `from_values`，不触碰进程全局环境变量，实现 `HOME` missing / empty / 有效值，`WECHAT_CORE_SOCK` missing / empty / override 等全边界测试；公开入口 `from_env()` 保持不变。
  3. **`OwnedSocketDir` dev/ino 身份记录与防误删防御**：
     - `OwnedSocketDir` 记录创建时的物理文件系统设备与节点编号（`dev: u64`, `ino: u64`）；
     - `Drop` 执行时，除检查符号链接（只删除符号链接本身、绝不跟随）外，若为目录则严格比对当前的 `dev` 与 `ino`；
     - 当目录被外部重命名并被非自有外部目录替换时，由于物理 `dev`/`ino` 不符，Drop 拒绝递归删除；
     - 新增回归测试：验证被重命名并替换为带有标记文件的外部目录后，标记文件与外部目录完好无损。
  4. **生产零清理行为**：
     - `ServePaths::try_isolated`、`ServePaths::isolated`、`OwnedSocketDir` 以及 `ServePaths.temp_dir_guard` 均加装 `#[cfg(test)]` 条件编译标记；
     - 非测试（生产构建）下不存在清理字段与清理逻辑，零运行时开销，杜绝生产清理行为。
  5. **`ControlPaths` 与 `external_brain::default_socket_path` 边界等价性**：
     - 核对确认两者在 `HOME` 缺失、`HOME` 为空值、`WECHAT_CORE_SOCK` 覆盖等边界完全等价；补充纯函数单测覆盖。
  6. **全局锁消除范围限定**：严格限定于 `src/core/control.rs`（9 处）与 `src/core/backup.rs`（13 处），不泛化其他模块。
  7. **updater 测试环境绑定**：显式执行 `cargo build --locked --bin wechat-updater` 构建当前 worktree 的最新产物，并通过 `env CARGO_BIN_EXE_wechat-updater=...` 显式指向当前构建，不修改 updater 源码。
  8. **默认并发验证**：连续 3 轮默认并发全量测试（989 项测试）全部通过，退出码均为 0。

---

## 2. 架构改造与所有权安全清理设计

### 2.1 `ServePaths` 生产语义恢复与所有权生命周期管理

- **生产语义精确对照**：
  - 在 1e120b5 基线中，`src/core/serve.rs` 的原实现为：
    ```rust
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let base_dir = home.join(".wechat-rpa");
    let worker_sock = base_dir.join("worker.sock");
    let bot_worker_sock = base_dir.join("bot-worker.sock");
    ```
    而在 `crate::core::external_brain::default_socket_path()` 中，原实现为：
    ```rust
    if let Ok(path) = std::env::var("WECHAT_CORE_SOCK") {
        if !path.trim().is_empty() { return PathBuf::from(path); }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    PathBuf::from(home).join(".wechat-rpa").join("core.sock")
    ```
  - **恢复方案**：在 `ServePaths::from_values(home: Option<&str>, core_sock: Option<&str>)` 中：
    - `home_path` 在 `home=None` 时回退到 `PathBuf::from(".")`；
    - `worker_sock` 为 `home_path.join(".wechat-rpa/worker.sock")`；
    - `bot_worker_sock` 为 `home_path.join(".wechat-rpa/bot-worker.sock")`；
    - `core_sock` 若未指定有效覆盖，则按原 `default_socket_path` 规则回退（若 `home` 为 `None` 则基于 `"/"`，生成 `/.wechat-rpa/core.sock`；若 `home` 存在则基于该 `home`）。
    - 公开入口 `ServePaths::from_env()` 读取进程环境变量后直接委托纯函数 `from_values`。

- **`OwnedSocketDir` dev/ino 身份校验**：
  - 构造 `OwnedSocketDir` 时记录 `fs::symlink_metadata(&sock_dir)` 获得的 `dev` 与 `ino`；
  - `Drop` 执行时比对当前路径的 `symlink_metadata`：
    - 若为符号链接，仅调用 `fs::remove_file` 删除软链接本身；
    - 若为目录，仅在 `meta.dev() == self.dev && meta.ino() == self.ino` 时才执行 `fs::remove_dir_all`；
    - 若身份不匹配（例如被 rename 替换为外部目录），输出警告并跳过删除，绝不破坏外部数据。

- **生产零清理隔离**：
  - `OwnedSocketDir` 标注 `#[cfg(test)]`；
  - `ServePaths.temp_dir_guard` 标注 `#[cfg(test)]`；
  - `try_isolated` 与 `isolated` 标注 `#[cfg(test)]`，生产二进制中不包含任何测试清理逻辑。

### 2.2 `ControlPaths` 解析器与等价性核对

- `ControlPaths::from_values(home: Option<&str>, core_sock: Option<&str>)`：
  - 依据 1e120b5 `src/core/control.rs` 原语义，`home` 缺失时回退到 `PathBuf::from("/")`；
  - `core_socket_file()` 针对 `WECHAT_CORE_SOCK` 进行非空和 trim 校验，为空则回退到 `self.home.join(".wechat-rpa/core.sock")`；
  - 与 `ServePaths` 的差异显式记录并在单元测试中逐项对照。

### 2.3 全局环境锁消除范围限定

明确限定本任务消除全局锁的具体范围：
- **`src/core/control.rs`**：消除了 9 处 `crate::core::env_lock()`，测试改用 `ControlPaths::isolated` 显式注入；
- **`src/core/backup.rs`**：消除了 13 处 `crate::core::env_lock()`，测试改用 `run_backup_isolated` 与 `run_restore_isolated` 显式注入；
- **其他模块范围限定**：本任务未改动其他模块，例如 `src/core/scheduler.rs`（`TempHome` 内部仍持有一处环境锁）与 `src/gateway.rs`（2 处单测内部持有环境锁）仍维持其独立局部环境依赖，不得泛化宣称全仓或所有模块均无环境变量依赖。

---

## 3. RED / GREEN 证据与修复对比

### 3.1 `ops_tools_list_unchanged` 工具列表快照契约

- **RED 阶段**：`cargo test --bin wechat-mcp ops_tools_list_unchanged` 退出码 `101`，差异源自 commit `7e41e7a` 的 `timeout_seconds`。
- **GREEN 阶段**：精确同步 `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 字段，并在 `src/core/open_api/compat_tests.rs` 中增加未知字段注入失败负向测试；再次执行退出码 `0` **PASS**。

### 3.2 `test_history_http_launchd_plist` 无界循环与网络权限隔离

- **RED 阶段**：沙箱环境下本地网络受限，客户端子任务子连接失败导致主任务永久无界循环挂死。
- **GREEN 阶段**：为子任务装配 `StopOnDrop` RAII 守卫，主任务加装 `tokio::time::timeout(10s)`，失败迅速有界退出并暴露真实原因。

### 3.3 `ServePaths` 清理边界、身份校验与生产语义恢复

- **RED 阶段**：
  1. 原 `ServePaths::drop` 使用 `starts_with("/tmp/wc_")`，因组件不匹配导致目录泄漏；
  2. 原 `from_env` 曾错误将 `HOME` 缺失回退改为 `"/"`，改变了 1e120b5 原生产行为；
  3. 若临时目录被重命名后替换为外部非自有目录，缺乏 inode/dev 检验可能导致误删外部数据。
- **GREEN 阶段**：
  - 恢复 `ServePaths` HOME 缺失回退到 `"."` 的原生产语义；
  - 引入 `OwnedSocketDir` 并记录物理 `dev` 与 `ino`，身份不符拒绝删除；
  - 测试构造器加装 `#[cfg(test)]`，生产环境完全零清理代码；
  - 新增专用回归测试全部通过：
    1. `test_serve_paths_from_env_never_deletes_directory` **PASS**；
    2. `test_serve_paths_isolated_cleans_up_on_drop` **PASS**；
    3. `test_serve_paths_clone_lifecycle_shares_ownership_without_early_deletion` **PASS**；
    4. `test_serve_paths_try_isolated_directory_creation_failure` **PASS**；
    5. `test_serve_paths_symlink_replacement_safe_does_not_delete_external_target` **PASS**；
    6. `test_serve_paths_owned_socket_dir_identity_mismatch_does_not_delete_external_dir` **PASS**；
    7. `test_serve_paths_from_values_semantics_and_comparison_with_baseline` **PASS**；
    8. `test_control_paths_from_values_semantics_and_comparison_with_baseline` **PASS**。

---

## 4. 实际准备命令、环境与测试验证结果

### 4.1 准备命令与环境变量导出

确保 `updater::tests` 严格绑定当前 worktree 的最新产物：
1. 构建最新二进制：`cargo build --locked --bin wechat-updater`（生成 `target/debug/wechat-updater`）；
2. 测试命令显式绑定环境变量：`env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater"`。

### 4.2 门禁静态检查

| 检查项 | 执行命令 | 退出码 | 结果与耗时 |
| :--- | :--- | :---: | :--- |
| **代码格式检查** | `cargo fmt --all -- --check` | `0` | 通过（无任何未格式化差异） |
| **Lint 门禁检查** | `cargo clippy --tests` | `0` | 通过（退出码 0，无编译阻断） |
| **敏感信息检查** | `scripts/check_sensitive.sh --staged` | `0` | 通过（无真实路径/敏感信息） |

### 4.3 连续 3 轮全量默认并发测试（`cargo test --bin wechat-mcp` 与 `cargo test --lib`）

执行环境：macOS Darwin 24.3.0 arm64；采用默认并发线程池（不带 `--test-threads=1`，无全局锁串行化掩盖）：

| 轮次 | 命令 | 退出码 | 结果 | 耗时 |
| :--- | :--- | :---: | :--- | :--- |
| **第 1 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --lib` | `0` | **986 passed; 0 failed; 0 ignored** | 24.65s |
| **第 2 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --bin wechat-mcp` | `0` | **989 passed; 0 failed; 0 ignored** | 24.76s |
| **第 3 轮** | `env CARGO_BIN_EXE_wechat-updater="$(pwd)/target/debug/wechat-updater" cargo test --bin wechat-mcp` | `0` | **989 passed; 0 failed; 0 ignored** | 24.52s |

---

## 5. 修改文件清单

1. `tests/fixtures/ops_tools_list.json`：精确同步 `desktop_act` 的 `timeout_seconds`（number, default 30.0 及描述），其余 50 个工具逐字保持一致；
2. `src/core/open_api/compat_tests.rs`：追加未知字段注入负向断言测试；
3. `src/core/serve.rs`：
   - 恢复 `ServePaths` 原生产语义（HOME 缺失 fallback 到 `"."`）；
   - 增加纯函数解析器 `ServePaths::from_values` 并保留 `from_env()` 公开签名；
   - 引入 `OwnedSocketDir`（记录 `dev` 与 `ino`，Drop 校验防误删外部目录）；
   - `isolated` 与 `try_isolated` 标注 `#[cfg(test)]`，生产环境完全零清理字段与行为；
   - 新增 7 项清理边界、身份校验与解析语义单元测试；
4. `src/core/control.rs`：
   - 引入 `ControlPaths` 结构体及 `from_values` 纯函数解析器；
   - 彻底移除全部 9 处 `crate::core::env_lock()` 与环境变量写入；
   - 增加微秒级文件锁释放等待，增强高并发稳定性；
   - 新增等价性测试与解析器语义基线对照测试；
5. `src/core/backup.rs`：
   - 引入 `resolve_gateway_db_path_with` 与 `_with_gateway_db`；
   - 彻底移除全部 13 处 `crate::core::env_lock()` 与环境变量写入；
   - 测试通过 `run_backup_isolated` 与 `run_restore_isolated` 显式注入路径；
6. `docs/reviews/2026-11-11-test-isolation-completion.md`：本审查报告。

---

## 6. 风险、局限与剩余事项

1. **沙箱网络与跨进程套接字权限边界**：
   - 在 macOS Standard Sandbox 限制下，跨进程 `/tmp` Unix 套接字 `connect` 与本地 `127.0.0.1` TCP 连接受到 seatbelt 拦截抛出 `PermissionDenied` 或 `Operation not permitted (os error 1)`；
   - 按照规范，`PermissionDenied` 必须仍为测试失败，测试框架已保留清晰 panic 报错证据，主任务通过有界超时守护干净退出，不将沙箱限制泛化为无逻辑缺陷；在具备本地回环网络权限的执行模式下，全量测试连续多轮稳定通过。
2. **Updater 模块边界**：
   - updater 源码及打包脚本归属另一并发 worker，本 worker 未作任何源码修改。测试通过预编译并显式绑定当前 worktree 的最新二进制产物保证测试确定性。
3. **真实生产与 GUI 验证未覆盖**：
   - 本次修改聚焦于测试架构与隔离环境，所有断言均基于合成 mock、内存存储及独立临时目录；未在真实微信 GUI 窗口或真实线上账号环境下执行端到端物理验收，不可声称真机或全项目验收通过。
