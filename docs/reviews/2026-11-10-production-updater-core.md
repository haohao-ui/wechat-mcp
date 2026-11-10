# I5 完整应用版本包与可靠更新核心交付报告（最终修订版）

日期：2026-11-10
任务：I5 完整应用版本包与可靠更新核心（`task_ff20dea66edb` / Dispatch `ctx_bf7ce919eb21`）
状态：已完整消费并核对 `msg_81605e646c4e` 全部审查意见与派发要求；完成生产 CLI/control 测试 key 注入移除与安全加固；13 项核心单测全通过；如实记录默认全量既有基线失败。

---

## 1. 协调者审查要求落实与安全闭环对照表

| 序号 | 协调者审查要求（含 msg_81605e646c4e） | 落实方案与实现代码 | 状态 |
|---|---|---|---|
| 1 | **读取并核销待收邮件**：依 Orca check 读取当前任务待收邮件，逐条确认 `msg_81605e646c4e` 全部要求 | 已通过 `orca orchestration check --terminal ... --all --full` 完整消费 `msg_81605e646c4e` 并逐条落地核实。 | **已落实** |
| 2 | **移除生产 CLI 与 control 公钥注入**：移除 `--test-pubkey-hex` 选项；生产 check 无受信根一律 fail closed 报 `trust_root_unconfigured`（退出码 1），不输出 verified；测试验签仅限 `cfg(test)` | `src/bin/wechat-updater.rs` 彻底删除了 `--test-pubkey-hex` 选项，传入未知参数以退出码 2 拒绝；`check` 在无可信根时退出码 1，输出 `trust_root_unconfigured`；`src/core/control.rs` 彻底移除了 `test_pubkey_hex` 解析，生产检查统一输出 `status: "unverified_trust_root"`，`publisher_verified: false`。 | **已落实** |
| 3 | **补充 CLI 拒绝测试 key 与无副作用回归**：验证 CLI 拒绝测试 key 选项，且 apply/rollback/recover 无任何文件修改副作用 | 在 `src/updater/tests.rs` 中新增 `test_cli_rejects_test_pubkey_and_mutations_without_side_effects`，断言 `--test-pubkey-hex` 退出码 2（`unrecognized argument`）；断言 `check` 退出码 1（`trust_root_unconfigured`）；断言 `apply/rollback/recover` 退出码 1（`unsupported`）且前后文件列表完全不变、无任何副作用。 | **已落实** |
| 4 | **严格区分程序指针回退与数据库回退**：`has_new_data_since_backup` 不能将 false 解释为可安全恢复旧 DB；在无完整一致快照证明前真实回退保持禁用 | 在 `src/updater/rollback.rs` 中将 `has_new_data_since_backup` 严格限定为仅测试 fixture 探测 messages/turns 时间戳；新增 `restore_database_backup_guarded`，在缺乏全量 WAL 快照与全 schema 证明前直接返回 `unsupported_db_rollback` 拒绝执行。真实回退保持禁用。 | **已落实** |
| 5 | **JSON 严格重复键检查与嵌套闭合**：拒绝重复 JSON 键、异常整数及嵌套未知字段 | 在 `src/updater/manifest.rs` 中实现了流式/AST 递归检查器 `ensure_no_duplicate_json_keys` 与 `parse_signed_envelope`，任何层级存在重复 key 均报错 `duplicate_json_key_rejected`；结合结构体 `#[serde(deny_unknown_fields)]` 严格拒绝任何嵌套未知字段。并在 `test_duplicate_json_keys_rejected` 进行了顶层与嵌套重复键验证。 | **已落实** |
| 6 | **签名先于解包与客户端硬上限**：状态流转为 `PublisherVerified` → `Staged`；首版禁用系统 tar；支持客户端硬上限与 NFC/大小写冲突检查 | `safe_unpack_and_verify` 移除外部 tar 调用返回 `unsupported_archive`；仅保留 `safe_verify_staged_dir` 针对隔离暂存目录进行安全审计，强制执行客户端硬上限（500MB / 2000 个文件）、硬链接拒绝（`nlink > 1`）、软链接拒绝、Unicode NFC 规范化检查与大小写冲突检查（`case_collision_detected`）。 | **已落实** |
| 7 | **如实记录默认全量测试与既有基线失败**：报告准确区分纯模块验证与禁用集成，如实保留既有基线失败，不宣称 100% 整体通过 | 详见本文档第 3、4 节，如实记录默认全量测试退出码 101 及 `core::open_api::compat_tests::ops_tools_list_unchanged` 基线失败项。 | **已落实** |

---

## 2. 架构契约与安全边界

1. **统一组件清单（ReleaseManifest）与签名封套（SignedEnvelope）**：
   - 封套基于带域分离前缀（`WECHAT_MCP_UPDATE_MANIFEST_V1\n`）的规范化 JSON（RFC 8785）进行 Ed25519 签名；
   - 包含架构（规范化映射 `arm64` 与 `aarch64` 等价）、macOS 12.0+、单调递增 `sequence`（防降级）、组件白名单、权限位（0755/0644）与 SHA-256；
   - 生产环境未配置可信根公钥时严格 fail closed，绝不接受外部注入覆盖；
   - 解析时严格拒绝同层重复 JSON 键与未定义嵌套字段。
2. **隔离暂存目录安全审计（Safe Staged Directory Audit）**：
   - 首版禁用外部 tar 调用；对传入的暂存目录执行独立安全审计；
   - 拒绝软链接、硬链接（`nlink > 1`）、特殊设备文件及非法权限；
   - 拒绝路径穿越（`..`、绝对路径、NUL 字符）；
   - 执行 Unicode NFC 规范化检查与不区分大小写的冲突检查（`case_collision_detected`）；
   - 执行客户端硬上限（`CLIENT_HARD_MAX_UNPACKED_BYTES`: 500MB，`CLIENT_HARD_MAX_FILE_COUNT`: 2000）；
   - 逐文件比对 SHA-256、文件大小和权限位，任何不一致均拒绝并清理。
3. **数据库新业务数据保护与回退守卫**：
   - 严格区分程序版本指针回退（`rollback_release_atomic`）与数据回退；
   - `has_new_data_since_backup` 仅用于 fixture 探测，其返回 false 不构成恢复旧 DB 的充分条件；
   - 在缺少全量 WAL 快照证明前，`restore_database_backup_guarded` 严格报错 `unsupported_db_rollback`，真实回退保持禁用，防止静默覆盖丢失用户数据。
4. **事务日志（UpdateJournal）与状态读模型**：
   - 原子持久化（temp -> `fsync` -> `rename` -> dir `fsync`），权限 `0600`/`0700`；
   - `load_highest_accepted_sequence` 遇文件损坏严格返回 `Err`，不重置为 0；
   - `status` 命令遇 journal 或 sequence 损坏明确返回 `recovery_required`。
5. **CLI 副作用入口全部关闭**：
   - `wechat-updater apply/rollback/recover` 真实入口均无副作用退出（退出码 1，提示 unsupported）；
   - 绝不修改任何本地状态、钥匙串或当前版本指针。

---

## 3. 验证证据与测试矩阵

### 3.1 核心命令与测试结果

| 命令 | 退出码 | 结果说明 |
|---|---:|---|
| `cargo test --locked --lib updater::tests` | 0 | 13 项更新核心单测全部 PASS（覆盖签名、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、日志原子持久化、重复 JSON 键拒绝、CLI 选项拒绝与无副作用验证） |
| `cargo test --locked --bin wechat-updater` | 0 | wechat-updater 独立可执行文件测试 PASS |
| `cargo test --locked test_control_update_status_and_check` | 0 | control 接口状态与检查查询测试 PASS（未配置公钥根时输出 unverified_trust_root） |
| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 169 项生命周期断言全部 PASS |
| `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 17 项 supervisor 集成测试全部 PASS |
| `scripts/check_no_python.sh` | 0 | 运行时零 Python 门禁通过 |
| `scripts/check_sensitive.sh` | 0 | 敏感信息检查通过 |
| `scripts/test-gates.sh` | 0 | 门禁通过 |
| `bash scripts/test-gate-fixtures.sh` | 0 | 门禁夹具通过 |
| `cargo fmt --all -- --check` | 0 | 代码格式校验完全一致 |
| `cargo clippy --all-targets --locked` | 0 | 编译检查通过（无编译错误） |

### 3.2 默认并行全量测试实际退出码与基线失败声明

根据协调者要求，对全量测试进行实测与审查：
1. **测试退出码**：默认全量测试 `cargo test --locked` 退出码为 `101`（存在既有基线失败，且未隔离的环境用例因无本地 socket/worker 服务而等待超时）；
2. **既有基线失败项**：
   - `core::open_api::compat_tests::ops_tools_list_unchanged`：基线既有工具列表快照不一致失败（退出码 101）；
   - `core::worker_client::tests::*` 与 `gateway::tests::*`：在沙箱默认并行环境下因缺乏隔离本地 mock 服务出现连接超时或中断。
本报告明确记录上述基线与环境失败，不宣称 100% 整体通过。

---

## 4. 风险、限制与未验证项声明（Verified vs Unsupported/Unknown）

- **已验证（Verified）**：
  - 本地测试 Ed25519 签名及 RFC 8785 域分离验签、篡改包与篡改签名拒绝；
  - 架构规范化映射（arm64 / aarch64）、系统版本检查、单调递增 `sequence` 防降级；
  - 隔离暂存目录安全审计：软链拒绝、硬链（`nlink > 1`）拒绝、特殊文件拒绝、Unicode NFC 校验、大小写冲突校验、客户端硬上限（500MB / 2000 文件）超限拒绝；
  - JSON 严格解析：顶层与嵌套对象重复键拒绝（`duplicate_json_key_rejected`）、未定义字段拒绝（`deny_unknown_fields`）；
  - 数据库防覆盖回退保护：损坏、缺失、SQL 错误、未知表严格拒绝；未持有一致 WAL 快照时数据库恢复拒绝（`unsupported_db_rollback`）；
  - 事务日志原子持久化、文件损坏 fail closed、状态读模型 `recovery_required` 判定；
  - 生产 CLI 与 `control` 拒绝外部测试 key 注入，生产无可信根时 fail closed（退出码 1，提示 `trust_root_unconfigured`）；
  - CLI `apply/rollback/recover` 退出码 1（`unsupported`），回归测试证明无任何文件修改副作用。
- **不支持 / 未实现（Unsupported）**：
  - 首版删除外部可执行 tar 解包路径，调用 `safe_unpack_and_verify` 返回 `unsupported_archive`；
  - 真实 `wechat-updater apply/rollback/recover` 入口关闭，返回 `unsupported`（退出码 1）；
  - 自动覆盖恢复旧数据库操作关闭，返回 `unsupported_db_rollback`。
- **未验证 / 待真机环境（Unknown）**：
  - 生产私钥严禁入库，生产环境真实根公钥分发与轮换需另行配置；
  - 本地 macOS 钥匙串与真实签名证书集成需真机交互授权（本次仅保留本地抽象适配器）。
