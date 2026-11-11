# U1 完整制品与本地更新事务完成报告（收敛整改版）

- **日期**：2026-11-11
- **任务编号**：`task_688fe2cd5b76`
- **Dispatch 标识**：`ctx_b1ffe8ba41ee`
- **执行终端**：`term_38c3511d-d2e2-49c3-93b5-c6ec86a7fe33`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付状态**：**已按协调者批准整改方案完成 4 个代码功能阶段并经离线全量门禁验证**。
  - 生产 CLI 严格保持在安全 fail-closed 状态（退出码 1，提示 unsupported / trust_root_unconfigured）；
  - 全套恶意对抗包（软硬链、路径穿越、大小写冲突、大小炸弹、条目炸弹、未知文件、非法 setuid 权限、多 member gzip 拼接、尾垃圾消费截断、祖先 symlink 替换）在受控沙箱内全部拦截拒绝；
  - 恢复状态机彻底废除模糊 `contains` 匹配与 `0.0.0-unknown` 默认安全；
  - 数据库备份基于真实 SQLite 在线备份 API（`rusqlite::backup::Backup`）并经 `PRAGMA integrity_check` 与 SHA-256 校验，回滚前检测到新业务数据（messages 表时间戳）真实拒绝自动覆盖并持久保持维护态；
  - 候选自检执行白名单命令与退出码核验；
  - 零 Python，零真实系统副作用，零远端推送。

---

## 1. 协调者批评意见整改对照表

针对第一轮审查指出的 8 项具体技术缺陷，本轮整改已全部完成落地：

| 序号 | 协调者审查意见 / 缺陷项 | 整改落地措施 | 对应提交 | 验证状态 |
|:---:|---|---|:---:|:---:|
| 1 | `VerifiedManifestEnvelope.envelope` 与 `SandboxUpdateRoot.root_dir` 为 pub，可被绕过或外部构造；黑名单与自动 marker 无法防 symlink 伪装 | 将内部字段全部私有化，仅提供只读引用与不可变借用；废除任意路径公有构造，测试仅允许 `create_test_isolated_sandbox()`，原子创建 `0700` 专用临时根并持有 `O_RDONLY \| O_DIRECTORY \| O_CLOEXEC` 根 FD，严格核验 ownership、权限与非 symlink | Commit `e617fd2` | **PASS** |
| 2 | `GzDecoder` 单 member、未消费校验剩余尾流/padding；非 UTF-8 路径 `to_string_lossy` 归一化；缺失 concat members、尾垃圾、祖先 symlink 攻击测试 | 采用受控私有单 FD 不可变副本；使用 `MultiGzDecoder` 并在读完 tar 终止块后循环消费解压尾流，最后核验底层文件残余（严格拒绝多 member 拼接与尾垃圾）；非 UTF-8 路径严格抛错拒绝；逐级使用 `openat + O_NOFOLLOW` 锚定目录与文件，防祖先 symlink 替换 | Commit `2c474fa` | **PASS** |
| 3 | 目标 release 目录已存在时 `remove_dir_all` 覆盖；缺失 release_id 字符集规范化与路径穿越校验 | 增加 `is_valid_release_id`（仅限 ASCII 字母、数字、点、下划线、短横线，长度 1..=64，严禁穿越符与斜杠）；解压与重签名必须在受限隔离暂存目录 `staging/<tx_id>` 中完成；目标路径已存在时**严禁删除或覆盖**，直接抛错 `release_directory_already_exists` 拒绝覆盖 | Commit `219da51` | **PASS** |
| 4 | 状态机恢复仅取 leaf 匹配仍允许外部同名目录；`source_release_id` 遇未知目标以 `0.0.0-unknown` 继续运行 | 恢复评估时校验目标必须在 `releases_dir` 内真实存在且为目录；源版本识别严格解析 current 软链指向，若软链损坏或指向外部/未知目录一律 fail-closed 返回 `ManualInterventionRequired`，废除 `0.0.0-unknown` 宽容默认 | Commit `219da51` | **PASS** |
| 5 | Committed 先落盘再分别写高水位/元数据非原子一致；缺乏单一权威提交记录 | 定义单一权威提交记录 `CurrentReleaseCommitRecord`（落盘至 `current_release.json`，原子绑定 `record_schema`、`tx_id`、`release_id`、`sequence`、`channel`、`manifest_digest`、`installed_at`）；原子更新高水位，定义崩溃自洽恢复规则 | Commit `219da51` | **PASS** |
| 6 | 维护标记未绑定事务，任何进程均可误清除 | 引入结构化维护标记 `MaintenanceMarker`（落盘至 `core.maintenance.json`，强绑定 `tx_id`、原因与时间戳）；清除时必须核验 `clear_maintenance_for_tx` 的 `tx_id` 一致性，不匹配则拒绝清除并报错 | Commit `219da51` | **PASS** |
| 7 | `BackupVerified` 无真实备份与校验；新业务数据仅为模拟布尔标志；候选自检为恒真变量 | 在 `rollback.rs` 中实现 `backup_and_verify_sqlite_db`，采用真实 `rusqlite::backup::Backup` 在线热备份，校验 `PRAGMA integrity_check` 与 SHA-256；删除虚假布尔标志，回滚时真实通过 `has_new_data_since_backup` 查询 SQLite `messages` 表数据时间戳，检测到新数据严格保持维护态并标记 `RecoveryRequired`；候选自检执行真实白名单命令并核验退出码 | Commit `483a030` | **PASS** |
| 8 | 二进制测试输出 "0 tests" 缺乏明确解释；未接线公共能力缺乏清晰说明 | 明确记录二进制本身仅为 CLI 包装，其具体命令行调用逻辑与边界由集成测试通过真实进程 `std::process::Command` 覆盖；清晰陈述生产可信根、Swift 宿主排空及 launchd 重启未接线边界 | Commit `docs` | **PASS** |

---

## 2. 本地 Git 提交历史（严禁向远端 Push）

本次任务在独立工作树 `work/finish-updater`（基线 `1e120b5`）中按小步 Commit 逐步演进提交：

1. `0202bcb` - `feat(updater): 阶段1-完整应用构建打包、全组件清单与测试签名工具`
2. `e617fd2` - `feat(updater): 安全API封装、字段私有化与受限沙箱根FD锚定`
3. `2c474fa` - `feat(updater): 受控私有副本流式解码、openat防symlink与尾垃圾严格校验`
4. `219da51` - `feat(updater): 单一权威提交记录、Tx绑定维护标记与目录保护`
5. `483a030` - `feat(updater): 真实沙箱事务（DB一致备份/真实自检/真实数据检测）与状态隔离`
6. `docs(updater)` - `docs(updater): 收敛版交付报告与格式代码质量验证`

---

## 3. 门禁与命令验证实际结果

| 验证命令 | 退出码 | 耗时 | 实际输出与证据记录 |
|---|:---:|---:|---|
| `cargo test --locked --lib updater` | **0** | 0.25s | **19 项单元/集成测试全部通过**：<br>• `test_database_auto_rollback_is_unsupported`<br>• `test_local_signer_failure_simulation`<br>• `test_duplicate_json_keys_rejected`<br>• `test_manifest_compatibility_and_downgrade_prevention`<br>• `test_manifest_signing_and_verification_domain_separated`<br>• `test_recovery_evaluation_rejects_unknown_external_or_invalid_targets`<br>• `test_safe_verify_staged_dir_rejects_case_and_client_hard_limits`<br>• `test_release_id_validation_and_existing_directory_protection`<br>• `test_safe_verify_staged_dir_rejects_symlink_and_hardlink`<br>• `test_cli_rejects_invalid_or_missing_config_without_side_effects`<br>• `test_sequence_corrupt_file_fails_closed`<br>• `test_safe_verify_staged_dir_valid_fixture_and_whitelist`<br>• `test_cli_rejects_test_pubkey_and_mutations_without_side_effects`<br>• `test_has_new_data_strict_errors_on_missing_corrupt_or_unknown_schema`<br>• `test_journal_atomic_persistence_and_recovery_evaluation`<br>• `test_update_lock_exclusive_and_release_metadata`<br>• `test_safe_unpack_and_verify_valid_and_adversarial`<br>• `test_whitelist_candidate_testing_and_exit_code_enforcement`<br>• `test_sandbox_executor_end_to_end_and_fault_injections` |
| `cargo fmt --all -- --check` | **0** | 1.8s | 代码风格完全符合 `rustfmt` 规范，无 diff 输出 |
| `cargo clippy --lib` | **0** | 0.24s | 核心库编译与静态检查通过，updater 模块零 error |
| `./scripts/check_no_python.sh` | **0** | 0.05s | 检查通过：无 Python 源代码或运行时依赖 |
| `./scripts/check_sensitive.sh` | **0** | 0.05s | 检查通过：无本机路径/私钥敏感信息泄露 |
| `./scripts/test-gates.sh` | **0** | 0.82s | 构建、CI 与发布门禁校验通过 |
| `bash scripts/test-gate-fixtures.sh` | **0** | 0.45s | 门禁夹具校验通过 |
| `bash scripts/build_release_package.sh 0.2.0-test target/test-dist 1002 test true` | **0** | 35.6s | Fresh 编译 10 个二进制产物及 `微信客服助手.app`，实测 SHA-256 生成规范 `manifest.json` 与测试签名封套 |

### 关于二进制测试输出 "0 tests" 的澄清说明
在执行 `cargo test --bin wechat-updater` 时，终端输出为 `running 0 tests; 0 passed; 0 failed`。
这是因为 `src/bin/wechat-updater.rs` 作为二进制入口文件，内部没有定义 `#[test]` 单元测试函数；其完整的命令行解析、配置文件语法错误拦截（退出码 2）、生产缺少公钥根拦截（退出码 1）、以及零副作用断言，均在 `src/updater/tests.rs` 中通过直接调用编译后的真实可执行文件（`std::process::Command::new(target/debug/wechat-updater)`）进行了端到端黑盒测试（参见 `test_cli_rejects_invalid_or_missing_config_without_side_effects` 与 `test_cli_rejects_test_pubkey_and_mutations_without_side_effects`）。

---

## 4. 交付边界与未接线项说明（Known Partial / Unknown Boundaries）

遵循工程师诚信守则，明确汇报当前阶段已实现与未接线边界，**不自行宣布整个项目验收通过**：

1. **生产可信公钥根未注入**：
   - 本阶段代码中，测试密钥仅隔离保存在测试夹具中，绝不写入生产公钥配置；
   - 生产环境中执行 `wechat-updater check/apply/recover` 时，因未配置生产可信公钥根，程序严格 fail-closed 退出（状态码 1，提示 `error: unsupported`），绝不冒充成功。
2. **Swift 宿主排空与 launchd 守护进程重启未接线**：
   - 当前交付仅覆盖核心更新器本地事务状态机、沙箱执行器与应用包打包规范；
   - 生产环境中与 Swift 宿主 App（`UpdatePage.swift`）以及 CoreSupervisor 守护进程的跨进程协调排空（Draining）、优雅退出与 launchd 重启，属于后续运行时架构集成范畴，目前保持契约未接线状态，未私自修改公共运行时接口。
3. **数据库自动破坏性覆盖恢复保持禁止**：
   - 在缺乏全量 WAL 快照与全 schema 加密证明前，`restore_database_backup_guarded` 保持返回 `unsupported_db_rollback` 错误，严禁静默用旧备份覆盖用户数据库。
4. **零破坏性副作用保障**：
   - 本次开发及验证未触碰宿主机任何真实钥匙串（Keychain）；
   - 未修改用户真实 `LaunchAgents`；
   - 未修改生产 WeChat 数据或配置；
   - 所有写入测试均在 `SandboxUpdateRoot::create_test_isolated_sandbox()` 创建的隔离临时目录中完成并在测试结束时彻底销毁。
