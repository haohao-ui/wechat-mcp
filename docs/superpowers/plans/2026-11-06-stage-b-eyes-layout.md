# 阶段 B：感知层读界面地图 + 画面识别骨架 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 感知代码（气泡识别、会话列表、聊天区坐标）改为从界面地图取区域与主题阈值，行为逐像素不变；新增"当前在哪个画面"的识别骨架（只识别、不接入操作）。

**Architecture:** worker 按微信版本加载 `~/.wechat-rpa/layout/wechat-<版本>.json`（阶段 A 的 `WechatLayout.path(forVersion:directory:)`），按修改时间缓存为当前地图；`wechat_bubbles` 命令行由环境变量 `WECHAT_LAYOUT` 指定文件（测试用仓库默认文件）。画面识别为纯函数，输入窗口列表与焦点摘要，输出画面种类。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel），Python 3 unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-layered-architecture-design.md` §3.1、§4、§5；阶段 A 计划 `docs/superpowers/plans/2026-11-05-stage-a-common-layout.md`。

## Global Constraints

- **行为不变**：`tests/test_native_frames.py` 现有用例不改断言、原样通过；`tests/test_layout_consistency.py` 中已迁移项改为断言"代码里不再有该常量"，未迁移项（Rust 输入区、Python `SCROLL_STEP_POINTS`）保持原断言。
- 迁移范围（B1）：`native/wechat_bubbles.swift`（窗口 880×640 判定、标题裁剪、`chat_content` 裁剪、媒体裁剪偏移 315/60、深浅主题阈值）、`native/bot/Sessions.swift`（`sessionListRect`、行名偏移 12）、`native/bot/Worker.swift`（`scroll_chat` 的 `chat` 区域、每格行数）、`native/bot/Files.swift`（媒体右键的 `chat` 偏移、另存为菜单文字 `menu_labels.save_as`）。
- 不改 `native/action/`、`src/`；`scripts/wechat_bot/` 只改 `coordinator.py` 的 `WINDOW_UNAVAILABLE`（加 `layout_missing`、`layout_unreadable`）。
- 地图缺失或加载失败：worker 请求返回 `invalid("layout_missing")` 或对应 `layout_*` 错误码；**不回退到写死的默认值**。
- 版本号：`NSRunningApplication` 中微信（`com.tencent.xinWeChat` / `com.tencent.WeChat`）bundle 的 `CFBundleShortVersionString`。
- Swift 5.7 语法；不新增依赖。

## Review Focus

1. 地图缺失、损坏、版本找不到：worker 明确报错且机器人进入等待，不发生任何点击。
2. 地图在运行中被替换（重新定位后）：下一次请求使用新文件（按修改时间重读），不需重启 worker。
3. 主题阈值从 JSON 读出后，深浅两套样本帧（含 Retina 2x 夹具）的气泡方向与类型和迁移前完全一致。
4. 画面识别把"微信有多个窗口"（图片查看器打开）判为非 main。
5. 焦点信息缺失（保存面板由系统进程绘制）时，画面识别返回 unknown 而不是 main。

---

### Task B1: 感知与坐标改读界面地图（agy）

**Files:**
- Modify: `native/wechat_bubbles.swift`、`native/bot/Sessions.swift`、`native/bot/Worker.swift`、`native/bot/Files.swift`
- Modify: `native/common/WechatLayout.swift`（只增，不改已有接口）
- Modify: `tests/test_native_frames.py`（编译加入 `native/common/*.swift`；子进程设 `WECHAT_LAYOUT`；新增测试）
- Modify: `tests/test_layout_consistency.py`
- Modify: `scripts/wechat_bot/coordinator.py`、`tests/test_bot_routing.py`
- Modify: `scripts/install.sh`（复制 `native/layout/*.json` 到 `~/.wechat-rpa/layout/`，目录 700、文件 600）、`scripts/build.sh`（`wechat_bubbles` 编译加入 `native/common/*.swift`）、`.github/workflows/ci.yml`、`.github/workflows/release.yml`（同上，并打包 `native/layout/*.json`）

**Interfaces:**
- Consumes: 阶段 A 的 `WindowRect`、`WechatLayout.load/path(forVersion:directory:)/region(_:)`。
- Produces:
  - `func currentLayout() throws -> WechatLayout`：`WECHAT_LAYOUT`（文件路径）优先；否则按微信版本在 `WECHAT_LAYOUT_DIR`（默认 `~/.wechat-rpa/layout`）中找；按修改时间缓存；失败抛 `Failure.invalid("layout_missing")` 或 `layout_*`。
  - `func themeProfiles(_ layout: WechatLayout) -> [ThemeProfile]`：名称与判定逻辑与现在完全相同，阈值来自 JSON。
  - `analyzeFrame`、`normalizedFrame`、`SessionProbe.probe`、`openSessionRow`、`scroll_chat`、`saveAttachment` 内部改用当前地图；对外参数与返回值不变。

- [ ] **Step 1: 写失败测试**
  - 现有 `CASES` 全部用例在 `WECHAT_LAYOUT=<仓库默认文件>` 下照常通过（测试改为设置该环境变量）。
  - `test_missing_layout_is_refused`：`WECHAT_LAYOUT=/nonexistent.json` → 错误含 `layout_missing` 或 `layout_unreadable`，无气泡输出。
  - `test_layout_drives_the_crop`：复制默认文件、把 `chat_content.h` 改小 200 → 原本靠下的气泡不再出现。
  - `test_layout_errors_wait_instead_of_pausing`（`test_bot_routing.py`）：`invalid("layout_missing")` → `waiting_for_wechat_window`，不暂停。
  - 一致性测试中已迁移项改为断言源码不再包含对应常量（例如 `CGRect(x:315,y:60,width:full.width-325`）。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_native_frames tests.test_layout_consistency tests.test_bot_routing -v`
- [ ] **Step 3: 实现**（逐个文件替换常量；每替换一处跑一次 `tests.test_native_frames`）
- [ ] **Step 4: 运行通过**；全量 `python3 -m unittest discover -s tests`、`cargo test` 通过；`swiftc -O -target x86_64-apple-macos12 -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/common/*.swift native/action/*.swift native/bot/*.swift -o $TMPDIR/w` 通过
- [ ] **Step 5: 提交** `refactor: 感知与坐标改读界面地图（行为不变，夹具逐帧一致）`

### Task B2: 画面识别骨架（agy，与 B1 并行）

**Files:**
- Create: `native/eyes/Screens.swift`
- Create: `tests/eyes_selftest/main.swift`
- Test: `tests/test_screens.py`

**Interfaces:**
- 不依赖 B1；只用 Foundation/CoreGraphics。
- Produces:
  - `enum ScreenKind: String { case main, contextMenu = "context_menu", savePanel = "save_panel", otherWechatWindow = "other_wechat_window", unknown }`
  - `struct WindowInfo { let owner: String; let pid: Int32; let layer: Int; let bounds: CGRect; let onScreen: Bool }`；`func windowInfos(_ rows: [[String:Any]]) -> [WindowInfo]`（由 `CGWindowListCopyWindowInfo` 行转换，缺字段的行丢弃）
  - `struct FocusInfo { let frontmostBundle: String?; let focusedIdentifier: String? }`
  - `func identifyScreen(windows: [WindowInfo], focus: FocusInfo, wechatPID: Int32, mainSize: CGSize) -> ScreenKind`，按顺序：
    1. `focus.focusedIdentifier == "saveAsNameTextField"` → `save_panel`。
    2. 微信进程有 layer ≠ 0、宽 20..600、高 20..900 的屏上窗口 → `context_menu`。
    3. 前台为微信，且微信进程 layer 0 的屏上窗口恰好 1 个、尺寸与 `mainSize` 相差 ≤2 点 → `main`。
    4. 微信进程 layer 0 的屏上窗口多于 1 个 → `other_wechat_window`。
    5. 其余 → `unknown`。
  - 本任务不接入 worker 请求流程。

- [ ] **Step 1: 写失败测试**（自测程序读 JSON 夹具 `{windows, focus, wechat_pid, main:[w,h]}`，输出画面种类）：`test_main_window_only`、`test_context_menu_popup`、`test_save_panel_focus`、`test_image_viewer_makes_other_window`、`test_missing_focus_info_is_unknown`、`test_other_app_frontmost_is_unknown`、`test_cg_rows_convert`。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_screens -v`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**；`swiftc -target x86_64-apple-macos12 -parse-as-library native/common/*.swift native/eyes/*.swift -emit-library -o $TMPDIR/e.dylib` 通过
- [ ] **Step 5: 提交** `feat: 画面识别骨架（主窗口/右键菜单/保存面板/其他微信窗口/未知）`

### Task B3: 整合与真机验证（整合者）

- [ ] 合并 B1、B2；worker 编译加入 `native/eyes/*.swift`（build.sh、CI）。
- [ ] 远端：复制 `native/layout/wechat-4.1.15.json` 到 `~/.wechat-rpa/layout/`，Swift 5.7 编译、备份旧二进制、部署 worker，重启机器人。
- [ ] 真机：三个会话正常监听；文字、图片、文件各一轮正常回复；临时改名地图文件 → 机器人等待且不点击 → 改回后恢复。
- [ ] 提交 `build: 接入 native/eyes；部署界面地图`
