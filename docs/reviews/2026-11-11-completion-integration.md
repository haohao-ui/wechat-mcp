# Q2 U1/T1 独立集成审查报告（全提交合入与闭环复验版）

- **审查日期**：2026-11-11
- **任务编号**：`task_bcaf745d617e`（接续 `task_71cb4e8b6b10` / `task_c7643457e405`）
- **Dispatch 标识**：`ctx_8cbeccb6a191`（接续 `ctx_413d75ec3376` / `ctx_8e57928ca33c`）
- **审查终端**：`term_54ccc4ac7a5d283fa5ceafcdabd44125`
- **协调者终端**：`term_28f81ca3-a0aa-4f1f-99fb-bb139bd007ad`
- **初始基线**：`1e120b5`（分支 `work/prod-integration-review`）
- **当前 HEAD**：合并提交 `8a97076`（依次合入 `827cc16`、`9d4fdc3`、`68ed684`）
- **审查定性**：**Partial（RED-1 CLI 编译阻断已消除；RED-2 发布包 JSON 白名单遗漏已通过 1d7c451/9d4fdc3 闭环并通过显式 --ignored 真实包测试；locale 隔离 68ed684 合入后默认并发核心测试连续 3 轮 999 项全过；Swift 92 更新因离屏环境尺寸差异 2 项失败如实保留；遵从指令暂不合 R1，生产根与真实排空未接线）**。

---

## 1. 独立集成合并与依赖核对

### 1.1 分支合并记录
1. **测试隔离初始分支**：合入 `work/finish-test-isolation` 提交 `e70de0b`（含 `bc0c87b`、`640f7e8`）；
2. **更新模块阶段 1**：合入 `work/finish-updater` 提交 `f0c9ecf`；
3. **更新模块 RED-1 修复**：合入协调者批准的 `827cc16dd2322891a132e01c0a51f6aab9f058e9`（生成合并提交 `bbf68a5`）；
4. **发布包修复与真实包专项加固**：合入协调者批准的 `9d4fdc34362dcf1cd4d0a1f1181835cb58d2ea69`（含祖先 `1d7c451`，生成合并提交 `7d584d3`）；
5. **T1 Locale 隔离最终修订**：合入协调者批准的 `68ed684`（含祖先 `099560b`，生成合并提交 `8a97076`）。核对 `src/core/serve.rs` diff，确认严格保持 `None` 回退默认、所有 `Some`（含空串/空白/有效值）保持原语义不覆盖，单测完全采用纯函数隔离；
6. **暂不合入 R1**：按照协调者明确指示，Rust guard 还未接线且 R1（`8173715`）审查未过正修，当前分支严格不合入 R1。

### 1.2 门禁与环境静态规范
- **Python 隔离**：`./scripts/check_no_python.sh` 退出码 0，项目中无 Python 依赖或脚本；
- **敏感信息**：`./scripts/check_sensitive.sh` 退出码 0，无私钥、Token 或真实主机路径硬编码；
- **代码格式**：`cargo fmt --all -- --check` 退出码 0，零格式差异。

---

## 2. 缺陷收敛追踪与复验结果

### 2.1 缺陷 RED-1：`wechat-updater` 二进制编译阻断（已闭环恢复）
- **代码位置**：[`src/bin/wechat-updater.rs:68-85`](src/bin/wechat-updater.rs#L68-L85)
- **修复提交**：`827cc16dd2322891a132e01c0a51f6aab9f058e9`
- **复验结果**：
  - `cargo build --locked --bin wechat-mcp --bin wechat-updater --bin wechat-test-signer` 退出码 **0**；
  - `cargo test --locked --lib updater` 常规 **24 passed, 0 failed, 1 ignored** 全部通过（耗时 0.55s，两项 CLI 测试恢复为 PASS）；
  - `cargo clippy --tests` 退出码 **0**，无编译错误阻断。

### 2.2 缺陷 RED-2：发布包 JSON 资源清单过滤导致受限解包白名单失败（已闭环恢复）
- **代码位置**：[`scripts/build_release_package.sh:121`](scripts/build_release_package.sh#L121)
- **修复提交**：`1d7c451`（脚本白名单调整）与 `9d4fdc3`（测试加固）
- **复验步骤与结果**：
  1. 在本树目标目录构建完整发布包：
     `./scripts/build_release_package.sh 0.2.0-test target/test-dist 1002 test true`（退出码 0，组件总数由 13 增加至 15，精确包含 `Apps/.../gateway.test.json` 与 `resources/gateway.example.json`）；
  2. 显式运行真实发布包专项测试（杜绝默认 ignored 假绿）：
     `cargo test --locked --lib test_real_built_release_package_verification_and_unlisted_rejection -- --ignored`
  3. **实际结果**：**1 passed, 0 failed**（退出码 0，耗时 1.84s），真实验签、全要素组件集合比对及未登记文件对抗拦截全部通过。

### 2.3 缺陷 RED-3：`test_ensure_locale_sets_vars` 全局环境变量污染与断言失败（已闭环恢复）
- **代码位置**：[`src/core/serve.rs:39-58, 753-775`](src/core/serve.rs#L39-L58)
- **修复提交**：`68ed684`（含 `099560b`）
- **复验结果**：
  - 纯函数 `required_locale_defaults` 严格对齐基线原语义（`None` 补默认，所有 `Some` 保持不覆盖）；
  - 单元测试使用纯函数隔离断言，彻底消除全局写与并发竞态，核心测试连续 3 轮均稳定通过。

---

## 3. 门禁与命令验证实际记录

| 验证项 | 执行命令 | 实际退出码 | 实际测试数量 / 输出结果 | 定性判定 |
|:---|:---|:---:|:---|:---:|
| **无 Python 检查** | `./scripts/check_no_python.sh` | **0** | `✅ Python 检查通过` | **PASS** |
| **敏感路径/信息检查** | `./scripts/check_sensitive.sh` | **0** | `✅ 敏感信息检查通过` | **PASS** |
| **代码格式检查** | `cargo fmt --all -- --check` | **0** | 零 diff，完全符合格式规范 | **PASS** |
| **All-bin 新鲜构建** | `cargo build --locked --bin wechat-mcp --bin wechat-updater --bin wechat-test-signer` | **0** | 3 个二进制完整生成至 `target/debug/` | **PASS** |
| **All-target Clippy** | `cargo clippy --tests` | **0** | 编译阻断消除，通过 Clippy 门禁 | **PASS** |
| **更新模块库测试** | `cargo test --locked --lib updater` | **0** | **24 passed, 0 failed, 1 ignored** | **PASS** |
| **显式真实包解包专项** | `cargo test --locked --lib test_real_built_release_package_verification_and_unlisted_rejection -- --ignored` | **0** | **1 passed, 0 failed**（耗时 1.84s，实测真实验签解包） | **PASS** |
| **协议烟雾测试** | `cargo test --test protocol_smoke --locked` | **0** | **2 passed, 0 failed** | **PASS** |
| **Swift 103 绑定契约** | `cargo test --test swift_brain_ui --locked` | **0** | **103 项内部断言全部通过**（耗时 30.68s） | **PASS** |
| **Swift 92 更新契约** | `cargo test --test swift_update_ui --locked` | **101** | **90 PASS, 2 FAIL**（2 项离屏尺寸断言与环境差异） | **PARTIAL（如实记录）** |
| **默认并发核心测试第 1 轮** | `cargo test --bin wechat-mcp` | **0** | **999 passed, 0 failed, 1 ignored**（耗时 24.69s） | **PASS** |
| **默认并发核心测试第 2 轮** | `cargo test --bin wechat-mcp` | **0** | **999 passed, 0 failed, 1 ignored**（耗时 24.55s） | **PASS** |
| **默认并发核心测试第 3 轮** | `cargo test --bin wechat-mcp` | **0** | **999 passed, 0 failed, 1 ignored**（耗时 24.86s） | **PASS** |

---

## 4. 权限失败与环境隔离精确核实

依照指示，严格区分环境权限与测试实际结果，杜绝将权限失败泛化为逻辑通过：
1. **标准沙箱环境限制（Standard Sandbox）**：
   - 命令：`cargo test --locked --lib`；
   - 结果：898 passed, 98 failed。其中 96 项因系统 seatbelt 隔离机制拦截跨进程 Unix 套接字（`/tmp/wechat_worker_*.sock`）或本地回环 HTTP 端口 `connect`，抛出 `PermissionDenied` 或 `Operation not permitted (os error 1)`；
   - **定性原则**：规范要求 `PermissionDenied` 必须如实记录为沙箱限制下的测试未通过，严禁擅自归类为“预期通过”。
2. **回环套接字授权环境复验**：
   - 在具备本地 IPC 套接字权限模式下，执行 `cargo test --bin wechat-mcp`；
   - 连续 3 轮默认并发测试均达到 **999 passed, 0 failed, 1 ignored**（退出码 0），证明代码逻辑与测试隔离在具备必要系统 IPC 权限时完全稳定。

---

## 5. 8 项关键防御边界核验与局限性分析（非全面安全证明）

下述 8 项边界仅代表当前在特定测试与代码静态实现层面的核查，**不等于端到端全面安全验证**：

1. **未验签类型不可构造/修改 (Verified in Design)**：[`src/updater/executor.rs:22-52`](src/updater/executor.rs#L22-L52) 私有字段强类型封装，唯一构造入口强制验签与兼容性核验；
2. **沙箱根构造仅限测试环境 (Verified in Design)**：[`src/updater/executor.rs:55-120`](src/updater/executor.rs#L55-L120) 私有字段，构造器 `create_test_isolated_sandbox` 加装 `#[cfg(test)]`，生产环境零构造可能；
3. **归档尾流与 PAX/GNU 预算防御 (Verified in Specific Tests)**：[`src/updater/unpack.rs:245-280, 485-535`](src/updater/unpack.rs#L245-L280) 严格拦截 PAX/GNU 扩展头，流式校验尾部零填充，物理底层流防多 member 尾垃圾；
4. **父 Symlink 竞态与目录清理受控 (Verified in Specific Tests)**：全程 `openat` + `O_NOFOLLOW` 遍历；清理仅针对已解包哈希表逐文件删除并尝试空目录清理，杜绝 `remove_dir_all`；[`OwnedSocketDir`](src/core/serve.rs#L260) 比对 `dev/ino` 防误删非自有目录；
5. **Source 与 Current 受限根校验 (Verified in Specific Tests)**：软链接目标严格限制在 `releases/` 下且名称合法；
6. **Record -> Journal -> 清维护原子提交协议与崩溃恢复 (Verified in Specific Tests)**：顺序严格为 `CandidateTestingPassed` -> 持久化 proof -> 原子提交 `current_release.json` -> 推进 `Committed` -> 清除维护态；崩溃恢复全要素比对；
7. **既有防降级高水位不回 0 (Verified in Specific Tests)**：文件损坏或序列冲突严格 fail-closed 报错，绝不回退至 0；
8. **自检后代 Pipe 与超时回收及局限性说明 (Partial Defense)**：通过 `setpgid` 与向 `-pgid` 发送 `SIGKILL` 清理进程组后代。**明确指出局限性：若孙子进程调用 `setsid()` 建立新 Session 脱离原进程组，则无法被回收，且无毫秒级绝对保证**。

---

## 6. 残余边界与当前局限说明（Known Partial Boundaries）

1. **Swift 92 更新测试 2 项失败**：`tests/host_update_selftest/main.swift` 中的两项离屏尺寸断言与环境分辨率差异失败（90 PASS, 2 FAIL），保持原样记录退回 Owner；
2. **R1 暂未合入**：Rust guard 运行时接线尚未完成，R1（`8173715`）保持未合入；
3. **生产信任根未配置**：生产环境中执行更新命令保持在 fail-closed 状态（提示 `error: unsupported: trust_root_unconfigured`）；
4. **真实生产排空与 launchd 未接线**：Swift 宿主 App 与 CoreSupervisor 守护进程的真实进程排空与 launchd 重启未接线，当前测试全部基于 Fixture 适配器；
5. **Mock Pass 自检不能代表真实应用通过**：当前更新阶段仅实现了受控解包与沙箱候选机制，通过的自检为受信 Mock Fixture，绝不可声称真实微信 App 自检或真实备份通过；
6. **本地小步审查，零远端推送**：本地阶段合并保留在分支 `work/completion-integration`，未向远端进行任何 git push。
