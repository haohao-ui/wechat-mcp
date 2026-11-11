# U1 受限解码与 sandbox 候选实现待复核报告（CLI 对齐与完整制品打包版）

- **日期**：2026-11-11
- **任务编号**：`task_e939c24d4e0d`
- **Dispatch 标识**：`ctx_16f550ba9692`
- **执行终端**：`term_ada00d6c799ab749648d724801bcb6e9`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **基线分支**：`work/prod-integration-review`（基线提交 `1e120b5`）
- **交付范围与状态**：**受限解码与 sandbox 候选实现待复核（明确为 Partial 阶段性收敛，暂不声称安全完成）**。
  - 针对 Q2 报告（`de8e2c0`）发现的 `src/bin/wechat-updater.rs` 编译阻塞完成最小契约对齐修复；
  - 严格不恢复不安全旧 API、不用伪 tx、不改动 `journal.rs`（避免干扰 R1 并行开发）；
  - 全量二进制、24 项 updater 单元/集成测试、代码格式与完整发布包打包全部通过并经本地真实验证；
  - 生产 CLI 严格保持在安全 fail-closed 状态（退出码 1，提示 unsupported / trust_root_unconfigured）；
  - 零 Python，零真实生产 DB / 钥匙串 / 生产应用副作用，零远端推送。

---

## 1. CLI 编译阻塞最小契约对齐（Q2 报告响应）

### Fresh Build RED 记录
在执行修复前，运行 `cargo build --locked --bin wechat-updater` 真实重现构建失败：
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

error: could not compile `wechat-mcp` (bin "wechat-updater") due to 2 previous errors (exit code: 101)
```

### 最小契约对齐落地
| 问题点 | 整改落地措施 | 安全与架构保障 |
|---|---|---|
| `set_maintenance_active` 调用缺失 | 移除只读 `status` 查询命令中的写维护标记调用 | 状态查询命令无 update 事务上下文，严禁编造伪 tx 写 marker；当 journal 或 seq 文件损坏时，直接在输出 JSON 中精准标识 `status: "recovery_required"` 与 `maintenance_active: true`。 |
| `load_current_release_metadata` 调用缺失 | 替换为现有的单一权威记录加载函数 `journal::load_current_release_record` | 输出由权威提交记录 `CurrentReleaseCommitRecord` 支撑的单一真实数据。 |
| 维护文件检测兼容 | 同时核验 `core.maintenance.json` 与 `core.maintenance` | 兼容新版结构化维护标记与历史标记。 |
| 外部接口与运行时隔离 | 完全不修改 `src/updater/journal.rs` | 零公共接口变更，彻底杜绝与另一 worktree 中 R1 的冲突。 |
| 写操作入口 fail-closed | `apply` / `rollback` / `recover` 保持返回 unsupported（退出码 1） | 绝不触碰真实 HOME、数据库或钥匙串。 |

---

## 2. 真实构建、测试与完整发布包打包验证记录

所有步骤均在本地工作树中执行并通过，无任何破坏性副作用：

| 验证项 | 执行命令 | 退出码 | 验证结果与证据说明 |
|---|---|:---:|---|
| **CLI 编译** | `cargo build --locked --bin wechat-updater` | **0** | 编译成功，零 error |
| **全量 Bin 构建** | `cargo build --locked --bins` | **0** | 全部二进制产物（含 wechat-mcp、wechat-updater 等）构建成功 |
| **Updater 测试套件** | `cargo test --locked --lib updater` | **0** | **24 项测试全部 PASS**（耗时 0.51s，零 failure） |
| **格式与静态分析** | `cargo fmt --all -- --check` & `cargo clippy --bin wechat-updater` | **0** | 源码格式规范，clippy 零 error |
| **安全与门禁脚本** | `./scripts/check_no_python.sh` & `./scripts/check_sensitive.sh` | **0** | 零 Python 依赖，无私钥或敏感信息泄露 |
| **门禁夹具回归** | `bash scripts/test-gate-fixtures.sh` | **0** | 门禁回归夹具全部通过 |
| **完整发布包构建** | `bash scripts/build_release_package.sh 0.2.0-test target/test-dist 1002 test true` | **0** | **成功生成完整应用发布包及签名封套**（详见下文） |

### 完整发布包产物真实信息
- **产物目录**：`target/test-dist`
- **制品归档**：`target/test-dist/wechat-mcp-0.2.0-test-macos-arm64.tar.gz`
  - 文件大小：`9,289,035` 字节
  - 归档 SHA-256：`5c8c42aeca87bf93df207e863cd071152a26a638b32ef9429fcbcc59d0f17593`
- **清单组件**：13 个组件（解压展开总计 `22,753,788` 字节），包含 `微信客服助手.app` 及全部二进制
- **发布清单**：`target/test-dist/manifest.json`
- **测试签名封套**：`target/test-dist/envelope.json`（由隔离测试私钥生成，签名公钥导出至 `target/test-dist/test_pubkey.hex`）

---

## 3. 残余边界说明（Known Partial Boundaries，暂不声称安全完成）

遵循诚实汇报原则，明确记录当前未接线与局部性边界：

1. **交付定性**：
   - 当前交付仅代表针对 CLI 阻塞与 A/B 缺陷的可复现整改收敛，仍定性为**受限解码与 sandbox 候选实现待复核（Partial）**，**暂不声称整体更新事务安全完成**。
2. **生产可信公钥根保持未注入**：
   - 生产环境中执行 `wechat-updater check/apply/recover` 时，因生产可信公钥根未配置，程序严格 fail-closed 退出（状态码 1，提示 `error: unsupported: trust_root_unconfigured`）。
3. **真实生产宿主排空与 launchd 重启未接线**：
   - Swift 宿主 App 与 CoreSupervisor 守护进程的跨进程真实排空与 launchd 重启保持契约未接线状态，不私自修改公共运行时。
4. **数据库变更自动恢复保持禁止**：
   - 遇真实 SQLite 数据库自检失败时保持维护态并标记 `RecoveryRequired`，严禁自动回滚覆盖用户业务数据库。
5. **本地小步提交保障**：
   - 代码变动保留在本地独立分支 `work/finish-updater`，未向远端进行任何 git push。
