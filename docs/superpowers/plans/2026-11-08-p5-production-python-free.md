# P5 生产上线与撤除 Python Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 产品运行时和仓库里都不再有 Python，只剩一套程序、一个部署入口，升级后不需要重新授权，具备生产运维能力。

**Spec:** `docs/superpowers/specs/2026-11-08-production-python-free-design.md`（以其中第 2 节的用户决定为准）

## Global Constraints
- 不改变已经在真机上验证过的行为，包括：发送路径（`input-text` → 核对标题 → `return`）、幂等键、日志文件格式、数据库结构、现有 MCP 工具的契约。
- 只用 Vision 做文字识别，不引入新的 OCR 依赖。
- 测试统一放在 `cargo test` 下：Swift 组件的测试改成 Rust 集成测试（`tests/*.rs`），由 `swiftc` 编译自测程序后检查输出。
  - 开发机上有 `swiftc` 时才运行，没有时用 `eprintln!` 说明原因后跳过，不算失败；
  - CI 的 macOS runner 上一定有 `swiftc`。
- 测试必须在前台运行。开发机负载高，不要运行 Python 全量测试。
- 不部署，不 push。真机步骤由协调者执行。

## Review Focus
1. 测试迁移不能删减断言：每个 Python 测试都要对应一个同名（或带前缀）的 Rust 测试，worker 的报告里要附上对照表。
2. 删除 RapidOCR 以后，`wechat_ocr` 和 `wechat_bubbles` 在 Vision 识别失败时，必须给出明确的错误，不能悄悄返回空结果。
3. 备份：服务运行期间（数据库有 WAL 时）做备份，备份出来的库也必须能正常打开，并通过 `PRAGMA integrity_check`。
4. 签名：部署脚本找不到签名证书时，要报错停止，不能退回临时签名。退回临时签名会让授权失效，而且很难发现原因。
5. 日志轮换：发生轮换的那一刻，日志不能丢失，也不能有两个进程同时写同一个文件。

---

### Task 1: 历史查询 MCP 工具（Rust）
**对应 Python：** `scripts/wechat_bot/history/mcp_server.py`（工具名、参数、返回结构保持一致）。
**Files：** `src/core/history/mcp_tools.rs`（新建），`src/tools.rs`（登记工具），`src/core/history/mod.rs`（登记子模块）。
**做法：** 读 T2 的 `HistoryStore` 和查询模块，只读不写。历史库的路径从配置的 `state_dir` 推出来，读不到配置就返回明确的错误。
**测试：** 翻译 `tests/test_history_servers.py` 中 MCP 部分的用例，并检查 `tools/list` 里包含这些工具、原有工具不变。
**Commit：** `feat(mcp): 历史查询 MCP 工具（替代 history/mcp_server.py）`

### Task 2: 备份与日志轮换（Rust core）
**Files：** `src/core/backup.rs`（新建），`src/main.rs`（新增 `backup` 子命令），`src/core/serve.rs`（空闲时每天备份一次）。
**内容：**
- `wechat-mcp backup --config <cfg> [--out <dir>]`：用 rusqlite 的在线备份 API，备份 `state/history/history.sqlite3` 和 `~/.wechat-rpa/test.sqlite3`，输出到 `<state_dir>/backups/YYYYMMDD-HHMMSS/`，权限 0700/0600。只保留最近 7 份。
- `serve-core`：没有正在处理的消息、并且距离上次备份超过 24 小时时，自动做一次备份。
**测试：**
- 在 WAL 模式下一边写一边备份，备份出来的库要通过 `integrity_check`；
- 只保留 7 份；
- 权限正确；
- 翻译 `scripts/backup_db.py` 的行为。
**Commit：** `feat(core): wechat-mcp backup 与每日自动备份（替代 backup_db.py）`

### Task 3: Rust 核心的画面录制
**Files：** `src/core/worker_client.rs`（在 `frame` 和 `scroll` 里加录制），`src/core/replay.rs`（`--journals` 改为同时支持 Rust 写的日志）。
**内容：** 行为和 Python 的 `dump_frame` 完全一样：
- 开关 `<state_dir>/frame-dump.enabled`；
- 写到 `sessions/<id>/frames-YYYYMMDD.jsonl`，权限 0600；
- 只保留 3 天，单个文件最大 50MB；
- 写入失败不影响截屏。

测试对照 `tests/test_frame_dump.py` 翻译。
**Commit：** `feat(core): serve-core 画面录制与回放支持 Rust 日志`

### Task 4a: Swift 组件测试迁移（动作、点击、宿主）
**把这些 Python 测试翻译成 Rust 集成测试：** `tests/test_action_module.py`、`tests/test_host_status.py`、`tests/test_host_supervisor.py`、`tests/test_host_core.py`。新文件放在 `tests/swift_action.rs`、`tests/swift_host.rs`。
`tests/action_selftest`、`tests/host_selftest` 里的 Swift 自测程序保持不变，Rust 负责编译和检查结果。
测试通过以后，删除对应的 `.py` 文件。
**Commit：** `test: 动作/点击/宿主的 Swift 测试迁移为 Rust 集成测试`

### Task 4b: Swift 组件测试迁移（识别、屏幕、布局、协议）
**把这些 Python 测试翻译成 Rust 集成测试：** `tests/test_native_frames.py`（不含 RapidOCR 的用例）、`tests/test_screens.py`、`tests/test_anchors.py`、`tests/test_common_layout.py`、`tests/test_common_layer.py`、`tests/test_layout_consistency.py`、`tests/test_emoji_lexicon.py`、`tests/protocol_smoke.py`、`tests/desktop_runtime_smoke.py`。
测试通过以后，删除对应的 `.py` 文件。
`test_native_frames` 依赖 Vision，开发机负载高时可能超时，这种情况允许跳过，并在报告里写明。
**Commit：** `test: 识别/屏幕/布局/协议测试迁移为 Rust 集成测试`

### Task 5: 签名、版本号与宿主日志轮换（Swift 和脚本）
**Files：** `scripts/setup_signing.sh`（新建）、`scripts/deploy_host.sh`、`native/bootstrap/{main,StatusPanel,CoreSupervisor}.swift`、`native/bootstrap/Info.plist`。
**内容：**
- `setup_signing.sh <host>`：在专用 Mac 的登录钥匙串里生成自签名的代码签名证书 `WeChat CS Local Signing`（用 openssl 生成，再用 `security import`），证书已经存在时什么都不做。
- `deploy_host.sh`：
  - 所有产物都用这个证书签名，找不到证书就报错停止；
  - 把 git 提交号和时间写进 Info.plist 的 `CFBundleVersion`，以及 `~/.wechat-rpa/VERSION`；
  - 删除 `--bot`。
- 面板显示版本号。
- `CoreSupervisor` 写 `core.log`、宿主写 `host.log` 时做轮换：超过 10MB 就改名为 `.1` 到 `.5`。
**测试：** 版本号写入和轮换逻辑的单元测试；部署脚本的 dry-run 输出检查。
**Commit：** `feat(deploy): 本机固定签名证书、版本号与日志轮换`

### Task 6: 删除 Python 与 RapidOCR（等 Task 7 的观察期通过后才合并）
- 删除：
  - `scripts/wechat_bot/`
  - `scripts/agent_bridge/`
  - `scripts/backup_db.py`
  - `native/rapidocr_server.py`
  - 其余所有 `tests/*.py`，即 P2 已经翻译过的机器人测试
- `wechat_ocr.swift`、`wechat_bubbles.swift`：删除 RapidOCR 分支，Vision 识别失败时返回明确的错误。
- `install.sh`、`build.sh`、`.github/workflows/*.yml`：删除 python、pip、rapidocr 相关内容，CI 只跑 `cargo test` 和交叉编译。
- 用 `git ls-files '*.py'` 检查，结果必须为空。
**Commit：** `chore: 移除 Python 机器人、agent_bridge 与 RapidOCR，仓库零 Python`

### Task 7: 真机上线（协调者）
1. 在专用 Mac 上执行 `setup_signing.sh`，然后部署。这是最后一次需要重新授权；第一次签名时，钥匙串可能会弹窗，需要用户点"始终允许"。
2. 切换：
   - `launchctl bootout local.wechat-rpa.bot`；
   - 创建 `core.enabled`；
   - 在示例群逐项验收：文字、图片、文件、群聊、转人工后再恢复、外部大脑的 send / skip / handoff / 超时、发送失败重试、校准。
3. 观察 3 天。
4. 合并 Task 6，部署零 Python 的版本，在专用 Mac 上删除 Python 的 launchd plist、`~/.wechat-rpa/bot` 和 pip 包（删除前先列出清单，请用户确认）。
5. 再部署一次，验证升级后不需要重新授权。

### Task 8: P2 迁移缺口补齐（2026-11-08 协调者核对发现）

删除 Python 之前逐条比对：以下 Python 测试在 Rust 中没有对应测试。P2 各 worker 曾报告"全部翻译"，与事实不符。必须逐条迁移（缺功能就实现），否则不得进入 Task 6b。

不需迁移（有理由）：test_agent_bridge.py（删除 agent_bridge）、test_rapidocr_server.py（只用 Vision）、test_transport_local.py 的 local/remote argv 三项（只在本机运行）、test_wechat_bot_service.py::test_main_builds_coordinator_inside_running_loop 与 ::test_load_config_accepts_brain（Python 专属；Rust 配置测试已覆盖 brain）。

必须迁移：
- tests/test_attachment_limit.py test_store_attach_media_follows_configurable_limit_and_names_it
- tests/test_attachment_limit.py test_swift_sources_use_max_bytes_and_no_received_10mb_constants
- tests/test_calibrate.py test_no_4_1_15_fallback_in_source
- tests/test_calibrate.py test_version_regex_validation
- tests/test_calibrate.py test_worker_missing_version_aborts_without_writing
- tests/test_calibrate.py test_worker_calibrate_response_fields_in_swift
- tests/test_calibrate.py test_candidate_written_to_remote_wechat_host
- tests/test_calibrate.py test_accept_refuses_version_mismatch_with_zero_remote_writes
- tests/test_calibrate.py test_accept_refuses_corrupt_candidate_with_zero_remote_writes
- tests/test_calibrate.py test_accept_executes_backup_then_mv_atomically
- tests/test_calibrate.py test_cli_calibrate_command
- tests/test_calibrate.py test_cli_calibrate_accept
- tests/test_calibrate.py test_calibrate_never_clicks
- tests/test_calibrate.py test_default_paths_land_in_remote_home
- tests/test_calibrate.py test_accept_with_defaults_backs_up_then_replaces
- tests/test_calibrate.py test_missing_host_is_refused
- tests/test_calibrate.py test_failed_remote_write_raises
- tests/test_calibrate.py test_calibrate_refuses_while_bot_running
- tests/test_calibrate.py test_control_calibrate_round_trip
- tests/test_calibrate.py test_control_calibrate_requires_running_bot
- tests/test_calibrate.py test_anchors_measure_the_raw_capture
- tests/test_calibrate.py test_text_anchors_are_not_reported_missing
- tests/test_history_servers.py test_history_http_launchd_plist
- tests/test_transport_local.py test_mcp_timeout_raises_timeout_error
- tests/test_wechat_bot.py test_one_model_reply_through_existing_idempotent_send
- tests/test_wechat_bot.py test_model_latency_cannot_authorize_sending_into_changed_chat
- tests/test_wechat_bot.py test_non_allowlisted_title_never_reaches_model
- tests/test_wechat_bot.py test_unknown_send_is_not_regenerated_or_resent_on_restart
- tests/test_wechat_bot_service.py test_default_config_is_in_user_home
- tests/test_wechat_bot_service.py test_status_reads_configured_state_after_config_move
- tests/test_wechat_bot_service.py test_stop_waits_until_launchd_task_is_unloaded
- tests/test_wechat_bot_service.py test_start_reenables_and_loads_unloaded_task
- tests/test_wechat_bot_service.py test_history_failure_blocks_model_and_send
- tests/test_wechat_bot_service.py test_history_read_failure_keeps_message_pending
- tests/test_wechat_bot_service.py test_scroll_recovered_messages_are_recorded_in_history
- tests/test_wechat_bot_service.py test_nine_image_turn_sends_one_reply
- tests/test_wechat_bot_service.py test_few_images_go_to_the_model_directly
- tests/test_wechat_bot_service.py test_unsupported_file_reason_reaches_the_model
- tests/test_wechat_bot_service.py test_final_generation_failure_still_pauses
- tests/test_wechat_bot_service.py test_media_is_fetched_from_the_current_frame
- tests/test_wechat_bot_service.py test_model_is_told_this_turns_image_count_separately
- tests/test_wechat_bot_service.py test_earlier_context_images_follow_this_turns_and_are_counted
- tests/test_wechat_bot_service.py test_images_over_the_request_size_are_named_not_paused

### Task 9: Codex 生产链审查问题修复（2026-11-08）
审查报告：`docs/reviews/2026-11-08-codex-production-flow-review.md`（16 项已确认 + A–F 待验证）。全部修复、真机验证后才可删除 Python。按文件归属分批（避免冲突）：
- W1 一轮消息安全（turns.rs、serve.rs 信号、brain/builtin.rs）：#3 停止信号实时传入发送前检查；#4 发送前持久化失败必须阻断；#8 回显与推进游标的帧必须过 check_frame；#14 交给模型的文件只含 filename/正文/截断/状态；#15 写入模型 provenance；疑点 A（会话创建时注入 BrainFn）。
- W2 外部大脑接线（gateway.rs、brain_tools.rs、external_brain.rs）：#1 网关分发 brain 工具 + 真实 handle_request→Gateway→core.sock 契约测试；#13 领取不随单次调用断开而释放（lease/token）。
- W3 管理与校准（control.rs、scheduler.rs 校准段、main.rs、CoreSupervisor.swift、backup.rs）：#6 control 对接 core.enabled/宿主；#7 校准接上真实候选写入/接受；#12 历史维护命令；#9 区分配置错误(2)与运行错误；疑点 E、F。
- W4（附件修复合并后）：#10 事件写生产日志；#11 后台媒体任务错误边界；#16 删除 dummy 摘要后门；#2 codex-exec 后端（实现或明确拒绝并改默认）；疑点 B、C、D。
