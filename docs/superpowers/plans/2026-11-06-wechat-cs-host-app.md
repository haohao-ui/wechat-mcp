# 微信客服助手 + 机器人迁到专用 Mac Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:**
- 专用 Mac 上的宿主程序改名为"微信客服助手"，增加状态面板、保活、一键操作和常用配置；
- 机器人本体在专用 Mac 上以本机模式运行。

**Architecture:**
- Python 侧（T1）：
  - 增加 `host: "local"` 本机模式，所有 ssh 调用点统一经过一个函数；
  - 增加 `config-check`、`pause`、`resume` 三个子命令；
  - 兼容 Python 3.9。
- Swift 宿主（T2、T3）：
  - T2 负责改名、子进程退避托管、LaunchAgent 和部署脚本；
  - T3 负责界面：状态面板、一键操作、配置表单。
  - 两者只通过下文列出的接口交互。
- T4 由整合者完成：合并、部署、迁移和真机验收。

**Tech Stack:** Python 3.9+ 标准库 unittest；Swift 5.7 AppKit（专用 Mac 为 macOS 12，Intel）。

**Spec:** `docs/superpowers/specs/2026-11-06-wechat-cs-host-app-design.md`

## Global Constraints

- 运行环境：专用 Mac 用系统 `/usr/bin/python3`（3.9.6）。不使用 3.10 以上才有的标准库 API，也不引入第三方依赖。
- 名称：显示名"微信客服助手"，App 为 `微信客服助手.app`，bundle ID `local.wechat-mcp.host`，可执行文件 `wechat-cs-host`。
- `worker.sock` 的路径（`~/.wechat-rpa/worker.sock`）、权限 0600、对端 UID 校验，以及"一次只接一个连接"的语义都不变。
- 宿主不截取聊天画面，不读消息内容；读取 `health.json` 时只取状态字段。
- 模型密钥不出现在界面、日志和诊断信息里，只显示"已配置 / 未配置"。
- 不改 `src/`，即 Rust MCP 的工具契约；不改 `native/action/`；不改现有测试的断言。

## Review Focus

1. 本机模式下，所有原本走 ssh 的路径都不再出现 `ssh`，包括 calibrate 写候选文件和 `__main__` 的旧入口。
2. 桌面服务子进程连续崩溃：退避间隔为 5 秒、15 秒、60 秒，同一小时内第 5 次失败后停止重启；用户点"重启"后计数清零。
3. 保存配置时机器人正在运行：配置先校验，再原子替换，然后重启机器人；校验失败时旧配置原样不动。
4. `pause` 期间：机器人不读、不点、不发；`resume` 后继续，并且不补发暂停期间的消息（沿用现有的 rebaseline）。
5. Python 3.9 下 `asyncio.wait_for` 超时，抛出的错误与原来 `asyncio.timeout` 的行为一致，调用方能正确识别。

---

### Task T1：Python 本机模式 + 管理子命令 + 3.9 兼容（agy）

**Files:**
- Create：`scripts/wechat_bot/transport.py`
- Modify：
  - `scripts/agent_bridge/mcp.py`（第 30 行）
  - `scripts/wechat_bot/service.py`（第 658 行；`load_config` 中的 host 校验）
  - `scripts/wechat_bot/coordinator.py`（第 199 行；主循环读取暂停标记）
  - `scripts/wechat_bot/capture.py`（第 21 行）
  - `scripts/wechat_bot/native_capture.py`（第 154 行）
  - `scripts/wechat_bot/__main__.py`（第 17 行）
  - `scripts/wechat_bot/control.py`（第 256 行 `default_remote_runner`；新增子命令）
- Test：`tests/test_transport_local.py`、`tests/test_control_admin.py`

**Interfaces:**
- Produces：
  - `transport.remote_argv(host: str, command: str) -> list[str]`：
    - `host == "local"` 时返回 `['/bin/sh', '-c', command]`；
    - 否则返回 `['ssh', '-o', 'BatchMode=yes', '-o', 'LogLevel=ERROR', host, command]`。
    - 上面 6 个 ssh 调用点全部改用它。
  - `load_config`：`host` 可以是 `"local"`，其他值的校验规则不变。
  - `control config-check <path>`：调用 `load_config(path)`。
    - 通过时打印 `{"ok": true}`，退出码 0；
    - 失败时打印 `{"ok": false, "error": "<load_config 的消息>"}`，退出码 2；
    - 不写入任何文件。
  - `control pause` / `control resume`：
    - pause 写 `<state_dir>/admin-pause.json`，内容 `{"paused": true, "at": <epoch>}`，权限 600；resume 删除这个文件；
    - 协调器每轮开始时读取：文件存在就把整体 health 写为 `paused`，`reason` 为 `"admin_pause"`，并跳过这一轮，不读、不点、不发；
    - 文件消失后照常运行。
  - `scripts/agent_bridge/mcp.py`：`async with asyncio.timeout(40)` 改为 `asyncio.wait_for(..., 40)`，超时时抛出的异常类型保持为 `TimeoutError`。
- [ ] Step 1 先写失败测试：
  - `test_local_argv_has_no_ssh`
  - `test_remote_argv_unchanged`
  - `test_every_call_site_uses_remote_argv`：源码检查，`scripts/` 下除了 `transport.py` 不再出现 `'ssh'`
  - `test_load_config_accepts_local`
  - `test_config_check_ok_and_error_exit_codes`
  - `test_config_check_writes_nothing`
  - `test_pause_file_skips_cycle`：假 capture 断言零调用
  - `test_resume_continues`
  - `test_mcp_timeout_raises_timeout_error`
- [ ] Step 2 运行这些测试，确认确实失败。
- [ ] Step 3 实现。
- [ ] Step 4 全量运行 `python3 -m unittest discover -s tests`，全部通过。另外用 `/usr/bin/python3`（若本机有 3.9）或 `python3.9` 跑一遍；本机没有 3.9 时在报告中注明，由 T4 在专用 Mac 上跑。
- [ ] Step 5 提交：`feat: 机器人本机模式（host=local）、control config-check/pause/resume、兼容 Python 3.9`

### Task T2：宿主改名 + 子进程退避托管 + LaunchAgent + 部署脚本（agy，与 T1 并行）

**Files:**
- Modify：`native/bootstrap/Info.plist`、`native/bootstrap/SocketWorker.swift`、`native/bootstrap/README.md`
- Create：
  - `native/bootstrap/Supervisor.swift`：纯逻辑的退避状态机
  - `native/bootstrap/local.wechat-mcp.host.plist`：LaunchAgent 模板
  - `scripts/deploy_host.sh`
- Test：`tests/test_host_supervisor.py`，以及一个 swiftc 编译的自测入口 `tests/host_selftest/main.swift`，写法照 `tests/eyes_selftest`

**Interfaces:**
- Produces：
  - `struct RestartPolicy { mutating func recordExit(at: Date) -> RestartDecision; mutating func reset() }`
    - `enum RestartDecision { case restart(after: TimeInterval), giveUp }`
    - 退避间隔依次为 5 秒、15 秒、60 秒，之后都按 60 秒；
    - 滚动 1 小时内退出 5 次，返回 `.giveUp`；
    - `reset()` 清空计数。
  - `SocketWorker`：
    - 子进程退出后按 `RestartPolicy` 自动重启；
    - 提供 `var status: WorkerStatus { get }`，其中 `enum WorkerStatus { case running(pid: Int32), restarting(at: Date), stopped(reason: String) }`；
    - 提供 `func restartChild()`：手动重启，并调用 `reset()`。
    - 每次退出都写一行日志到 `~/.wechat-rpa/logs/host.log`，内容含退出码或信号，用来查"宿主跟着退出"的原因；目录权限 700。
  - Info.plist 按 Global Constraints 里的名称修改。
  - LaunchAgent 模板：
    - `Label=local.wechat-mcp.host`，`ProgramArguments=[~/Applications/微信客服助手.app/Contents/MacOS/wechat-cs-host]`，`RunAtLoad=true`，`KeepAlive={SuccessfulExit=false}`，`ThrottleInterval=30`；
    - 路径由安装脚本按用户家目录展开。
  - `scripts/deploy_host.sh <host> [--app] [--worker] [--bot]`，默认三项都部署：
    - 在远端用 swiftc 编译，编译命令写在 README 里；
    - 每个被替换的文件先备份为 `<名称>.before-<时间戳>`；
    - `--app`：
      - 首次部署时把 `WeChat RPA Test.app` 挪到 `~/Applications/.old/`；
      - 安装 LaunchAgent，执行 `launchctl bootstrap gui/$UID`；
    - `--bot`：把 `scripts/` 拷到 `~/.wechat-rpa/bot/`，并安装机器人的 launchd；
    - 脚本里不使用 `pkill`。重启通过 `launchctl kickstart -k` 完成。
    - 脚本默认只打印将要执行的步骤（dry-run），必须加 `--yes` 才真正执行。
- [ ] Step 1 先写失败测试：
  - `test_backoff_sequence`：5、15、60、60
  - `test_give_up_after_five_in_an_hour`
  - `test_window_rolls`：间隔超过 1 小时的退出不累计
  - `test_reset_clears`
  - `test_info_plist_names`
  - `test_launchagent_keepalive_only_on_crash`
  - `test_deploy_script_dry_run_by_default`：不带 `--yes` 时不执行 ssh 或 scp，用假 PATH 断言
  - `test_deploy_script_has_no_pkill`
- [ ] Step 2 运行这些测试，确认确实失败。
- [ ] Step 3 实现。
- [ ] Step 4 全量测试通过；`swiftc -O -target x86_64-apple-macos12 native/bootstrap/*.swift -o /tmp/host-check` 编译通过。T3 尚未合入时，单独编译 T2 涉及的文件。
- [ ] Step 5 提交：`feat: 宿主改名"微信客服助手"、子进程退避托管、LaunchAgent 保活与部署脚本`

### Task T3：界面——状态面板、一键操作、配置表单（agy，T2 合入后开始）

**Files:**
- Modify：`native/bootstrap/main.swift`
- Create：
  - `native/bootstrap/StatusModel.swift`：纯逻辑，把采集到的数据转成每一项的绿 / 黄 / 红和原因
  - `native/bootstrap/StatusPanel.swift`
  - `native/bootstrap/ConfigForm.swift`
  - `native/bootstrap/Actions.swift`
- Test：在 `tests/host_selftest/main.swift` 增加 `status` 和 `config` 两个子命令，由 `tests/test_host_status.py` 调用

**Interfaces:**
- Consumes：
  - T2 的 `SocketWorker.status` 和 `restartChild()`；
  - T1 的 `control config-check`、`pause`、`resume`；
  - 现有的 `Probe.swift`；
  - `native/eyes/Screens.swift` 中的 `windowInfos` 和 `blockingScreen`，宿主编译时一并编进去。
- Produces：
  - `func evaluate(_ s: StatusInput) -> [StatusItem]`：
    - `StatusItem` 包含 `key`、`level`（green/yellow/red）、`reason`（中文）、`fix`（中文，可为空）；
    - `key` 依次为 `permissions`、`desktop_service`、`bot`、`sessions`、`wechat`、`layout`、`disk`；
    - 阈值：health 超过 120 秒未更新为红；`needs_recalibration` 为红；`paused` 为黄；任何会话处于 `waiting_for_wechat_window` 为黄，原因带上会话名和错误码；有窗口挡在主窗口前面为红；剩余空间少于 5GB 为黄。
  - 配置表单：
    - 只开放 spec §5.5 列出的字段；
    - 保存流程：写临时文件 → 调 `config-check`（使用 `~/.wechat-rpa/bot/` 下的代码和 `/usr/bin/python3`）→ 备份为 `config.json.bak-<时间戳>` → 原子替换 → `launchctl kickstart -k gui/$UID/local.wechat-rpa.bot`；
    - 校验失败时在表单上显示 error，不写入文件。
  - 一键操作按 spec §5.4 实现：
    - "放回标准位置"通过 AX 设置窗口的位置和大小（与 worker 的 manage_window 用同一套数值），不点击窗口内容；
    - "重启"之前，如果 health 显示正在 `sending`，先弹出确认框。
  - 诊断文本：把 `evaluate` 的结果和版本号拼成纯文本，不含密钥，不含消息内容。
- [ ] Step 1 先写失败测试：
  - `test_status_stale_health_red`
  - `test_status_recalibration_red`
  - `test_status_waiting_session_yellow_names_code`
  - `test_status_front_window_red_behind_ok`
  - `test_status_disk_low_yellow`
  - `test_config_form_rejects_invalid_without_write`：用假的 config-check 退出码 2
  - `test_config_save_backs_up_then_replaces`
  - `test_config_form_never_shows_key`
  - `test_diagnostics_has_no_secrets`
- [ ] Step 2 运行这些测试，确认确实失败。
- [ ] Step 3 实现。
- [ ] Step 4 全量测试通过；宿主编译通过，编译命令为 `swiftc -O -target x86_64-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift`，若有依赖再补上所需文件。
- [ ] Step 5 提交：`feat: 微信客服助手状态面板、一键操作与常用配置`

### Task T4：整合、部署、迁移、真机验收（整合者）

- [ ] 合并 T1、T2、T3。
- [ ] 在专用 Mac 上用 `/usr/bin/python3` 跑全量测试。
- [ ] 执行 `deploy_host.sh --yes`。用户重新授权屏幕录制和辅助功能；用户自己在专用 Mac 上配置模型密钥的环境变量。
- [ ] 迁移：
  1. 停掉开发电脑上的机器人，确认已卸载；
  2. 把状态目录拷到专用 Mac，配置改为 `host: "local"`；
  3. 启动机器人；
  4. 在测试群发消息，确认有回复。
- [ ] 按 spec §7 的 5 项验收；回滚步骤见 spec §5.6。
