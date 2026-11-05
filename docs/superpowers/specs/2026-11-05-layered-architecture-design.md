# 分层架构与界面地图 设计规格

日期：2026-11-05　分支：feat/wechat-bot-history　状态：用户已确认（2026-11-05）

## 1. 目的

把"看（感知）—做（执行）—流程—业务"拆开，让微信升级、界面变化、新消息类型都只需改一处：

- 坐标、区域、配色、菜单文字不再写死在代码里，集中到按微信版本保存的**界面地图**配置；升级后跑一次**重新定位**、确认后生效。
- 每次启动先**核对**界面地图，不符就停下报 `layout_mismatch`，绝不按旧坐标乱点。
- 识别当前在**哪个画面**（主窗口、图片查看器、内置浏览器、弹窗……），误入就按登记的方式退回，认不出就停。
- 识别**消息类型**（文字、图片、文件、卡片、表情、视频、语音、红包转账……），每类有明确的"允许做什么"。
- 微信界面只有一套"眼和手"：Rust MCP 宿主与 Python 机器人都调用同一个 worker 的操作流程。

## 2. 目标 / 非目标 / 反目标

- 目标：分层结构与模块边界；界面地图格式、启动核对、重新定位；画面目录；消息类型目录（含表情）；Rust 宿主改用 worker 操作流程；语音与来电的分期方案。
- 非目标：本期不做语音消息转写实现、不做 AI 接听电话、不做拖拽、不换 OCR 引擎（Vision 默认，RapidOCR 可选，已有设计）。
- 反目标：
  - 为了"学习界面"去点击用途未知的按钮（可能拨通话、删聊天、发消息）。
  - 全量扫描并保存白名单以外的联系人、群和预览（隐私原则：非白名单内容不离开微信电脑）。
  - 从微信本地数据库/缓存读取消息或语音（加密数据，属于逆向）。
  - 点击红包、转账、通话按钮。

## 3. 分层

```
business  业务编排：Python 机器人（轮次、白名单、模型、历史）；Rust MCP 宿主（协议、幂等、外发记录）
skills    操作流程：open_chat / send_text / send_file / save_as / scroll / dismiss_screen …
          每个流程 = 看 → 做 → 核对；等待一律"等到条件成立（有上限）"，不写固定秒数
eyes      感知：截图、OCR（Vision/RapidOCR）、气泡与消息类型识别、会话列表、画面识别；只看不动
hands     执行：native/action（已完成）：拟人轨迹、有效区域、抢鼠标检测
common    公共基础：坐标系与换算、显示器与缩放、界面地图加载与核对
```

依赖方向只能向下：business → skills → (eyes, hands) → common。eyes 与 hands 互不依赖。

### 3.1 现有代码归位（不推倒重写）

| 现在 | 归入 |
|---|---|
| `native/action/*` | hands（不动） |
| `native/wechat_bubbles.swift`：截图、OCR、主题判定、气泡解析、指纹 | eyes（拆成若干文件；主题配色读界面地图） |
| `native/bot/Sessions.swift`：会话列表识别 | eyes；`openSessionRow` → skills.open_chat |
| `native/bot/Files.swift`：另存为、菜单、按键、粘贴 | skills.save_as / skills.menu；按键与粘贴 → hands |
| `native/bot/Worker.swift`：请求分发 | worker 外壳，只路由到 skills |
| `src/wechat.rs`：宿主自己的 osascript/截图/OCR/点击发送 | 改为调用 worker 的 skills（见 §8） |
| 写死的 `315/60/880×640`、`sessionListRect`、输入框区域、滚动 90pt | 界面地图 |

### 3.2 common：坐标系

三种坐标，各有类型，不混用：

- `WindowPoint`：微信窗口内坐标，单位为"点"，原点为窗口左上角。**界面地图只用它**。
- `ScreenPoint`：全局屏幕坐标（CG 坐标，主屏左上为原点，副屏可为负），单位"点"。hands 只接受它。
- `ImagePixel`：截图像素，= 点 × 缩放倍数（1 或 2）。eyes 处理图像时用它。

换算只在 common 中进行：`window → screen` 需要窗口位置；`screen ↔ image` 需要缩放倍数与截图原点。显示器列表用 `CGGetActiveDisplayList` + `CGDisplayBounds`（直接是 CG 坐标），替代 `NSScreen`（审查 minor 4）。

## 4. 界面地图（配置文件）

### 4.1 位置与版本

- 仓库内置默认：`native/layout/wechat-4.1.15.json`（随代码发布、可审阅）。
- 微信电脑生效副本：`~/.wechat-rpa/layout/<微信版本>.json`；重新定位生成新文件，旧文件保留为 `.bak-<时间>`。
- 选择规则：按当前微信版本号（`CFBundleShortVersionString`）找同名文件；没有则用最近的旧版本并**强制核对**，核对不过即 `layout_mismatch`。

### 4.2 格式（示意）

```json
{
  "schema": 1,
  "wechat_version": "4.1.15",
  "window": {"width": 880, "height": 640},
  "regions": {
    "session_list": {"x": 60,  "y": 60,  "w": 240, "h": 580},
    "title":        {"x": 250, "y": 0,   "w": 620, "h": 50},
    "chat":         {"x": 315, "y": 60,  "w": 565, "h": 436},
    "input":        {"x": 315, "y": 496, "w": 565, "h": 144}
  },
  "session_row": {"height": 64, "name_offset": 12, "time_min_x_ratio": 0.75},
  "themes": {
    "wechat-4-dark":  {"background_max": 37, "incoming": [38, 82], "own": "green"},
    "wechat-4-light": {"background": [250,250,250], "incoming": [238,238,240], "own": "green"}
  },
  "menu_labels": {"save_as": ["另存为…", "另存为...", "另存为", "存储为…", "Save As…"], "copy_link": ["复制链接"]},
  "scroll": {"lines_per_tick": 10, "points_per_tick": 90},
  "anchors": [
    {"name": "input_divider", "region": "chat", "edge": "bottom", "kind": "horizontal_line"},
    {"name": "list_border",   "region": "session_list", "edge": "right", "kind": "vertical_line"},
    {"name": "title_text",    "region": "title", "kind": "text_present"}
  ],
  "screens": {"…": "见 §5"},
  "message_types": {"…": "见 §6"}
}
```

坐标全部是 `WindowPoint`。主题、缩放、窗口在屏幕上的位置变化都不需要改这份文件。

### 4.3 启动核对（每次启动、每次窗口复位后）

1. 截图主窗口，按 `anchors` 逐项检查（分隔线在、列表边界在、标题区有字、背景色属于某个已标定主题）。
2. 全部通过 → 正常工作；记录 `layout_verified`（版本、主题、缩放）。
3. 任一不过 → 不读不点，联系人进入 `waiting_for_wechat_window`，健康状态写 `layout_mismatch` 与失败项；连续失败 N 次（默认 5，待定 D3）升级为 `needs_recalibration`，在 `control status` 醒目显示（兼顾审查 minor 1：不要悄悄罢工）。

### 4.4 重新定位（升级后运行一次）

`python3 -m scripts.wechat_bot.control calibrate`：

- 只看为主：截图找分隔线、列表边界、标题位置、配色；滚动一次测量每格点数。
- 唯一的点击：在**文件传输助手**里对一条样本消息右键读出菜单文字，然后 Esc（§5.2 的样本标定同理）。
- 生成候选文件 + 一张标注了各区域的截图，**不自动生效**；用户确认（`control calibrate --accept`）后替换生效副本。

## 5. 画面目录

### 5.1 登记内容

每个画面：识别特征、允许的操作、退出方式。

| 画面 | 进入 | 识别特征（示例） | 允许 | 退出 |
|---|---|---|---|---|
| main | — | anchors 全部通过 | 全部流程 | — |
| image_viewer | 单击图片 | 独立窗口、底部工具栏（下载/上一张/下一张） | 读、关闭 | Esc / 关闭 |
| browser | 单击卡片/链接 | 独立窗口、地址栏或网页标题栏 | 关闭 | 关闭按钮 |
| file_preview | 双击文件 | 预览窗口 | 关闭 | 关闭 |
| video_player | 单击视频 | 播放控制条 | 关闭 | Esc |
| context_menu | 右键 | 弹出层（不在辅助功能树） | 点登记的菜单项 | Esc |
| save_panel | 另存为 | `saveAsNameTextField` | 填名、回车、取消 | Esc / 取消 |
| incoming_call | 外部 | 接听/挂断按钮 | **无** | **不操作，报告** |
| unknown | — | 以上都不是 | **无** | **停止，报告** |

### 5.2 守卫规则

- 每个 skills 流程开始前识别当前画面：main 才继续；已登记的其他画面先按"退出方式"退回并重新核对；unknown/incoming_call 停止并报 `unexpected_screen:<名>`。
- 流程中途失败时，同样执行"退回 main"（审查 minor 2：保存面板失败后不再残留）。
- 标定方式：在文件传输助手发样本（一张图、一个链接卡片、一个文件），标定程序打开一次、记下特征和退出方式、关闭。**只在文件传输助手**；危险画面（通话）只登记特征、从不触发。

## 6. 消息类型目录

| 类型 | 识别（eyes） | 允许（skills/business） | 本期 |
|---|---|---|---|
| text | 气泡 + OCR | 读 | 已有 |
| image | 无文字的图片气泡、感知哈希 | 读、另存为 | 已有 |
| file | 文件名 + 大小 | 另存为、提取 | 已有 |
| link_card | 卡片框 + 标题 + 来源行 | 读卡片文字；需要正文时右键"复制链接" → 现有 `fetch_article`；**不单击打开** | 做 |
| sticker（表情包） | 无气泡底色、近方形、偏小 | 先查**表情包记忆**（§6.2）；没有则截图交模型理解并记入 | 做 |
| emoji（小黄脸，嵌在文字中） | 文字气泡内的小图标 | 与**表情词典**模板比对，还原为"[OK]"等名称，并附词典释义 | 做 |
| video | 播放图标 + 时长 | 识别并在回复中说明暂不支持 | 做（仅识别） |
| voice | 语音条 + 秒数 | 本期识别并说明暂不支持；后期见 §9 | 做（仅识别） |
| quote（引用） | 气泡内引用块 | 读出引用与正文 | 做 |
| system（撤回、入群） | 居中灰字 | 只记录，不回复 | 做 |
| red_packet / transfer | 红包/转账卡片样式 | **禁止点击**；识别后提醒人工，回复只说明"已通知人工处理" | 做（识别+禁止） |
| 其他未登记 | — | 只读，不点 | — |

### 6.1 表情与"可以不回复"

- 回复策略新增"可不回复"：模型输出 `{"reply": "...", "skip": true|false, "skip_reason": "..."}`；`skip=true` 仅允许在**本轮只有确认类内容**（OK、👍、"好的"表情包、"收到"）时生效，记录 `turn_skipped`；其余情况必须回复。**是否启用由用户决定（待定 D1）**，默认关闭（一律简短回复）。

### 6.2 表情的含义记录（要"学会"微信表情的意思）

微信表情有约定俗成的含义，只看图会误解（例：[微笑] 在微信里常表示无语、敷衍或不满，而不是开心）。因此两份记录：

**表情词典**（小黄脸，固定集合）——`native/layout/emoji-lexicon.json`，随仓库发布、可审阅：

```json
{
  "schema": 1,
  "wechat_version": "4.1.15",
  "entries": {
    "[OK]":  {"templates": {"1x": "<phash>", "2x": "<phash>"}, "meaning": "好的、没问题",
              "chat_uses": ["确认收到", "同意", "结束对话"],
              "service_hint": "多为确认；若前面是我方提问，视为同意"},
    "[微笑]": {"meaning": "字面是微笑", "chat_uses": ["客气", "无语", "敷衍", "不满（常用作阴阳怪气）"],
              "service_hint": "不要当作满意；前面是投诉或等待时按不满处理并致歉"},
    "[旺柴]": {"meaning": "调侃、俏皮（狗头）", "chat_uses": ["开玩笑", "反话"], "service_hint": "前面的话可能是玩笑"}
  }
}
```

- 字段：`templates`（识别用，标定时采集）、`meaning`（字面）、`chat_uses`（微信语境常见用法）、`service_hint`（客服处理建议）。
- 建立：我写常用表情的释义初稿（含微信语境的特殊用法）→ 用户按自己客户的习惯审改 → 在文件传输助手发一遍，标定程序截取 1x/2x 模板。微信新增表情时追加条目。
- 使用：识别出的表情在消息文字中写作 `[OK]`，同时把该条的 `meaning/chat_uses/service_hint` 作为"表情释义"交给模型；模型结合上下文判断本次含义，不得只按字面理解。
- 未收录的小黄脸：写作 `[表情]`，按表情包流程理解（截图交模型），并提示补充词典。

**表情包记忆**（自定义表情、动图，无法事先列全）——存于历史库（新迁移：`stickers` 表，键为感知哈希）：

- 字段：`phash`、`meaning`（如"一只猫敬礼：收到、遵命"）、`source`（`model` / `user`）、`first_seen`、`seen_count`、可选样例缩略图。
- 第一次出现：截图交模型理解，结果记入（`source=model`）；再次出现：直接用记录，理解前后一致、不重复花费模型调用。
- 用户可查看与修正（只读 MCP / HTTP 历史接口增加查询；修正经控制命令写入），`source=user` 的记录优先且不被模型覆盖。
- 隐私：只记录白名单会话里出现的表情包。

## 7. skills 层的统一写法

- 每个流程：`precheck(画面=main, 布局已核对)` → `act`（hands） → `await condition(上限)` → `verify` → 失败则 `dismiss_to_main`。
- 等待统一用"等到条件成立"（如"文件名框获得焦点"），每个条件有上限与错误码；不再用固定秒数做判定。
- 被打断（`user_input_detected`、`target_not_reached`）一律"本次放弃、稍后重试"，不降级（审查 I2/I3 已落实），并计入 §4.3 的升级计数。

## 8. Rust 宿主改用 worker

- worker 新增对宿主开放的操作：`send_text`、`send_file`、`read_chat`、`open_chat`（复用 skills），走现有 `bot-worker.sock`。
- `src/wechat.rs` 中 osascript 点击/粘贴/OCR 核对的发送路径改为调用这些操作；幂等、外发记录、路径校验（`prepare_send_file`）仍在 Rust。
- 过渡期保留旧路径作为回退开关（`WECHAT_HOST_LEGACY_UI=1`），验收后删除。

## 9. 语音与来电（后期，分期）

- 语音消息：优先微信自带"转文字"（右键菜单项）→ OCR 读出 → 按文字处理；没有该菜单项时再评估虚拟音频设备 + 语音识别。不读本地语音文件。回复保持文字。
- 来电第一期：识别 `incoming_call` 画面 → 不接，通知人工；通话结束后自动发文字"我是AI客服，暂时不能接听语音，请文字留言"。
- 来电第二期（另立设计）：配置显式开启、开场声明 AI 身份、专用 Mac 配虚拟音频；需用户确认合规前提。

## 10. 推进顺序与分工

| 阶段 | 内容 | 可交给 agy（Orca） | 我负责 |
|---|---|---|---|
| A | common：坐标类型与换算、显示器列表；界面地图加载 + 内置默认文件；单元测试 | 是 | 接口定义、评审 |
| B | eyes 拆分：主题配色/区域读界面地图；画面识别骨架 | 是（A 接口合并后并行） | 真机核对 |
| C | 启动核对 + `needs_recalibration`；`control calibrate`（只看 + 文件传输助手样本） | 部分（只看部分） | 真机标定与确认流程 |
| D | skills 统一写法；save_as/open_chat/scroll 迁移；`dismiss_to_main` | 是（逐个流程） | 真机验收 |
| E | 消息类型：link_card、sticker（表情包记忆）、emoji（表情词典比对）、video/voice 识别、quote、system、红包转账禁止 | 是（类型之间独立） | 表情词典释义初稿、样本采集、真机验收 |
| F | Rust 宿主改用 worker 操作 | 是 | 回归与切换 |
| G | 语音转文字、来电第一期 | 后期 | — |

每阶段：我写计划与任务说明 → agy 实现（各自 worktree、TDD）→ 我整合、在微信电脑验收 → 阶段末全分支审查。

## 11. 已定事项（2026-11-05）

- D1：确认类表情（OK、👍）暂按默认**一律简短回复**；"可不回复"保留设计、默认关闭，以后由用户决定是否开启。
- D2：阶段 E 顺序按默认：表情 > 链接卡片 > 引用 > 语音识别 > 视频 > 系统消息。
- D3：`layout_mismatch` 连续 5 次升级为 `needs_recalibration`。
- 表情词典与表情包记忆可随使用持续更新（用户确认）。

## 12. 风险

- 拆分 eyes 时识别结果必须逐帧一致：用现有帧夹具（`tests/fixtures/frames`）做"拆分前后输出完全相同"的回归测试。
- 界面地图核对过严会误停：anchors 先少量（3 个）且容差可配，真机统计误报率后再加。
- 表情模板比对在不同缩放下的稳定性需真机验证（1x/2x 各采一套）。
- 表情释义有地区与人群差异（如 [微笑]）：词典只给候选含义与建议，由模型结合上下文判断；用户审改词典为准。
- 表情包记忆若被模型理解错，会被一致地错用：提供查看与修正入口，并在回复异常时可追溯到所用记录。
- 多个 agy 并行改 Swift 同一文件易冲突：按 §10 的文件边界分配，A 的接口先合并再并行 B–F。
