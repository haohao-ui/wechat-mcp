# U1 完整制品与本地更新事务完成报告

- **日期**：2026-11-11
- **任务编号**：`task_33c8c50ff1d6`
- **Dispatch 标识**：`ctx_e298c1fd2dd5`
- **执行终端**：`term_38c3511d-d2e2-49c3-93b5-c6ec86a7fe33`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付状态**：**已按协调者批准方案分阶段落地完成**（可离线验证的完整应用制品构建、受限归档解码器、加固事务日志状态机与排他锁、强类型沙箱更新执行器、全套恶意对抗包与故障注入测试全部通过；生产 CLI 严格保持前置条件不足时 fail-closed 拒绝）。

---

## 1. 协调者审查要求落实对照

| 阶段 / 要求 | 协调者批复指标 | 落实方案与代码路径 | 验证状态 |
|---|---|---|---|
| **依赖审计** | 精确限定 `flate2 = "=1.1.11"` (`rust_backend`) 与 `tar = "=0.4.46"`，纯 Rust 无 C 绑定，无 Python，兼容 MSRV 1.88 | `Cargo.toml` 与 `Cargo.lock` 精确锁定两 crate，无外部 C 依赖，零 Python 代码 | **已落实 (PASS)** |
| **阶段 1：完整应用构建与制品打包** | 真实 fresh 构建全套产物（禁止仅文件存在跳过）；统一完整组件清单含 `.app`/资源；schema 来自实际支持范围；严格校验参数；测试签名工具生成可验证真实签名，未签包标注 unsigned | 重构 `scripts/build_release_package.sh`，强制 fresh cargo/swiftc 构建 10 个 Rust/Swift 二进制及 `微信客服助手.app`（包含 Info.plist、Resources、wechat-cs-host）；组件清单逐文件实测 SHA-256、mode 与大小；新增 `src/bin/wechat-test-signer.rs` 工具生成隔离测试签名与未签名封套 | **已落实 (Commit `0202bcb`)** |
| **阶段 2：受限流式解码与对抗防护** | 禁止外部 tar 先解后审；同一 FD 校验哈希后 rewind 解码防 TOCTOU；禁止 `entry.unpack`/`archive.unpack`；gzip 全流与单文件落盘双重硬上限；路径/NFC/大小写冲突/白名单精确校验；局部受控清理不任意 remove_dir_all；先 RED 后 GREEN | 重写 `src/updater/unpack.rs` 的 `safe_unpack_and_verify`；引入 `LimitedReader` 限制解压总输出；逐 entry 审查类型、长度、深度、穿越与大小写冲突；`O_CREAT \| O_EXCL` 写入；完成全套对抗测试（软硬链、路径穿越、大小写、大小炸弹、条目炸弹、未知文件、setuid 权限）RED/GREEN 轮回验证 | **已落实 (Commit `7a27872`)** |
| **阶段 3：事务日志排他锁与精准恢复状态机** | 全事务持有跨进程排他锁 `update.lock`；消除 `contains` 模糊匹配，精确规范化比对 current/previous 路径；未知 current 目标严格 fail-closed 进入 `RecoveryRequired`，绝不当旧版默认安全；最高水位与当前 release 元数据原子持久化；fsync 失败 fail closed | 加固 `src/updater/journal.rs`：新增 `acquire_update_lock`（`fs2::FileExt::lock_exclusive`）、`CurrentReleaseMetadata` 持久化结构、`set_maintenance_active`；重构 `evaluate_recovery` 为精确相对路径与叶子名匹配，未知 current 严格返回 `ManualInterventionRequired` | **已落实 (Commit `03cf4c3`)** |
| **阶段 4：强类型沙箱执行器与集成验证** | API 强类型避免“未验签 manifest”直接进入写执行器；所有 sandbox 写运行强制验证专用 marker 且禁止指向生产目录；测试适配器与生产实现严格区分，不模拟成功；生产 CLI 在前置条件未齐前继续 unsupported 拒绝 | 新增 `src/updater/executor.rs`：定义 `VerifiedManifestEnvelope` 强类型、`SandboxUpdateRoot`（安全 marker 与路径防御）及 `SandboxUpdateExecutor`；实现 Committed、CandidateTesting 失败回滚、新数据产生阻止回滚进入维护态等全流程；`wechat-updater` 生产入口严格 fail-closed | **已落实 (Commit `03cf4c3`)** |
| **阶段 5：验证与审查报告** | 运行必要门禁与测试，记录命令退出码与实际结果；提交阶段 commit 本地不 push；不自行整个项目验收通过 | 编写本文档，记录各门禁实际输出与退出码，总结已完成与未接线边界 | **已落实** |

---

## 2. 核心架构与安全机制实现细节

### 2.1 制品构建与规范组件清单（Phase 1）
- **Fresh 构建流程**：在 `scripts/build_release_package.sh` 中统一编译 `wechat-mcp`、`wechat-updater`、`wechat_ocr`、`wechat_click`、`desktop_helper`、`wechat_window`、`wechat_bubbles`、`wechat_runtime`、`wechat_overlay`、`wechat_bot_worker` 及 `wechat-cs-host`。
- **macOS App Bundle 统一归档**：将 `wechat-cs-host`、`Info.plist`、`AppIcon.icns`、`gateway.test.json` 组织入 `Apps/微信客服助手.app`，并与 `resources/gateway.example.json` 和 `bin/*` 一同归档进 `wechat-mcp-<VERSION>-macos-<ARCH>.tar.gz`。
- **清单模式**：`manifest.json` 包含动态实测的 13 个常规文件条目（含 path, kind, file_mode, sha256, size_bytes），设置合理的 `max_unpacked_bytes` 与 `max_file_count` 上限。
- **测试签名工具**：`src/bin/wechat-test-signer.rs` 采用 RFC 8785 JSON 规范化与带域分离前缀 `WECHAT_MCP_UPDATE_MANIFEST_V1\n` 的 Ed25519 签名；测试公钥仅输出到独立 fixture 文件，严禁污染生产信任根。

### 2.2 受限流式归档解码器（Phase 2）
- **防 TOCTOU 攻击**：通过单一 `File::open` 获取文件描述符后，在同一个 FD 上流式校验总字节数与 SHA-256 摘要，校验无误后 `seek(SeekFrom::Start(0))` 倒带该 FD 进行解包，杜绝路径替换竞争漏洞。
- **流式字节硬上限**：解包 reader 嵌套于 `LimitedReader` 之中，无论 gzip 内部是否存在 trailing 垃圾、多 member 拼接或超大解压比，只要未压缩字节流累计超过 `min(expected.max_unpacked_bytes, 500MB)` 立即返回 `unpacked_size_limit_exceeded` 错误。
- **条目安全与路径防穿透**：
  - 仅接受 `Regular`、`Continuous` 与 `Directory` 条目类型，拒绝 `Symlink`、`Link` 及 PAX/GNU 稀疏扩展；
  - 路径限制最大长度 255 字节、目录深度最多 16 层；
  - 严格拒绝绝对路径（`/` 起始）、穿越段（`..`）、反斜杠（`\`）及 NUL 字符；
  - 强制检验 Unicode NFC 规范化及大小写不敏感冲突（`case_collision_detected`）；
  - 严格检查父子路径冲突（已有同名文件不能创建目录，已有目录不能创建同名文件）。
- **落盘与权限控制**：
  - 文件落盘严格使用 `OpenOptions::new().write(true).create_new(true).mode(mode).open(...)`（`O_CREAT | O_EXCL`），避免覆盖文件或跟随父级软链接；
  - 权限模式严格剔除 setuid (`04000`)、setgid (`02000`)、sticky (`01000`) 及组/全局可写位；
  - 逐文件在流式写入过程中核验写入大小不能超过清单值，写完立即 `fsync` 并核对 SHA-256 与 `expected_components` 白名单；
  - 任何错误发生时，仅安全删除已解出的文件与目录，禁止随意 `remove_dir_all` 外部传入的路径。

### 2.3 事务日志排他锁与精准恢复状态机（Phase 3）
- **事务排他锁**：更新启动时调用 `acquire_update_lock`，在 state 目录下以 `0600` 权限打开并持有 `update.lock` 文件的排他锁（`fs2::FileExt::lock_exclusive`），直到整个事务结束；并发更新尝试将直接报错 `concurrent_update_in_progress`。
- **原子日志与 Fail-Closed**：状态迁移通过临时文件写入 -> `fsync` -> `rename` -> 目录 `fsync` 原子持久化，任何一步失败全量 fail closed。
- **恢复状态机消除模糊匹配**：
  - 彻底移除 `target.to_string_lossy().contains(&self.target_release_id)` 模糊匹配；
  - 提取 `current` 软链所指向目标的叶子名与规范化末段，必须精确等于 `target_release_id`（自检恢复）或 `source_release_id`（恢复旧版）；
  - 若指向未知目录或软链损坏，严格返回 `RecoveryAction::ManualInterventionRequired` 并持久激活 `core.maintenance`，禁止静默当旧版安全处理。

### 2.4 强类型沙箱更新执行器（Phase 4）
- **API 强类型防错**：定义 `VerifiedManifestEnvelope`，其私有构造函数仅能在 `verify_and_construct`（包含公钥验签、兼容性核验、防降级校验）成功后返回，编译期阻断未验签 Manifest 进入执行器。
- **沙箱专用根与安全 Marker**：`SandboxUpdateRoot` 强制要求沙箱目录包含 `.sandbox_update_marker`，且校验路径严禁落在系统目录（`/System`、`/Library`、`/Applications`、`/usr`）或生产应用目录（包含 `.wechat-rpa`、`.wechat-mcp`）。
- **故障注入与回滚保护**：在候选自检失败时自动切回 previous 版本；若在切换后检测到已产生新业务数据（`has_new_data_since_backup`），则拒绝静默回退，持久化记录 `RecoveryRequired` 并保持 `core.maintenance` 激活待人工介入。
- **生产 CLI 守卫**：生产环境调用 `wechat-updater apply/rollback/recover` 在缺少真实受信根公钥及系统运行时启动守卫时，继续稳定返回 `error: unsupported`（退出码 1），不伪装模拟成功。

---

## 3. 验证矩阵与命令测试结果

| 测试项 / 脚本 | 退出码 | 耗时 | 实际结果与证据说明 |
|---|---:|---:|---|
| `cargo test --locked --lib updater` | **0** | 0.14s | **16 项测试全部通过**（涵盖签名封套、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、排他锁并发互斥、当前Release元数据持久化、精确恢复状态机、受限流式解包与全套对抗包、沙箱端到端执行与故障注入测试） |
| `cargo test --locked --bin wechat-updater` | **0** | 0.00s | 生产更新器独立二进制单元测试编译与运行通过 |
| `cargo test --locked --bin wechat-test-signer` | **0** | 0.00s | 测试签名独立工具编译与运行通过 |
| `bash scripts/build_release_package.sh 0.2.0-test target/test-dist 1002 test true` | **0** | 35.6s | 真实 fresh 编译全组件产物，成功打包 `wechat-mcp-0.2.0-test-macos-arm64.tar.gz`（9,282,871 字节），生成包含 13 个组件的规范 `manifest.json` 与可离线验证的真实 Ed25519 测试签名封套 `envelope.json` |
| `scripts/check_no_python.sh` | **0** | 0.05s | 检查通过：无 Python 代码运行或生成 |
| `scripts/check_sensitive.sh` | **0** | 0.06s | 检查通过：无本机/个人敏感信息泄露 |
| `scripts/test-gates.sh` | **0** | 0.82s | CI/Release 门禁校验通过 |
| `bash scripts/test-gate-fixtures.sh` | **0** | 0.45s | 门禁夹具校验通过 |

### 对抗测试与先 RED 后 GREEN 证据记录
1. **RED 阶段**：
   - 在未实现流式受限解包前，针对 `safe_unpack_and_verify` 运行真实解包测试：
     ```text
     thread 'updater::tests::test_safe_unpack_and_verify_valid_and_adversarial' panicked at src/updater/tests.rs:221:5:
     Expected valid unpack to succeed: Some(unsupported_archive: streaming archive extraction is disabled in this phase)
     test result: FAILED. 0 passed; 1 failed; 0 ignored
     ```
2. **GREEN 阶段**：
   - 完整实现受限解码器与流式限制后，再次运行完整测试套件：
     ```text
     running 16 tests
     test updater::tests::test_duplicate_json_keys_rejected ... ok
     test updater::tests::test_manifest_compatibility_and_downgrade_prevention ... ok
     test updater::tests::test_database_auto_rollback_is_unsupported ... ok
     test updater::tests::test_manifest_signing_and_verification_domain_separated ... ok
     test updater::tests::test_local_signer_failure_simulation ... ok
     test updater::tests::test_cli_rejects_test_pubkey_and_mutations_without_side_effects ... ok
     test updater::tests::test_safe_verify_staged_dir_rejects_case_and_client_hard_limits ... ok
     test updater::tests::test_safe_verify_staged_dir_valid_fixture_and_whitelist ... ok
     test updater::tests::test_safe_verify_staged_dir_rejects_symlink_and_hardlink ... ok
     test updater::tests::test_cli_rejects_invalid_or_missing_config_without_side_effects ... ok
     test updater::tests::test_sequence_corrupt_file_fails_closed ... ok
     test updater::tests::test_update_lock_exclusive_and_release_metadata ... ok
     test updater::tests::test_has_new_data_strict_errors_on_missing_corrupt_or_unknown_schema ... ok
     test updater::tests::test_journal_atomic_persistence_and_recovery_evaluation ... ok
     test updater::tests::test_safe_unpack_and_verify_valid_and_adversarial ... ok
     test updater::tests::test_sandbox_executor_end_to_end_and_fault_injections ... ok

     test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 963 filtered out; finished in 0.14s
     ```

---

## 4. 本地 Git 提交历史（未向远端 Push）

- `0202bcb` - `feat(updater): 阶段1-完整应用构建打包、全组件清单与测试签名工具`
- `7a27872` - `feat(updater): 阶段2-受控流式归档解码与对抗攻击防护测试`
- `03cf4c3` - `feat(updater): 阶段3与4-事务锁与精准恢复状态机加固、离线沙箱执行器与故障注入验证`

---

## 5. 交付边界与未接线风险说明（已知 Partial / Unknown）

1. **生产可信公钥根配置**：
   - 生产公钥信任根配置（`TrustedKeyStore`）在当前阶段保持未配置状态；
   - 生产 CLI `wechat-updater check` 与写操作继续严格判定为 `trust_root_unconfigured` 并 fail-closed（退出码 1），绝不把测试公钥冒充为生产根。
2. **Swift 宿主 App 与 I4 运行时排空/启动守卫接线**：
   - 当前交付完成了本地更新事务内核、沙箱执行器与完整应用制品打包；
   - 生产环境中 Swift 宿主 `UpdatePage.swift` 与 CoreSupervisor 在实际更新时的主动排空（Draining）、优雅退出与 launchd 重启接线属于跨进程契约演进，本阶段保持明确的接口契约边界，不私自修改公共运行时接口，亦不把未实际接线宣称为完整。
3. **真实系统级副作用保持禁用**：
   - 未执行真实安装当前运行应用；未动用目标机真实钥匙串；未进行真实发布、推送或生产下载。所有写操作均限制在具备安全标记的临时隔离沙箱中。
