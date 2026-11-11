# U1 受限解码与 sandbox 候选实现待复核报告（A/B 两缺陷收敛整改版）

- **日期**：2026-11-11
- **任务编号**：`task_5a8dbd5baee1`
- **Dispatch 标识**：`ctx_d963ec4b9d6e`
- **执行终端**：`term_38c3511d-d2e2-49c3-93b5-c6ec86a7fe33`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付范围与状态**：**受限解码与 sandbox 候选实现待复核（明确为 Partial 阶段性收敛，暂不声称安全完成）**。
  - 本轮严格针对协调者指出的两个可复现缺陷（A: record 原子提交协议顺序倒置与崩溃恢复漏洞；B: 候选自检子进程/孙进程挂死与任意命令构造风险）完成独立闭环修复；
  - 生产 CLI 严格保持在安全 fail-closed 状态（退出码 1，提示 unsupported / trust_root_unconfigured）；
  - 零 Python，零真实生产 DB / 钥匙串 / 系统签名副作用，零远端推送。

---

## 1. 本轮两大缺陷整改对照与落地细节

针对协调者第 3 轮审查指示，本轮分别以两个独立小 Commit 完成收敛：

| 缺陷编号 | 协调者审查意见 / 缺陷项 | 整改落地措施 | 对应提交 | 验证状态 |
|:---:|---|---|:---:|:---:|
| **A** | executor 候选成功分支仍 journal.stage=Committed; persist 先于 atomic_commit_release_record，与批准 record->journal->清维护顺序相反；先写测试模拟 record 写/fsync 失败，要求 journal 绝不 Committed、维护不清、恢复不猜成功。调整状态及 record 原子提交协议，测试 record 成功 journal 失败窗口的精确恢复；不能仅 presence proof String 通过，必须 tx+digest+current+sequence 等一致。 | 1. **状态与协议顺序重构**：候选自检通过后，journal 先进入 `CandidateTestingPassed` 并持久化 Proof 与 `manifest_digest`（绝非 Committed）；随后执行 `atomic_commit_release_record`；成功落盘后 journal 才能推进至 `Committed` 并持久化；最后调用 `clear_maintenance_for_tx`。<br>2. **写 record 失败防护验证**：新增 `fail_record_commit` 故障注入测试，断言失败后 journal 停留在 `CandidateTestingPassed`、维护标记保留未清、权威 record 不存在、恢复评估拒绝盲猜成功并返回 `ManualInterventionRequired`。<br>3. **崩溃窗口精确全要素恢复**：新增 `fail_after_record_commit` 测试，当 record 成功但 journal 未写 Committed 崩溃时，恢复评估比对 record 与 journal 的 `tx_id`、`release_id`、`sequence`、`manifest_digest`、`current` 指针及 Proof 证据；全要素完全匹配时精确恢复为 `CompletePendingCommit`；若 sequence/digest 等任一要素被篡改或不匹配，严格返回 `ManualInterventionRequired`。<br>4. **Proof 防伪造加固**：废除空串 presence 校验，反序列化 Proof 真实核验 `tx_id == journal.tx_id` 与 `exit_code == 0`，不符严格拒绝。 | Commit `883cb5f` | **PASS** |
| **B** | SandboxFixtureCandidateAdapter child.kill 仅杀直接子进程，成功/超时后 stdout_handle.join 无界；命令 fork 后代持有 pipe 会无限挂死。先写受控 fixture 父进程退出后子进程持 pipe 及 timeout fork 场景，执行器必须全组自有 process kill+reap/有界读关闭，保证 deadline 不被 join 绕过，超输出不能持续洪泛到无限资源（有总时间/读字节策略）。adapter fields 目前仍 pub+new 任意 absolute 不算硬编码受信 fixture，至少构造器/字段 cfg(test) 并私有，工厂只能预定义 fixture 枚举，不能外部任意命令。 | 1. **自有进程组与全组回收**：通过 `libc::setpgid(0, 0)` 将子进程设置为独立新进程组 leader；实现 `kill_process_group` 向 `-pgid` 发送 `SIGKILL`，确保所有派生子孙进程全部被内核强制杀灭，彻底关闭管道写端。<br>2. **消除无界 Join 挂死**：针对直接子进程退出但孙进程后台持久 hold pipe 的场景，父进程退出后短缓冲读取即向全组发送 SIGKILL，管道写端瞬间关闭，`stdout_handle.join()` 在毫秒级内安全返回，彻底杜绝挂死。<br>3. **有界输出与洪泛硬上限策略**：引入 `max_total_drain_bytes`（硬上限）。即使在丢弃模式下，一旦读满总读取限额，读线程立即触发标记，主线程在 20ms 内强制杀灭进程组报错退出，杜绝持续占用 CPU/IO。<br>4. **受信枚举与字段私有化**：字段全部私有化；公有构造废除任意路径注入，改为 `from_fixture(SandboxCandidateFixtureKind)` 受信枚举（Pass, FailExitCode, TimeoutHang, OrphanForkHoldsPipe, TimeoutForkProcessGroup, OutputFlood）；任意命令构造严格收缩为 `#[cfg(test)] pub(crate) for_test_raw_command`。 | Commit `90918b5` | **PASS** |

---

## 2. 门禁与真实测试执行结果

全部 24 项测试在本地完全通过，相关安全脚本与发布门禁均为退出码 0：

| 验证命令 | 退出码 | 耗时 | 验证结果与证据说明 |
|---|:---:|---:|---|
| `cargo test --locked --lib updater` | **0** | 0.52s | **24 项测试全部 PASS**（新增 `test_atomic_commit_protocol_failure_and_crash_window_recovery` 与加固版进程组/孤儿 pipe/洪泛硬限测试全部通过） |
| `cargo fmt --all -- --check` | **0** | 1.6s | 源码格式完全符合 `rustfmt` 规范，无 diff 输出 |
| `./scripts/check_no_python.sh` | **0** | 0.05s | 检查通过：项目中无 Python 依赖或脚本 |
| `./scripts/check_sensitive.sh` | **0** | 0.05s | 检查通过：无硬编码私钥、Token 或敏感路径泄露 |
| `./scripts/test-gates.sh` | **0** | 0.82s | 构建、发布门禁完整性核验通过 |
| `bash scripts/test-gate-fixtures.sh` | **0** | 0.45s | 门禁回归测试夹具校验通过 |

### 本轮核心加固测试覆盖点清单
1. `test_atomic_commit_protocol_failure_and_crash_window_recovery`（**新增**）：
   - 验证 `fail_record_commit` 发生时：journal 停留在 `CandidateTestingPassed`，权威 record 为 None，维护态保留，恢复评估严格返回 `ManualInterventionRequired`（绝不盲猜成功）；
   - 验证 `fail_after_record_commit` 崩溃窗口：全要素一致时精确识别 `CompletePendingCommit`；篡改 sequence 或 `manifest_digest` 后恢复评估直接拒绝；
   - 验证 Committed 状态下篡改 `drain_proof.tx_id`：恢复评估精准识别并拦截 `drain_proof_tx_mismatch`。
2. `test_candidate_adapter_timeout_output_flood_and_process_cleanup`（**加固**）：
   - 场景 1（直接超时）：200ms 内超时返回，耗时严格受限；
   - 场景 2（超时 fork 孙进程）：孙进程在后台运行，超时精确收割全进程组，deadline 不被 join 绕过；
   - 场景 3（孤儿孙进程持 pipe）：父进程退出后孙进程 hold pipe，毫秒级安全收割，join 绝不挂死；
   - 场景 4（输出洪泛硬上限）：超出 `max_total_drain_bytes` 立即杀组报错，防止无限读空资源耗尽；
   - 场景 5（相对路径逃逸）：`for_test_raw_command("../escape_bin")` 严格静态拒绝。

---

## 3. 残余边界说明（Known Partial Boundaries，暂不声称安全完成）

遵循诚实汇报原则，明确记录当前未接线与局部性边界：

1. **交付定性**：
   - 当前交付仅代表针对缺陷 A 与缺陷 B 的可复现整改收敛，仍定性为**受限解码与 sandbox 候选实现待复核（Partial）**，**暂不声称整体更新事务安全完成**。
2. **生产可信公钥根保持未注入**：
   - 生产环境中执行 `wechat-updater check/apply/recover` 时，因生产可信公钥根未配置，程序严格 fail-closed 退出（状态码 1，提示 `error: unsupported: trust_root_unconfigured`）。
3. **真实生产宿主排空与 launchd 重启未接线**：
   - Swift 宿主 App 与 CoreSupervisor 守护进程的跨进程真实排空与 launchd 重启保持契约未接线状态，不私自修改公共运行时。
4. **数据库变更自动恢复保持禁止**：
   - 遇真实 SQLite 数据库自检失败时保持维护态并标记 `RecoveryRequired`，严禁自动回滚覆盖用户业务数据库。
5. **本地小步提交保障**：
   - 代码变动保留在本地独立分支 `work/finish-updater`（提交 `883cb5f` 与 `90918b5`），未向远端进行任何 git push。
