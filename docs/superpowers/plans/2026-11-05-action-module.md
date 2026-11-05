# 公共动作组件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 `native/action/`（拟人轨迹 + 有效区域判定 + 抢鼠标检测），把现有 5 处点击/指针移动改走它；拖拽留作第二步，本计划不做。

**Architecture:** 纯计算的 `Trajectory`（可注入随机种子）生成带时间戳的轨迹；`Executor` 通过可替换的 `EventSink` 发事件（生产用系统事件，测试用记录器），按下/抬起前判定有效区域，每步检测光标是否被外部移动；`Actions` 是对外接口。测试用一个 Swift 自测程序跑大量种子采样并输出 JSON，由 Python unittest 断言性质（项目无 XCTest，沿用"编译 Swift + Python 断言"的模式）。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel，`-target x86_64-apple-macos12`），Python unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-action-module-design.md`

## Global Constraints

- 成功标准：按下与抬起都在有效区域内；不在区域内绝不按下。修正上限 3 次，失败报 `target_not_reached`。
- 有效区域：四边各内缩 20%；宽或高 < 16 点时改为各内缩 2 点；内缩后为空则用矩形中心 1×1 点。
- 坐标单位为"点"（CGEvent 屏幕坐标），不是截图像素。
- 默认 profile：单次点击总时长约 0.3–0.8 秒；到位停留 50–150ms；按下持续 60–120ms；事件 60–120Hz。
- `instant` profile：无中间轨迹，直接到落点，仍做区域判定与抢鼠标检测；用于测试与排障（环境变量 `WECHAT_ACTION_PROFILE=instant` 可全局切换）。
- 抢鼠标阈值 3 点，报 `user_input_detected`。
- **滚动的滚轮刻度不变**：仍是每页 1 个 10 行刻度（`merge_pages` 依赖 90pt/步的标定），只拟人化指针移动和刻度之间的间隔。
- 不用 Swift 5.9+ 语法（含 `some` 参数类型）；不新增第三方依赖。
- 不改按键 `key()` 与粘贴 `paste()` 的行为（按键节奏归入以后的"回复节奏"设计）。

## Review Focus

1. 目标区域部分超出屏幕：落点与轨迹必须限制在屏幕内，且仍在有效区域内；有效区域与屏幕无交集时报 `target_not_reached`，不按下。
2. 起点已在有效区域内（距离≈0）：不应产生 NaN（费茨公式、方向向量除零），应原地或微动后点击。
3. 极小目标（例如 4×4 点）：有效区域退化为 2 点内缩或中心点，仍能命中且修正次数有界。
4. 抢鼠标发生在按下之后、抬起之前：必须仍发出抬起（避免按键卡住），然后报 `user_input_detected`。
5. 多显示器/负坐标（副屏在主屏左侧或上方）：屏幕边界取包含起点或目标的那块屏，负坐标不被当成越界。

---

### Task 1: Trajectory 与 Profile（纯计算）+ 自测程序

**Files:**
- Create: `native/action/Profile.swift`、`native/action/Trajectory.swift`
- Create: `tests/action_selftest/main.swift`（自测程序：读命令行子命令，输出 JSON）
- Test: `tests/test_action_module.py`

**Interfaces:**
- Produces:
  - `struct ActionProfile { name, fittsA, fittsB, durationJitter, tremorAmplitude, driftAmplitude, scatter, maxCorrections, dwell: ClosedRange<Double>, press: ClosedRange<Double>, eventHz: ClosedRange<Double> }`，静态 `.standard`、`.precise`、`.instant`；`static func current() -> ActionProfile`（读 `WECHAT_ACTION_PROFILE`）。
  - `struct SeededRandom: RandomNumberGenerator`（SplitMix64，`init(seed: UInt64)`）；生产用 `SystemRandomNumberGenerator`。
  - `func effectiveRegion(_ rect: CGRect) -> CGRect`（按 Global Constraints 内缩）。
  - `struct TrajectoryPoint { t: Double; point: CGPoint }`。
  - `func sampleLanding<R: RandomNumberGenerator>(in region: CGRect, from start: CGPoint, profile: ActionProfile, rng: inout R) -> CGPoint`。
  - `func movement<R>(from: CGPoint, to: CGPoint, targetWidth: Double, screen: CGRect, profile:, rng: inout R) -> [TrajectoryPoint]`（贝塞尔路径 + 最小加加速度 + 峰值前移 + 手抖×(1−s)² + 限制在 screen 内；最后一点 = 主移动终点）。
  - `func planClick<R>(from: CGPoint, rect: CGRect, screen: CGRect, profile:, rng: inout R) -> ClickPlan`，`ClickPlan { moves: [[TrajectoryPoint]]（主移动 + 修正）; press: CGPoint?（nil = target_not_reached）; dwell: Double; hold: Double }`。主移动终点 = 采样落点 + 误差（σ ∝ 距离，`profile.scatter`）；在有效区域外则修正（误差 ×0.3），最多 `maxCorrections` 次。

- [ ] **Step 1: 写失败测试** `tests/test_action_module.py`：setUpClass 用 `swiftc -O native/action/*.swift tests/action_selftest/main.swift` 编译（macOS + swiftc 才跑，否则跳过）。自测程序子命令 `clicks <n> <seed>` 对随机起点/随机矩形（含 4×4、超出屏幕一半、起点已在区域内三类）输出每次的统计 JSON。断言：
  - `test_press_always_inside_effective_region_or_refused`：n=5000，所有 `press` 在 `effectiveRegion(rect)∩screen` 内；无交集的样本 `press` 为 null。
  - `test_peak_speed_in_first_half`：主移动速度峰值时刻 / 总时长 < 0.5 的比例 ≥ 95%。
  - `test_duration_follows_fitts`：主移动时长在 `[0.5, 2.0] × (a + b·log2(D/W+1))` 内；远/小目标平均时长 > 近/大目标。
  - `test_tremor_bounded_and_vanishes`：路径点到"无噪声理想曲线"的偏差 ≤ 2×tremor+drift；最后 10% 进度内偏差 ≤ 0.5 点。
  - `test_points_stay_on_screen_and_no_nan`：所有点有限且在 screen 内（含负坐标屏幕用例）。
  - `test_small_targets_need_more_corrections`：4×4 目标平均修正次数 > 120×40 目标；所有修正次数 ≤ 3。
  - `test_same_seed_same_plan`：同种子两次输出完全相同。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_action_module -v` → 编译失败（文件不存在）
- [ ] **Step 3: 实现** Profile、SeededRandom、effectiveRegion、sampleLanding、movement、planClick；自测程序。
- [ ] **Step 4: 运行通过** — 同上 → PASS；并 `swiftc -target x86_64-apple-macos12 -parse-as-library native/action/*.swift -emit-library -o <scratch>/a.dylib`（确认可按 macOS 12 目标编译）
- [ ] **Step 5: 提交** `feat: 动作组件轨迹（费茨用时、前快后慢、手抖衰减、有效区域修正）`

### Task 2: Executor、EventSink 与 Actions

**Files:**
- Create: `native/action/Executor.swift`、`native/action/Actions.swift`
- Modify: `tests/action_selftest/main.swift`（新增子命令，用记录型 sink）
- Test: `tests/test_action_module.py`

**Interfaces:**
- Consumes: Task 1 全部。
- Produces:
  - `protocol EventSink { func cursor() -> CGPoint; func post(_ type: CGEventType, at: CGPoint, button: CGMouseButton) throws; func scroll(lines: Int32, at: CGPoint) throws; func sleep(_ seconds: Double) }`；`SystemEventSink`（CGEvent 发到 `.cghidEventTap`，sleep 用 `Thread.sleep`）。
  - `enum ActionError: Error, CustomStringConvertible { case targetNotReached, userInputDetected, eventFailed }`，描述分别为 `target_not_reached`、`user_input_detected`、`mouse_event_failed`（沿用现有错误码）。
  - `final class Actions { init(sink: EventSink = SystemEventSink(), profile: ActionProfile = .current(), seed: UInt64? = nil); func move(to rect: CGRect) throws -> CGPoint; func click(_ rect: CGRect, button: CGMouseButton = .left) throws; func rightClick(_ rect: CGRect) throws; func scroll(in rect: CGRect, lines: Int32, ticks: Int) throws }`。
  - 屏幕边界：`NSScreen.screens` 中包含起点或目标中心的那块（找不到用主屏）；自测程序可注入屏幕矩形。
  - 抢鼠标：每发一个移动事件前比较 `sink.cursor()` 与上一个自己发出的位置，偏差 > 3 点即中止；若已按下，先发抬起再抛错。
  - 滚动：先 `move(to: rect)`，之后每个刻度 `sink.scroll(lines:…)`，刻度之间间隔 80–200ms 随机，刻度大小由调用方给定、不改。
- [ ] **Step 1: 写失败测试**（自测程序子命令 `exec <scenario> <seed>`，输出记录的事件序列 JSON）：
  - `test_click_event_order`：若干 mouseMoved → leftMouseDown → leftMouseUp；down 与 up 都在有效区域内；down→up 间隔在 profile.press 内。
  - `test_unreachable_target_never_presses`：区域在屏幕外 → 无 down/up，错误 `target_not_reached`。
  - `test_user_move_aborts`：sink 在第 k 个移动后把光标挪开 50 点 → `user_input_detected`，之后无 down。
  - `test_user_move_between_down_and_up_still_releases`：按下后挪开光标 → 仍有 up，然后报错。
  - `test_instant_profile_single_move`：`instant` 下只有 1 个 mouseMoved，仍在区域内。
  - `test_scroll_keeps_tick_size`：`scroll(lines:10, ticks:3)` → 恰好 3 个 scroll 事件，每个 10 行，都在区域内，间隔在 80–200ms。
  - `test_right_click_uses_right_button`。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现** Executor、SystemEventSink、Actions。
- [ ] **Step 4: 运行通过**；macOS 12 目标编译通过
- [ ] **Step 5: 提交** `feat: 动作组件执行器（有效区域判定、抢鼠标检测、滚动刻度不变）`

### Task 3: worker 内 4 处改用 Actions

**Files:**
- Modify: `native/bot/Files.swift`（"另存为"菜单项点击、媒体右键）、`native/bot/Sessions.swift`（会话行点击）、`native/bot/Worker.swift`（`scroll_chat` 指针移动与刻度）
- Modify: `scripts/build.sh`（worker 编译加入 `native/action/*.swift`）
- Modify: `scripts/wechat_bot/native_capture.py`（如需：新错误码按可重试处理）
- Test: 现有全量测试；worker 编译；新增 `test_user_input_detected_is_retryable`

**Interfaces:**
- Consumes: Task 2 `Actions`。
- 每处的目标矩形（屏幕点坐标）：
  - 菜单项：OCR 命中文字中心为圆心，宽 = 弹窗宽度 −16，高 22 点。
  - 媒体右键：现有 `item` 的 x/y/width/height 换算成屏幕矩形（`window.x+315+x`, `window.y+60+y`, `w`, `h`）。
  - 会话行：x 为会话列表区（窗口 x+60，宽 240），y 为现有点击点 ±24 点（行高约 64，取 48 高保证不跨行）。
  - 聊天区滚动：聊天区矩形（窗口 x+315, y+60, 宽 565, 高 436）。
- worker 内共用一个 `Actions` 实例（保持光标跟踪连续）。
- [ ] **Step 1:** 先跑 `python3 -m unittest discover -s tests` 记录基线。
- [ ] **Step 2:** 写 `test_user_input_detected_is_retryable`（`user_input_detected`、`target_not_reached` 不导致暂停，按"窗口暂不可用、稍后重试"处理），确认失败。
- [ ] **Step 3:** 改 4 处与 Python 分类；上层逻辑与已有错误码不变。
- [ ] **Step 4:** `swiftc -O -target x86_64-apple-macos12 -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/action/*.swift native/bot/*.swift -o <scratch>/wechat_bot_worker` 通过；全量测试通过。
- [ ] **Step 5: 提交** `refactor: worker 点击与滚动改用公共动作组件`

### Task 4: wechat_click 改用 Actions

**Files:**
- Move: `native/wechat_click.swift` → `native/click/main.swift`（多文件编译时顶层代码须在 main.swift）
- Modify: `scripts/build.sh`（`swiftc native/click/main.swift native/action/*.swift -o target/release/wechat_click`）
- Test: `tests/test_action_module.py`（新增 `WechatClickCliTests`）

**Interfaces:**
- 命令行兼容：`wechat_click <x> <y>` 照旧（Rust `src/wechat.rs` 不改）；新增可选 `<width> <height>`，缺省时目标矩形为以 (x,y) 为中心的 12×12 点（有效区域按小目标规则内缩 2 点）。
- 新增 `--plan-only`：不发事件，输出计划的事件序列 JSON（用于测试）。诊断审计 `click.json` 逻辑保留。
- [ ] **Step 1: 写失败测试**：`--plan-only 500 400` 的 down/up 在 (500±4, 400±4) 内；`--plan-only 500 400 200 40` 的 down/up 在内缩 20% 的矩形内；无效参数退出码非 0。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**；`git mv` 保留历史。
- [ ] **Step 4: 运行通过**；`cargo test` 仍通过
- [ ] **Step 5: 提交** `refactor: wechat_click 改用公共动作组件，兼容原参数`

### Task 5: 部署与真机验收、文档

**Files:**
- Modify: `skills/wechat-operator/references/bot.md`、`docs/allowlist-bot.md`（点击方式、`WECHAT_ACTION_PROFILE`、新错误码）

- [ ] **Step 1:** 远端部署：源码（含 `native/action/`）拷到 `~/.wechat-rpa/src-media/`，远端编译 worker 与 `wechat_click`，备份旧二进制（`.before-action-<时间戳>`）后替换，重启 worker。
- [ ] **Step 2:** 真机：请用户在测试会话发文字、图片、PDF 各一轮，确认回复正常；记录每次点击平均用时。
- [ ] **Step 3:** 手动一次 `send_file`（需用户指定对象）确认宿主点击正常。
- [ ] **Step 4:** 抢鼠标实测：动作进行中移动鼠标，确认报 `user_input_detected` 且机器人下一轮恢复。
- [ ] **Step 5:** 文档更新并提交 `docs: 公共动作组件说明`
