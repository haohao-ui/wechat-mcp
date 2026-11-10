# 微信 MCP 深度审计与优化方案

日期：2026-11-09 ｜ 分支：`feat/wechat-bot-history` ｜ 证据基线：`42ef72a`（审计开始时工作区含 17 个已改文件 + 7 个未跟踪文件；审计过程中这些改动被外部流程提交为 `addaae0`、`42ef72a`，本报告行号已按 `42ef72a` 复核）

方法：4 个只读审计分组并行（并发/数据/构建/架构）+ Lead 独立复核。全部结论以 `file:line` 或可复现命令为依据；本报告写盘前未修改任何源码。
原始明细（gitignored）：`target/analysis/{async,data,build,arch}-audit.md`。

---

## 0. 结论摘要

**这个项目不是"没写完"，而是"功能跑得通、工程化掉队"：核心思路（账号隔离、SQLite 发送幂等、跨进程桌面锁、worker 单一执行者、大量失败注入测试）是对的，但门禁、数据事务、超时边界三处存在系统性缺口，且 CI 目前一条都不绿。**

按投入产出排序，最该做的 6 件事：

1. **修 CI 红灯链**：`native/click/Input.swift` 漏编译 → 源码构建/CI/发布三处同时失败；CI 仍调用已删除的 `.py`；shellcheck 门禁自身红并阻塞全部测试；fmt/clippy 全红且未锁工具链。
2. **历史库写路径补事务**：`record_frame` 整帧无事务、`record_bot_reply` 幂等查错表 → 崩溃后重复消息 / 回复永久卡死。
3. **统一"服务运行中"判定**：`migrate` 查的是 Python 遗留的 `service.lock`，Rust 守护进程持的是 `core.lock`，所谓"先停服务"保护实际不存在。
4. **补超时与 panic 隔离**：桌面操作、osascript、Chrome 全链路无超时；一个工具 task panic 直接退出整个 MCP 服务。
5. **备份说真话**：备份不含附件目录 `media/`、`--out` 会 `remove_dir_all` 用户目录、无恢复命令、无摘要校验。
6. **对外契约工程化**：51 个工具 0 个 `outputSchema`、错误只有自然语言、tracing 在网关层为 0。

一句话：**先把"可信的门禁 + 不丢数据的写路径 + 不挂死的边界"补齐，再谈架构拆分和功能扩展。**

---

## 1. 项目现状（可核验数字）

| 维度 | 事实 | 来源 |
|---|---|---|
| 代码量 | `src/` 51,465 行 = 生产 26,137 / 测试 25,328（约 1:1）；全仓 rs+swift+py+sh 85,939 行 | 审计统计 |
| Rust 测试 | src 内约 750 个 `#[test]` 属性；15 个测试 target，实跑 **942 个用例**（基线 `d555a68`+WIP；`42ef72a` 新增用例未重计） | 审计统计 |
| Python 测试 | 25 个文件、**388 个用例**；CI 只执行约 231 个（6 个文件 + `test_bot_*` discover），**157 个从未在 CI 跑** | `ci.yml:77-80` |
| 对外工具 | 运行时 `tools/list` = **51 个工具**，`annotations` 23 个，`outputSchema` **0** 个 | Lead 实测 MCP 会话 |
| 出错契约 | 错误仅 `{"content":[{"text":"core_not_running"}],"isError":true}`，无错误码/可重试/副作用标记 | Lead 实测 |
| 构建产物 | `target/` ≈ **18 GB**（deps 15 GB + incremental 1.7 GB） | `du -sh` |
| 历史 | 301 次提交；审计开始时 17 个已改 + 7 个未跟踪文件，审计中被外部流程提交为 `addaae0`、`42ef72a`，现工作区仅本报告未跟踪 | `git status` |
| 构建定义 | 4 套（`build.sh` / `ci.yml` / `release.yml` / `deploy_host.sh`），部署目标出现 macOS 12 / 13 / 默认 27 三种 | 审计矩阵 |

做得好、不要在重构中破坏的部分：

- 安全边界：`core.sock` 目录 0700 + socket 0600（[src/core/external_brain.rs:454](src/core/external_brain.rs)）；`api_key_file` 校验绝对路径/非软链/属主/0600（[src/core/config.rs:288](src/core/config.rs)）；`upload_file` 文件名与大小校验（[src/wechat/files.rs:78](src/wechat/files.rs)）；SQL 全部参数化（见数据审计 §7）。
- 发送幂等：先写 `attempting` 再发、`finish_send` 用 CAS（`storage.rs:143-197`）；崩溃后不盲目重发。
- 桌面互斥：进程内 tokio gate + 跨进程 `flock`（[src/desktop/actor.rs:23](src/desktop/actor.rs)）；Swift worker 侧同样持有 `desktop.lock`。
- 测试密度与失败注入：`with_test_send_fault`、触发器注入、FakeWorker/FakeMcpServer 等手法值得保留。

---

## 2. P0：CI / 构建 / 发布链已断（当天可修）

### P0-1 `native/click/Input.swift` 漏编译，三套构建同时失败

- **证据**：[native/click/main.swift:14](native/click/main.swift) 调用 `runClearInput`、`:20` 调用 `runInputText`，二者只定义在 `native/click/Input.swift`；而 [scripts/build.sh:6](scripts/build.sh)、[.github/workflows/ci.yml:61](.github/workflows/ci.yml)、[.github/workflows/release.yml:48](.github/workflows/release.yml) 都只编了 `native/click/main.swift`。只有 [scripts/deploy_host.sh:87](scripts/deploy_host.sh) 用了 `native/click/*.swift`。
- **已实测**：按 CI 命令编译 → `error: cannot find 'runClearInput' in scope`。
- **后果**：`bash scripts/build.sh` 必中断（README 的"从源码构建"、`install.sh --from-source` 不可用）；CI 原生工具步骤失败；release 两个架构都编不过 → **打 tag 也发不出包**。
- **改法**：三处改为 `native/click/*.swift`（S）；推荐抽 `scripts/build_native.sh --out DIR [--target triple] [--universal]`，文件清单只维护一份（M）。
- **验收**：`bash scripts/build.sh` 全绿；`swiftc native/click/*.swift native/common/*.swift native/action/*.swift -o /tmp/wc && /tmp/wc --plan-only key cmd+v`。

### P0-2 CI 引用已删除的 Python 冒烟测试

- **证据**：`250afb1` 已把 `tests/protocol_smoke.py`、`tests/desktop_runtime_smoke.py` 迁成 `.rs`；[.github/workflows/ci.yml:75](.github/workflows/ci.yml)、[README.md:190](README.md)、[docs/rpa-runtime.md:91](docs/rpa-runtime.md) 仍指向 `.py`。仓库自己的 [docs/reviews/2026-11-08-agent-tool-optimization.md](docs/reviews/2026-11-08-agent-tool-optimization.md) 已记为 P0 但未修。
- **改法**：删掉 CI 两行（`cargo test --locked` 已覆盖），文档改 `cargo test --locked --test protocol_smoke`；加守卫脚本检查文档里引用的 `tests/*.py` 是否存在（S）。

### P0-3 `security` job 自身红，且 `needs` 阻塞全部测试

- **证据**：[scripts/deploy_host.sh:167](scripts/deploy_host.sh) 的 `echo "=== $([ $YES -eq 1 ] && echo done || ...)"` 触发 `SC1010`；CI 用 `shellcheck -S warning`（[ci.yml:32](.github/workflows/ci.yml)）→ exit 1；`test` job `needs: security`（[ci.yml:40](.github/workflows/ci.yml)）→ 测试根本不跑。
- **改法**：改成显式 `if/else`；shellcheck 与测试解耦（独立 job 或 `continue-on-error`）（S）。

### P0-4 `cargo fmt --check` 红：1652 处 diff / 51 文件；P0-5 clippy 红：204 + 25 个 error

- **证据**：`cargo fmt --all -- --check` → 51 文件；`cargo clippy --all-targets --locked -- -D warnings` → bin 204 + 7 个 test target 共 25。约 90% 是风格 lint（`collapsible_if` 125、`unnecessary_map_or` 24…），真正值得处理的是 `suspicious_open_options` 2 处、`unnecessary_unwrap` 9 处。
- **根因**：CI 用 `dtolnay/rust-toolchain@stable`（[ci.yml:44](.github/workflows/ci.yml)），**未锁版本**，Rust 每个稳定版都会新增 lint → 代码不动也会从绿变红。
- **改法**：① 新增 `rust-toolchain.toml` 锁 `1.99.0` + `rustfmt/clippy`，CI 改用固定版本；② 单独一个纯格式化 commit（不混业务改动）+ pre-commit；③ `[lints.clippy]` 把风格类降为 `warn`，门禁只保留 `correctness/suspicious`；④ `cargo clippy --fix --allow-dirty` 批量机械修复（S/M）。
- **验收**：`cargo fmt --all -- --check`、`cargo clippy --all-targets --locked -- -D warnings` 均 exit 0；换一个 Rust 小版本结果不变。

> 另：`release.yml` 手动触发时 `VERSION="${GITHUB_REF_NAME}"`（[release.yml:37](.github/workflows/release.yml)）会拿到分支名 → 手动发布必产错版本号（S）。

---

## 3. P1：数据可靠性（会丢 / 会重 / 会卡死）

### D-1 `record_frame` 整帧非事务 → 崩溃重放重复消息

- **证据**：[src/core/history/store.rs:1167](src/core/history/store.rs) 逐条 `_append`（每条独立隐式事务），[store.rs:1177](src/core/history/store.rs) 最后才更新 `contact_cursor`，全程无 `BEGIN`。对照 `upsert_profile` 已用事务（`store.rs:1360`），说明是遗漏。
- **后果**：消息插入后、游标更新前被杀（panic/断电/SIGKILL/升级）→ 下次轮询把同一批气泡再插一遍；`record()` 只写 health 不阻断（`turns.rs:872-882`），重复会静默累积。
- **改法**：`record_frame` 内 `unchecked_transaction()`，`_append` 与游标 upsert 同事务提交（S，约 30 行）。
- **验收**：用 `CREATE TRIGGER ... RAISE(ABORT)` 让第 2 条插入失败 → 断言 `messages` 0 行、游标未推进；重放只写一次。

### D-2 `record_bot_reply` 幂等查错表 + 两条 INSERT 非原子 → 回复永久卡死

- **证据**：[store.rs:645](src/core/history/store.rs) 先查 `messages`，随后 `INSERT INTO sends`（`:658`）再 `INSERT INTO messages`（`:671`），三段无事务；`sends.send_key` 是主键。
- **后果**：进程死在两条 INSERT 之间 → 重试时 `messages` 查不到 → 再插 `sends` → `UNIQUE constraint failed`；错误经 `turns.rs:1535` 的 `?` 上抛，该会话每轮重试都失败，**自动回复永久卡死**（重启也不会自愈）。
- **改法**：单事务 + 幂等判断改查 `sends`，`INSERT OR IGNORE` 后补缺失 message 行（S）。
- **验收**：构造"只有 sends 行"的夹具，再次调用成功且不报 UNIQUE。

### D-3 历史库跨进程无互斥；`migrate` 的"先停服务"检查锁名不对

- **证据**：[src/core/history/migrate.rs:27](src/core/history/migrate.rs) 检查 `state/service.lock`（Python 遗留名）；Rust 守护进程实际持 `state/core.lock`（[src/core/serve.rs:51](src/core/serve.rs)）。`control` 的 `history-status / merge-contact / unmerge-contact / history-backup` 直接开库，不做运行检查（[src/core/control.rs:1176](src/core/control.rs) 起）。
- **后果**：`control migrate-history` 可与 `serve-core` 并发写；`MAX(seq)+1` 是读-改-写（[store.rs:1205](src/core/history/store.rs)），并发时唯一索引冲突 → 被 D-1 放大为重复/丢消息。
- **改法**：`ensure_stopped` 改探 `core.lock`（与 `serve::acquire_core_lock` 共用一个函数）或提供 `--allow-running`；所有写命令统一检查；`seq` 改单语句子查询插入或捕获 `SQLITE_CONSTRAINT` 重试（M）。
- **验收**：`serve-core` 运行时执行 `control migrate-history` 必须报"先停服务"；两连接各写 50 条断言 100 条且 `integrity_check=ok`。

### D-4 备份不含 `media/` 附件目录

- **证据**：附件落盘在 `state/history/media/<sha前2>/…`（[store.rs:943](src/core/history/store.rs)）；`backup.rs` 全文无 `media`，manifest 只列 DB（[src/core/backup.rs:253-292](src/core/backup.rs)）。
- **后果**：恢复后图片/文件全部打不开（`query.rs` 报 media file missing）；备份"完整"是假象。
- **改法**：内容寻址增量复制 `media/` 到备份目录，manifest 记录文件数与字节数，恢复时交叉校验（M）。

### D-5 `--out` 指向已存在目录会被 `remove_dir_all`

- **证据**：[src/core/backup.rs:295](src/core/backup.rs)：`if out_dir.exists() { fs::remove_dir_all(&out_dir)?; }`；自定义 `--out` 直接作为 `out_dir`（`:232`）。
- **后果**：`wechat-mcp backup --out ~/Documents/backup` 会清空该目录里所有无关文件；rename 失败时旧备份也没了。
- **改法**：只允许替换空目录或本程序生成的备份目录（manifest + 结构校验），否则 bail；删除前先 rename 到 `.trash-<uuid>`（S）。

### D-6 在线备份无 BUSY 重试，源库以读写方式打开

- **证据**：[backup.rs:87](src/core/backup.rs) `SQLITE_OPEN_READ_WRITE`、`sqlite3_backup_step(p, -1)` 一次拷完、无 `busy_timeout`。
- **后果**：服务运行中手动备份遇到写入竞争直接失败并删除临时文件。
- **改法**：源库 READONLY + `busy_timeout(5000)`，按 SQLite 官方模式循环 `step(128)` 处理 `BUSY/LOCKED`（S）。

### D-7 没有恢复路径，也没有摘要

- **证据**：子命令只有 `backup`（[src/main.rs:95](src/main.rs)）；manifest 只有版本/时间/文件名单（[backup.rs:285](src/core/backup.rs)），无每库 sha256、schema 版本、`foreign_key_check`。
- **后果**：灾难恢复只能手工 `cp`；若残留旧 `-wal` 会重放出陈旧数据且无人报警。
- **改法**：新增 `restore`（校验 manifest → 抢 `core.lock` → 旧库移到 `.restore-old-<ts>` → 复制 DB+media → `integrity_check`/`foreign_key_check`/行数对比 → 失败自动回滚）（M）。

数据侧 P2（随下次 schema 变更一起做）：`media_jobs(contact_id,status)`、`outbox(platform,account_id,created_at)`、`events(platform,account_id,observed_at)` 三个缺失索引；`search_messages` 上下文 N+1、`list_contacts` O(N²)；无保留期/清理；未发送的 bot 回复也写进 `messages` 出现在历史；HTTP 服务 bind 未限 loopback。

---

## 4. P1：运行时可靠性（超时 / 取消 / panic / 信号）

### R-1 桌面操作全链路无超时，锁也没有租约

- **证据**：[src/adapters.rs:80](src/adapters.rs) 取得 `DesktopLease` 后整段操作无 `timeout`；[src/wechat.rs:256](src/wechat.rs) 的 `osascript` 用 `wait_with_output()` 无限等；[src/article.rs:207](src/article.rs) Chrome、`:262` curl 同样；[src/main.rs:233](src/main.rs) 每请求无超时，且 `send_message/send_file/focus_wechat` 被显式排除在可取消之外（[main.rs:283](src/main.rs)）。
- **后果**：任一 AX/权限弹窗/Chrome 首次运行卡住 → 该请求永不返回，并持有进程内 gate + 跨进程 `desktop.lock`；其它账号/进程 30s 后报"桌面被另一个 MCP 进程占用"，64 个 JoinSet 槽耗尽后全部 `-32000`。**一台机器上的微信自动化整体停摆，直到杀进程。**
- **改法**：`WechatDesktop::execute` 外层加操作级超时（读 30s / 发送 60s），超时返回"结果未知，请用 get_send_status 核对"；`run_applescript`/Chrome 统一 15s 超时 + `start_kill()`+`wait()` 回收；`main.rs` 加请求级超时（S/M）。
- **验收**：把 helper 换成 `sh -c 'sleep 3600'`，断言超时后返回错误且 lease 已释放。

### R-2 一个工具 task panic 直接退出整个 MCP 服务

- **证据**：[src/main.rs:169](src/main.rs)：`Some(Err(error)) if error.is_cancelled() => {}`，其余 `return Err(error.into())` → 进程退出；非测试代码仍有 117 处 `.unwrap()`，其中 [main.rs:358](src/main.rs)、[gateway.rs:81](src/gateway.rs) 在每次 `tools/call`/截图返回路径上。
- **后果**：单个 panic 带走所有在途会话，故障域是"整个服务"。
- **改法**：`JoinError::is_panic()` 记录 + 返回 JSON-RPC internal error 后 `continue`；`gateway.call` 外层 `catch_unwind`；`conn.lock().unwrap()` 统一防毒化（S）。

### R-3 core 热路径在 async 里同步跑 rusqlite/fs

- **证据**：[src/core/turns.rs:864-897](src/core/turns.rs) 的 `async fn record` 内直接 `history.record_frame/observe_name/attach_media`；`turns.rs:1535` `record_bot_reply`；`scheduler.rs:1269/1559`。`AppState` 用 `Arc<Mutex<Connection>>`，`busy_timeout=5000`（[store.rs:258](src/core/history/store.rs)）。对照 `storage.rs:126` 的 `Store::run` 已正确用 `spawn_blocking`，core 目录 0 处。
- **后果**：WAL 写锁被历史 HTTP/备份占用时，单次同步调用最多阻塞 runtime worker 5 秒；多会话 sweep 串行放大，定时器/取消精度被打穿。
- **改法**：`HistoryStore` 增加 `async` 包装走 `spawn_blocking`；长期改为单 DB actor（mpsc 串行）彻底去掉跨线程 `Mutex<Connection>`（M）。

### R-4 SIGTERM 只在 `serve-core`、且只在 sweep 边界检查

- **证据**：信号只在 [src/core/serve.rs:30](src/core/serve.rs) 安装；检查点 `serve.rs:302/384` 都在 `sweep()` 返回后；单轮 sweep 可包含"至少 10 分钟"的通讯录全量（`scheduler.rs:2533` 注释）；换号时旧 core.sock 连接任务不 abort（`serve.rs:422` / `external_brain.rs:494`）。
- **后果**：SIGTERM 后可能 10 分钟不退出；宿主 `control stop` 只等 60s 后强杀 → 留下 `attempting` journal → 下次启动 `paused` 停掉自动回复。
- **改法**：`CancellationToken` 贯穿 serve→coordinator→service→worker，替换散落的 `Arc<AtomicBool>` 轮询；sweep 可被取消；关停时把 running journal 标 `stopped_before_send`；socket accept 用 JoinSet 统一 abort（M）。

运行时 P2：`worker_client.call` 持 35s `rpc_lock` 队头阻塞（`worker_client.rs:447`）；`external_brain` 的 `notify_waiters()` 丢失唤醒（`:201` vs `:310`，最多空等 60s）；overlay 取消后不 `resume`（`overlay.rs:132`）；`ClipboardGuard::Drop` 在 runtime 线程同步跑 `pbcopy`（`clipboard.rs:96`）；`let _` 吞掉 `update_send`/`get_send_status`/自动同步错误（`turns.rs:711/1117`、`scheduler.rs:1814`）。

### R-5 测试不隔离：锁与 Swift 配置保存都写真实 `$HOME`

- **证据**：[src/resource_lock.rs:49](src/resource_lock.rs) 固定 `$HOME/Library/Application Support/message-gateway/desktop.lock`；Swift 侧 `getSecureTempDirectory()` 固定 `~/.wechat-rpa/tmp`（[native/bootstrap/ConfigForm.swift:340](native/bootstrap/ConfigForm.swift)），且 `homeDirectoryForCurrentUser` 不读 `$HOME`。
- **本次实测**：`cargo test --locked` 在沙箱中 3 个单测 + 9 个 `swift_host` 失败，错误均为 `Operation not permitted` / "没有权限在 tmp 中保存"。**这不是业务断言漂移，而是测试无法 hermetic 运行**；换到可写 HOME 的机器/CI 会通过，但开发者本机已有实例运行、或 CI 容器 HOME 不可写时会随机红。
- **改法**：`ResourceLock` 支持注入 state 目录（`WECHAT_MCP_STATE_DIR` 或参数）；Swift 的 secure temp/bot 目录支持环境变量注入（测试用 `target/` 下目录）；测试统一 `--no-fail-fast`（S/M）。

---

## 5. P1：对外契约与可观测性

- **C-1 工具契约缺元数据**：运行时 51 个工具，23 个有 `annotations`，**0 个 `outputSchema`**；错误只有自然语言（实测 `core_not_running`），调用方无法区分"不可重试/结果未知/可重试"。Agent 只能靠字符串匹配（如 `未触发发送`）做安全决策。
  → 改法：给全部工具补 `readOnlyHint/idempotentHint/destructiveHint` + `outputSchema`；错误结构统一 `{code, retryable, side_effect, send_id, recovery}`，`structuredContent` 成功/失败一致（M）。
- **C-2 可观测性过低**：`tracing::` 计数 gateway/mcp/tools/storage/mcp_client = 0，scheduler = 3，turns = 2；主要依赖 `health.json`，而多处写入是 `let _ =`（`serve.rs:79/333/411`）。自动化停止的根因（如超时、锁等待）在日志里没有痕迹。
  → 改法：在 `gateway.execute` 入口/出口、`mcp_client.request` 失败、`worker_client.call` 超时、`external_brain.handle_client` 增加结构化 `tracing`（tool/op/耗时/error_type，**不带消息正文**）；`log_event` 统一 `{:#}`（S）。
- **C-3 文档与代码漂移**：README 写"部署三个文件"（实际 release 打 9 个）；README/`docs/rpa-runtime.md` 仍给 `.py` 命令；`docs/reviews/2026-11-08-*.md` 中已修的 P0（brain_tools 未分发、codex-exec 缺后端、信号未共享）在 `d555a68` 已修复，但文档仍列为未修，误导后续判断。
  → 改法：文档命令改为 `cargo test --locked --test ...`；产物清单以 `install.sh` 的 `TOOLS` 为单一事实源生成；给历史 review 加"已修复/仍有效"状态标注（S）。

---

## 6. P1：构建 / 发布 / 测试体系

- **B-1 四套构建定义漂移**：`build.sh`/`ci.yml`/`release.yml`/`deploy_host.sh` 文件集与 `-target` 不一致；`native/bootstrap` 19 个 Swift 文件只有 `deploy_host.sh` 编译；release 不打包 `rapidocr_server.py`（`build.sh:14` 有），导致 release 安装与源码安装产物集合不同。
  → 单一 `scripts/build_native.sh`（manifest 一份），统一 `MACOSX_DEPLOYMENT_TARGET=12.0`，release 加产物一致性校验（M）。
- **B-2 发布无签名/公证，安装脚本主动摘 quarantine**：`release.yml` 无 `codesign/notarytool/stapler`；`install.sh:940-946` 从同一 release 取 tar 和 sha（同源校验不防篡改）；`:1001` `xattr -dr com.apple.quarantine`。程序要申请辅助功能/屏幕录制权限，这是实质风险。
  → release 加 Developer ID 签名+公证；安装时 `codesign --verify` + `spctl -a`，失败即停；校验值入库或走 GitHub Attestations；去掉无条件 `xattr -dr`（L/S）。
- **B-3 Python 测试体系半失效**：388 个用例中 157 个不在 CI；12+ 个文件单独运行即 `ModuleNotFoundError`（无 `conftest.py`/`sys.path` 设置）；`python3 tests/x.py` 与 `unittest discover` 两套入口并存。
  → 加 `tests/conftest.py` + `pyproject.toml [tool.pytest.ini_options]`，CI 一条 `pytest tests -q`；随后按 P5 计划迁移到 Rust，用黄金值固化后再删 Python（S/M）。
- **B-4 工具链/供应链未锁**：无 `rust-toolchain.toml`；CI 无 `cargo audit`/`cargo deny`/依赖许可检查。
  → 锁 1.99.0；加 `cargo audit`（或 `cargo deny`）为独立 job（S）。
- **B-5 `target/` 18 GB**：debug/deps 15 GB、incremental 1.7 GB。
  → CI/本地加 `cargo clean -p` 文档、`debug = 0`/`split-debuginfo` 调研、`sccache` 可选；这属于开发体验，不是产品问题（S）。

---

## 7. P2：架构与重复（值得做 / 不值得做）

**值得做（按顺序）**

1. **history 工具面双写**：[src/core/history/serve.rs:117-330](src/core/history/serve.rs) 与 [mcp_tools.rs:44-367](src/core/history/mcp_tools.rs) 各写一份 8 工具表 + 参数校验 + 分发（约 1,560 行重复窗口），且 `category` 参数只在一边支持——**已经行为漂移**。抽 `core/history/tools.rs`，两个薄适配；先加两入口 JSON 等价测试（S–M，净删代码）。
2. **跨层循环**：`CoordinatorCapture` trait 定义在高层的 [scheduler.rs:285](src/core/scheduler.rs)，反被 `worker_client`/`contacts-sync` 实现，形成 `scheduler↔worker_client/control/contacts-sync`、`turns↔worker_client` 4 条循环。抽 `core/ipc.rs` + `core/capture.rs`（M–L）。
3. **`control.rs` 巨函数**：`execute_control_full`（`control.rs:583-1832`，1,250 行，近似 CC≈194），`--config` 解析复制 6 次。按 args/lifecycle/status/history-admin/calibrate/contacts 拆文件，公开签名不变；现有 2,099 行测试可作 characterization（M）。
4. **5 套 Unix socket 客户端**：`mcp_client` 40s、`worker_client` 35s、`control` 20/60/10s、`brain_tools` 完全无超时。抽统一 `ipc` 客户端（带超时/重连/请求 ID 跳过期响应）（M）。
5. **网关内嵌 SQL/CRM**：`gateway.rs` 直接建 Store、`control` 直接开历史库/跑迁移 → 收敛到 store 层（S）。

**明确不建议做（避免过度设计）**

- 不按表拆 `store.rs`（单写者 + 跨表事务，拆了反而破坏事务边界）。
- 不把巨型文件里的测试迁出（测试占全仓 49%，迁出要泄 `pub(crate)`，收益低）。
- 不重写 `policy.rs`（纯函数 + Python 黄金对拍，重写风险高）。
- 不给 `model_api` 造 provider trait（当前只有一个后端分支）。
- 不引入 SwiftPM/多 crate workspace（收益 < 迁移成本）。
- **不要先删 Python**：CI 里 Rust 测试仍用 Python 做 oracle，先固化黄金值再删。

---

## 8. 优化路线图（可按阶段独立交付）

### Phase 0 — 恢复门禁（0.5–1 天，全部可当天验证）

- 修 P0-1（click 文件清单）/ P0-2（.py 入口）/ P0-3（shellcheck）。
- `cargo fmt --all`（单独 commit）+ `rust-toolchain.toml` + clippy 分级/`--fix`。
- release 手动触发的版本号取错修正。
- **验收**：CI 两个 job 全绿；本地 `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --locked`、`bash scripts/build.sh` 全绿。

### Phase 1 — 数据正确性（3–5 天）

- D-1/D-2 事务化；D-3 统一 `core.lock` 运行判定；D-4 备份含 media；D-5 `--out` 保护；D-6 BUSY 重试；D-7 `restore` + manifest 摘要。
- **验收**：新增故障注入测试（触发器/只读库/两连接并发）全绿；"备份→删源→恢复→抽查附件"脚本通过；`serve-core` 运行时 `control migrate-history` 被拒绝。

### Phase 2 — 运行时边界与可观测性（1–2 周）

- R-1 操作级/请求级超时；R-2 panic 隔离；R-3 `spawn_blocking`；R-4 CancellationToken；R-5 state dir 注入使测试 hermetic。
- C-1 错误结构 + 关键工具 `outputSchema`；C-2 结构化日志。
- **验收**：注入"helper 挂起"用例 ≤ 配置超时返回且锁释放；`kill -TERM` ≤ 2s 退出且无 `attempting` 残留；`cargo test` 在只读 HOME 的沙箱中也能全绿。

### Phase 3 — 构建/发布/契约工程化（2–3 周）

- B-1 统一 `build_native.sh`；B-2 签名/公证/安装校验；B-3 pytest 单一入口 + Python 迁移清单；B-4 `cargo audit`。
- 工具面：按场景提供工具预设（客服/历史/桌面/文章），默认集合从 51 收敛；补 `annotations`。
- **验收**：release 产物清单与 `install.sh` 自动比对通过；未签名包装不上；`pytest tests` 收集 ≥388 全绿。

### Phase 4 — 架构收敛（持续，按季度）

- history 工具面单一事实源 → IPC 抽取 → control 拆分 → 网关 SQL 内收。
- **验收**：`cargo tree`/模块依赖脚本无环；`serve.rs`/`mcp_tools.rs` 双写删除后，两入口契约测试仍绿。

---

## 9. 审计可信度：已否证 / 已修正 / 未复现

| 项 | 结论 |
|---|---|
| ~~P0：`mcp_client` 超时后 transport 毒化、永久 ID mismatch~~ | **否证**。`request()` 在任何错误后已 `state.transport = None`（[src/core/mcp_client.rs:131-135](src/core/mcp_client.rs)），下次自动重连；`request_internal` 只被该 wrapper 调用。审计代理误读了内部函数，未纳入本报告。 |
| 审计代理称"9 个 `swift_host` 失败是真实行为漂移" | **修正**。直接运行 `host_selftest` 复现错误为"没有权限在 tmp 中保存文件"，根因是 `getSecureTempDirectory()` 写真实 `~`（`ConfigForm.swift:340`），本会话沙箱拒绝；真实问题是**测试不隔离**（R-5），不是 Swift 契约漂移。CI 可写 HOME 上大概率通过。 |
| `cargo test` 3 个单测失败 | 同上，`ResourceLock` 固定真实 `$HOME`；用可写 HOME 复跑 `gateway::tests::failure_injected_at_click_records_not_sent` 已通过。 |
| 已独立实测确认 | click 漏编译（`42ef72a` 复测 8 个 error）、CI `.py` 不存在、shellcheck SC1010、fmt 1652 处 diff/51 文件（`42ef72a` 复测）、51 工具/0 outputSchema、错误无结构、panic 退出服务、`record_frame` 无事务、`record_bot_reply` 非原子、`migrate` 锁名不一致、备份无 media、`--out` remove_dir_all、adapters/osascript 无超时、`~/.wechat-rpa/tmp` 写拒绝。 |
| 仅代码证据、未运行复现 | clippy 各 lint 计数、release 签名结论、Python CI 覆盖数、N+1/O(N²) 查询性能、历史库并发冲突（未做真实双进程压测）。 |

---

## 10. 复现命令（全部只读/只写 `target/`、`/tmp`）

```bash
# 门禁现状
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --no-fail-fast
shellcheck -S warning scripts/*.sh scripts/hooks/*

# P0-1 复现（应报 cannot find 'runClearInput'）
swiftc native/click/main.swift native/common/*.swift native/action/*.swift -o /tmp/wc

# P0-2 复现
for f in $(grep -rhoE 'tests/[A-Za-z0-9_./-]+\.py' .github README.md docs/rpa-runtime.md); do test -f "$f" || echo "MISSING $f"; done

# 运行时工具契约
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"a","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
 | HOME="$PWD/target/fakehome" ./target/debug/wechat-mcp | jq -c 'select(.id==2).result.tools | length'
```

> 原始分组报告（含每条证据与更细的 P2 清单）：`target/analysis/async-audit.md`、`data-audit.md`、`build-audit.md`、`arch-audit.md`。
