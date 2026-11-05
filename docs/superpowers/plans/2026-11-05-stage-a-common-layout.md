# 阶段 A：公共坐标与界面地图 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `native/common/`（三种坐标类型与换算、显示器列表）与界面地图（JSON 格式、加载校验、按现有常量生成的默认文件），并用一致性测试锁住"默认地图 = 现有写死常量"。**本阶段不改任何现有行为**，迁移在后续阶段。

**Architecture:** 纯 Swift、无第三方依赖；`native/common/*.swift` 可被 worker、`wechat_click`、自测程序一起编译。测试沿用"Swift 自测程序输出 JSON + Python unittest 断言"的模式（同 `tests/action_selftest`）。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel，`-target x86_64-apple-macos12`），Python 3 unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-layered-architecture-design.md`（§3.2、§4）

## Global Constraints

- 不改现有文件的行为；新代码只新增，不替换调用（Task 3 只做编译接入与一致性测试）。
- Swift 5.7 语法（不用 `if` 表达式、`some` 参数、`consume`、宏）；不新增依赖。
- 类型名避开 `native/macos/Layout.swift` 已有的 `Layout`/`RegionRect`：用 `WindowPoint`、`ScreenPoint`、`ImagePixel`、`WindowRect`、`WindowFrame`、`WechatLayout`。
- 坐标单位：`WindowPoint`/`WindowRect`、`ScreenPoint` 为"点"；`ImagePixel` 为像素；缩放倍数只允许 1 或 2。
- 界面地图只用窗口坐标；不得包含屏幕坐标或像素。
- 默认地图数值必须与当前代码一致：窗口 880×640；会话列表 x60 y60 w240 h580；聊天区 x315 y60 w565 h436；标题区 x250 y0 w620 h50（`wechat_bubbles.swift` 标题裁剪 `x:250,y:0,width:full.width-260,height:50`）；输入区 x315 y496 w565 h144；滚动每格 10 行、90 点；菜单"另存为"文字与 `native/bot/Files.swift` 中 `titles` 数组相同。

## Review Focus

1. 副屏在主屏左侧或上方（负坐标）：`displays()` 返回的矩形与 `screen ↔ window` 换算不出错。
2. Retina（scale 2）截图：像素换算后裁剪区域恰好对应同一块窗口区域（不偏一半）。
3. 界面地图缺字段、区域超出窗口、宽高为 0 或负、未知 schema 版本：加载时报具体错误码，不静默用默认值。
4. 默认地图与写死常量不一致：一致性测试必须失败（防止后续迁移悄悄改值）。
5. 界面地图文件被改成非 JSON 或超大文件：拒绝加载（上限 256KB）。

---

### Task 1: 坐标类型与换算（agy）

**Files:**
- Create: `native/common/Geometry.swift`
- Create: `tests/common_selftest/main.swift`
- Test: `tests/test_common_layer.py`

**Interfaces（Produces）:**
- `struct WindowPoint { var x: Double; var y: Double }`、`struct WindowRect: Decodable { var x, y, w, h: Double; func contains(_ p: WindowPoint) -> Bool }`
- `struct ScreenPoint { var x: Double; var y: Double }`、`struct ImagePixel { var x: Int; var y: Int }`
- `struct WindowFrame { let origin: ScreenPoint; let width: Double; let height: Double; let scale: Int; init?(origin:width:height:scale:) }`（scale 只允许 1、2，否则 nil）
  - `func toScreen(_ p: WindowPoint) -> ScreenPoint`、`func toScreen(_ r: WindowRect) -> CGRect`
  - `func toWindow(_ p: ScreenPoint) -> WindowPoint`
  - `func toImage(_ r: WindowRect) -> CGRect`（像素，= 点 × scale，用于裁剪窗口截图）
- `func displays() -> [CGRect]`：`CGGetActiveDisplayList` + `CGDisplayBounds`（CG 全局坐标）。
- `func display(containing p: ScreenPoint, in displays: [CGRect]) -> CGRect?`

- [ ] **Step 1: 写失败测试**（自测程序子命令 `geometry <case>` 输出 JSON）
  - `test_window_screen_round_trip`：窗口原点 (100,50)、点 (315,60) → 屏幕 (415,110) → 回到 (315,60)。
  - `test_negative_display_origin`：窗口原点 (-1800,-120) 时换算正确；`display(containing:)` 在 [主屏 (0,0,1440,900), 左副屏 (-1920,-180,1920,1080)] 中对点 (-1700,-100) 选出左副屏。
  - `test_retina_crop_matches_points`：scale 2 时聊天区 (315,60,565,436) → 像素 (630,120,1130,872)；scale 1 时与点相同。
  - `test_rejects_bad_scale`：scale 3 → 构造返回 nil。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_common_layer -v` → 编译失败
- [ ] **Step 3: 实现** `Geometry.swift`；`displays()` 只读系统显示器，不访问 NSScreen。
- [ ] **Step 4: 运行通过**；并 `swiftc -target x86_64-apple-macos12 -parse-as-library native/common/*.swift -emit-library -o <tmp>/c.dylib`
- [ ] **Step 5: 提交** `feat: common 坐标类型与换算（窗口/屏幕/像素，CG 显示器列表）`

### Task 2: 界面地图格式与加载（agy，与 Task 1 并行）

**Files:**
- Create: `native/common/WechatLayout.swift`
- Create: `native/layout/wechat-4.1.15.json`
- Create: `tests/common_selftest/layout_main.swift`（独立自测入口，子命令 `layout <path>` / `fallback <dir> <version>`；与 Task 1 的 `main.swift` 分开，避免并行冲突）
- Test: `tests/test_common_layout.py`

**Interfaces（Produces）:**
- 依赖 Task 1 的 `WindowRect`。并行开发时在本任务内临时声明同名结构、合并前删除，由整合者解决。
- `struct WechatLayout: Decodable { let schema: Int; let wechatVersion: String; let window: LayoutSize; let regions: [String: WindowRect]; let sessionRow: SessionRowSpec; let themes: [String: ThemeSpec]; let menuLabels: [String: [String]]; let scroll: ScrollSpec; let anchors: [AnchorSpec] }`（JSON 键为 snake_case：`wechat_version`、`session_row`、`menu_labels`、`points_per_tick`、`lines_per_tick`）
  - `static func load(path: String) throws -> WechatLayout`：文件 ≤256KB；`schema == 1`；必需区域 `session_list`、`title`、`chat`、`input`；每个区域 w>0、h>0、完全在 `window` 内；`scroll.points_per_tick > 0`。错误码：`layout_unreadable`、`layout_too_large`、`layout_schema_unsupported`、`layout_missing_region:<名>`、`layout_region_out_of_window:<名>`、`layout_invalid_value:<字段>`。
  - `func region(_ name: String) throws -> WindowRect`（未知名称抛 `layout_missing_region:<名>`）
  - `static func path(forVersion v: String, directory: String) -> String?`：先找 `wechat-<v>.json`；没有则返回版本号最大且不高于 v 的文件（按数字逐段比较）；都没有返回 nil。
- 默认文件 `native/layout/wechat-4.1.15.json`：按 Global Constraints 的数值；`themes` 写入 `wechat_bubbles.swift` 当前深色/浅色判定阈值；`anchors` 写 spec §4.2 的三项；`screens`、`message_types` 本阶段不写（schema 1 允许缺省）。

- [ ] **Step 1: 写失败测试**
  - `test_default_layout_loads`：加载默认文件，`chat` = (315,60,565,436)，`scroll.points_per_tick` = 90。
  - `test_invalid_layouts_are_refused`：缺 `chat`、区域超出窗口、`w` 为 0、`schema` 2、非 JSON、300KB 文件 → 各自错误码。
  - `test_version_fallback_picks_nearest_lower`：目录含 4.1.12、4.1.15、4.2.0 → 请求 4.1.15 得 4.1.15；4.1.20 得 4.1.15；4.0.1 得 nil。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**；macOS 12 目标编译通过
- [ ] **Step 5: 提交** `feat: 界面地图格式、加载校验与 WeChat 4.1.15 默认地图`

### Task 3: 编译接入与"默认地图 = 现有常量"一致性测试（整合者）

**Files:**
- Modify: `scripts/build.sh`、`.github/workflows/ci.yml`、`.github/workflows/release.yml`（worker 与 `wechat_click` 编译加入 `native/common/*.swift`；不改调用）
- Create: `tests/test_layout_consistency.py`

**Interfaces:**
- Consumes: Task 1、2 全部。

- [ ] **Step 1: 写失败测试** `test_default_layout_matches_hardcoded_constants`：解析默认 JSON，逐项与源码常量比对（正则读取 `Sessions.swift` 的 `sessionListRect`、`wechat_bubbles.swift` 的聊天区与标题裁剪、`policy.py` 的 `SCROLL_STEP_POINTS`、`Files.swift` 的另存为菜单文字、`src/wechat.rs` 的输入区偏移 `315`/`160`/`325`），任一不同即失败。故意改一个值确认会失败，再改回。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 接入编译**；worker、`wechat_click` 在 macOS 12 目标下编译通过；远端 Swift 5.7 编译通过（只编译，不部署）。
- [ ] **Step 4:** 全量 `python3 -m unittest discover -s tests`、`cargo test` 通过
- [ ] **Step 5: 提交** `build: 接入 native/common；一致性测试锁定默认界面地图与现有常量`
