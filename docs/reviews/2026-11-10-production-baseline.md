# Q0 既有功能回归基线与本地门禁修复

日期：2026-11-10。结果：**partial（门禁修复完成，业务基线有失败，非项目验收通过）**。

## 基线、范围与授权

- 初始 HEAD：`32bb12e550b6739c5c7d6874bbd7388cd771ef5c`；`git status --short` 输出为空，退出 0；分支 `work/prod-regression-gates`。
- 仅修改两个本地门禁、独立 shell 夹具和本文；没有改业务源码、GUI、workflow、数据库结构或业务资料。原 checkout 只读读取方案和任务总表。
- 先阅读本 worktree AGENTS.md。codebase-memory-mcp fast 索引成功，搜索返回测试节点；scripts/docs 被索引排除，因此直接读文件。Swift 图解析部分覆盖不足，测试入口以源码核对。
- Orca 协调者答复 `msg_96053c1ca1a1` 明确批准仅三个已跟踪的离线商业报告生成器：`docs/business/build_model.py`、`build_compliance_model.py`、`render_report.py`。保留原资料，其他任意 `.py` 包括同目录新增文件仍拒绝；不是“全仓无 Python”。

## 测试安全边界

先核对 Rust 单测、协议 smoke 及所选 Swift harness：协议在临时目录建立 osascript/screencapture/click/window 替身；模型单测使用本地响应/脚本替身；所选 Swift 只运行几何、时间标签、语音识别纯逻辑与有界超时，没有启动 App/微信。

执行脚本位于本机 `/tmp/q0-baseline-evidence/run.sh`。每轮设置独立 `/tmp/q0-isolation.*/home`、`state/gateway.sqlite3`，导出 `HOME`、`WECHAT_MCP_HOME`、`MESSAGE_GATEWAY_DB`；Cargo/Rustup 缓存路径保持为工具缓存。单线程运行测试，未并发 cargo；PATH 前置拒绝真实 osascript、screencapture、launchctl、open、微信 helper、codex/claude 的退出 125 替身；清除 OPENAI/ANTHROPIC/DEEPSEEK API key 环境变量。测试内部替身优先于这些防护。

未读取真实业务 DB，未发送真实消息，未启动 GUI、付费模型、部署、发布或远端诊断操作。账号单测核对的是合成 lsof 文本和临时目录，不以此证明真实账号检测链路。

## 实际命令与结果

环境：macOS Darwin 27.3 arm64；rustc 1.99.0，Apple Swift 6.4.2。以下 Rust/Swift 命令使用上述隔离变量；`--test-threads=1` 避免环境变量测试相互污染。运行时业务源码保持基线原样；其后只编辑门禁和报告，未重复无关测试。

| 命令 | 退出码 | 实际结果 |
| --- | ---: | --- |
| 基线 `bash scripts/check_no_python.sh` | 1 | 拒绝三个已提交商业资料脚本 |
| 基线 `bash scripts/test-gates.sh` | 1 | 先被上述 Python 检查阻断 |
| 基线门禁复制到独立临时 Git 仓库、不放商业资料后执行 | 1 | 继续暴露过时 `grep continue-on-error: true`，当前 CI 实际无该配置 |
| `cargo test --locked --bin wechat-mcp -- --test-threads=1` | 101 | **893 passed，1 failed，0 ignored**；构建 35.78s，测试 67.46s |
| `cargo test --locked --test protocol_smoke -- --test-threads=1` | 0 | 2 passed，0 ignored；协议替身链路 1.38s |
| `cargo test --locked --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1` | 0 | common 7、time 1、voice 3、bounded 3：共 14 passed，0 ignored |
| `rustfmt --edition 2024 --check $(git ls-files '*.rs' \| grep -v '^src/interaction.rs$')` | 0 | 与当前 CI 同一目标，未扩大既有排除；非全仓 fmt 证明 |
| `cargo clippy --all-targets --locked` | 0 | 14.87s；既有大量警告，test target 报 1937 warnings（含 1439 duplicates）；未放松 lint |
| 修复后 `bash scripts/test-gate-fixtures.sh` | 0 | 18 个独立场景通过，详见下节 |
| 修复后 `bash scripts/test-gates.sh` | 0 | 构建/CI/release 静态检查与原有两个负向夹具通过；release branch 拒绝日志是预期 |
| 修复后 `bash scripts/check_no_python.sh` | 0 | 精确离线例外、其余 Python 禁令及运行入口静态引用检查通过 |
| `shellcheck -S warning scripts/test-gates.sh scripts/check_no_python.sh scripts/test-gate-fixtures.sh` | 0 | warning 级检查通过 |
| `bash -n`（上述三个文件逐个执行） | 0 | shell 语法通过 |
| `git diff --check` | 0 | 最终无空白错误 |

日志保留于本机 `/tmp/q0-baseline-evidence/`（临时证据，不保证跨机器存在），包含三组 cargo test、fmt、clippy 输出及隔离目录路径。没有将编译/安装完成当作真机链路成功。

## RED / GREEN 与修复

1. 先新增独立夹具，未改门禁时退出 **1（RED）**：三个获批离线脚本与现有硬 CI 应被允许，却被旧全仓 Python 禁令拒绝。
2. 另用基线脚本构造无商业资料的临时仓库，退出 **1（RED）**：当前硬 CI 被过时 continue-on-error 断言拒绝。
3. `test-gates.sh` 改为要求 test job 存在 fmt/clippy 步骤及实际检查命令，拒绝 test job 内任何 continue-on-error 字段。不改 workflow，不给 fmt/clippy 加软门禁。
4. `check_no_python.sh` 只允许三个精确且已跟踪路径；活动文档规则不变。对 scripts、workflows、src、native、skills 及根构建/安装入口扫描 helper 文件名或 `docs/business` 引用，出现则失败，避免离线资料被产品调用。
5. **GREEN**：独立夹具 18 场景：两次允许正常配置；五个目录新增 Python 均拒绝；fmt/clippy 各自 continue-on-error 均拒绝；各自检查命令替成 `run: true` 均拒绝；build/install/src/native/CI/release 六处引用离线 helper 均拒绝；活动文档 Python 引用拒绝。原有脚本另有两个 Python/活动文档反向夹具。

静态门禁按当前 workflow 的常规缩进与直接命令结构检查，并非通用 YAML 解释器；未来重构 workflow 应同时调整夹具。引用检查覆盖现有直接路径、文件名和整个 business 目录打包引用，不声称能识别动态拼接或外部下载的任意执行代码。

## 既有业务失败与已确认范围

唯一 Rust 失败：`core::open_api::compat_tests::ops_tools_list_unchanged`，`src/core/open_api/compat_tests.rs:49` 对当前 `list_tools_with_legacy(true)` 和 `tests/fixtures/ops_tools_list.json` 的 pretty JSON 全量相等断言失败。原样基线已复现，未改 fixture、删测试或隐瞒失败，具体协议差异留给所属任务调查。

本次通过数（不替代真实微信验收）：history 125、config 12、builtin brain 8、model API mock 23、external brain 33、turns 113、scheduler 123、policy 42、wechat_account 5、backup 17、open_api 54（另 1 失败）、worker_client 52；其余通过项在完整日志中。包含历史账号隔离、外部权限、发送幂等及未知发送合成故障路径。

## 未验证与后续不回归集合

未执行全量 integration、完整原生 App 构建、安装/部署脚本、host GUI、真实窗口/屏幕权限/微信消息、真实模型连接、远端状态和连续运行；这些是 **unknown**，没有以跳过计全绿。未审查的 integration 入口不能仅凭隔离 HOME 就推定无 GUI 副作用，本轮未运行。没有执行发布与两个架构完整构建；构建环境无阻塞，但警告与业务基线失败独立记录。

后续功能回归至少保留：

- 本次 `--bin wechat-mcp`、协议 smoke 和四个 Swift 纯逻辑目标，以及独立门禁夹具；Open API schema 失败必须在所属变更中解决或继续明确阻塞，不能更新快照掩盖契约回归。
- 历史按账号/联系人隔离，备份恢复保留附件与数据，旧配置兼容；外部大脑模式、模型/连接、白名单来源、群聊 @、转人工和会话暂停规则不变。
- 同幂等键重试不重复触发；发送未知不盲重发；观察回显不升级为送达事实；权限缺失/遮挡/超时/人工干预释放资源并保留具体状态。
- 手/眼真机另行授权：首条未读顶到上方后的衔接恢复与回到底部确认、同时长语音身份/位置跟踪、相同文本连续到达、多会话、重名/改名、附件/语音、微信退出与换号。

本任务只交付局部门禁修复和可复核基线；非零业务测试意味着整体回归仍为 partial。
