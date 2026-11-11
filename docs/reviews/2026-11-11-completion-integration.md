# Q2 U1/T1 独立集成审查报告

- **审查日期**：2026-11-11
- **任务编号**：`task_71cb4e8b6b10`（接续 `task_c7643457e405`）
- **Dispatch 标识**：`ctx_413d75ec3376`（接续 `ctx_8e57928ca33c`）
- **审查终端**：`term_bd98bc0b-af90-40dd-bd4b-02827255af5f`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **初始基线**：`1e120b5`（分支 `work/prod-integration-review`）
- **合并提交**：`c4aebee`（合入 `work/finish-test-isolation` 最新 `e70de0b` 与 `work/finish-updater` 最新 `f0c9ecf`）
- **审查定性**：**Partial（未通过完整集成：存在 1 处阻塞级构建编译 RED 缺陷；8 项关键边界仅完成局部代码审查与有限用例断言，不等于全面安全验证；多项核心契约与全量测试因构建阻断处于 Pending 状态）**。

---

## 1. 独立集成合并与依赖核对

### 1.1 分支合并记录
- 合入 `work/finish-test-isolation` 最新提交 `e70de0b`（含 `bc0c87b`、`640f7e8`）：Fast-forward 合入，无冲突；
- 合入 `work/finish-updater` 最新提交 `f0c9ecf`（含完整 U1 阶段，含缺陷 A/B 整改提交 `883cb5f` 与 `90918b5`）：采用 Git `ort` 策略合并，双方修改文件集合完全正交（`comm -12` 结果为空），实现 Clean Merge，无机械冲突与语义冲突；生成合并提交 `c4aebee`。

### 1.2 Cargo 依赖与文件范围审查
- 核对 `Cargo.toml` 变更：
  - `rusqlite` 仅新增 `backup` feature（用于 SQLite 在线快照备份）；
  - 新增 `flate2 = "=1.1.11"`（`default-features = false, features = ["rust_backend"]`）与 `tar = "=0.4.46"`（`default-features = false`）；
  - 新增测试签名二进制 `[[bin]] name = "wechat-test-signer"`；
  - 严格保持无任何非必要外部依赖引入，无 Python 运行时与脚本引入（`./scripts/check_no_python.sh` 退出码 0）；
  - 全局敏感信息静态检查通过（`./scripts/check_sensitive.sh` 退出码 0，无硬编码私钥、Token 或真实主机路径）。

---

## 2. 发现的阻塞级缺陷（RED 退 Owner）

在执行协调者要求的实际 fresh 二进制构建与测试时，发现并定位 1 处致命编译中断缺陷：

### 缺陷 RED-1：`wechat-updater` 二进制引用已废弃函数导致编译失败与 CLI 测试中断

- **代码位置**：[`src/bin/wechat-updater.rs:68`](src/bin/wechat-updater.rs#L68) 与 [`src/bin/wechat-updater.rs:85`](src/bin/wechat-updater.rs#L85)
- **触发命令**：`cargo build --locked --bin wechat-updater`
- **实际退出码**：`101`
- **实际编译报错**：
  ```text
  error[E0425]: cannot find function `set_maintenance_active` in module `journal`
    --> src/bin/wechat-updater.rs:68:38
     |
  68 |                     let _ = journal::set_maintenance_active(
     |                                      ^^^^^^^^^^^^^^^^^^^^^^ not found in `journal`

  error[E0425]: cannot find function `load_current_release_metadata` in module `journal`
     --> src/bin/wechat-updater.rs:85:41
      |
   85 |             let current_meta = journal::load_current_release_metadata(&state_dir)
      |                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ not found in `journal`
     ::: src/updater/journal.rs:556:1
      |
  556 | pub fn load_current_release_record(state_dir: &Path) -> Result<Option<CurrentReleaseCommitRecord>>
  ```
- **根因分析**：
  1. 在 `work/finish-updater` 分支的提交 `219da51`（单一权威提交记录、Tx绑定维护标记与目录保护）及后续提交中，`src/updater/journal.rs` 进行了加固重构：
     - 将原非绑定的简单维护标志 `set_maintenance_active` 废除，替换为绑定 `tx_id` 的原子结构化落盘函数 `set_structured_maintenance_marker`；
     - 将原 `load_current_release_metadata` 重命名并重构为单一权威记录读取函数 `load_current_release_record`；
  2. 但二进制入口 `src/bin/wechat-updater.rs` 未随模块重构同步更新调用；
  3. `work/finish-updater` 交付时仅执行了库级测试 `cargo test --locked --lib updater`，未执行 `cargo build --locked --bin wechat-updater`，导致 CLI 入口处于实质不可编译状态。
- **连锁影响**：
  1. `cargo build --locked --bin wechat-updater` 退出码 101，无法生成目标二进制；
  2. `./scripts/build_release_package.sh` 在 step 42 构建二进制时退出码 101，完整发布包无法组装；
  3. 依赖该二进制的单元测试 `updater::tests::test_cli_rejects_invalid_or_missing_config_without_side_effects` 与 `updater::tests::test_cli_rejects_test_pubkey_and_mutations_without_side_effects` 在寻找 `target/debug/wechat-updater` 时因文件不存在（`Os { code: 2, kind: NotFound }`）直接 panic；
  4. Cargo 在 fresh 构建运行集成测试（`tests/*.rs`）及 `cargo clippy --tests` 时因编译二进制报错而阻断。
- **处理状态**：
  - 严格遵守《Software Engineer》工作规则及协调者指示，审查方不越权篡改业务代码；
  - 协调者已将该 RED 缺陷派发给 U1 Owner（`ctx_16f550ba9692`）修复，当前 worktree 保持原授权边界，不轮询等待。

---

## 3. 门禁与命令验证实际记录

### 3.1 已执行命令与结果记录

| 验证项 | 执行命令 | 实际退出码 | 实际输出/结果摘要 | 定性判定 |
|:---|:---|:---:|:---|:---:|
| **无 Python 检查** | `./scripts/check_no_python.sh` | **0** | `✅ Python 检查通过：项目中无 Python 依赖或脚本` | **PASS** |
| **敏感路径/信息检查** | `./scripts/check_sensitive.sh` | **0** | `✅ 敏感信息检查通过` | **PASS** |
| **代码格式检查** | `cargo fmt --all -- --check` | **0** | 零 diff，代码格式完全符合规范 | **PASS** |
| **更新 CLI 新鲜构建** | `cargo build --locked --bin wechat-updater` | **101** | `cannot find function set_maintenance_active / load_current_release_metadata` | **RED（已退 Owner）** |
| **测试签名器构建** | `cargo build --locked --bin wechat-test-signer` | **0** | 二进制成功生成至 `target/debug/wechat-test-signer` | **PASS** |
| **完整发布包构建** | `./scripts/build_release_package.sh` | **101** | 因依赖 `wechat-updater` 编译失败而中断退出 | **BLOCKED（受 RED-1 阻断）** |
| **更新模块库测试** | `cargo test --locked --lib updater` | **101** | 22 passed, 2 failed（2 项 CLI 二进制调用测试因二进制缺失报 NotFound，其余 22 项纯逻辑与对抗测试通过） | **PARTIAL（CLI 失败）** |
| **全量库测试（默认并发）** | `cargo test --locked --lib` | **101** | 898 passed, 98 failed（2 项为 CLI 缺失；96 项为测试套接字权限失败） | **FAILED（见 3.2 节分析）** |

### 3.2 权限失败核对（96 项失败分析）

在执行 `cargo test --locked --lib`（macOS Darwin 24.3.0 arm64，默认并发线程池，未加 `--test-threads=1`）时，出现 98 项测试失败：
- **CLI 缺失失败（2 项）**：`updater::tests::test_cli_rejects_*` 因二进制文件不存在 panic。
- **沙箱权限失败（96 项）**：集中于 `core::worker_client::tests::*`、`core::open_api::http::tests::*`、`core::control::tests::*` 及 `gateway::tests::*`。测试尝试在 `/tmp` 下创建或 `connect` 本地 Unix domain socket（如 `private GUI bot worker`、`core.sock`）或监听本地 HTTP 端口，在标准沙箱限制下受到系统安全策略拦截，抛出 `PermissionDenied` 或 `Operation not permitted (os error 1)`。
- **定性准则**：按照团队规范，**`PermissionDenied` 必须如实记录为测试失败，严禁擅自归类为“预期通过”**。该 96 项在 Standard Sandbox 模式下定性为 **FAILED（受沙箱权限限制失败）**。

---

## 4. 待复验项清单（Pending List，受 RED-1 阻断）

因 `wechat-updater` 编译失败，Cargo 无法完成包含该 target 的编译管线，以下验证项处于 **Pending（未执行/待复验）** 状态，必须在收到协调者明确修复 commit 后完整复验：

1. **Swift 103 绑定契约测试**：`cargo test --test swift_brain_ui --locked`（**Pending**，受二进制编译中断阻断）；
2. **Swift 92 更新契约测试**：`cargo test --test swift_update_ui --locked`（**Pending**，受二进制编译中断阻断）；
3. **协议烟雾测试**：`cargo test --test protocol_smoke --locked`（**Pending**，受二进制编译中断阻断）；
4. **三轮默认并发核心测试**：`cargo test --bin wechat-mcp` 及相关核心二进制测试（**Pending**，需至少连续运行 3 轮并记录退出码与数量）；
5. **All-target Clippy 检查**：`cargo clippy --tests`（**Pending**，因 `wechat-updater` 编译失败中断退出码 101）；
6. **完整包实际 Manifest / 验签 / 受限解包轮回检查**：
   - 实际运行 `./scripts/build_release_package.sh` 构建出完整 `tar.gz` 与 `manifest.json`；
   - 核验实际 tar 与 manifest 枚举一致性、所有应有 `.App` 与辅助组件完整性；
   - 验证测试签名真实验签与受限 unpack 轮回闭环（**Pending**，因打包脚本 step 42 依赖 updater 编译中断）。

---

## 5. 8 项关键防御边界核验与局限性分析（非全面安全证明）

下述 8 项边界核验仅代表当前在特定单元测试、源码静态实现层面的合规性核查，**不等于端到端全面安全验证**，并明确记录其已知防御局限：

### 5.1 未验签类型不可构造与不可修改 (Verified in Design)
- **代码核验**：[`src/updater/executor.rs:22-52`](src/updater/executor.rs#L22-L52)
- **核验结论**：`VerifiedManifestEnvelope` 的内部字段 `envelope: SignedEnvelope` 私有（无 `pub` 访问符），唯一的公有构造入口为 `verify_and_construct`，强制经过验签与兼容性核验。模块内部保证类型不可外部篡改。

### 5.2 沙箱根构造仅限测试环境 (Verified in Design)
- **代码核验**：[`src/updater/executor.rs:55-120`](src/updater/executor.rs#L55-L120)
- **核验结论**：`SandboxUpdateRoot` 的私有字段及构造器 `create_test_isolated_sandbox` 加装了 `#[cfg(test)]` 条件编译，生产代码中无法构造沙箱根。

### 5.3 归档尾流与 PAX/GNU 预算防御 (Verified in Specific Tests)
- **代码核验**：[`src/updater/unpack.rs:245-280, 485-535`](src/updater/unpack.rs#L245-L280)
- **核验结论**：
  1. Entry 类型限制为常规文件与目录，任何 Symlink、Link 或 PAX/GNU 扩展头直接拦截（`pax_or_gnu_extensions_rejected`）；
  2. 流式解包后校验尾部零填充，遇到非零字节报错；
  3. 检查物理底层文件流（`into_inner()`），读出额外字节则拦截多 member gzip 混杂；
  4. 解包大小与文件数量受限。

### 5.4 父 Symlink 竞态与目录清理受控 (Verified in Specific Tests)
- **代码核验**：[`src/updater/unpack.rs:600-690`](src/updater/unpack.rs#L600-L690)、[`src/core/serve.rs:260-310`](src/core/serve.rs#L260-L310)
- **核验结论**：
  1. 解包与目录创建全程使用 `openat` 结合 `O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC` 逐级校验；
  2. `cleanup_staged_content` 仅根据本次解包组件哈希表逐文件清理，对 `seen_dirs` 由深至浅尝试空目录删除，未采用递归 `remove_dir_all`；
  3. `OwnedSocketDir` 在 `Drop` 时比对 `dev/ino` 物理身份，防止外部重命名后误删非自有目录。

### 5.5 Source 与 Current 受限根校验 (Verified in Specific Tests)
- **代码核验**：[`src/updater/executor.rs:677-705`](src/updater/executor.rs#L677-L705)
- **核验结论**：读取 `current` 软链接时，严格校验目标目录名称符合 `is_valid_release_id` 且物理存在于 `releases_dir` 下，杜绝未知或外部逃逸路径。

### 5.6 Record -> Journal -> 清维护原子提交协议与崩溃窗口恢复 (Verified in Specific Tests)
- **代码核验**：[`src/updater/executor.rs:818-860`](src/updater/executor.rs#L818-L860) 及 [`src/updater/tests.rs:1880-2010`](src/updater/tests.rs#L1880-L2010)
- **核验结论**：自检通过后 journal 先持久化 `CandidateTestingPassed` 与 Proof；随后原子提交 `current_release.json`；权威记录生效后 journal 推进为 `Committed`；最后清理维护标志。测试模拟故障注入表明：record 提交失败严格返回 `ManualInterventionRequired`，崩溃后全要素一致才恢复为 `CompletePendingCommit`。

### 5.7 既有防降级高水位不回 0 (Verified in Specific Tests)
- **代码核验**：[`src/updater/journal.rs:670-690`](src/updater/journal.rs#L670-L690)
- **核验结论**：`load_highest_accepted_sequence` 核验单一权威记录与旧版文件。文件损坏直接抛出 `corrupt_highest_accepted_sequence_file`（fail-closed），绝不高水位静默归零；冲突时抛出 `sequence_conflict_detected` 拒绝更新。

### 5.8 候选自检后代 Pipe 与超时回收及局限性说明 (Partial Defense)
- **代码核验**：[`src/updater/executor.rs:430-550`](src/updater/executor.rs#L430-L550)
- **实现机制**：通过 `libc::setpgid(0, 0)` 将子进程作为独立进程组 leader；子进程退出后向 `-pgid` 发送 `SIGKILL` 清理进程组后代并关闭管道写端；具备 `max_total_drain_bytes` 洪泛硬上限。
- **局限性明确指出（无过度承诺）**：
  1. **不涵盖 `setsid` 逃逸**：若被测子进程派生的孙子进程主动调用 `setsid()` 创建独立 Session 并脱离原进程组，则向原 `-pgid` 发送信号无法杀灭该逃逸进程，该机制无法保证此类场景下的后代回收；
  2. **无毫秒级绝对保证**：进程调度与内核回收受系统负载影响，不能做出绝对毫秒级退出承诺。

---

## 6. 残余边界与当前局限说明（Known Partial Boundaries）

遵循诚实汇报规范，记录当前系统残余边界：

1. **构建中断（RED-1）**：`src/bin/wechat-updater.rs` 引用已废弃签名，导致二进制无法构建、发布脚本中断，已派发 U1 Owner 修复；
2. **生产信任根未配置**：生产环境中执行更新命令保持在 fail-closed 状态（提示 `error: unsupported: trust_root_unconfigured`）；
3. **真实生产排空与 launchd 未接线**：Swift 宿主 App 与 CoreSupervisor 守护进程的真实进程排空与 launchd 重启未接线，当前测试全部基于 Fixture 适配器；
4. **Mock Pass 自检不能代表真实应用通过**：当前更新阶段仅实现了受控解包与沙箱候选机制，通过的自检为受信 Mock Fixture，绝不可声称真实微信 App 自检或真实备份通过；
5. **本地小步审查，零远端推送**：本地阶段合并保留在分支 `work/completion-integration`，未向远端进行任何 git push。
