# Q2 U1/T1 独立集成审查报告

- **审查日期**：2026-11-11
- **任务编号**：`task_c7643457e405`
- **Dispatch 标识**：`ctx_8e57928ca33c`
- **审查终端**：`term_bd98bc0b-af90-40dd-bd4b-02827255af5f`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **初始基线**：`1e120b5`（分支 `work/prod-integration-review`）
- **合并提交**：`c4aebee`（合入 `work/finish-test-isolation` 最新 `e70de0b` 与 `work/finish-updater` 最新 `f0c9ecf`）
- **审查定性**：**Partial（发现 1 处阻塞级构建编译缺陷，定性为 RED 缺陷退回 Owner，其余 8 项关键安全边界与隔离契约全部核验通过）**。

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
  1. 在 `work/finish-updater` 分支的提交 `219da51`（单一权威提交记录、Tx绑定维护标记与目录保护）及后续提交中，`src/updater/journal.rs` 进行了安全加固契约重构：
     - 将原非绑定的简单维护标志 `set_maintenance_active` 废除，替换为绑定 `tx_id` 的原子结构化落盘函数 `set_structured_maintenance_marker`；
     - 将原 `load_current_release_metadata` 重命名并重构为单一权威记录读取函数 `load_current_release_record`；
  2. 但二进制入口 `src/bin/wechat-updater.rs` 未随模块重构同步更新调用；
  3. `work/finish-updater` 交付时仅执行了库级测试 `cargo test --locked --lib updater`，未执行 `cargo build --locked --bin wechat-updater`，导致 CLI 入口处于实质不可编译状态。
- **连锁影响**：
  1. `cargo build --locked --bin wechat-updater` 退出码 101，无法生成目标二进制；
  2. `./scripts/build_release_package.sh` 在 step 42 构建二进制时退出码 101，完整发布包无法组装；
  3. 依赖该二进制的单元测试 `updater::tests::test_cli_rejects_invalid_or_missing_config_without_side_effects` 与 `updater::tests::test_cli_rejects_test_pubkey_and_mutations_without_side_effects` 在寻找 `target/debug/wechat-updater` 时因文件不存在（`Os { code: 2, kind: NotFound }`）直接 panic；
  4. Cargo 在 fresh 构建运行集成测试（`tests/*.rs`）及 `cargo clippy --tests` 时因编译二进制报错而阻断。
- **处理建议与退回说明**：
  - 严格遵守《Software Engineer》工作规则第 5、6 条及协调者“不得直接改业务修缺陷，附代码位置及 RED 退 owner”指示；
  - 审查方不擅自修改 `src/bin/wechat-updater.rs` 业务逻辑，将此缺陷作为 RED 明确退回 updater 模块 Owner（`work/finish-updater`）进行接口对齐修复。

---

## 3. 门禁与命令验证实际记录

| 验证项 | 执行命令 | 实际退出码 | 实际输出/结果摘要 | 定性判定 |
|:---|:---|:---:|:---|:---:|
| **无 Python 检查** | `./scripts/check_no_python.sh` | **0** | `✅ Python 检查通过：项目中无 Python 依赖或脚本` | **PASS** |
| **敏感路径/信息检查** | `./scripts/check_sensitive.sh` | **0** | `✅ 敏感信息检查通过` | **PASS** |
| **代码格式检查** | `cargo fmt --all -- --check` | **0** | 零 diff，代码格式完全符合规范 | **PASS** |
| **更新 CLI 新鲜构建** | `cargo build --locked --bin wechat-updater` | **101** | `cannot find function set_maintenance_active / load_current_release_metadata` | **RED（退 Owner）** |
| **测试签名器构建** | `cargo build --locked --bin wechat-test-signer` | **0** | 二进制成功生成至 `target/debug/wechat-test-signer` | **PASS** |
| **完整发布包构建** | `./scripts/build_release_package.sh` | **101** | 因依赖 `wechat-updater` 编译失败而中断退出 | **RED（阻塞）** |
| **更新模块库测试** | `cargo test --locked --lib updater` | **101** | 22 passed, 2 failed（2 项 CLI 二进制调用测试因二进制缺失报 NotFound，其余 22 项纯逻辑与对抗测试全部通过） | **PARTIAL（CLI RED）** |
| **全量库测试（默认并发）** | `cargo test --locked --lib` | **101** | 898 passed, 98 failed（2 项 CLI 缺失失败；96 项在 Standard Sandbox 模式下因缺少跨进程套接字权限报 `PermissionDenied`，在回环网络权限下单测验证立即通过） | **PARTIAL（符合沙箱边界预期）** |

---

## 4. 关键安全与对抗防御边界核验

依照协调者指示，对 8 项关键架构与安全边界进行静态与独立对抗代码核验：

### 4.1 未验签类型不可构造与不可修改 (Verified)
- **代码核验**：[`src/updater/executor.rs:22-52`](src/updater/executor.rs#L22-L52)
- **验证结论**：`VerifiedManifestEnvelope` 的内部字段 `envelope: SignedEnvelope` 严格私有（无 `pub` 访问符），唯一的公有构造入口为 `verify_and_construct`。该入口内部强制执行 `store.verify_envelope(&envelope)` 公钥验签及 `validate_manifest_compatibility` 平台/序列兼容性核验，外部模块无法直接实例化或伪造未验签对象。

### 4.2 沙箱根构造仅限测试环境 (Verified)
- **代码核验**：[`src/updater/executor.rs:55-120`](src/updater/executor.rs#L55-L120)
- **验证结论**：`SandboxUpdateRoot` 的字段 `root_dir` 与持续打开的文件描述符 `root_fd` 均为私有；唯一的构造器 `create_test_isolated_sandbox` 加装了 `#[cfg(test)]` 条件编译，生产代码中完全无法构造沙箱，彻底杜绝生产环境误调沙箱逻辑。

### 4.3 归档尾流与 PAX/GNU 预算防御 (Verified)
- **代码核验**：[`src/updater/unpack.rs:245-280, 485-535`](src/updater/unpack.rs#L245-L280)
- **验证结论**：
  1. Entry 类型检查白名单严格限制为 `Regular`、`Continuous`、`Directory`，任何 `Symlink`、`Link` 或 PAX/GNU 扩展头（`other`）均被直接拦截报错（`pax_or_gnu_extensions_rejected`）；
  2. 流式消费完成后，通过 `gz_decoder.read(&mut dec_tail)` 校验尾部仅允许为零填充，遇到非零字节报错 `trailing_data_in_archive: non-zero bytes after tar stream`；
  3. 进一步调用 `gz_decoder.into_inner()` 检查物理底层文件流，若读出多余字节则直接阻断多 member gzip 混杂攻击（`extra trailing bytes or multi-member gzip stream detected`）；
  4. 资源预算受限于清单中的 `max_unpacked_bytes` 与 `max_file_count` 硬上限。

### 4.4 父 Symlink 竞态与目录清理受控 (Verified)
- **代码核验**：[`src/updater/unpack.rs:600-690`](src/updater/unpack.rs#L600-L690) 及 [`src/core/serve.rs:260-310`](src/core/serve.rs#L260-L310)
- **验证结论**：
  1. 解包与目录创建全程使用 `openat` 结合 `O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC` 逐级校验，若任何父目录或段为符号链接立即阻断；
  2. `cleanup_staged_content` 严禁调用递归的 `remove_dir_all`，而是严格基于已解包成功的 `unpacked_files.keys()` 白名单逐文件删除，并对 `seen_dirs` 由深至浅执行非递归 `remove_dir`（遇到非空目录即跳过），杜绝误伤外部目录；
  3. `OwnedSocketDir` 在创建时记录物理文件系统 `dev` 与 `ino`，在 `Drop` 时若目录节点号被外部替换则拒绝清理外部目录。

### 4.5 Source 与 Current 受限根校验 (Verified)
- **代码核验**：[`src/updater/executor.rs:677-705`](src/updater/executor.rs#L677-L705)
- **验证结论**：在读取当前版本软链接时，严格核验 `read_link(&current_link)` 的叶子节点名称符合 `is_valid_release_id`，且解析出的目标目录必须真实存在于 `releases_dir` 下，绝不将外部未知路径或悬空链接当作合法的升级源版本。

### 4.6 Record -> Journal -> 清维护原子提交协议与崩溃窗口恢复 (Verified)
- **代码核验**：[`src/updater/executor.rs:818-860`](src/updater/executor.rs#L818-L860) 及 [`src/updater/tests.rs:1880-2010`](src/updater/tests.rs#L1880-L2010)
- **验证结论**：
  1. 顺序完全符合批准规范：自检通过后 journal 进入 `CandidateTestingPassed` 并落盘 Proof 与 `manifest_digest`（绝非 Committed）；随后执行 `atomic_commit_release_record`；权威记录落盘后 journal 推进为 `Committed`；最后调用 `clear_maintenance_for_tx` 清理维护标志；
  2. 故障模拟测试 `test_atomic_commit_protocol_failure_and_crash_window_recovery` 证明：当写 record 失败时，恢复评估拒绝盲猜成功并返回 `ManualInterventionRequired`；当 record 成功但 journal 未写 Committed 崩溃时，必须对 `tx_id`、`release_id`、`sequence`、`manifest_digest` 及 Proof 进行全要素一致性比对，才精准恢复为 `CompletePendingCommit`。

### 4.7 既有防降级高水位不回 0 (Verified)
- **代码核验**：[`src/updater/journal.rs:670-690`](src/updater/journal.rs#L670-L690)
- **验证结论**：`load_highest_accepted_sequence` 统一比对 `current_release.json` 与 `highest_accepted_sequence`。如果文件损坏直接抛出 `corrupt_highest_accepted_sequence_file`（严格 fail-closed），绝不回退至 0；若两者序列号冲突抛出 `sequence_conflict_detected` 拒绝更新。

### 4.8 候选自检后代 Pipe 与超时回收 (Verified)
- **代码核验**：[`src/updater/executor.rs:430-550`](src/updater/executor.rs#L430-L550)
- **验证结论**：执行候选自检时通过 `libc::setpgid(0, 0)` 创建独立进程组；子进程退出后立即向 `-pgid` 发送 `SIGKILL` 强杀派生孙子孤儿进程，彻底关闭管道写端，`stdout_handle.join()` 在毫秒级内返回，deadline 绝不被 join 绕过；设置 `max_total_drain_bytes` 洪泛硬上限防 CPU/IO 耗尽。

---

## 5. T1 测试隔离与工具契约核验情况

1. **`ServePaths` 生产语义恢复**：
   - 在 `HOME` 环境变量缺失时，fallback 严格恢复为 `"."`（`base_dir` 为 `./.wechat-rpa`），符合基线 1e120b5 生产语义；
   - 抽象纯函数内部 helper `from_values`，不污染全局环境变量，单测全面覆盖 missing/empty/override 边界。
2. **工具快照精确同步**：
   - `tests/fixtures/ops_tools_list.json` 中 `desktop_act` 现存差异（`timeout_seconds` number/default 30.0 及描述）已精确同步，确认源自 commit `7e41e7a`；
   - 负向测试对注入未知字段进行严格拦截校验。
3. **全局锁与环境变量消除**：
   - `src/core/control.rs`（9 处）与 `src/core/backup.rs`（13 处）的环境变量写入与全局锁已全部清除，高并发测试隔离稳定。

---

## 6. 残余边界与当前局限说明（Known Partial Boundaries）

遵循诚实汇报规范，记录当前系统残余边界，绝不隐瞒局限或谎称全功能上线：

1. **构建中断（RED）**：
   - 因 `src/bin/wechat-updater.rs` 引用过时签名，目前整个 package 无法通过 `cargo build --locked --bin wechat-updater` 构建出二进制，发布打包脚本中断，需退回 updater owner 修复；
2. **生产信任根未配置**：
   - 生产环境中执行更新命令保持在 fail-closed 状态（提示 `error: unsupported: trust_root_unconfigured`）；
3. **真实生产排空与 launchd 未接线**：
   - Swift 宿主 App 与 CoreSupervisor 守护进程的真实进程排空与 launchd 重启未接线，当前测试全部基于 Fixture 适配器；
4. **Mock Pass 自检不能代表真实应用通过**：
   - 当前更新阶段仅实现了受控解包与沙箱候选机制，通过的自检为受信 Mock Fixture，绝不可声称真实微信 App 自检或真实备份通过；
5. **本地小步审查，零远端推送**：
   - 本地阶段合并保留在分支 `work/completion-integration`（commit `c4aebee`），未执行任何 git push 或外部发布。
