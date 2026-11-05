# 微信界面自动适配 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让白名单客服 Bot 在 Retina/1 倍屏 × 浅色/深色、macOS 12–27 上都能工作，窗口自动固定为 880×640。

**Architecture:** Swift worker 内新增几个环节：先获取窗口图像（ScreenCaptureKit 与旧接口二选一），再归一化到 880×640 的 1 倍像素，然后按帧识别主题规则；现有的气泡和会话列表解析改为读取规则参数。Python 侧只改 profile 门控和可重试错误清单。用 `wechat_bubbles` CLI 对入库的脱敏截图样本做回归测试。

**Tech Stack:** Swift（CoreGraphics、ScreenCaptureKit、Vision、ApplicationServices），Python 3.12 标准库 + unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-ui-auto-adapt-design.md`

## Global Constraints

- 微信 4.1.15；worker 最低 macOS 12，编译命令 `swiftc -O -target "$(uname -m)-apple-macos12" -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/bot/*.swift`。
- 解析坐标系固定为 880×640 1 倍，现有坐标常量不改。
- profile 名：`wechat-4-dark`、`wechat-4-light`；帧结果另带 `scale`（1 或 2）。不保留 `wechat-4-dark-1x`。
- 不改气泡指纹算法，不改历史库结构（用户选方案 A）。
- 只把微信主窗口的尺寸调到 880×640，不动位置；最小化、全屏或有多个候选窗口时不调整。
- 进仓库的截图样本只含测试会话，非测试联系人区域涂掉；原图用完删除。
- Python 只用标准库；新增的 Swift 测试通过 Python unittest 调 CLI 运行，没有 swiftc 或不是 macOS 时 skip。

## Review Focus

1. 浅色主题下消息方向判反（自己的绿色气泡与对方的白色气泡），会把 Bot 自己的回复当成客户消息再回复。→ 每个浅色样本都断言 direction。
2. 主题识别误判：深色画面命中浅色规则或反之，必须报 `uncalibrated_theme`，不能继续解析。→ 对未知画面样本（如加载中的白屏、图片查看器）断言报错。
3. Retina 缩放后小气泡（单个表情、短文本）面积低于阈值而被漏掉。→ Retina 样本包含单字和单表情气泡。
4. 自动调尺寸时调到了图片查看窗口等非主窗口。→ 多窗口时断言不调整、返回不可用。
5. Vision 冷启动导致第一次 `capture` 超时，Bot 每次启动都先进入等待。→ 预热在 socket 开始监听前完成。

---

### Task 1: Vision 预热与超时可重试

**Files:**
- Modify: `native/wechat_bubbles.swift`（`recognize` 增加 `timeout` 参数，默认 12）
- Modify: `native/bot/Worker.swift`（`BotWorkerMain` 在 bind/listen 前预热一次，`timeout: 60`；预热失败只写 stderr，不退出）
- Modify: `scripts/wechat_bot/coordinator.py`（`WINDOW_UNAVAILABLE` 加 `vision_timeout`）
- Test: `tests/test_bot_routing.py`

- [ ] **Step 1:** 写 `test_vision_timeout_retries_without_permanent_pause`：`connect` 抛 `ValueError('private visual operation failed: invalid("vision_timeout")')`，断言 `child.paused is False`、health 为 `waiting_for_wechat_window`。
- [ ] **Step 2:** 运行 `python3 -m unittest -k vision_timeout tests.test_bot_routing`。Expected: FAIL（paused 为 True）。
- [ ] **Step 3:** 加入 `vision_timeout`；Swift 侧加预热：用 200×60 的纯色小图跑一次 `recognize(_, timeout:60)`。
- [ ] **Step 4:** 重跑 Step 2。Expected: PASS；本机编译 `wechat_bubbles` CLI 通过（`swiftc -O -parse-as-library native/wechat_bubbles.swift -o <tmp>/wb`）。
- [ ] **Step 5:** Commit `fix: worker 预热 Vision 模型，识别超时改为可重试`。

### Task 2: 截图样本采集与 CLI 回归测试框架

**Files:**
- Create: `tests/fixtures/frames/<env>-<theme>-<case>.png` 及同名 `.json`（期望值）
- Create: `tests/test_native_frames.py`
- Create: `scripts/dev/capture_frame.sh`（在指定主机上截取微信窗口，拷回本地临时目录，不入库）

**Interfaces:**
- Produces: CLI 调用约定为 `wechat_bubbles <png> '<window-json>'`，stdout 输出 JSON（`profile`、`scale`、`bubbles[].direction/kind/text`、`title_lines`）。期望 JSON 只断言这些：`profile`、`scale`、`title_lines`、每个气泡的 `direction` 和 `kind`、文本包含关系、`clipped_bubble`。错误样本断言 stderr 含对应错误码。

- [ ] **Step 1:** 和用户配合采集样本，共五组：远端 1 倍深色、远端 1 倍浅色、本机 Retina 深色、本机 Retina 浅色、本机外接 1 倍屏。每组至少包括：短文本、长文本、单个表情、图片、文件、时间分隔、自己发的绿色气泡、未读红点行、选中行、没打开聊天的空白页。
- [ ] **Step 2:** 用 PIL 在本地临时目录把会话列表中非测试联系人的行和头像涂成纯色，再复制进 `tests/fixtures/frames/`。每张图人工核对后才入库，原图删除。
- [ ] **Step 3:** 写 `tests/test_native_frames.py`：`setUpClass` 把 CLI 编译到 `tempfile` 目录（没有 `swiftc` 或不是 Darwin 时 `skipTest`）；对每个 fixture 跑 CLI，与期望 JSON 比较。先只写远端 1 倍深色样本的期望。
- [ ] **Step 4:** 运行 `python3 -m unittest tests.test_native_frames`。Expected: 深色 1 倍样本 PASS，证明框架与现有行为一致；profile 期望暂用旧名，Task 3 再改。
- [ ] **Step 5:** Commit `test: 加入脱敏微信截图样本与 CLI 回归测试`。

### Task 3: 主题规则抽象 + profile 改名

**Files:**
- Modify: `native/wechat_bubbles.swift`（新增 `struct ThemeProfile`。规则必须放在这个文件里，因为它也会被单独编译成 CLI；`color()`、背景判定、`foreground()` 和结果里的 `profile` 都改为读取规则）
- Modify: `native/bot/Sessions.swift`（`isBadgeRed` 和返回的 `profile` 改为读取规则）
- Modify: `scripts/wechat_bot/policy.py:64`（接受 `{'wechat-4-dark','wechat-4-light'}`，且 `scale in (1,2)`）
- Modify: `tests/test_wechat_bot.py`、`tests/test_wechat_bot_service.py`、`tests/test_bot_routing.py` 中的 profile 字面值
- Test: `tests/test_wechat_bot.py`、`tests/test_native_frames.py`

**Interfaces:**
- Produces:
  - `struct ThemeProfile { name; isBackground(r,g,b); kind(r,g,b) -> 0/1(对方)/2(自己); isForeground(r,g,b); isBadgeRed(r,g,b) }`
  - `func detectTheme(chat:, list:) throws -> ThemeProfile`：两种主题都不像时抛 `uncalibrated_theme`。
  - `let darkProfile` 的参数与现有常量逐一相同。

- [ ] **Step 1:** 改 `tests/test_wechat_bot.py` 的 profile 测试：`wechat-4-dark`/`wechat-4-light` 加 `scale` 1 或 2 能通过；`wechat-4-dark-1x`、`scale: 3`、缺 `scale` 都拒绝。fixture 期望改为新名。
- [ ] **Step 2:** 运行两组测试。Expected: FAIL（policy 仍要求旧名，CLI 仍输出旧名）。
- [ ] **Step 3:** 实现 `ThemeProfile` 和 `detectTheme`（先只有深色），替换所有颜色判断；结果带 `profile` 和 `scale: 1`；改 policy。
- [ ] **Step 4:** 运行 `python3 -m unittest discover -s tests -p 'test_*bot*.py'` 与 `tests.test_native_frames`。Expected: 全部 PASS，深色样本的输出除 profile 名外与 Task 2 完全一致。
- [ ] **Step 5:** Commit `refactor: 颜色判断抽成主题规则，profile 改为 wechat-4-dark`。

### Task 4: 浅色主题规则

**Files:**
- Modify: `native/wechat_bubbles.swift`（新增 `lightProfile`，`detectTheme` 支持浅色）
- Modify: `native/bot/Sessions.swift`（核对浅色下的红点阈值和选中行规则）
- Test: `tests/test_native_frames.py`（浅色样本期望）

- [ ] **Step 1:** 为远端 1 倍浅色样本写期望 JSON：每个气泡的 direction 和 kind 正确；未知画面样本期望 `uncalibrated_theme`。
- [ ] **Step 2:** 运行。Expected: 浅色样本 FAIL（`uncalibrated_theme`）。
- [ ] **Step 3:** 从样本取色，标定浅色参数（背景、对方气泡、绿色气泡、前景、选中行、红点），实现 `lightProfile`。如果浅色背景和对方气泡颜色接近，导致连通域合并，就用"气泡左右锚点 + 与背景的色差"区分，并在 ledger 记一条 Ruling。
- [ ] **Step 4:** 运行全部 native 样本与 bot 测试。Expected: 全部 PASS，深色样本不变。
- [ ] **Step 5:** Commit `feat: 支持微信浅色主题`。

### Task 5: Retina 归一化与原图裁剪

**Files:**
- Modify: `native/wechat_bubbles.swift`（`analyzeFrame` 入口计算 `scale = full.width / window.width`，只接受 1 或 2（允许 ±0.01）；不是 1 时用 CGContext 高质量插值缩放到 880×640 再解析；图片媒体从原图按 `box * scale` 裁剪；结果带 `scale`）
- Modify: `native/bot/Sessions.swift`（`probe` 和 `titleNames` 使用同一个归一化函数）
- Modify: `native/bot/Worker.swift`（`capture` 里 panel 的 hash 基于归一化后的图）
- Test: `tests/test_native_frames.py`（Retina 样本期望，含单表情和短文本；外接屏 1 倍样本期望）

**Interfaces:**
- Produces: `func normalizedFrame(_ full: CGImage, window: [String:Any]) throws -> (image: CGImage, scale: Int)`；几何不符时抛 `unsupported_image_geometry`。

- [ ] **Step 1:** 写 Retina 深色和浅色样本的期望（scale 2），以及外接屏样本的期望（scale 1）。
- [ ] **Step 2:** 运行。Expected: Retina 样本 FAIL（`unsupported_image_geometry`）。
- [ ] **Step 3:** 实现归一化。必要时微调面积和颜色阈值，每次调整后重跑 1 倍样本，确保不回退。
- [ ] **Step 4:** 运行全部 native 样本与 bot 测试。Expected: 全部 PASS。
- [ ] **Step 5:** Commit `feat: Retina 截图归一化到 1 倍坐标解析`。

### Task 6: macOS 14+ 用 ScreenCaptureKit 截窗口

**Files:**
- Modify: `native/bot/Worker.swift`：
  - 新增 `func windowImage(_ window: [String:Any]) throws -> CGImage`。macOS 14+ 通过 `SCShareableContent` 找到同一 `windowID` 的 `SCWindow`，再用 `SCScreenshotManager.captureImage` 截图；输出宽高为窗口逻辑尺寸乘以所在屏幕的 `backingScaleFactor`，不含阴影；用信号量等待，5 秒超时。macOS 12–13 用 `CGWindowListCreateImage`。
  - 三处截图调用都改用它。
  - 删除 macOS 15 的启动拒绝。

- [ ] **Step 1:** 在本机（macOS 27）和远端（macOS 12）分别编译 worker 二进制。Expected: 都编译通过，没有 `CGWindowListCreateImage` 不可用的错误。
- [ ] **Step 2:** 在本机临时运行 worker，对已打开的测试会话调用 `capture`。Expected: 返回 `profile`、`scale: 2` 和正确的标题；缺少屏幕录制权限时返回 `window_capture_failed`，stderr 写明原因。
- [ ] **Step 3:** Commit `feat: macOS 14+ 使用 ScreenCaptureKit 截取微信窗口`。

（这个任务依赖系统框架和真实权限，没法用 fixture 做单测。验证方式就是 Step 1–2 的真实运行，结果写入 ledger。）

### Task 7: 主窗口尺寸自动固定

**Files:**
- Modify: `native/bot/Worker.swift`：新增 `func ensureWindowSize(_ window:) throws -> [String:Any]`，在 `capture`、`sessions`、`open_session` 截图前调用。逻辑如下：
  1. 用 AX 取微信进程的 `AXMainWindow`，核对它的位置和尺寸与 CG 选中的窗口一致。
  2. `AXMinimized` 或 `AXFullScreen` 为真，或者候选窗口不唯一 → 报 `wechat_not_visible`。
  3. 尺寸不是 880×640 → 设置 `kAXSizeAttribute`，等 0.3 秒后重新取窗口核对；仍不符 → 报 `unsupported_image_geometry`。
  4. 调整成功时，在结果里带 `window_resized: true`。
- Modify: `scripts/wechat_bot/native_capture.py`（帧结果含 `window_resized` 时调用 `log_event('window_resized')`）
- Test: `tests/test_wechat_bot.py`（Python 日志事件）

- [ ] **Step 1:** 写 Python 测试：`frame()` 收到 `window_resized: True` 时记录 `window_resized` 事件。运行。Expected: FAIL。
- [ ] **Step 2:** 实现 Python 与 Swift 部分。
- [ ] **Step 3:** 运行 bot 测试。Expected: PASS。
- [ ] **Step 4:** 远端真机验证，结果写入 ledger：
  - 手动拉大窗口 → 下一轮自动调回并继续；
  - 最小化 → 进入 `waiting_for_wechat_window`，恢复后继续；
  - 打开图片查看窗口 → 不缩放这个窗口。
- [ ] **Step 5:** Commit `feat: 自动把微信主窗口固定为 880×640`。

### Task 8: 部署与四环境验收

**Files:**
- Modify: `docs/allowlist-bot.md`（支持矩阵、权限说明、`waiting_for_wechat_window` 的原因清单）
- Create: `research/ui-auto-adapt-acceptance-2026-11-05.md`（验收记录，不写 IP 和真实联系人）

- [ ] **Step 1:** 在远端编译 worker 并原子替换（保留备份），重启 Bot。
- [ ] **Step 2:** 按设计 §8，在远端 1 倍深色和远端 1 倍浅色两个环境跑第 1–4 项，再跑第 5、6 项。
- [ ] **Step 3:** 在本机搭临时测试环境（Bot 连接本机 worker），跑 Retina 深色和浅色的第 1–4 项，以及第 7 项；结束后停掉本机测试环境。
- [ ] **Step 4:** 运行全部 Python 测试和 native 样本测试。Expected: 全部 PASS。
- [ ] **Step 5:** 把远端外观恢复为固定深色。Commit `docs: 界面自动适配验收记录与支持矩阵`。
