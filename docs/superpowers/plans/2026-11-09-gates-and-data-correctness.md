# 门禁恢复、数据正确性与移除 Python —— Implementation Plan

> **For agentic workers:** 本计划按 Orca 派发，每个任务在独立 worktree、独立分支完成，完成后用 `worker_done` 报告。**不要合并、推送、部署。** 不要修改 `src/interaction.rs`、`native/interaction/`、`tests/interaction_*`、`docs/interaction/`（另一个会话的进行中工作）。

**Goal:** 让构建、CI、发布链重新可用；修复历史库和备份的数据正确性缺陷；移除剩余的 Python 代码与依赖。

**依据：** 2026-11-09 深度审计（另一会话所写，未入库；以下所需内容已原样摘录进各任务）、P5 设计 `docs/superpowers/specs/2026-11-08-production-python-free-design.md` 第 4.2 节。用户 2026-11-09 确认范围：审计 Phase 0（门禁）+ Phase 1（数据正确性）+ 删除剩余 Python；不含运行时超时、对外契约工程化、架构收敛。

## Global Constraints

- Rust 工具链锁定 `1.99.0`（本机与 CI 一致）；不新增 crate 依赖。
- Swift 目标 macOS 12；`Process` 管道先读完再 `waitUntilExit`。
- 每个任务结束时：`cargo test --locked` 除已知基线缺陷 `swift_interaction::generic_hand_and_observation_binding_post_no_real_events`（另一会话的通用桌面改动未提交所致）外全部通过。
- 提交信息用中文，`fix(...)`/`chore(...)` 前缀，末尾加派发说明给出的 `Co-Authored-By` 行。
- 现有公共接口和数据库表结构不变（需要新索引或新命令时在任务中写明）。
- `cargo fmt` / clippy 的批量清理**不在** Task A–C 内（会与其他任务大面积冲突），由 Task D 在合并后单独完成。

## 派发与依赖

| 任务 | 执行者 | 依赖 | 主要文件 |
|---|---|---|---|
| A 门禁与移除 Python | codex | — | `scripts/*.sh`、`.github/workflows/*`、`native/*.swift`（OCR）、`scripts/wechat_bot/`、`scripts/agent_bridge/`、`tests/*.py`、`src/core/control.rs` 测试段、`src/core/history/store.rs` 互操作测试、README/docs |
| B 历史库写路径事务与互斥 | codex | — | `src/core/history/store.rs`（非测试段）、`src/core/history/migrate.rs`、`src/core/control.rs`（写命令的运行检查）、`src/core/serve.rs`（锁函数） |
| C 备份与恢复 | agy | — | `src/core/backup.rs`、`src/main.rs`（`restore` 子命令） |
| D fmt + clippy 门禁 | agy | A、B、C 合并后 | 全仓机械修改 + `Cargo.toml [lints]` |
| 集成、部署、全面验证 | 协调者 | 全部 | — |

A 与 B 都会碰 `store.rs`、`control.rs`：A 只改**测试模块中调用 python3 的那几个测试**，B 只改**非测试代码**。合并冲突由协调者处理。

---

### Task A：门禁恢复与移除剩余 Python（codex）

**已核实事实：**
- `native/click/main.swift:14,20` 调用 `runClearInput`、`runInputText`，定义在 `native/click/Input.swift`；`scripts/build.sh:6`、`.github/workflows/ci.yml:61`、`.github/workflows/release.yml:48-49` 只编了 `main.swift` → 编译失败。`scripts/deploy_host.sh` 已用 `native/click/*.swift`。
- `ci.yml:75-76` 调用已不存在的 `tests/protocol_smoke.py`、`tests/desktop_runtime_smoke.py`（已迁为 `.rs`）；`ci.yml:77-84` 还跑其他 Python 测试。`README.md:190`、`docs/rpa-runtime.md:91` 仍写 `.py` 命令。
- `scripts/deploy_host.sh` 约 167 行 `echo "=== $([ $YES -eq 1 ] && echo done || ...)"` 触发 shellcheck SC1010；CI `shellcheck -S warning`（ci.yml:32）失败，而 `test` job `needs: security`（ci.yml:40）→ 测试从不运行。
- CI 用 `dtolnay/rust-toolchain@stable`（ci.yml:44），未锁版本。
- `release.yml:37` 手动触发时 `VERSION="${GITHUB_REF_NAME}"` 取到分支名。
- 已有分支 `work/p5-t6a-remove-rapidocr`（提交 `08c507a`）完成了：删除 `native/rapidocr_server.py`、去掉 `wechat_ocr.swift`/`wechat_bubbles.swift` 的 RapidOCR 分支、install/build/CI 去掉 rapidocr/pip。它基于旧基线，需要 cherry-pick 后解决冲突。
- 仍依赖 Python 的 Rust 测试：`src/core/control.rs` 约 2365–2430 行（`python3 -m scripts.wechat_bot.control` 的 config-check / resume / calibrate 对照）；`src/core/history/store.rs` 约 2436–2490 行（导入 `scripts.wechat_bot.history.store` 的互操作测试）。`src/core/policy.rs:1131` 只是注释（黄金值已固化），不用改。
- 待删除：`scripts/wechat_bot/`（22 个 .py）、`scripts/agent_bridge/`（6 个）、`scripts/backup_db.py`、`tests/*.py`（25 个）、`deploy_host.sh` 的 `--bot` 选项及相关步骤。

**步骤：**
1. cherry-pick `08c507a`，解决冲突；保持 Vision 为唯一 OCR 引擎，`WECHAT_OCR_ENGINE=rapidocr` 不再接受。
2. 三处构建改为 `native/click/*.swift`；`build.sh`、`ci.yml`、`release.yml` 的 Swift 部署目标统一为 macOS 12（与 `deploy_host.sh` 一致）。
3. CI：删除所有 `python3 …` 步骤和 pip；shellcheck 改为独立 job，`test` 不再 `needs: security`；toolchain 改为 `dtolnay/rust-toolchain@1.99.0`（components: rustfmt, clippy）；新增 `rust-toolchain.toml`（`channel = "1.99.0"`，components 同上）。fmt/clippy 步骤暂时保留但设 `continue-on-error: true`，由 Task D 去掉。
4. 修 SC1010（改成显式 if/else）；全仓 `shellcheck -S warning scripts/*.sh` 必须 exit 0。
5. `release.yml` 手动触发时：没有 tag 就失败退出，并给出提示（不再用分支名当版本）。
6. 把两处调用 python3 的测试改成**纯 Rust 断言**：先在改动前运行一次 Python 版本，记录真实输出，把这些输出作为固定期望值写进测试（不得只是删除测试）；store 互操作测试改为断言"Rust 写入的库按 Python 版本当时的读取方式读出的字段一致"，即对表结构和字段内容做断言。
7. 删除上面列出的 Python 文件；`deploy_host.sh` 去掉 `--bot`；README、`docs/rpa-runtime.md`、`docs/agent-bridge.md` 中的 Python 命令改为 Rust 等价命令，或标注"已移除"。
8. 新增守卫脚本 `scripts/check_no_python.sh`（仓库中不得出现 `.py` 文件或文档引用 `tests/*.py`），并在 CI 中运行。

**完成标准（证据）：**
- `bash scripts/build.sh` 成功；`swiftc native/click/*.swift native/common/*.swift native/action/*.swift -o /tmp/wc && /tmp/wc --plan-only key cmd+v` 成功。
- `shellcheck -S warning scripts/*.sh` exit 0；`bash scripts/check_no_python.sh` exit 0；`git ls-files '*.py'` 为空。
- `cargo test --locked`（只允许上述基线例外）。
- 汇报：提交哈希、各项命令的原始输出摘要、固化的期望值来源说明。
- 提交可以分 2–3 个（构建/CI、移除 Python、文档），末尾 `Co-Authored-By: Codex <noreply@openai.com>`。

---

### Task B：历史库写路径事务与跨进程互斥（codex）

**已核实事实：**
- D-1：`src/core/history/store.rs:1167` `record_frame` 逐条 `_append`（各自隐式事务），`:1177` 最后才更新 `contact_cursor`，全程无事务。崩溃在中间 → 下次把同一批气泡再插一遍。`upsert_profile`（:1360）已有事务可参照。
- D-2：`store.rs:645` `record_bot_reply` 先查 `messages`，再 `INSERT INTO sends`（:658）、`INSERT INTO messages`（:671），无事务；`sends.send_key` 是主键。死在两条 INSERT 之间 → 重试时查 `messages` 没有 → 再插 `sends` 报 UNIQUE → 错误经 `turns.rs:1535` 的 `?` 上抛，**该会话自动回复永久卡死**。
- D-3：`src/core/history/migrate.rs:27` 检查的是 Python 遗留的 `state/service.lock`；Rust 守护进程实际持有 `state/core.lock`（`src/core/serve.rs:51`）。`control` 的 `history-status / merge-contact / unmerge-contact / history-backup` 直接开库，不做运行检查（`control.rs:1176` 起）。`MAX(seq)+1`（store.rs:1205）是读-改-写。

**步骤（先写失败测试再改）：**
1. `record_frame`：整帧在一个事务内（`unchecked_transaction()`），`_append` 与游标更新一起提交。测试 `record_frame_is_atomic`：用 `CREATE TRIGGER … RAISE(ABORT)` 让第 2 条插入失败 → `messages` 0 行、游标未推进；去掉触发器重放 → 只写一次。
2. `record_bot_reply`：单事务；幂等判断改查 `sends`；`sends` 已存在而 `messages` 缺失时补写 `messages`。测试 `record_bot_reply_recovers_half_written`：构造"只有 sends 行"的库，再次调用成功、不报 UNIQUE、最终各一行。
3. 从 `serve.rs` 抽出 `pub fn core_lock_path(state_dir) -> PathBuf` 和 `pub fn core_is_running(state_dir) -> bool`（尝试非阻塞加锁判断），`migrate::ensure_stopped` 和 `control` 的写命令（merge-contact、unmerge-contact、migrate-history）统一调用它；运行中执行 → 报错"先停止服务"，并提供 `--allow-running` 仅用于只读命令。测试 `migrate_refuses_while_core_running`。
4. `seq` 分配改为单条语句子查询插入，或捕获 `SQLITE_CONSTRAINT` 后重试（最多 3 次）。测试 `concurrent_writers_keep_unique_seq`：两个连接各写 50 条 → 共 100 条，`PRAGMA integrity_check` 为 `ok`。

**禁止：** 修改表结构；修改 `store.rs`/`control.rs` 测试模块中调用 python3 的测试（Task A 负责）。

**完成标准：** 上述 4 个测试先失败后通过；`cargo test --locked`（只允许基线例外）；汇报提交哈希和测试汇总。`Co-Authored-By: Codex <noreply@openai.com>`。

---

### Task C：备份完整性与恢复（agy）

**已核实事实：**
- D-4：附件存放在 `state/history/media/<sha 前 2 位>/…`（`store.rs:943`）；`src/core/backup.rs` 全文不含 media，manifest 只列数据库（:253-292）。恢复后图片和文件全部打不开。
- D-5：`backup.rs:295` `if out_dir.exists() { fs::remove_dir_all(&out_dir)?; }`；自定义 `--out` 直接作为 `out_dir`（:232）→ 会清空用户目录。
- D-6：`backup.rs:87` 以 `SQLITE_OPEN_READ_WRITE` 打开源库，`sqlite3_backup_step(p, -1)` 一次拷完，没有 `busy_timeout`。
- D-7：只有 `backup` 子命令（`src/main.rs:95`），没有恢复；manifest 只有版本、时间、文件名单。

**步骤（先写失败测试再改）：**
1. 源库以只读方式打开，`busy_timeout(5000)`，按 SQLite 官方方式循环 `step(128)`，遇到 BUSY/LOCKED 等待后继续。测试 `backup_survives_concurrent_writer`。
2. 备份包含 `history/media/`：按相对路径复制，manifest 记录每个数据库的 sha256、`PRAGMA user_version`、文件数和总字节数，以及 media 的文件数和总字节数。测试 `backup_includes_media_and_digests`。
3. `--out`：只允许不存在的目录、空目录，或带有效 manifest 的本程序备份目录；其他情况报错。替换旧备份时先 rename 到 `.trash-<uuid>`，新备份完成后再删除。测试 `backup_refuses_foreign_directory`（目录里放一个无关文件 → 报错且文件仍在）。
4. 新增 `wechat-mcp restore --from <备份目录> [--config <path>]`：校验 manifest 和 sha256 → 确认核心未运行（用 Task B 的 `core_is_running`；如果 Task B 未合入，先在 backup.rs 内实现同样的非阻塞加锁检查，并加 TODO 注释说明合并时改用公共函数）→ 当前库和 media 移到 `.restore-old-<ts>` → 复制 → `integrity_check`、`foreign_key_check`、行数与 manifest 对比 → 任何一步失败就自动回滚。测试 `restore_roundtrip`、`restore_rolls_back_on_digest_mismatch`、`restore_refuses_while_core_running`。

**禁止：** 修改历史库表结构；修改 `store.rs`。

**完成标准：** 上述测试全部通过；`cargo test --locked`（只允许基线例外）；**命令都用前台方式运行，等到结果再继续，不要放后台后结束本轮**；汇报提交哈希、测试汇总原文、一份示例 manifest。`Co-Authored-By: Antigravity <noreply@google.com>`。

---

### Task D：fmt 与 clippy 门禁（agy，A–C 合并后派发）

- `Cargo.toml` 增加 `[lints.clippy]`：风格类（`collapsible_if`、`unnecessary_map_or`、`too_many_arguments` 等）设为 `warn`，`correctness`、`suspicious` 保持 `deny`。
- 单独一个提交：`cargo fmt --all`（只格式化，不改逻辑）。
- 另一个提交：修完所有 `correctness`/`suspicious` 级问题（审计指出的 `suspicious_open_options` 2 处、`unnecessary_unwrap` 9 处必须人工核对，不能机械改）；其余用 `cargo clippy --fix` 后逐个检查。
- 去掉 CI 中 fmt/clippy 步骤的 `continue-on-error`。
- 完成标准：`cargo fmt --all -- --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked` 全部通过（基线例外除外）。

---

## 集成、部署与全面验证（协调者）

1. 依次合并 A、B、C → 派发 D → 合并 D；每次合并后跑完整测试。
2. 部署 `--mcp --worker --app`（RapidOCR 移除改了 worker 使用的 Swift 文件）；删除专用机 `~/.wechat-rpa/bin/rapidocr_server.py`。
3. 本地按 CI 步骤逐条执行（shellcheck、fmt、clippy、test、native build、no-python 守卫），全部通过。
4. 专用机端到端（需要用户在示例群发消息）：文字、图片、文件的接收与回复；通讯录同步结果；历史查询；对外接口（临时开启后恢复）；重启后不重发；备份与恢复演练（在专用机上对空测试数据执行一次 backup → restore）。
