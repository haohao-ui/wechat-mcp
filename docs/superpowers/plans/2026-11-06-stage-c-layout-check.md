# 阶段 C：启动核对 + needs_recalibration + 重新定位候选 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** worker 每次截图读取前按界面地图的 `anchors` 核对画面（分隔线、列表边界、标题有字、主题背景），不过就报 `layout_mismatch` 且不读不点；机器人连续 5 次不过升级为 `needs_recalibration` 并在 `control status` 醒目显示；`control calibrate` 只看不点，生成候选地图与标注截图，`--accept` 后才生效。

**Architecture:** `native/eyes/Anchors.swift` 是纯函数（输入归一化后的窗口图、缩放倍数、地图；输出核对报告与实测位置），同一组定位函数既用于核对也用于重新定位。Python 侧只消费错误码 `invalid("layout_mismatch:<失败项,逗号分隔>")`，已有的 `layout_unavailable` 会让会话进入等待。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel），Python 3 unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-layered-architecture-design.md` §4.2–§4.4、决定 D3；阶段 B 计划 `docs/superpowers/plans/2026-11-06-stage-b-eyes-layout.md`。

## Global Constraints

- 核对失败、地图缺失时：**零点击、零打字、零导航**；只等待。
- D3：`layout_mismatch` 连续 5 次（任一成功的 cycle 清零）→ 健康状态 `needs_recalibration`，仍然只等待、继续重试。
- `calibrate` 本阶段**完全不点击**（spec §4.4 的"文件传输助手右键读菜单文字"推迟，候选文件沿用当前地图的 `menu_labels`）；不自动生效。
- 地图文件只用窗口坐标（点）；不写屏幕坐标或像素。
- 不改 `native/action/`、`src/`；不改现有夹具断言。
- Swift 5.7 语法；不新增依赖（Python 不得引入 PIL）。

## Review Focus

1. 右键菜单、保存面板等浮层盖住锚点时：判为 mismatch 而不是误判通过；浮层消失后下一次自动恢复。
2. Retina 2x 帧：实测位置换算回点后与 1x 一致（误差 ≤1 点）。
3. 地图在运行中被 `--accept` 替换：下一次请求按修改时间读新文件，不需重启 worker。
4. `needs_recalibration` 之后核对恢复通过：状态回到 watching，计数清零。
5. `--accept` 遇到候选文件损坏或已过期（版本不符）：拒绝替换、原文件保持不变，并保留备份。

---

### Task C1: 锚点定位与核对（agy）

**Files:**
- Create: `native/eyes/Anchors.swift`
- Create: `tests/eyes_selftest/anchors_main.swift`（独立入口：`anchors <png> <layout.json> [--shift dx,dy] [--blank <region>] [--patch x,y,w,h,r,g,b]`，输出 JSON）
- Test: `tests/test_anchors.py`

**Interfaces:**
- Consumes: 阶段 A 的 `WechatLayout`（`regions`、`themes`、`anchors`）、`WindowRect`。不依赖 `wechat_bubbles.swift`（主题判定按 `layout.themes` 的 `background_max` / `background_min` 自行实现，语义与 `themeProfiles` 的背景判定相同）。
- Produces:
  - `struct AnchorReport { let passed: Bool; let failures: [String]; let theme: String?; let measured: [String: Double] }`（`failures` 为锚点名或 `"theme"`；`measured` 为窗口点坐标，如 `input_divider` → y，`list_border` → x）
  - `func locateHorizontalLine(_ image: CGImage, scale: Int, in rect: WindowRect) -> Double?`、`func locateVerticalLine(_ image: CGImage, scale: Int, in rect: WindowRect) -> Double?`（找贯穿区域 ≥70% 长度、与两侧背景有稳定色差的单条线；返回窗口点坐标）
  - `func textPresent(_ image: CGImage, scale: Int, in rect: WindowRect) -> Bool`（非背景像素占比阈值，不用 OCR）
  - `func detectTheme(_ image: CGImage, scale: Int, layout: WechatLayout) -> String?`（在 `chat_content` 内采样背景）
  - `func verifyAnchors(_ image: CGImage, scale: Int, layout: WechatLayout, tolerance: Double = 3) -> AnchorReport`：对每个 anchor，在其 `region` 的 `edge` 两侧各 12 点的带状区域内定位；与地图位置相差 ≤ tolerance 为通过。`image` 是 `normalizedFrame` 的输出（整窗，像素 = 点 × scale）。

- [ ] **Step 1: 写失败测试**（`tests/test_anchors.py`，夹具 `tests/fixtures/frames/*.png`，scale = 图宽 / 880）
  - `test_all_fixtures_pass`：8 张 r1/retina 夹具全部 `passed`，`theme` 与 `tests/test_native_frames.py` 的 `CASES` 一致。
  - `test_measured_positions_match_layout`：`input_divider` ≈ 496、`list_border` ≈ 300（误差 ≤1，1x 与 2x 都测）。
  - `test_shifted_frame_fails`：`--shift 0,30` → failures 含 `input_divider`；`--shift 30,0` → 含 `list_border`。
  - `test_blank_title_fails`：`--blank title`（用该帧背景色填满）→ failures 含 `title_text`。
  - `test_unknown_gray_fails_theme`：`unknown-gray.png` → failures 含 `theme`，`theme` 为 null。
  - `test_popup_over_divider_fails`：`--patch 315,420,200,150,240,240,240`（盖住分隔线左段的不透明浅色矩形，模拟菜单）→ 不通过。
  - 若夹具中某锚点实际不可见（例如列表右侧无分隔线），**不要改锚点语义或编造阈值**：用 `orca orchestration ask` 问 Tech Lead。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_anchors -v`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**；全量 `python3 -m unittest discover -s tests` 通过；`swiftc -target x86_64-apple-macos12 -parse-as-library native/common/*.swift native/eyes/*.swift -emit-library -o $TMPDIR/e.dylib` 通过
- [ ] **Step 5: 提交** `feat: 锚点定位与界面核对（分隔线/列表边界/标题/主题）`

### Task C2: layout_mismatch 计数与 needs_recalibration（agy，与 C1 并行）

**Files:**
- Modify: `scripts/wechat_bot/coordinator.py`、`scripts/wechat_bot/control.py`
- Test: 新建 `tests/test_recalibration.py`

**Interfaces:**
- Consumes: worker 错误文本 `invalid("layout_mismatch:input_divider,title_text")`（C3 接入；测试里直接构造 `ValueError`）。
- Produces:
  - 协调器属性 `layout_mismatches: int`；`RECALIBRATION_LIMIT = 5`。
  - mismatch 时：计数 +1；`< 5` → 会话健康 `waiting_for_wechat_window`，附 `reason`、`failures`（列表）、`attempt`；`≥ 5` → `needs_recalibration`，附同样字段。两者都 `close_connection()`，不暂停。
  - 任一会话 cycle 成功 → 计数清零。`layout_missing` 等其他 `layout_*` 错误不计数（维持阶段 B 行为）。
  - `control status` 输出新增顶层 `"needs_recalibration": true|false`（任一会话健康为 `needs_recalibration` 即 true），为 true 时向 stderr 打印一行中文提示：`界面核对连续失败，请运行 control calibrate 重新定位`。

- [ ] **Step 1: 写失败测试**
  - `test_mismatch_waits_without_pausing`：1 次 mismatch → `waiting_for_wechat_window`，`failures == ['input_divider','title_text']`，未暂停，无点击/导航调用。
  - `test_five_mismatches_need_recalibration`：连续 5 次 → `needs_recalibration`。
  - `test_success_resets_counter`：4 次 mismatch、1 次成功、再 1 次 mismatch → `waiting_for_wechat_window`，attempt 1。
  - `test_layout_missing_not_counted`：5 次 `layout_missing` → 仍是 `waiting_for_wechat_window`。
  - `test_status_flags_recalibration`：健康文件含 `needs_recalibration` → status JSON `needs_recalibration: true`，stderr 有提示。
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行通过**；全量 `python3 -m unittest discover -s tests`、`cargo test` 通过
- [ ] **Step 5: 提交** `feat: layout_mismatch 连续 5 次升级为 needs_recalibration，control status 醒目提示`

### Task C3: 接入 worker 并真机验证（整合者）

- [ ] worker `capture` 与 `sessions`：`normalizedFrame` 之后、读取之前调用 `verifyAnchors`；不过则 `throw Failure.invalid("layout_mismatch:" + failures.joined(separator:","))`；响应附 `layout_check: {theme, measured}`。
- [ ] 测试：`tests/test_native_frames.py` 新增"夹具经 CLI 解析时核对通过"与"标题区涂空后 CLI 报 layout_mismatch"。
- [ ] 远端：Swift 5.7 编译、备份、部署、重启；三个会话正常监听。
- [ ] 演练：打开一个右键菜单盖住输入区分隔线 → 机器人等待、零点击；Esc 后自动恢复。
- [ ] 提交 `feat: worker 读取前核对界面地图锚点`

### Task C4: control calibrate（只看）（agy，C1 合并后）

**Files:**
- Modify: `native/bot/Worker.swift`（新 operation `calibrate`：截图、用 C1 的定位函数实测，返回 `{measured, theme, window, scale, annotated_png_base64}`；标注图在各区域画框）
- Modify: `scripts/wechat_bot/control.py`（`calibrate` 与 `calibrate --accept`）
- Test: `tests/test_calibrate.py`

**Interfaces:**
- Produces:
  - `control calibrate`：调用 worker → 以当前生效地图为底，按实测值改写派生字段（`chat.h` = divider − chat.y、`input.y` = divider、`session_list.w` = border − session_list.x 等），写 `~/.wechat-rpa/layout/candidate-wechat-<版本>.json` 与 `.png`（权限 600），打印与当前地图的差异；实测缺失项保留原值并在输出中列出。
  - `control calibrate --accept`：候选文件按 `WechatLayout.load` 同款规则校验（必需区域、区域在窗口内、schema 1）通过、版本与正在运行的微信版本一致 → 原文件备份为 `wechat-<版本>.json.bak-<时间>`，原子替换；否则拒绝且原文件不变。
- [ ] Step 1 失败测试：`test_candidate_from_measurements`、`test_accept_replaces_with_backup`、`test_accept_refuses_invalid_candidate`、`test_accept_refuses_version_mismatch`、`test_calibrate_never_clicks`（worker `calibrate` 分支不得调用 `botActions` / `click` / `rightClick` / 键盘事件）。
- [ ] Step 2–4 同上；Step 5 提交 `feat: control calibrate 只看生成候选地图，--accept 备份后生效`
