# 阶段 C 补充：calibrate 由运行中的机器人代办 + 画面闸门 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (D1) `control calibrate` / `--accept` 在真机可用：交给正在运行的机器人，用它手里活着的 worker 连接执行；界面核对失败时不断开 worker。(D2) worker 在读取与点击前检查微信自己是否开着菜单、保存面板、图片查看器等额外窗口，有则只等待。

**Architecture:** worker 一次只服务一个连接，连接断开即退出；重新拉起只能经 MCP 的 GUI 导航（会点击、打字）。因此：mismatch 时保持连接；calibrate 走"状态目录请求文件 → 协调器在循环间隙用现有 `self.capture.call('calibrate')` 执行 → 结果文件"。画面闸门复用 `native/eyes/Screens.swift` 的窗口规则，但**不要求微信在前台**（用户在别的程序工作时机器人照常读）。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel），Python 3 unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-layered-architecture-design.md` §4.3–§4.4、§5；前序计划 `docs/superpowers/plans/2026-11-06-stage-c-layout-check.md`。

## Global Constraints

- calibrate 全程零点击、零滚动、零按键；不触发 MCP 导航。没有活连接时直接报错 `worker_not_connected`，不去拉起 worker。
- 闸门命中时零点击；错误码 `screen_not_main:<kind>`，协调器进入 `waiting_for_wechat_window`，**不计入** layout_mismatch 次数、不暂停、不断开连接。
- 闸门只在操作**开始前**检查；`saveAttachment` 自己打开的右键菜单与保存面板不受影响。
- 不改 `native/action/`、`src/`；不改现有测试断言（测试环境设置可以调整，须在汇报中说明）。

## Review Focus

1. 协调器在 needs_recalibration 期间仍保持 worker 连接，calibrate 请求能被执行；地图修好后核对通过，回到 watching。
2. calibrate 请求在协调器正在发送/生成回复时到达：等当前 cycle 结束后再执行，不打断发送。
3. 请求文件残留（机器人重启前写的）：超过 10 分钟的请求作废并写结果 `expired`。
4. 用户在别的程序工作（微信不在前台）：闸门不拦截，机器人照常读。
5. 图片查看器开着时：不点会话行、不另存，等待；关闭后自动恢复。

---

### Task D1: calibrate 由运行中的机器人代办（agy）

**Files:**
- Modify: `scripts/wechat_bot/coordinator.py`、`scripts/wechat_bot/control.py`
- Test: `tests/test_calibrate.py`、`tests/test_recalibration.py`

**Interfaces:**
- Consumes: 现有 `run_calibrate(config, worker_result=..., remote_runner=...)`、`calibrate_accept(config, worker_func=..., remote_runner=...)`（control.py，已可注入）；worker 操作 `calibrate`。
- Produces:
  - 协调器：`layout_mismatch` 时**不再** `close_connection()`（计数与健康状态不变）；`layout_missing` 等其他 `layout_*` 仍按原样处理。
  - 请求文件 `<state_dir>/calibrate-request.json`：`{"id": <32 hex>, "action": "calibrate"|"accept", "created_at": <epoch>}`，权限 600。
  - 协调器方法 `async def serve_calibration(self)`：每个循环间隙调用一次（不在 cycle 中途）；读请求 → 超过 600 秒写 `expired` → 无活连接（`self.capture` 为空）写 `worker_not_connected` → 否则 `result = await self.capture.call('calibrate')`，`calibrate` 动作调用 `run_calibrate(self.config, worker_result=result)`，`accept` 动作调用 `calibrate_accept(config=self.config, worker_func=lambda: result)`；结果写 `<state_dir>/calibrate-result.json` `{"id", "ok", "result"|"error"}`（600），然后删除请求文件。任何异常都写进结果，不影响协调器继续运行。
  - `control calibrate [--accept]`：机器人未运行（`loaded()` 为假）→ 报错 `机器人未运行：请先 control start`；否则原子写请求文件，轮询结果文件（同 id）最多 120 秒，打印结果；超时报错。删除"机器人运行时拒绝"的旧逻辑。
- [ ] Step 1 失败测试：`test_mismatch_keeps_worker_connection`、`test_serve_calibration_writes_candidate_via_live_capture`（假 capture 记录调用，只允许 `calibrate`）、`test_serve_accept_uses_live_version`、`test_request_without_connection_reports_worker_not_connected`、`test_stale_request_expires`、`test_control_calibrate_round_trip`（假协调器线程写结果）、`test_control_calibrate_requires_running_bot`。
- [ ] Step 2 运行确认失败；Step 3 实现；Step 4 全量 `python3 -m unittest discover -s tests` 通过
- [ ] Step 5 提交 `feat: calibrate 交给运行中的机器人用现有 worker 连接执行；mismatch 时保持连接`

### Task D2: 画面闸门（agy，与 D1 并行）

**Files:**
- Modify: `native/eyes/Screens.swift`（只增）、`native/bot/Worker.swift`、`scripts/wechat_bot/coordinator.py`（只在 `WINDOW_UNAVAILABLE` 元组加 `'screen_not_main'`）
- Test: `tests/test_screens.py`、`tests/eyes_selftest/main.swift`、`tests/test_layout_consistency.py`、`tests/test_bot_routing.py`

**Interfaces:**
- Consumes: B2 的 `WindowInfo`、`windowInfos(_:)`、`ScreenKind`。
- Produces:
  - `func blockingScreen(windows: [WindowInfo], wechatPID: Int32, mainSize: CGSize, focusedIdentifier: String?) -> ScreenKind?`：按顺序 `focusedIdentifier == "saveAsNameTextField"` → `.savePanel`；微信进程有 layer ≠ 0、宽 20..600、高 20..900 的屏上窗口 → `.contextMenu`；微信进程 layer 0 的屏上窗口多于 1 个 → `.otherWechatWindow`；否则 `nil`。**不看前台程序。**
  - Worker：`func screenGate(_ window: [String:Any]) throws`（`CGWindowListCopyWindowInfo` → `windowInfos` → `blockingScreen`；焦点标识用现有 AX 读取方式，读不到传 nil）；命中抛 `Failure.invalid("screen_not_main:"+kind.rawValue)`。在 `capture`、`sessions`、`open_session`、`scroll_chat`、`prepare_file`、`prepare_image` 的开头（`readyWindow` 之后、任何截图或点击之前）调用。
- [ ] Step 1 失败测试：`test_blocking_menu`、`test_blocking_image_viewer`、`test_blocking_save_panel_focus`、`test_other_app_frontmost_not_blocking`、`test_main_only_not_blocking`（eyes 自测夹具）；`test_every_operation_gates_first`（Worker.swift 源码中上述 6 个分支都先调 `screenGate(`）；`test_screen_not_main_waits_without_counting`（协调器：等待、不暂停、不计入 layout_mismatches）。
- [ ] Step 2 运行确认失败；Step 3 实现；Step 4 全量通过，且 `swiftc -O -target x86_64-apple-macos12 -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift native/action/*.swift native/bot/*.swift` 编译通过
- [ ] Step 5 提交 `feat: 画面闸门——微信开着菜单/保存面板/其他窗口时不读不点`

### Task D3: 整合与真机验证（整合者）

- [ ] 合并 D1、D2；部署 worker 与机器人。
- [ ] 演练 1：远端地图分隔线临时改高 30 点 → needs_recalibration、连接保持 → `control calibrate` 生成候选（差异：chat.h、chat_content.h、input.y、input.h 改回 436/436/496/144）→ `--accept` → 自动回到 watching；备份文件存在。
- [ ] 演练 2：在微信里打开一张图片（查看器窗口）→ 机器人等待、零点击；关闭后恢复。
- [ ] 文件回复：xlsx/PDF 一轮正常。
