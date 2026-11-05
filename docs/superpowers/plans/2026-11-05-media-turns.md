# 图片与文件轮次 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 客户发完（安静 10 秒）后，机器人把这一轮的文字、全部图片、文件都交给模型理解，结合最近对话只回一条；图片加载/高图片不再造成断档；发送前确认没人回过。

**Architecture:** 
- Swift 解析器给图片气泡加一个抗加载变化的感知哈希 `phash`，Python 在比较前用它把"同一张图"对齐（`policy.reconcile`）。
- 断档时先向上滚动找回锚点（新 worker 操作 `scroll_chat` + `policy.stitch`）。
- `BotService` 增加落盘的"轮次"（`turn.json`）：先累积，满足安静或上限后再走取件 → 理解（必要时分批）→ 发送前检查 → 发送。
- 理解文字存进历史库 `media.understanding`，供后续上下文使用。

**Tech Stack:** Python 3.12 asyncio + unittest；Swift 5.7+（远端 macOS 12，本机 macOS 27）；SQLite（`history/schema.py` 迁移）；OpenAI 兼容接口（`deepseek-flash`）。

**Spec:** `docs/superpowers/specs/2026-11-05-media-turns-design.md`

## Global Constraints

- 安静时长 `turn_quiet_seconds` 默认 **10**；一轮上限 `turn_max_seconds` 默认 **60**；单次模型调用图片数 `images_per_model_call` 默认 **4**。都是 config 可选键，类型 int，范围分别 0–120、10–600、1–16。
- 回复仍受 `validate_reply`：1–200 字，提示词要求 100 字以内。
- 单个附件上限 10MB（`HistoryStore.MAX_MEDIA`、worker `file_size_limit`）。
- 日志只记元数据，不记消息正文、回复正文、理解文字。
- 只操作测试会话。fixture 中其他联系人打码，原始截图用完即删。
- 远端 Swift 5.7 / SDK 13.1：新 Swift 代码不能用 5.9+ 语法（`if` 表达式、宏等），除非包在 `#if compiler(>=5.9)` 里。
- 不改 MCP host（`wechat-desktop-probe`）的公共工具接口。只在 bot worker 协议里新增操作。
- 数据库结构变更只限：新迁移 3，给 `media` 加两列（spec §5.4，已获批准）。

## Review Focus

1. **连续几张同尺寸截图**（手机截图常见）：phash 不同就必须算不同消息。不能被当成"重复锚点"跳过。见 Task 1 `test_same_size_different_images_are_distinct`。
2. **轮次进行中服务重启**：`turn.json` 落盘。重启后照常继续计时，不重复回复，也不丢掉这一轮。见 Task 4 `test_open_turn_survives_restart`。
3. **机器人自己的回复回显出现在屏幕上**：发送前的屏幕检查只看发送之前的画面；发送后的回显不能被下一轮当成"人工已回复"。见 Task 5 `test_own_echo_is_not_human_reply`。
4. **第 9 张图下载失败而其他成功**：回复必须点名"第 9 张图片"。见 Task 7 `test_failed_item_named_when_model_omits_it`。
5. **滚动找回锚点后没能回到底部**：必须确认回到了底部，再继续处理。否则下一帧会把旧消息当新消息。见 Task 3 `test_scroll_recovery_requires_return_to_bottom`。

---

### Task 1: 图片感知哈希与对齐、文件卡片识别

**Files:**
- Modify: `native/wechat_bubbles.swift`（媒体分支，约 196–232 行）
- Modify: `scripts/wechat_bot/policy.py`
- Test: `tests/test_wechat_bot.py`、`tests/test_native_frames.py`

**Interfaces:**
- Produces:
  - 图片气泡新增字段 `phash: String`，16 位小写十六进制，是 64 位 dHash。`digest` 改为 `sha256("image:\(width/4):\(height/4)")`。`media_id` 不变，仍是像素 SHA256，用于下载。
  - `policy.PHASH_TOLERANCE = 12`。
  - `policy.fingerprint(b) -> tuple`，返回 `(direction, digest, phash 或 None)`。
  - `policy.reconcile(previous: list[dict], current: list[dict]) -> list[dict]`：返回 current 的副本。对每个 current 图片，在 previous 里找 direction、digest 都相同，且 phash 汉明距离 ≤ 12 的图片；找到就把它的 `phash` 和 `media_id` 复制过来。每个 previous 图片最多被匹配一次，按顺序贪心匹配。
  - `policy.FINGERPRINT_VERSION = 2`。

- [ ] **Step 1: 写失败测试**（`tests/test_wechat_bot.py`）
  - `test_loading_image_reconciles_to_same_fingerprint`：previous 图片 `phash='0f0f0f0f0f0f0f0f'`，current 同 digest，phash 只差 5 位。断言 `fingerprint(reconcile(prev,cur)[0]) == fingerprint(prev[0])`，并且 `new_bubbles(prev, reconcile(prev,cur)) == []`。
  - `test_same_size_different_images_are_distinct`：三张图 digest 相同，phash 两两距离 ≥ 30。cursor 是前两张，current 是三张全部。断言 `new_bubbles` 返回最后一张，并且不抛 `AmbiguousAnchor`。
  - `test_far_phash_is_not_reconciled`：距离 20。断言 phash 不被改写。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests/test_wechat_bot.py -k "reconcile or distinct or far_phash" -q`
  - Expected: FAIL（ImportError: reconcile）
- [ ] **Step 3: 在 `policy.py` 实现 `reconcile`、`PHASH_TOLERANCE`、`FINGERPRINT_VERSION`，把 `fingerprint` 改成三元组**
  - 汉明距离用 `bin(int(a,16)^int(b,16)).count('1')`。
- [ ] **Step 4: Swift 实现 phash 和新 digest**
  - phash 的算法：
    1. 从**原始截图**（与缩略图同一裁剪区域，乘 scale）缩放到 9×8 灰度，用 `CGContext` 加 `.high` 插值；
    2. 每行相邻像素比较 `left > right`，得到 64 位；
    3. 按行优先输出成十六进制。
  - 文件卡片判定改为：`names.count == 1`，并且以下两个条件满足其一：
    - 有大小；
    - `body` 的非空行数 ≤ 3。
- [ ] **Step 5: 更新 `tests/test_native_frames.py`**
  - `BOTTOM` 里 `岗位信息更新2.13.pdf` 的 kind 改为 `'file'`。
  - 新增 `test_image_bubbles_carry_phash`：对 `CASES` 里每个 image 气泡，断言 `len(b['phash'])==16`（现有 fixture 没有图片时，跳过并说明原因）。
- [ ] **Step 6: 运行**
  - Run: `python3 -m pytest tests/test_wechat_bot.py tests/test_native_frames.py -q`
  - Expected: PASS
- [ ] **Step 7: 远端校准（需要用户配合）**
  1. 用 `scripts/dev/capture_frame.sh`，在测试会话里抓"图片加载中"和"加载完成"两帧；
  2. 记录两帧 phash 的距离，以及两张不同截图之间的距离；
  3. 距离不符合 ≤ 12 / ≥ 20 时，调整 `PHASH_TOLERANCE`，并在 ledger 里记 Ruling；
  4. 把打码后的两帧存为 `tests/fixtures/frames/image-loading.png` 和 `image-loaded.png`，加一条 native 测试断言两帧 reconcile 后 fingerprint 相等。
- [ ] **Step 8: Commit** `feat: 图片感知哈希对齐，文件卡片识别不依赖大小行`

### Task 2: 游标与历史库迁移到指纹 v2

**Files:**
- Modify: `scripts/wechat_bot/service.py`（`__init__`、`advance`、`cycle` 开头）
- Modify: `scripts/wechat_bot/history/store.py`（`record_frame`）
- Test: `tests/test_wechat_bot_service.py`、`tests/test_history_store.py`

**Interfaces:**
- Consumes: Task 1 的 `fingerprint`、`reconcile`、`FINGERPRINT_VERSION`。
- Produces:
  - 机器人游标 JSON 增加 `"fingerprint_version": 2`。`advance(frame)` 写入时自动加上。
  - `HistoryStore.record_frame` 存三元组。读到长度为 2 的旧元组，视为旧游标：只覆盖 `contact_cursor`，返回 `[]`，不插入任何消息，并记日志 `history_cursor_reset`。

- [ ] **Step 1: 写失败测试**
  - `test_legacy_cursor_rebaselines_without_reply`（service）：cursor 没有 `fingerprint_version`，下一帧多一条入站。断言模型没被调用，cursor 被替换并带 version 2。
  - `test_legacy_history_cursor_resets_without_duplicates`（store）：预先写入二元组游标，再 `record_frame` 同屏 3 条。断言返回 `[]`，`messages` 表行数不变。
  - `test_reconciled_image_is_not_recorded_twice`（store）：先记录加载中的图片，再记录 phash 相近的同一张图。断言只有 1 行。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests/test_wechat_bot_service.py tests/test_history_store.py -k "legacy or reconciled" -q`
  - Expected: FAIL
- [ ] **Step 3: 实现**
  - service：`cycle` 里在 `new_bubbles` 之前先 `frame['bubbles']=reconcile(self.cursor['bubbles'],frame['bubbles'])`。cursor 没有 version 时走"空基线"同一路径，`historical_messages_ignored=True`，并记日志 `legacy_cursor_rebaseline`。
  - store：`record_frame` 中先用上一帧对齐。为此 contact_cursor 需要能还原 phash：`frame_fingerprints` 改存 `[direction,digest,phash]` 列表，对齐时以 `{'direction','digest','phash'}` 字典参与。
- [ ] **Step 4: 运行全部 Python 测试**
  - Run: `python3 -m pytest tests -q -x --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 5: Commit** `feat: 指纹 v2 迁移，旧游标重建基线且不重复写历史`

### Task 3: 断档时向上滚动找回锚点

**Files:**
- Modify: `native/bot/Worker.swift`（`request` 新增 `scroll_chat`）
- Modify: `scripts/wechat_bot/native_capture.py`（新增 `scroll`）
- Modify: `scripts/wechat_bot/policy.py`（新增 `stitch`）
- Modify: `scripts/wechat_bot/service.py`（`except ContinuityGap` 分支）
- Test: `tests/test_wechat_bot.py`、`tests/test_wechat_bot_service.py`、`tests/test_bot_routing.py`

**Interfaces:**
- Produces:
  - worker 操作 `scroll_chat`：参数 `direction: "up"|"down"`，`pages: 1...5`，以及常规的 `recipient`、`accepted_names`、`manage_window`。
    - 在聊天区中心（窗口坐标 `315+282, 60+218`）发送 `CGEvent(scrollWheelEvent2Source:units:.line,wheelCount:1,wheel1:±12)`，次数为 `pages` 次，每次间隔 0.15 秒；
    - 然后返回 `capture(...)` 的结果；
    - 标题不符时抛 `allowlisted_title_mismatch`。
  - `NativeCapture.scroll(direction: str, pages: int = 1) -> dict`：返回一帧。
  - `policy.stitch(cursor: list, pages: list[list]) -> list`：pages 从最旧到最新排列，最后一个是当前帧。按以下步骤拼接：
    1. 找到与 cursor 有重叠、且最旧的那一页 `pages[i]`；
    2. 结果先取 `new_bubbles(cursor, pages[i])`；
    3. 之后每相邻两页追加 `new_bubbles(pages[j], pages[j+1])`。

    任何一对没有重叠就抛 `ContinuityGap`。
  - `BotService.recover_gap(capture, frame) -> list | None`：最多向上滚 3 页，找到重叠后执行 `scroll('down', pages=k+2)`，再 `frame()`。只有回到底部的帧经 reconcile 后的 fingerprint 序列与 `frame` 相同，才返回 changes；否则返回 None，走原来的重建基线规则。

- [ ] **Step 1: 写失败测试**
  - `test_stitch_joins_pages_after_cursor`（policy）：cursor `[a,b]`，pages 为 `[[a,b,c],[c,D]]`。断言结果是 `[c,D]`。
  - `test_stitch_without_overlap_raises`（policy）。
  - `test_tall_image_gap_recovered_by_scrolling`（service）：fake capture 的 `frame` 先返回 `[TALL]`，`scroll('up')` 返回 `[b,c,TALL]`，`scroll('down')` 后的 `frame` 返回 `[TALL]`。断言这一轮包含 `c` 和 `TALL`，并且没有 `continuity_gap_rebaseline` 日志。
  - `test_scroll_recovery_requires_return_to_bottom`（service）：回到底部后的帧和原帧不同。断言返回 None，cursor 不前移，健康状态是 `waiting_for_stable_frame`。
  - `test_scroll_call_carries_manage_window`（routing）：断言 params 带有 `direction`、`pages`、`manage_window`。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests -q -k "stitch or scroll" --ignore=tests/test_native_frames.py`
  - Expected: FAIL
- [ ] **Step 3: 实现 Python 部分**（`stitch`、`NativeCapture.scroll`、`recover_gap`；service 在 ContinuityGap 时先调用它，`RemoteCapture` 没有 `scroll` 时跳过）
- [ ] **Step 4: 实现 Swift `scroll_chat`**
  - 本机编译：`swiftc -O -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/bot/*.swift -o /tmp/wbw`（按现有构建命令），Expected: 无错误。
- [ ] **Step 5: 运行**
  - Run: `python3 -m pytest tests -q --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 6: Commit** `feat: 断档时向上滚动找回锚点并确认回到底部`

### Task 4: 轮次累积（10 秒安静 / 60 秒上限）

**Files:**
- Modify: `scripts/wechat_bot/service.py`（`load_config`、`cycle` 拆分）
- Modify: `scripts/wechat_bot/coordinator.py`（`due_sessions`、`sweep`）
- Test: `tests/test_wechat_bot_service.py`、`tests/test_bot_routing.py`

**Interfaces:**
- Produces:
  - config 键 `turn_quiet_seconds`、`turn_max_seconds`、`images_per_model_call`（见 Global Constraints），在 `load_config` 里校验。
  - `BotService.clock`：默认为 `time.time`，测试中可替换。
  - `turn.json`（`child.root`）：`{"opened_at": float, "last_inbound_at": float, "baseline": <开轮时的 cursor frame>, "bubbles": [<按顺序累积的新入站气泡>], "frame": <最近确认帧>}`。
  - `BotService.turn: dict | None`，`BotService.turn_due() -> bool`。
  - `cycle` 的流程：
    1. 计算 changes（含 Task 3 的恢复）；
    2. 有入站就追加到 turn，更新 `last_inbound_at`，`advance(confirmed)`，并记日志 `turn_opened`（首条时）；
    3. 出站气泡（人工）出现在 turn 之后：丢弃 turn，记日志 `already_replied source=screen`（与 Task 5 共用 `drop_turn(source)`）；
    4. 只有 `turn_due()` 时才进入 `process_turn(client, capture)`，也就是原来从 `limited()` 开始的那一段，输入为 `turn['bubbles']`，批次键为 `batch_key(turn['baseline'], turn['bubbles'])`。
  - `turn_closed` 日志字段：images、files、texts、wait_seconds。
  - coordinator：`due_sessions` 对 `child.turn is not None` 的联系人给出 reason `'turn_open'`。`download_pending_media` 的条件里增加"所有 session 都没有 turn"。

- [ ] **Step 1: 测试 setUp 调整**：`tests/test_wechat_bot_service.py` 的 config 加 `'turn_quiet_seconds':0`，旧测试保持"一轮即回"的语义。在 ledger 里记 Ruling。
- [ ] **Step 2: 写失败测试**
  - `test_reply_waits_for_quiet_period`：quiet=10，clock 先在 t=0 收到图片，t=5 收到文字，t=9 时无新内容（不回复）。t=15 时断言模型只被调用一次，输入包含两条。
  - `test_turn_closes_at_max_even_while_customer_keeps_sending`：每 5 秒来一条，到 t=60 时回复一次。之后的消息进入新的一轮。
  - `test_open_turn_survives_restart`：t=3 时重建 `BotService`（同一个 state_dir），到 t=11 时回复一次，输入包含重启前的那条。
  - `test_image_only_turn_replies`：只有一个 `kind:'image'` 气泡的轮次。断言模型被调用（attachments 用 fake）。
  - `test_open_turn_session_stays_due`（routing）：A 有 turn 且不是活动会话时，`due_sessions` 包含 A。
  - `test_config_turn_keys_validated`：类型和范围越界时抛 ValueError。
- [ ] **Step 3: 运行确认失败**
  - Run: `python3 -m pytest tests/test_wechat_bot_service.py tests/test_bot_routing.py -q -k "quiet or max or restart or image_only or turn"`
  - Expected: FAIL
- [ ] **Step 4: 实现**（`load_config`、`turn.json` 读写用 `write_private`、`cycle` / `process_turn` 拆分、coordinator reason）
- [ ] **Step 5: 修改 `test_new_activity_during_model_generation_suppresses_stale_send`**
  - 改名为 `test_new_activity_during_generation_still_sends_and_stays_pending`；
  - 断言照常发送、journal 状态为 `triggered`、新消息留到下一轮（spec §5.8）；
  - 实现：删掉 pre-send 那段 "fingerprint 不同就 superseded" 的逻辑，改为只用 `check_frame(latest)` 检查标题。
- [ ] **Step 6: 运行全部 Python 测试**
  - Run: `python3 -m pytest tests -q --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 7: Commit** `feat: 按联系人累积轮次，安静 10 秒或满 60 秒后回复一条`

### Task 5: 发送前确认没回复过

**Files:**
- Modify: `scripts/wechat_bot/history/store.py`（新增 `replied_after`）
- Modify: `scripts/wechat_bot/service.py`（`process_turn` 发送前、journal 已存在分支）
- Test: `tests/test_history_store.py`、`tests/test_wechat_bot_service.py`

**Interfaces:**
- Consumes: Task 4 的 `drop_turn(source)`、`process_turn`。
- Produces:
  - `HistoryStore.replied_after(contact_id, message_ids: list[str]) -> bool`：在这些消息里最大的 seq 之后，有 `sender IN ('bot','self_other')` 的出站消息时返回 True。bot 消息如果对应的 send 状态在 `EXCLUDED_SEND` 里，不计入。
  - `BotService.drop_turn(source: str)`：清空 turn、删除 `turn.json`，记日志 `already_replied source=<history|screen|journal>`，并把健康状态设为 `watching`。
  - journal 已存在时：状态为 `triggered` → `drop_turn('journal')`；其他状态保持原来的暂停行为。

- [ ] **Step 1: 写失败测试**
  - `test_replied_after_sees_manual_reply`、`test_replied_after_ignores_failed_bot_send`（store）。
  - `test_manual_reply_during_wait_cancels_turn`（service）：turn 开启后，下一帧在入站之后出现一条出站气泡。断言没有调用模型，也没有发送，并且有 `already_replied`。
  - `test_history_reply_blocks_send`（service）：在 `send_message` 之前，`replied_after` 返回 True。断言没有调用 `send_message`。
  - `test_reprocessed_triggered_turn_is_not_resent`（service）：同一个 key 的 journal 状态为 `triggered`。断言不暂停、不发送。
  - `test_own_echo_is_not_human_reply`（service）：机器人发送并回显之后，下一轮客户又发一条。断言照常回复，不因为上一条回显而判定为人工已回复。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests/test_history_store.py tests/test_wechat_bot_service.py -q -k "replied or manual or triggered_turn or own_echo"`
  - Expected: FAIL
- [ ] **Step 3: 实现**：发送前依次检查 `history.replied_after(contact_id, input_ids)`，以及 `latest` 帧在 turn 最后入站之后有没有出站气泡。命中任何一项就 `drop_turn`，并把 journal 状态记为 `already_replied`。
- [ ] **Step 4: 运行全部 Python 测试**
  - Run: `python3 -m pytest tests -q --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 5: Commit** `feat: 发送前确认人工或历史未回复过`

### Task 6: 取件：全部图片、原图优先、逐项失败

**Files:**
- Modify: `native/bot/Worker.swift`（新增 `prepare_image`）、`native/bot/Files.swift`（`saveAttachment` 加 `checkSize: Bool = true`）
- Modify: `scripts/wechat_bot/native_capture.py`（`attachments`）
- Modify: `scripts/wechat_bot/service.py`（`process_turn` 取件段、删除 `recent_files`）
- Test: `tests/test_bot_routing.py`、`tests/test_wechat_bot_service.py`

**Interfaces:**
- Produces:
  - worker 操作 `prepare_image`：参数 `media_id`、`snapshot_id`。
    - 右键图片 → "另存为"，保存到 `media/<id>/original.<ext>`；
    - 返回 `{media_id, extension, size, original_bytes:true}`；
    - ext 由文件魔数决定（png/jpg）；
    - 失败时抛 `Failure.invalid(<code>)`。
  - `NativeCapture.attachments(items, frame) -> (images: list[dict], files: list[dict], failed: list[dict])`：
    - `images` 元素为 `{'path', 'label', 'message_bubble', 'source':'original'|'chat_thumbnail'}`；
    - `files` 元素为 `{'filename','label', ...extract 内容, 'original_bytes'}`；
    - `failed` 元素为 `{'label','kind','reason'}`；
    - label 为 "第N张图片"（N 按本轮图片顺序从 1 开始）或文件名；
    - **不再限制 4 张**。图片原图失败时退回缩略图；缩略图也失败，就进入 failed。
  - reason 代码：`download_failed`、`too_large`、`unsupported_type`、`extract_failed`、`empty`、`snapshot_changed`。
  - 合计提取文字超过 24000 字符时，按文件顺序截断，最后一个放不下的文件进入 failed，原因记为 `too_large`。
  - `process_turn` 对每个成功项调用 `history.attach_media(message_id, path, kind, filename, source)`，message_id 来自 `match_recent_inbound` 与 turn 气泡的对应关系；失败项记日志 `media_failed kind reason`。

- [ ] **Step 1: 写失败测试**
  - `test_nine_images_all_returned`（routing，fake call/download）：9 个图片气泡，断言 `len(images)==9`，label 依次为 第1张图片…第9张图片。
  - `test_original_failure_falls_back_to_thumbnail`：`prepare_image` 抛错时，断言 source 为 `chat_thumbnail`。
  - `test_unsupported_file_goes_to_failed`：`prepare_file` 抛 `unsupported_file_type`，断言 failed 里有 `{'label':'a.zip','reason':'unsupported_type'}`，并且不抛异常。
  - `test_oversized_extracts_truncate_not_pause`：两个大文件，断言其中一个进入 failed（`too_large`），并且不抛异常。
  - `test_turn_media_attached_to_history`（service）：断言 `media` 表行数等于成功项数。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests/test_bot_routing.py tests/test_wechat_bot_service.py -q -k "nine or fallback or unsupported or oversized or attached"`
  - Expected: FAIL
- [ ] **Step 3: 实现 Python 部分；实现 Swift `prepare_image`**（复用 `saveAttachment(item,window:,destination:,checkSize:false)`），本机编译通过。
- [ ] **Step 4: 运行全部 Python 测试**
  - Run: `python3 -m pytest tests -q --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 5: Commit** `feat: 一轮全部图片文件取件，原图优先，逐项失败不暂停`

### Task 7: 理解、分批与回复

**Files:**
- Modify: `scripts/wechat_bot/history/schema.py`（迁移 3）、`scripts/wechat_bot/history/store.py`（`set_understanding`、`recent_context` 带媒体信息）
- Modify: `scripts/wechat_bot/model_api.py`（`generate_api` 泛化）
- Create: `scripts/wechat_bot/understanding.py`
- Modify: `scripts/wechat_bot/reply.py`（`generate` 新参数）、`scripts/wechat_bot/service.py`（`process_turn` 理解段）
- Test: `tests/test_bot_model_api.py`、`tests/test_history_store.py`、`tests/test_wechat_bot_service.py`、新建 `tests/test_understanding.py`

**Interfaces:**
- Consumes: Task 6 的 `(images, files, failed)`。
- Produces:
  - 迁移 3：`ALTER TABLE media ADD COLUMN understanding TEXT; ALTER TABLE media ADD COLUMN understood_at INTEGER;`
  - `HistoryStore.set_understanding(media_id: str, text: str) -> None`：截断到 1000 字。
  - `HistoryStore.recent_context(...)` 每项可以多一个 `'media': {'kind','filename','understanding' 或 None,'path' 或 None}`。path 为 `media` 表 `path` 的绝对路径。
  - `model_api.generate_api(prompt, *, images, config, metadata=None, field='reply', check=validate_reply)`：系统提示词和 JSON 校验都使用 `field`；默认参数下行为与现在完全相同。
  - `understanding.understand(images: list[dict], *, text: str, config: dict, batch_size: int) -> list[str]`：
    - 按 batch_size 切分，每批调用一次 `generate_api(field='notes', check=<list[str]，长度等于该批图片数，每项 1–300 字>)`；
    - 返回每张图一段要点，顺序与 images 一致；
    - 某一批失败（ValueError、RuntimeError、超时）时，这批每张图对应的结果为 None，并记日志 `model_batch index images ok=false`；成功时记 `ok=true`。
  - `reply.generate(..., notes=(), failed=(), context_media=())`：prompt JSON 增加三个字段：
    - `image_notes`：`[{label,notes}]`；
    - `unreadable`：`[label]`；
    - `context` 中媒体的 understanding。

    提示词增加一句："unreadable 列出的每一项都必须在回复里点名并请对方重发。"
  - `service.ensure_unreadable_named(reply: str, failed: list[dict]) -> str`：回复里没有包含某个 failed label 时，在末尾追加 `"〈label〉打不开，麻烦重发一下。"`（多项合并为一句），并截断到 200 字，被截断的是原回复部分。
  - `process_turn` 的规则：
    - 图片数 ≤ batch_size：单次调用，直接带图生成回复。
    - 图片数 > batch_size：先调用 `understand`，成功的要点写入 `set_understanding`；理解失败的那批，其中每张图并入 failed（reason `model_failed`）；然后最后一次调用只带 `notes`，不带图。
    - 上下文里有图片但没有 understanding 时，在余量（batch_size 减去本次已带的图）内附上原图，余量不足时不带。
    - codex-exec 后端不分批，全部图片一次调用。

- [ ] **Step 1: 写失败测试**
  - `test_generate_api_custom_field`（model_api）：`field='notes'` 时，接受 `{"notes":[...]}`，拒绝 `{"reply":...}`。
  - `test_understand_batches_by_size`（understanding）：9 张图、batch_size 4，断言调用 3 次（4/4/1 张），返回 9 项。
  - `test_failed_batch_yields_none_not_exception`（understanding）。
  - `test_migration_adds_understanding_columns`、`test_recent_context_includes_media_understanding`（store）。
  - `test_failed_item_named_when_model_omits_it`（service）：模型回复"已收到"，failed 为 `[第9张图片]`。断言发送的文本包含"第9张图片打不开，麻烦重发一下。"，并且长度 ≤ 200。
  - `test_nine_image_turn_sends_one_reply`（service）：断言 `send_message` 只被调用 1 次，understand 被调用 1 次，最终 generate 不带 images。
  - `test_final_generation_failure_still_pauses`（service）：最后一次 generate 抛错。断言 paused（spec §5.8）。
- [ ] **Step 2: 运行确认失败**
  - Run: `python3 -m pytest tests -q -k "custom_field or understand or migration_adds or media_understanding or omits or nine_image or final_generation" --ignore=tests/test_native_frames.py`
  - Expected: FAIL
- [ ] **Step 3: 实现**（顺序：schema → store → model_api → understanding → reply → service）
- [ ] **Step 4: 运行全部 Python 测试**
  - Run: `python3 -m pytest tests -q --ignore=tests/test_native_frames.py`
  - Expected: PASS
- [ ] **Step 5: Commit** `feat: 图片分批理解存入历史，失败项在回复中点名请重发`

### Task 8: 部署、文档与远端验收

**Files:**
- Modify: `docs/allowlist-bot.md`（新配置键、轮次行为、已回复检查、日志事件）
- Create: `research/media-turns-acceptance-2026-11-05.md`

- [ ] **Step 1: 全量测试**
  - Run: `python3 -m pytest tests -q`
  - Expected: PASS（native 帧测试在本机 macOS 上运行）
- [ ] **Step 2: 部署远端 worker**
  1. 先备份 `~/.wechat-rpa/bin/wechat_bot_worker` 为 `.before-media-<epoch>`；
  2. 在远端编译新 worker 并替换；
  3. 重启本机 `local.wechat-rpa.bot`（`launchctl kickstart -k gui/$UID/local.wechat-rpa.bot`）；
  4. Expected：日志出现 `service_started`，两个会话都是 `watching`，没有 `legacy_cursor_rebaseline` 之外的报错。
- [ ] **Step 3: 与用户逐项执行 spec §7 验收 1–8**（暗色和浅色各至少一次）。在验收记录里写明每项发了什么、日志事件、回复是否符合预期。不自行宣布整体验收通过，结果交给用户判断。
- [ ] **Step 4: 更新文档并 Commit** `docs: 图片文件轮次说明与验收记录`
