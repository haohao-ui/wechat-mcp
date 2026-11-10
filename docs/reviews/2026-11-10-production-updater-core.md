# I5 完整应用版本包与可靠更新核心交付报告（最终收敛版）

日期：2026-11-10
任务：I5 完整应用版本包与可靠更新核心（`task_594d2d76b552` / Dispatch `ctx_34417ffbea80`）
状态：已严格依 Preambles 执行消费 check；删除所有 fallback 配置解析，强制校验完整合法配置；更新器写操作保持禁用，定位为本地基础校验交付；报告实事求是记录测试实际终止原因与基线对比。

---

## 1. 协调者审查要求落实情况

| 序号 | 协调者审查要求 | 落实方案与代码路径 | 状态 |
|---|---|---|---|
| 1 | **读取并消费派发邮件**：依当前 worker preamble 执行 check 消费启动邮件，不要用 `--all` 假装 | 已运行 `orca orchestration check --terminal term_5c30a26b-646d-4000-9bff-e0626e23a471`，成功消费待收邮件（再次 check 显示 `No messages.`）。 | **已落实** |
| 2 | **删除所有配置解析 fallback**：删除 `resolve_state_dir`；只允许 `BotConfig::load` 成功的 `state_dir`；缺失/无权限/坏JSON/非法schema必须非零 `invalid_config` 退出，不读猜测目录，不显示 idle | `src/bin/wechat-updater.rs` 中彻底删除了 `resolve_state_dir` 及其所有宽松 fallback（不猜 parent 目录，不提取任意 JSON）；`status` 与 `check` 统一使用 `BotConfig::load(&cfg_p)`，任何失败均输出 `error: invalid_config` 并退出码 2。 | **已落实** |
| 3 | **修测试 fixture 用合法配置并增加拒绝测试**：不能放松生产校验迁就 fixture；增加 CLI 坏配置、路径不存在拒绝及无写入测试 | `src/updater/tests.rs` 中引入 `write_valid_bot_config`，构造包含 `allowlist`、`schema_version: 1`、`host: local` 且权限 `0600` 的合法配置用于正向测试；新增 `test_cli_rejects_invalid_or_missing_config_without_side_effects`，测试不存在配置、坏 JSON、缺失必填字段，验证均以退出码 2 拒绝且无任何文件写入。 | **已落实** |
| 4 | **客观记录全量测试与基线对比**：给确切失败名、完整命令、退出码与耗时、是否自行终止；与 `d9dbea0` 基线比较，不以通配符代替 | 详见本文档第 3 节，如实记录此前 `cargo test --locked` 因 `worker_client` 等等待外部 socket 超时而由人工调用 `manage_task kill` 自行终止的实际情况，不谎称自然退出码 101；记录基线稳定失败项为 `core::open_api::compat_tests::ops_tools_list_unchanged`。 | **已落实** |
| 5 | **交付定位声明**：标明 partial/unsupported 项；I5 作为禁用真实写入口的本地基础交付，不称完整安装核心完成 | 详见本文档第 4 节，明确将真实写操作（apply/rollback/recover）、真实重签名、自动覆盖恢复旧数据库声明为 `unsupported`。 | **已落实** |

---

## 2. 核心架构契约与实现边界

1. **统一组件清单（ReleaseManifest）与签名封套（SignedEnvelope）**：
   - 封套基于带域分离前缀（`WECHAT_MCP_UPDATE_MANIFEST_V1\n`）的规范化 JSON（RFC 8785）进行 Ed25519 签名；
   - 包含架构（规范化映射 `arm64` 与 `aarch64` 等价）、macOS 12.0+、单调递增 `sequence`（防降级）、组件白名单、权限位（0755/0644）与 SHA-256；
   - 生产环境未配置可信根公钥时严格 fail closed，绝不接受外部测试公钥注入覆盖；
   - 解析时通过 `ensure_no_duplicate_json_keys` 与 `#[serde(deny_unknown_fields)]` 严格拒绝同层重复 JSON 键与未定义嵌套字段。
2. **严格配置读取准则**：
   - `wechat-updater` 严格调用 `BotConfig::load`；
   - 缺失文件、损坏 JSON、非 0600 权限、未知字段、非法 schema 均判定为 `invalid_config` 并退出码 2，杜绝任何将损坏配置误判为空闲或猜测目录的行为。
3. **隔离暂存目录安全审计（Safe Staged Directory Audit）**：
   - 首版禁用外部 tar 调用（`safe_unpack_and_verify` 返回 `unsupported_archive`）；仅针对隔离暂存目录执行独立安全审计；
   - 拒绝软链接、硬链接（`nlink > 1`）、特殊设备文件及非法权限；
   - 拒绝路径穿越（`..`、绝对路径、NUL 字符）；
   - 执行 Unicode NFC 规范化检查与不区分大小写的冲突检查（`case_collision_detected`）；
   - 执行客户端硬上限（`CLIENT_HARD_MAX_UNPACKED_BYTES`: 500MB，`CLIENT_HARD_MAX_FILE_COUNT`: 2000）；
   - 逐文件比对 SHA-256、文件大小和权限位。
4. **数据库保护与回退守卫**：
   - 严格区分程序版本指针回退（`rollback_release_atomic`）与数据回退；
   - `has_new_data_since_backup` 仅用于 fixture 时间戳启发式探测；
   - 在缺少全量 WAL 快照证明前，`restore_database_backup_guarded` 严格报错 `unsupported_db_rollback`，真实回退保持禁用，防止静默覆盖丢失用户数据。
5. **CLI 副作用入口全部关闭**：
   - `wechat-updater apply/rollback/recover` 真实入口均无副作用退出（退出码 1，提示 unsupported）；
   - 绝不修改任何本地状态、钥匙串或当前版本指针。

---

## 3. 验证证据与测试矩阵

### 3.1 核心命令与测试结果

| 命令 | 退出码 | 结果说明 |
|---|---:|---|
| `cargo test --locked --lib updater::tests` | 0 | 14 项更新核心单测全部 PASS（覆盖签名、防降级、安全暂存审计、硬上限、NFC/大小写冲突、DB防覆盖、日志原子持久化、重复 JSON 键拒绝、CLI 选项拒绝与无副作用验证、CLI 坏配置严格拒绝） |
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

### 3.2 默认并行全量测试实测记录与基线对比

1. **实测执行过程说明**：
   - 完整命令：`cargo test --locked`；
   - 运行耗时：前台运行约 4 分钟；
   - 退出状态：**自行终止（手动 kill）**。在执行到 `core::worker_client::tests` 与 `gateway::tests` 的部分用例（如 `test_dead_worker_relaunches_quietly_before_search`、`brain_tools_dispatched_via_gateway`）时，由于用例尝试连接外部本地 socket，在缺少运行中 worker 的沙箱环境下持续等待超时（超过 60 秒），由于没有整体退出，为避免挂起会话由管理工具手动取消，因此**未产生最终 cargo 退出码**；
2. **基线对比（基线提交 `d9dbea0`）**：
   - **确定基线失败项**：`cargo test --locked ops_tools` 执行耗时 0.07s，退出码 101，确切失败测试为 `core::open_api::compat_tests::ops_tools_list_unchanged`（历史工具列表快照不一致断言失败），该失败在 `d9dbea0` 基线代码中完全一致存在；
   - **未归因环境敏感项**：`core::worker_client::tests::*` 等用例在沙箱环境中缺少本地监听服务，属于对环境敏感且耗时漫长的非隔离用例，与本次 I5 纯本地更新器模块无关。
本报告准确记录测试实测过程，不以通配符敷衍，不谎称整个套件自然退出 101。

---

## 4. 交付范围与限制声明（Local Foundation vs Unsupported/Unknown）

- **已完成本地基础校验交付（Local Foundation / Verified）**：
  - 本地测试 Ed25519 签名及 RFC 8785 域分离验签、篡改包与篡改签名拒绝；
  - 架构规范化映射（arm64 / aarch64）、系统版本检查、单调递增 `sequence` 防降级；
  - 隔离暂存目录安全审计：软链拒绝、硬链（`nlink > 1`）拒绝、特殊文件拒绝、Unicode NFC 校验、大小写冲突校验、客户端硬上限（500MB / 2000 文件）超限拒绝；
  - JSON 严格解析：顶层与嵌套对象重复键拒绝（`duplicate_json_key_rejected`）、未定义字段拒绝（`deny_unknown_fields`）；
  - 配置严格拒绝：坏 JSON、缺失必填字段、非法路径均严格以退出码 2（`invalid_config`）拒绝，无任何状态写入副作用；
  - 数据库防覆盖回退保护：损坏、缺失、SQL 错误、未知表严格拒绝；未持有一致 WAL 快照时数据库恢复拒绝（`unsupported_db_rollback`）；
  - 事务日志原子持久化、文件损坏 fail closed、状态读模型 `recovery_required` 判定；
  - 生产 CLI 与 `control` 拒绝外部测试 key 注入，生产无可信根时 fail closed（退出码 1，提示 `trust_root_unconfigured`）；
  - CLI `apply/rollback/recover` 退出码 1（`unsupported`），回归测试证明无任何文件修改副作用。
- **不支持 / 未实现（Unsupported）**：
  - 首版删除外部可执行 tar 解包路径，调用 `safe_unpack_and_verify` 返回 `unsupported_archive`；
  - 真实 `wechat-updater apply/rollback/recover` 入口关闭，返回 `unsupported`（退出码 1）；
  - 自动覆盖恢复旧数据库操作关闭，返回 `unsupported_db_rollback`；
  - 本任务仅作为禁用真实写操作的本地基础安全交付，不宣称完整应用安装或线上自动更新核心已闭环。
- **未验证 / 待真机环境（Unknown）**：
  - 生产私钥严禁入库，生产环境真实根公钥分发与轮换需另行运维配置；
  - 本地 macOS 钥匙串与真实签名证书集成需真机交互授权（本次仅保留本地抽象适配器）。
