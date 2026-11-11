# U1 受限解码与 sandbox 候选实现待复核报告

- **日期**：2026-11-11
- **任务编号**：`task_aff16a9888eb`
- **Dispatch 标识**：`ctx_4e8464daa460`
- **执行终端**：`term_38c3511d-d2e2-49c3-93b5-c6ec86a7fe33`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付范围与状态**：**受限解码与 sandbox 候选实现待复核（严禁宣称本地更新事务完成）**。
  - 代码在独立 worktree 中完成收敛，严格遵循生产更新契约与零破坏性副作用原则；
  - 生产 CLI 保持在安全 fail-closed 状态（退出码 1，提示 unsupported / trust_root_unconfigured）；
  - 全套沙箱适配器（Drain, Backup, Candidate）实行证据模式（Proof-based），能力缺失在第 0 步产生任何写副作用前拒绝；
  - 候选自检严格限制绝对路径受信 fixture 命令，5s 超时强制 kill+wait 收割与 64KB 输出缓冲防死锁；
  - SQLite 备份增加 `verify_backup_restorable`；有 DB 时默认保持维护态 `RecoveryRequired`，严禁自动回滚破坏数据；
  - 根 FD 下子目录安全验证 `verify_subdir_secure_at` 杜绝外部同名软链逃逸；
  - 单一权威提交记录写入规范 RFC 8785 JSON SHA-256 摘要（`manifest_digest`），高水位双源核验防回退回 0；
  - 零 Python，零真实系统副作用，零远端推送。

---

## 1. 协调者审查意见整改对照表

针对协调者第二轮审查（`msg_16cf6e6caef5`）明确指出的架构与安全缺陷，本轮整改已全部落地：

| 序号 | 协调者审查意见 / 缺陷项 | 整改落地措施 | 验证状态 |
|:---:|---|---|:---:|
| 1 | `testing_passed` 默认 `!fault` 且 candidate 为 None 直接成功；有 spec 则 join 任意相对路径执行包内任意程序且无超时输出限制，未执行批准的受信测试适配器预定义绝对命令；缺能力在解包或切换后才报错 | 1. 废除 None 或 `!fault` 宽容默认，实现证明驱动机制（Proof-based）。<br>2. 抽象并实现 `SandboxCandidateAdapter`、`SandboxDrainAdapter`、`SandboxBackupAdapter` 三大受信适配器，测试适配器命名为 `SandboxFixture*`。<br>3. 候选自检适配器强制要求绝对路径命令，实施 5s 超时 kill+wait 回收子进程，设置 64KB 输出缓冲并在超限后持续读空丢弃以防管道死锁或 SIGPIPE。<br>4. 执行器在第 0 步执行能力预检：缺失任何必要适配器立即返回 `unsupported_update_execution`，零写副作用（零解包、零创建新目录、零改动软链）。 | **PASS** |
| 2 | Backup 的 `NoDatabase` 未核验沙箱真实是否存在 DB/WAL；真实 DB 备份无恢复校验 | 1. 在 `SandboxFixtureBackupAdapter` 中，若显式声明 `new_no_database()`，前置核验沙箱内真实不得存在任何 `*.db`、`*.db-wal` 或 `*.db-shm` 文件，否则直接拒绝。<br>2. 实现 `verify_backup_restorable`：在真实 DB 在线备份完成后，以只读模式连接备份文件，执行 `PRAGMA integrity_check;` 并查询 `sqlite_master` 验证其真实可读可恢复，通过后方才签发 `VerifiedSandboxBackupProof`。 | **PASS** |
| 3 | 单一权威 record 的 `manifest_digest` 错标/混淆为 `artifact.sha256`；`highest_accepted_sequence` 损坏时不能直接回 0 | 1. 实现 `calculate_manifest_digest`，严格按照 RFC 8785 规范生成标准 JSON 规范化字节并计算 SHA-256 哈希作为单一权威提交记录中的 `manifest_digest`。<br>2. 高水位读取实现 `load_raw_highest_accepted_sequence` 与 `load_highest_accepted_sequence`：执行旧文本文件与单一权威 `current_release.json` 双源核验；若文件损坏或两方冲突直接 fail closed 报错，绝不静默回退到 0。 | **PASS** |
| 4 | 业务数据变更安全准则违背：候选测试失败后静默回滚覆盖数据库 | 遵循业务数据安全准则：当沙箱中受控存在真实 SQLite 数据库时，首次发布默认视为 `UnknownDataState`，候选自检失败后**保持维护态并标记 `RecoveryRequired`**，严禁自动回滚覆盖用户数据库；仅在严格证明为无 DB 沙箱环境时，才允许安全回退当前程序指向 previous。 | **PASS** |
| 5 | 根 FD 与目标目录校验：目录校验仅按 leaf 或 path 判定，无法杜绝外部同名目录符号链接伪装 | 在 `journal.rs` 中实现 `verify_subdir_secure_at`：通过根目录 `openat(root_fd, "releases", O_RDONLY | O_DIRECTORY | O_NOFOLLOW)` 获取 `releases_dir` 的专属 FD，再在此 FD 下以 `openat(releases_fd, subdir_name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)` 逐级核验目标子目录，从系统内核层面确保证书目标路径绝非软链接且物理位于 releases 目录之内。 | **PASS** |
| 6 | 恢复状态机：`Committed` 状态恢复仅核验 leaf 即可恢复，缺少全要素证明校验 | 加固 `evaluate_recovery`：在进入 `Committed` 恢复时，不仅核验 `verify_subdir_secure_at`，更严格校验 `drain_proof`、`backup_proof`、`candidate_proof` 全要素证明的存在与完整性，缺少证明时严格判定为 `ManualInterventionRequired`。 | **PASS** |

---

## 2. 门禁与测试套件验证结果

全量单元测试与离线安全门禁均在本地干净通过，无任何破坏性副作用：

| 验证命令 | 退出码 | 耗时 | 验证结果与证据说明 |
|---|:---:|---:|---|
| `cargo test --locked --lib updater` | **0** | 0.32s | **23 项单元/集成测试全部 PASS**（新增 4 项深度加固测试全部通过，详见下文） |
| `cargo fmt --all -- --check` | **0** | 1.6s | 源码格式规范，无 diff 输出 |
| `cargo clippy --lib` | **0** | 0.28s | 核心库静态分析通过，updater 模块零 error/warning |
| `./scripts/check_no_python.sh` | **0** | 0.05s | 检查通过：项目中无 Python 依赖或脚本 |
| `./scripts/check_sensitive.sh` | **0** | 0.05s | 检查通过：无硬编码私钥、Token 或本地绝对路径泄露 |
| `./scripts/test-gates.sh` | **0** | 0.85s | 构建、发布流程门禁校验通过 |
| `bash scripts/test-gate-fixtures.sh` | **0** | 0.46s | 门禁夹具完整性核验通过 |

### 完整测试用例清单（23 passed）
1. `test_executor_fails_closed_before_write_if_any_adapter_missing`（**新增**：能力预检拒付，零写副作用验证）
2. `test_candidate_adapter_timeout_output_flood_and_process_cleanup`（**新增**：5s 超时 kill+wait 回收、64KB 缓冲区溢出读空防死锁、相对路径及逃逸拦截）
3. `test_backup_adapter_no_database_and_restore_verification`（**新增**：NoDatabase 虚报扫描拦截、真实 DB 在线热备与 restore 可读性校验）
4. `test_highest_accepted_sequence_conflict_and_recovery_evaluation`（**新增**：高水位损坏/冲突 fail closed 绝不回 0、Committed 缺少证明一律转人工干预）
5. `test_database_auto_rollback_is_unsupported`
6. `test_local_signer_failure_simulation`
7. `test_duplicate_json_keys_rejected`
8. `test_manifest_compatibility_and_downgrade_prevention`
9. `test_manifest_signing_and_verification_domain_separated`
10. `test_recovery_evaluation_rejects_unknown_external_or_invalid_targets`
11. `test_safe_verify_staged_dir_rejects_case_and_client_hard_limits`
12. `test_release_id_validation_and_existing_directory_protection`
13. `test_safe_verify_staged_dir_rejects_symlink_and_hardlink`
14. `test_cli_rejects_invalid_or_missing_config_without_side_effects`
15. `test_sequence_corrupt_file_fails_closed`
16. `test_safe_verify_staged_dir_valid_fixture_and_whitelist`
17. `test_cli_rejects_test_pubkey_and_mutations_without_side_effects`
18. `test_has_new_data_strict_errors_on_missing_corrupt_or_unknown_schema`
19. `test_journal_atomic_persistence_and_recovery_evaluation`
20. `test_update_lock_exclusive_and_release_metadata`
21. `test_safe_unpack_and_verify_valid_and_adversarial`
22. `test_whitelist_candidate_testing_and_exit_code_enforcement`
23. `test_sandbox_executor_end_to_end_and_fault_injections`

---

## 3. 交付边界与未接线项说明（Known Partial / Unknown Boundaries）

遵循工程师诚信守则，明确汇报当前阶段已实现与未接线边界，**不自行宣布整个项目验收通过**：

1. **交付性质**：
   - 本次交付定性为**受限解码与 sandbox 候选实现待复核**；
   - 本地更新事务在受控测试沙箱（`create_test_isolated_sandbox`）中具备全流程验证能力，但在真实生产运行前仍受限于宿主契约对接。
2. **生产可信公钥根未注入**：
   - 生产环境中执行 `wechat-updater check/apply/recover` 时，因未配置生产可信公钥根，程序严格 fail-closed 退出（状态码 1，提示 `error: unsupported: trust_root_unconfigured`），绝不冒充成功。
3. **真实生产宿主排空与 launchd 重启未接线**：
   - 生产环境中与 Swift 宿主 App（`UpdatePage.swift`）以及 CoreSupervisor 守护进程的真实 IPC 排空（Draining）、优雅退出与 launchd 重启，保留契约 Trait 接口，当前生产 CLI 保持未接线 fail-closed 状态，未私自破坏公共运行时。
4. **数据库变更自动恢复严格禁止**：
   - 在缺乏可证明无损还原保证前，数据库回滚保持禁止，自检失败遇真实 DB 保持维护态，需人工介入判定。
5. **零破坏性副作用保障**：
   - 未触碰宿主机任何真实钥匙串（Keychain）；
   - 未修改用户真实 `LaunchAgents`；
   - 未修改生产 WeChat 数据或配置；
   - 零远端 git push。
