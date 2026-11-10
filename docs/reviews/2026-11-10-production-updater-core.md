# I5 完整应用版本包与可靠更新核心交付报告

日期：2026-11-10
任务：I5 完整应用版本包与可靠更新核心（`task_93a3a35199ed` / Dispatch `ctx_7107a4be78a8`）
状态：本地实现完成并通过全量单测与合成包演练；未进行生产发布、部署、推送或真实微信操作。

---

## 1. 基线、文件所有权与集成范围

### 1.1 基线与集成提交
- 派发前基线：`32bb12e550b6739c5c7d6874bbd7388cd771ef5c`；
- 本 worktree 起始 commit：`d9dbea02b4c7f84bcde7d796af55ea93a98cb49b`（已包含 I1/I2 角色存储与运行接入）；
- 协调者前置依赖集成：
  - D2 更新与界面设计：`afed6b1134256988b3613f5ee475ea9fab8bd133` → 已本地 commit `d474c96`；
  - I4 监督器协调与本地维护标志：`b3c1365cba57e6f4ba5e94843eb086024bfb3fab` 与 `bc4cd1107afdfe297eecc35bbcb7c1ed292af924` → 已本地 commit `9f3874b`。

### 1.2 严格遵循文件所有权与约束
- **独占与新增文件**：
  - `src/updater/mod.rs`、`src/updater/manifest.rs`、`src/updater/unpack.rs`、`src/updater/journal.rs`、`src/updater/signing.rs`、`src/updater/rollback.rs`、`src/updater/tests.rs`；
  - `src/bin/wechat-updater.rs`；
  - `src/lib.rs`；
  - `scripts/build_release_package.sh`；
  - `docs/design/production-updater-contract.md`；
  - 本交付报告 `docs/reviews/2026-11-10-production-updater-core.md`。
- **允许修改的受控入口**：
  - `Cargo.toml`（显式注册 bin 与 lib，引入既有锁定传递依赖 ring 0.17.14 用于 Ed25519 验签）；
  - `src/main.rs`（暴露 updater 模块）；
  - `src/core/control.rs`（仅添加 `update-status` / `update-check` 只读查询，不改既有行为）；
  - `scripts/build.sh`、`scripts/install.sh`、`.github/workflows/release.yml`（仅打包构建指令更新）；
- **未触碰边界**：
  - 严禁修改任何 I3/I4 Swift 文件；
  - 严禁重写现有调度、发送状态机、账号隔离、白名单、模型连接或外脑模式；
  - 零 Python 引入；无真实网络下载、无生产密钥泄露、无真实部署。

---

## 2. 核心架构与功能实现

1. **统一组件清单（Manifest）与签名封套（SignedEnvelope）**：
   - 实现了受控闭合字段的 `ReleaseManifest`，包含架构（arm64 / x86_64）、最低 macOS 版本（12.0）、单调递增 `sequence`（防降级）、组件白名单及其模式（0755/0644）与 SHA-256；
   - 签名封套基于 RFC 8785 规范化 JSON 与带前缀域分离（`WECHAT_MCP_UPDATE_MANIFEST_V1\n`），使用内置受信任根公钥表验证 Ed25519 签名；
   - 拒绝对未知 key_id、被篡改字段或非法签名的信任。
2. **安全解包（Safe Unpack）与防压缩炸弹**：
   - 递归解包后进行沙箱级全量审计；
   - 严禁路径穿越（`..`、绝对路径、NUL 字符）；
   - 严禁软链接（symlink）和硬链接，严禁 FIFO/设备等特殊文件；
   - 严格限制最大解包文件数与总展开字节数；
   - 解包后逐文件比对 SHA-256 和权限位，多余未列文件或摘要不符全量清理暂存并拒绝更新。
3. **更新事务状态机（UpdateJournal）与断电恢复**：
   - 实现原子持久化写入（临时文件写入 -> `fsync` -> `rename` -> 父目录 `fsync`），权限受控为 `0600`/`0700`；
   - 状态机覆盖 `Staged`、`PublisherVerified`、`LocalSigned`、`Draining`、`BackupVerified`、`Switching`、`CandidateTesting`、`Committed`、`RollbackStarted`、`RolledBack`、`RecoveryRequired`；
   - 断电重启后根据状态机严格评估恢复动作，损坏或不一致状态立即 fail closed，保留 `core.maintenance` 标志，防止 Supervisor 盲目拉起核心。
4. **版本指针原子切换与数据保护回退**：
   - 实现原子切换 `current` 指针指向 `releases/<new_version>` 并保留 `previous`；
   - 联动 `core::backup::run_backup` 在停机前生成完整 v3 备份；
   - 核心防数据丢失保障：回退前检测数据库自备份后是否产生了新业务数据（`has_new_data_since_backup`），若已有新数据，严禁静默覆盖恢复旧 DB，转入 `RecoveryRequired` 保持维护暂停。
5. **本地代码重签名适配器（LocalSigner）**：
   - 适配专用钥匙串本机证书重签名，严格先校验发布者原始签名再进行本地重签名；
   - 签名后立即执行 `codesign --verify` 校验；钥匙串锁定或证书异常时保留旧版本，退出码非零。
6. **独立更新器命令行（`wechat-updater`）与控制读模型**：
   - 独立更新器支持 `status`、`check`、`apply`、`rollback`、`recover`；
   - `wechat-mcp control` 扩展了 `update-status` 和 `update-check`（支持传入本地受控清单），为后续 I6 UI 提供统一读模型。

---

## 3. 验证证据与测试矩阵

### 3.1 实际执行命令与退出码

| 命令 | 退出码 | 结果说明 |
|---|---:|---|
| `cargo test --lib updater::tests` | 0 | 8 项更新核心单测全部 PASS |
| `cargo test --bin wechat-mcp updater::tests` | 0 | 8 项集成单测全部 PASS |
| `cargo test --bin wechat-mcp test_control_update_status_and_check` | 0 | control 接口查询测试 PASS |
| `swiftc native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests && /tmp/wechat-lifecycle-tests` | 0 | 169 项生命周期断言全部 PASS |
| `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 17 项 supervisor 既有回归测试全部 PASS |
| `scripts/check_no_python.sh` | 0 | 产品运行时无 Python 门禁通过 |
| `scripts/check_sensitive.sh` | 0 | 敏感信息检查通过 |
| `scripts/test-gates.sh` | 0 | 门禁通过 |
| `bash scripts/test-gate-fixtures.sh` | 0 | 门禁夹具通过 |
| `cargo fmt --all -- --check` | 0 | 代码格式校验完全一致 |
| `cargo clippy --all-targets --locked` | 0 | 编译检查无错误 |
| `bash scripts/build_release_package.sh 0.2.0-test dist/releases/0.2.0-test test-root-v1` | 0 | 本地合成发布包生成成功 |
| `target/release/wechat-updater --version` | 0 | 独立更新器可执行文件输出 0.1.0 |

---

## 4. 风险、限制与未验证项声明（Verified vs Unknown）

- **已验证（Verified）**：
  - 本地测试 Ed25519 签名及 RFC 8785 域分离验签、篡改包与篡改签名拒绝；
  - 架构不匹配、系统版本低于要求、`sequence` 小于等于历史最高值的防降级拒绝；
  - 路径穿越、软链、硬链、超大包、压缩炸弹、多余文件与 SHA-256 摘要不符的安全拒绝并全量清理暂存；
  - 事务原子写入与断电恢复状态机流转；
  - 新业务数据产生后阻止静默覆盖恢复旧数据库的保护机制；
  - 独立更新器与 `control update-status/check` 只读输出 JSON 契约。
- **未验证 / 待真机环境（Unknown）**：
  - 真实远端分发与 CDN 基础设施尚未建立（本次仅完成本地合成包演练）；
  - 生产私钥严禁入库，生产环境信任根公钥轮换与吊销列表需另行运维配置；
  - macOS 目标机真实 TCC（辅助功能/录屏）权限在重签名后是否必须重新提示用户授权，取决于操作系统隐私策略，需在专用机另行实测验收；
  - 真实全进程排空与 launchd bootout 停机接线需在 I6 界面层接入并与专用机联调。
