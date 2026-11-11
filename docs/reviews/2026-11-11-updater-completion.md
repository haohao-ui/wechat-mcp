# U1 受限解码与 sandbox 候选实现待复核报告（CLI 对齐、JSON 资源与完整验签解包版）

- **日期**：2026-11-11
- **任务编号**：`task_e939c24d4e0d`
- **Dispatch 标识**：`ctx_16f550ba9692`
- **执行终端**：`term_ada00d6c799ab749648d724801bcb6e9`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付范围与状态**：**受限解码与 sandbox 候选实现待复核（明确为 Partial 阶段性收敛，暂不声称安全完成）**。
  - 针对 Q2 报告（`de8e2c0`）发现的 `src/bin/wechat-updater.rs` 编译阻塞完成最小契约对齐修复；
  - 针对打包脚本排除扩展名导致 payload 内 JSON 资源遗漏缺陷，改为按 payload 目录（`bin`/`Apps`/`resources`）精确枚举；
  - 严格不恢复不安全旧 API、不用伪 tx、不改动 `journal.rs`（避免干扰 R1 并行开发）；
  - 全量二进制、25 项 updater 单元/集成测试、代码格式与完整发布包打包全部通过并经本地真实验证；
  - 完成对真实生成发布包的端到端验签解包核验，以及多余未声明文件的负向拒绝测试；
  - 生产 CLI 严格保持在安全 fail-closed 状态（退出码 1，提示 unsupported / trust_root_unconfigured）；
  - 零 Python，零真实生产 DB / 钥匙串 / 生产应用副作用，零远端推送。

---

## 1. 核心整改与契约对齐落地

### A. CLI 编译阻塞最小契约对齐（Q2 报告响应）
- **Fresh Build RED 记录**：修复前，`cargo build --locked --bin wechat-updater` 因引用不存在的 `set_maintenance_active` 和 `load_current_release_metadata` 报错（退出码 101）。
- **整改措施**：
  1. 移除只读 `status` 查询命令中的写维护标记调用：状态查询命令无 update 事务上下文，严禁编造伪 tx 写 marker；当 journal 或 seq 文件损坏时，直接在输出 JSON 中精准标识 `status: "recovery_required"` 与 `maintenance_active: true`。
  2. 替换为现有的单一权威记录加载函数 `journal::load_current_release_record`。
  3. 维护文件检测同时兼容 `core.maintenance.json` 与 `core.maintenance`。
  4. 完全不修改 `src/updater/journal.rs`，零公共接口变更，不影响另一 worktree 中 R1 的开发。
  5. `apply` / `rollback` / `recover` 保持返回 unsupported（退出码 1），绝不触碰真实 HOME、数据库或钥匙串。

### B. 发布包脚本组件枚举与 JSON 资源修复
- **缺陷分析**：原脚本 `[[ "$rel_path" == *.json ]]` 粗暴跳过了所有 JSON，导致 `Apps/微信客服助手.app/Contents/Resources/gateway.test.json` 与 `resources/gateway.example.json` 被打进归档却遗漏在 `manifest.json` 之外，造成严格解包时因包内多余文件而失败。
- **整改措施**：
  1. 限制组件枚举范围仅为 payload 专属子目录（`bin`、`Apps`、`resources`）；
  2. 包含 payload 内所有 JSON 资源文件；
  3. 排除元数据文件靠其天然位于 payload 外部（`$OUT_DIR` 顶层），而非按扩展名过滤；
  4. 输出目录删除严格局限在本次构建目标 `target/test-dist`。

---

## 2. 真实构建、测试与真实包验签解包验证记录

所有步骤均在本地工作树中执行并通过，无任何破坏性副作用：

| 验证项 | 执行命令 | 退出码 | 验证结果与证据说明 |
|---|---|:---:|---|
| **CLI 编译** | `cargo build --locked --bin wechat-updater` | **0** | 编译成功，零 error |
| **全量 Bin 构建** | `cargo build --locked --bins` | **0** | 全部二进制产物（含 wechat-mcp、wechat-updater 等）构建成功 |
| **Updater 测试套件** | `cargo test --locked --lib updater` | **0** | **25 项测试全部 PASS**（耗时 1.62s，零 failure） |
| **格式与静态分析** | `cargo fmt --all -- --check` & `cargo clippy --bin wechat-updater` | **0** | 源码格式规范，clippy 零 error |
| **安全与门禁脚本** | `./scripts/check_no_python.sh` & `./scripts/check_sensitive.sh` | **0** | 零 Python 依赖，无私钥或敏感信息泄露 |
| **门禁夹具回归** | `bash scripts/test-gate-fixtures.sh` | **0** | 门禁回归夹具全部通过 |
| **完整发布包构建** | `bash scripts/build_release_package.sh 0.2.0-test target/test-dist 1002 test true` | **0** | **成功生成完整应用发布包及签名封套**（详见下文） |

### 完整发布包产物真实信息
- **产物目录**：`target/test-dist`
- **制品归档**：`target/test-dist/wechat-mcp-0.2.0-test-macos-arm64.tar.gz`
  - 文件大小：`9,288,768` 字节
  - 归档 SHA-256：`6765b8bc961369798e82291d82fbfbcee0d45543ddb46b950141fdfa75b7e0a4`
- **清单组件**：15 个组件（解压展开总计 `22,754,025` 字节），包含 `微信客服助手.app`、全部二进制以及 `gateway.test.json`、`gateway.example.json` 两个 JSON 资源文件
- **发布清单**：`target/test-dist/manifest.json`
- **测试签名封套**：`target/test-dist/envelope.json`（由隔离测试私钥生成，签名公钥导出至 `target/test-dist/test_pubkey.hex`）

### 真实生成包的验签解包与负向拦截验证（测试 `test_real_built_release_package_verification_and_unlisted_rejection`）
1. **密码学验签**：使用 `test_pubkey.hex` 导出的受信公钥成功验证 `envelope.json`，清单完整性校验通过；
2. **归档哈希核验**：实际 `.tar.gz` 字节 SHA-256 完全匹配清单中的 `artifact.sha256` 与大小；
3. **流式解压与组件核验**：通过 `safe_unpack_and_verify` 完整解包并在受限沙箱中通过 `safe_verify_staged_dir` 核验，15 个组件权限与哈希 100% 匹配；
4. **负向测试（多余未列出文件）**：故意在 staging 注入未在清单声明的文件，`safe_verify_staged_dir` 精确拦截并报错 `unexpected_file_in_staged_dir`；清理后恢复通过。

---

## 3. 残余边界说明（Known Partial Boundaries，暂不声称安全完成）

遵循诚实汇报原则，明确记录当前未接线与局部性边界：

1. **交付定性**：
   - 当前交付仅代表针对 CLI 阻塞与 JSON 资源枚举的可复现整改收敛，仍定性为**受限解码与 sandbox 候选实现待复核（Partial）**，**暂不声称整体更新事务安全完成**。
2. **生产可信公钥根保持未注入**：
   - 生产环境中执行 `wechat-updater check/apply/recover` 时，因生产可信公钥根未配置，程序严格 fail-closed 退出（状态码 1，提示 `error: unsupported: trust_root_unconfigured`）。
3. **真实生产宿主排空与 launchd 重启未接线**：
   - Swift 宿主 App 与 CoreSupervisor 守护进程的跨进程真实排空与 launchd 重启保持契约未接线状态，不私自修改公共运行时。
4. **数据库变更自动恢复保持禁止**：
   - 遇真实 SQLite 数据库自检失败时保持维护态并标记 `RecoveryRequired`，严禁自动回滚覆盖用户业务数据库。
5. **本地小步提交保障**：
   - 代码变动保留在本地独立分支 `work/finish-updater`，未向远端进行任何 git push。
