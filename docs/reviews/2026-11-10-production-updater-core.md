# I5 完整应用版本包与可靠更新核心交付报告（协调者审查意见落实修订版）

日期：2026-11-10
任务：I5 完整应用版本包与可靠更新核心（`task_b8f8480f9384` / Dispatch `ctx_e8ff3f14af4d`）
状态：根据协调者 7 项审查要求彻底闭环，关闭真实写操作入口，强化安全审计与数据库防覆盖保护。

---

## 1. 协调者 7 项审查意见落实对照清单

| 编号 | 协调者审查要求 | 落实情况与代码路径 | 状态 |
|---|---|---|---|
| 1 | **禁用可执行 tar 解包**：首版删除可执行解包路径，返回 `unsupported_archive` 并仅支持隔离合成目录校验；检查流式硬上限、硬链接与压缩炸弹安全 | 在 `src/updater/unpack.rs` 中，`safe_unpack_and_verify` 移除 `/usr/bin/tar` 调用，直接返回 `Err("unsupported_archive")`。仅保留 `safe_verify_staged_dir` 进行隔离暂存目录安全审计。客户端设立硬上限（500MB，2000 个文件），并拒绝软链、硬链（`nlink > 1`）、特殊权限位。 | **已落实** |
| 2 | **关闭 CLI 真实写操作副作用入口**：`wechat-updater apply/rollback/recover` 必须无副作用返回 `unsupported`（退出码 1）；仅 fixture 注入适配器可演练；`recover` 不得删除未知维护事务，损坏时写失败不能宣称已阻止 Supervisor | `src/bin/wechat-updater.rs` 的 `apply`、`rollback`、`recover` 真实入口已全部改为打印 `error: unsupported` 并退出码 1 返回。仅在注入 fixture 的适配器内才允许内存/模拟演练；`recover` 移除了对未知维护事务的删除逻辑，状态评估失败时保持 `recovery_required`。 | **已落实** |
| 3 | **`has_new_data_since_backup` 严格错误处理**：对 SQL 错误、未知表、损坏或缺失 DB 拒绝回退，严禁 `unwrap_or(0)` | `src/updater/rollback.rs` 中执行 `PRAGMA quick_check(1)`，严格校验 `messages` 表存在性；对缺失、损坏、SQL 错误或未知 schema 全部返回 `Err` 拒绝回退，绝不吞错。 | **已落实** |
| 4 | **状态读模型不吞错**：`status` 绝不吞 journal 错误；`highest_accepted_sequence` 损坏不得重置为 0，必须明确报 `recovery_required` | `src/updater/journal.rs` 中 `load_highest_accepted_sequence` 返回 `Result<u64>`，损坏时返回 `Err`；`src/bin/wechat-updater.rs` 与 `src/core/control.rs` 捕获 journal 与 sequence 错误，损坏时标记 `recovery_required`，绝不静默伪装空闲。 | **已落实** |
| 5 | **`check` 生产无可信根时 fail closed**：生产未配置公钥根时必须拒绝，测试 key 仅允许显式注入，禁止硬编码模拟 verified/consistent；同 sequence 标记 `already_installed` | `wechat-updater check` 在无 `--test-pubkey-hex` 且无内置生产公钥时，直接报 `trust_root_unconfigured` 并退出码 1；`control update-check` 亦同；若 sequence 等于当前系统 sequence，明确判定为 `already_installed`。 | **已落实** |
| 6 | **报告诚实客观（Verified vs Unsupported/Unknown）**：逐项审计实际链路，未实现或未验项明确写 unknown/unsupported；保留默认并行全量中既有基线失败，不声称 100% 整体通过 | 详见本文档第 3、4 节，明确将真实更新应用、真实重签名、真实云端下载列为 unsupported/unknown；保留 ops_tools 既有基线失败，不夸大测试范围。 | **已落实** |
| 7 | **安全与规范化细项落实**：签名顺序（域分离前缀优先）、journal 一致性、客户端硬上限、重复 JSON 键、case/NFC 冲突及 history schema 5 | 签名消息以 `WECHAT_MCP_UPDATE_MANIFEST_V1\n` 为域前缀；暂存目录与组件清单均进行 Unicode NFC 格式校验与大小写冲突比对（`case_collision_detected`）；`history_schema_max` 涵盖至 5。 | **已落实** |

---

## 2. 架构契约与实现边界

1. **统一组件清单（ReleaseManifest）与签名封套（SignedEnvelope）**：
   - 包含架构（规范化映射 `arm64` 与 `aarch64` 等价）、macOS 12.0+、单调递增 `sequence`（防降级）、组件白名单、权限位（0755/0644）与 SHA-256；
   - 封套基于 RFC 8785 规范化 JSON 与带前缀域分离（`WECHAT_MCP_UPDATE_MANIFEST_V1\n`），使用 Ed25519 签名；
   - 生产环境未配置可信根时严格 fail closed。
2. **安全暂存目录审计（Safe Staged Directory Audit）**：
   - 禁用 `/usr/bin/tar` 解包；对传入的解包后暂存目录，强制执行安全审计；
   - 拒绝软链接、硬链接（`nlink > 1`）、特殊设备文件及非法权限；
   - 拒绝路径穿越（`..`、绝对路径、NUL 字符）；
   - 执行 Unicode NFC 规范化检查与不区分大小写的冲突检查（`case_collision_detected`）；
   - 执行客户端硬上限（`CLIENT_HARD_MAX_UNPACKED_BYTES`: 500MB，`CLIENT_HARD_MAX_FILE_COUNT`: 2000）；
   - 逐文件比对 SHA-256、文件大小和权限位，任何不一致均拒绝并清理。
3. **数据库新业务数据保护（has_new_data_since_backup）**：
   - 回退前执行 `PRAGMA quick_check(1)` 验证数据库完整性；
   - 严格检查 `messages` 表存在性；对缺失、损坏、SQL 查询错误全部 fail closed 并拒绝回退，严禁吞错静默覆盖。
4. **事务日志（UpdateJournal）与状态读模型**：
   - 原子持久化（temp -> `fsync` -> `rename` -> dir `fsync`），权限 `0600`/`0700`；
   - `load_highest_accepted_sequence` 遇文件损坏严格返回 `Err`，不重置为 0；
   - `status` 命令遇 journal 或 sequence 损坏明确返回 `recovery_required`。
5. **CLI 副作用入口全部关闭**：
   - `wechat-updater apply/rollback/recover` 真实入口均无副作用退出（退出码 1，提示 unsupported）；
   - 仅 fixture 适配器在测试代码中进行受控模拟。

---

## 3. 验证证据与测试矩阵

### 3.1 核心命令与测试结果

| 命令 | 退出码 | 结果说明 |
|---|---:|---|
| `cargo test --locked --lib updater::tests` | 0 | 10 项更新核心单测全部 PASS（覆盖签名、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、日志原子持久化） |
| `cargo test --locked --bin wechat-updater` | 0 | wechat-updater 独立可执行文件测试 PASS |
| `cargo test --locked test_control_update_status_and_check` | 0 | control 接口状态与检查查询测试 PASS |
| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 169 项生命周期断言全部 PASS |
| `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 17 项 supervisor 集成测试全部 PASS |
| `scripts/check_no_python.sh` | 0 | 运行时零 Python 门禁通过 |
| `scripts/check_sensitive.sh` | 0 | 敏感信息检查通过 |
| `scripts/test-gates.sh` | 0 | 门禁通过 |
| `bash scripts/test-gate-fixtures.sh` | 0 | 门禁夹具通过 |
| `cargo fmt --all -- --check` | 0 | 代码格式校验完全一致 |
| `cargo clippy --all-targets --locked` | 0 | 编译检查通过（无编译错误） |

### 3.2 既有基线失败声明

在默认全量测试中，存在既有的 ops_tools 失败项：
- `ops_tools::tests::test_full_diagnostics_bundle_creation_and_redaction`（既有基线行为，涉及本地真实运行日志路径与环境）；
- 网关部分并发测试（如 `gateway::tests::brain_tools_dispatched_via_gateway`）在特定沙箱或单线程环境需外部 socket 超时支持。
本任务严格遵守不隐瞒基线失败的原则，不声称 100% 整体全部通过。

---

## 4. 风险、限制与未验证项声明（Verified vs Unsupported/Unknown）

- **已验证（Verified）**：
  - 本地测试 Ed25519 签名及 RFC 8785 域分离验签、篡改包与篡改签名拒绝；
  - 架构规范化映射（arm64 / aarch64）、系统版本检查、单调递增 `sequence` 防降级；
  - 隔离暂存目录安全审计：软链拒绝、硬链（`nlink > 1`）拒绝、特殊文件拒绝、Unicode NFC 校验、大小写冲突校验、客户端硬上限（500MB / 2000 文件）超限拒绝；
  - 数据库防覆盖回退保护：损坏、缺失、SQL 错误、未知表严格拒绝；
  - 事务日志原子持久化、文件损坏 fail closed、状态读模型 `recovery_required` 判定；
  - `control update-status` 与 `update-check` 只读输出契约及相同 sequence 识别。
- **不支持 / 未实现（Unsupported）**：
  - 首版删除外部可执行 tar 解包路径，调用 `safe_unpack_and_verify` 返回 `unsupported_archive`；
  - 真实 `wechat-updater apply/rollback/recover` 入口关闭，返回 `unsupported`（退出码 1），防止在无监督或不可靠环境中造成不可逆的本地环境变更。
- **未验证 / 待真机环境（Unknown）**：
  - 生产私钥严禁入库，生产环境真实根公钥分发与轮换需另行配置；
  - 本地 macOS 钥匙串与真实签名证书集成需真机交互授权（本次仅保留本地抽象适配器）。
