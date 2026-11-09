# 个人微信白名单客服 Bot

当前控制端为本机Mac，微信运行在已授权的专用测试机。模型后端是真实Codex CLI非交互执行器，使用本机已有ChatGPT登录；不需要将模型密钥放到微信测试机。没有使用模拟模型或固定话术。微信操作继续经过原有MCP与发送流水。

## 当前部署

- launchd任务：`local.wechat-rpa.bot`，登录自启、退出后由launchd重启。
- 本机配置：`~/.WeChatRPABot/config.json`。
- 私有状态与发送日志：以配置的 `state_dir` 为准；新安装默认 `~/.WeChatRPABot/state/`。已有部署迁移配置时保留原状态目录，防止重复处理历史消息。运行日志以 launchd 配置为准，新安装默认 `~/.WeChatRPABot/logs/`。
- 白名单：支持多个精确名称的私聊。当前部署为 `午夜咖啡`、`文件传输助手`，账号绑定 `test-desktop`。群聊和好友申请不自动回复；`ignorelist` 可排除多个私聊/群聊。
- 模型只生成回复文字；无应用、命令、搜索和微信工具权限。收件人由程序固定，模型不能修改白名单。
- 首次运行建立基线，不补发历史。常驻模式无测试前缀限制；默认两次发送至少间隔15秒、每小时最多20次。

## 管理

在项目工作目录执行：

```bash
wechat-mcp control status
wechat-mcp control stop
wechat-mcp control start
```

`stop`禁用登录自启并卸载当前任务，避免下次登录又自动回复；`start`重新启用并加载。已经被UI接受的发送需完成清理，不会为了停止而换键补发。模型生成期间收到停止信号后，不进入发送。

初次部署使用`control install --host <SSH目标> --account <绑定> --recipient <明确允许的私聊> --allow-replies`。初次安装创建一个私聊，随后可在 `allowlist` 中增加多个已授权私聊。配置使用本机私有JSON，不修改全局Agent/MCP配置或远端GUI应用签名。修改配置后先停止、核对状态归属，再启动；不同账号使用独立状态目录；同账号各联系人由调度器自动隔离到 `state_dir/sessions/<hash>/`，不能互用游标。

## 模块

- `native/wechat_bubbles.swift`：在微信机器上处理当前私有截图，以气泡颜色、位置及OCR提取视觉候选；窗口元数据由已授权GUI助手保存，SSH只读取文件。
- `capture.py`：复用GUI宿主MCP和SSH，管理有归属标识的私有诊断采集，恢复自身遗留标记而不抢占其他诊断会话。
- `policy.py`：精确标题白名单、连续锚点、双帧一致性、重复次数和回复边界。
- `reply.py`：真实模型结构化输出，服务模式提供最近可见文字上下文；没有业务知识库时不编造订单、商品、退款等事实。
- `routing.py`：多私聊白名单、多群/私聊排除、排除优先和严格配置验证。
- `coordinator.py`：共享唯一 MCP 连接，串行切换会话、独立状态、旧流水迁移和单会话异常隔离。
- `service.py`：每会话首次基线、批次幂等、限流、模型/发送前规则复查、持久日志和异常暂停。
- `control.py`：当前用户launchd安装、状态、启动和停止。

## 状态含义

`watching`为采集运行；`generating`为模型生成；`sending`为已进入发送阶段；`rate_limited`保留输入等待限流解除；`reconnecting`尝试恢复连接；`paused`需要人工检查发送日志、权限、布局或模型问题。

模型或发送结果不明确时保留原日志和幂等键，暂停而不盲目重复请求。不要通过删除日志或更换状态目录强行重发。`triggered`、客户端绿色气泡、归一化OCR回显都不是送达或已读回执。

`waiting_for_turn`为已收到客户消息、等客户发完（安静期）。

## 图片、文件与回复轮次

设计：`docs/superpowers/specs/2026-11-05-media-turns-design.md`。

- **一轮一回**：客户最后一条消息后安静 `turn_quiet_seconds`（默认 10，0..120）秒，或一轮已满 `turn_max_seconds`（默认 60，10..600）秒，就把这一轮所有文字、图片、文件合成一批回复一条。轮次保存在会话目录的 `turn.json`，重启后继续计时。日志 `turn_opened` / `turn_closed`。
- **图片识别**：图片按尺寸 + 感知哈希（`phash`）识别，加载前后视为同一张；同尺寸的不同截图仍是不同消息。高图片把旧消息挤出屏幕时向上翻页找回位置（`continuity_gap_recovered`），确认翻回底部后再继续；找不回才重建基线。
- **取件**：右键菜单由屏幕文字识别定位"另存为"（微信 4 的菜单不在辅助功能树里），保存名唯一（`wxbot-xxxx`）并核对文件名框后回车，存好后移入 worker 私有目录，失败自动按 Esc 关闭弹窗。图片优先原图，失败退回聊天缩略图（`image_original_unavailable`，`detail` 为 worker 错误代码）；屏幕外的图片使用翻页截下的缩略图；文件卡片显示上传中时继续等待；文件保存原件并提取文字。每项挂到历史消息下。收到的附件与原图上限由 `max_attachment_mb`（默认 50，范围 1..200）控制，打不开的项（下载失败、超过上限、格式不支持、提取为空、内容太多）记 `media_failed`，回复照常，并在回复里点名请客户重发。
- **分批理解**：图片多于 `images_per_model_call`（默认 4，1..16）张时，先分批让模型逐张写要点（`model_batch`），要点存入历史库 `media.understanding`，最后一次调用只带要点生成一条回复。某批失败时那几张按打不开处理；最后生成失败仍暂停复核。
- **发送前确认**：历史库或屏幕显示这一轮之后我方已有回复（人工手动回复或已发出的 Bot 回复）时不发送，记 `already_replied`（`source`：`screen`/`history`/`journal`），人工回过的这一轮 Bot 不再补充。
- **生成期间的新消息**：这一轮的回复照常发出，新消息归入下一轮。发送期间到达的新入站候选不会随发送后快照一起被提前推进游标。重启发现未完成的模型/发送阶段会暂停等待复核。

## 实机适用边界

视觉配置已标定微信 4.1.15 的浅色（`wechat-4-light`）与深色（`wechat-4-dark`）主题，窗口 880×640 逻辑点，1 倍屏与 Retina（2 倍）均可：截图先缩放到 1 倍再解析，客户图片从原始分辨率裁剪存档。主题按帧识别，运行中切换无须重启（切换瞬间可能触发一次基线重建，期间到达的消息只记录不回复）。未知主题、缩放或尺寸拒绝自动发送；不是通用自动标定。macOS 14+ 用 ScreenCaptureKit 截取窗口（需"屏幕录制"权限），macOS 12–13 用 CGWindowList；worker 启动时预热 Vision 识别模型（macOS 26+ 冷启动约 30 秒）。

**点击方式（公共动作组件 `native/action/`）**：worker 与 `wechat_click` 的点击、右键、滚动统一走同一个组件：传入控件矩形，按下点落在其有效区域（四边内缩 20%，小控件内缩 2 点）内的随机位置；移动路径为微弯曲线、前快后慢、带衰减的轻微手抖，用时按费茨定律；落点不在有效区域就修正（最多 3 次），控件不在屏幕上则不按下（`target_not_reached`）。动作中检测到有人移动鼠标即停止（按下后仍先松开），报 `user_input_detected`；这两种情况都不暂停联系人，按 `waiting_for_wechat_window` 下轮重试。滚动的刻度仍是每页 1 个 10 行刻度（翻页拼接依赖 90pt/步），只有指针路径与刻度间隔拟人化。单次点击约 0.3–0.8 秒；排障时可在 worker 环境设 `WECHAT_ACTION_PROFILE=instant`（不走轨迹，仍判定区域）。

**专用挂机电脑**：Bot 要求一台只挂微信的专用 Mac。配置 `manage_window`（默认 `true`）允许 worker 自动恢复最小化的微信主窗口、把窗口调回 880×640，放不下时挪回所在屏幕内，并记录 `window_resized`；设为 `false` 时只检查、不动窗口。只操作辅助功能接口报告的主窗口；全屏、多个候选窗口（如图片查看器）时不动。微信窗口暂时不可用（未知主题、尺寸不符或调不回、被隐藏、截图失败、识别超时、桌面忙）时，该联系人进入 `waiting_for_wechat_window`，不读取、不发送，每轮自动重试，恢复后无须重启即继续。其他窗口（如 "WeChat RPA Test" 环境检查窗口）挡住微信时操作会失败，保持微信在最前面。它仍是可见界面启发式，无法证明平台消息ID或微信ID级联系人身份，不能保证隐藏/密集消息全量无损。窗口标题不清会暂停该联系人；采集阶段的连续性缺口（如聊天记录被清空）在两帧一致后以当前画面重建基线，记录 `continuity_gap_rebaseline`，缺口内消息不回复，联系人不暂停；底部气泡不完整时等待下一帧；完全相同的重复气泡导致的锚点歧义只跳过无法判定的那几条（不调用模型、不发送），不暂停联系人。

此版本是使用可见界面的多私聊受控客服实现；没有商品知识库、订单系统或退款工具，不宣称完整生产客服平台。控制端、SSH网络、远端登录桌面与GUI宿主均需保持可用。服务占用远端GUI宿主唯一MCP连接，不应另起客户端争用。

测试：`cargo test --locked` 运行 Rust 测试；测试使用假模型/假桌面，不会发送微信。真实模型和发送结果另以实机记录为准。


## 自定义模型（OpenAI 兼容接口）

不配置 `model_config` 时继续使用 `codex-exec`。要直接接入 DeepSeek 或自建兼容服务，在 `~/.WeChatRPABot/config.json` 的顶层增加 `model_config`（与 `host`、`allowlist` 同级）：

```json
"model_config": {
  "backend": "openai-compatible",
  "base_url": "https://api.deepseek.com",
  "model": "deepseek-flash",
  "api_key_env": "DEEPSEEK_API_KEY",
  "supports_images": true,
  "timeout_seconds": 60,
  "json_output": true
}
```

默认从环境变量读取密钥：DeepSeek 官方域名默认 `DEEPSEEK_API_KEY`，其他兼容接口默认 `OPENAI_API_KEY`；可用 `api_key_env` 指定其他变量名。显式配置 `api_key_file` 时优先读取该文件（必须属于当前运行用户、权限为 `600`，不能是符号链接）。变量缺失时报错，不会静默换来源。密钥不写入聊天、模型请求正文或发送流水。`base_url` 填 API 根地址（可包含 `/v1`），程序追加 `/chat/completions`；远程要求 HTTPS，本机服务允许 localhost HTTP。模型名由用户指定，不硬编码。

DeepSeek 的当前示例名称和图片格式参考[官方快速开始](https://api-docs.deepseek.com/quick_start/agent_integrations/)及[视觉文档](https://api-docs.deepseek.com/guides/vision/)。其他提供方需按实际能力设置 `supports_images`；关闭时遇到图片会暂停，不能假装读图。文档先提取正文/表格，再交给模型；扫描 PDF 页面通过图片能力处理。语音尚未接入。

macOS 常驻 Bot 由 launchd 启动，不会自动读取终端的 `.zshrc`。仅在终端 `export` 不会更新 launchd 环境；需将变量设置到当前用户 launchd 环境后重启服务。可在已安全设置变量的终端运行 `launchctl setenv DEEPSEEK_API_KEY "$DEEPSEEK_API_KEY"`。该设置不跨注销/重启持久化，登录后需重新注入。不要把密钥写进 plist。

切换前 `control stop`，更新私有配置及环境变量（或可选密钥文件），再 `control start`。模型错误、超时、截断输出或非预期工具调用均暂停，不自动付费重试，也不自动切换到另一提供方。真实模型返回的名称与请求名称分别记入流水，密钥和原始错误正文不记录。

`model_api.py` 为独立适配层，复用同一回复约束、白名单和发送校验。新增协议应放在模型适配层，不改变微信操作接口。

## Bot 状态浮层

原生 Bot 采集进程显示单个 `● 微信 Bot · 运行中` 标识，位于微信窗口左上方。没有鼠标光圈或点击动画，也不随每次采集隐藏/恢复；浮层不参与窗口采集且不截获鼠标。断开采集连接后退出；无采集心跳超过 240 秒则隐藏，避免长时间显示失效状态。此标识说明采集进程活跃，不是送达回执或模型成功保证。Bot 工作期间抑制旧动作浮层，非 Bot 手动操作仍保留原动作提示。


## 运行调试日志

服务向 stdout 输出逐行 JSON 日志，由 launchd 的 `StandardOutPath` 收集；stderr 保留启动失败等进程错误。现有部署的实时查看命令：

```bash
tail -F "$HOME/Library/Application Support/WeChatRPABot/logs/stdout.log"
```

日志包含启动后端/模型、状态变化、新消息候选数、模型完成耗时、批次耗时和回显、异常阶段、遗留未完成记录。状态不变时每 60 秒记录一次心跳，避免每秒采集刷屏。时间为带时区的 UTC ISO 8601。普通日志不输出聊天正文、回复正文或密钥；详细消息流水仍在私有 `state_dir`。`unfinished_journal` 表示保留现场等待复核，不会因为重启自动重试模型或微信发送。


视觉连续性选择最长的上下文重叠，并要求该上下文在当前帧只出现一次。短“你好”等重复内容不覆盖包含不同消息的长上下文；全相同或整段周期重复的多重匹配无法确定位置：取所有可能重叠中最靠后的位置，其后的气泡在任何解释下都是新消息，照常处理；歧义部分记录 `ambiguous_bubbles_skipped` 并跳过，若无确定新消息则在两帧一致后以当前画面为新基线。发送后使用同一规则处理滚动回显，新的入站候选保留给下一轮；已有 triggered 流水不重发。发送后检查失败以 `postflight_failed` 记录，不把它误报为模型失败。


## 多私聊白名单和排除列表

以下字段放在 `~/.WeChatRPABot/config.json` 顶层；名称必须完整精确匹配，JSON 不支持注释：

```json
{
  "allowlist": [
    {"recipient": "午夜咖啡", "kind": "private"},
    {"recipient": "文件传输助手", "kind": "private"}
  ],
  "ignorelist": [
    {"recipient": "不处理的好友", "kind": "private"},
    {"recipient": "不处理的群名称", "kind": "group"}
  ]
}
```

这只是规则字段片段，需保留原有 `host`、`account_id` 等字段。未列入白名单默认不处理；排除优先于允许。桌面标题不能证明微信 ID 或区分同名群/私聊，因此同名排除项无论 kind 都优先阻止该标题，避免误处理。群聊只能加入排除项，尚不能加入自动回复白名单。重复、空白或错误 kind 会拒绝整个配置，不静默忽略。

调度器按白名单顺序串行处理，共享单个桌面连接，每个会话独立保存游标、文件上下文、限流、幂等流水和暂停状态；某个会话暂停不阻止其他会话。首次加入只建立基线，不回复历史；移除/排除后再启用会重新建立基线，跳过禁用期间的历史。规则在轮询和模型调用/发送前重新读取，修改规则无须重启；账号、SSH、模型等运行设置修改后需重启。

`poll_seconds` 是每轮会话扫描之间的间隔，不代表每个联系人的固定实时延迟；窗口切换、模型和发送耗时会增加完整轮询周期。联系人较多时延迟会增长，不能保证隐藏聊天全量无损。原生 worker 在会话切换时保持连接与状态浮层，逐次核验完整标题。`max_attachment_mb` 配置接收附件大小上限（整数 MB，默认 50，范围 1..200）。

旧单会话部署首次升级时，将原绑定联系人的游标、历史发送流水和文件上下文复制到该联系人独立目录，保留原文件备查；未完成的模型/发送记录继续暂停，不会因迁移而重复发送。`control status` 汇总子会话状态，存在 sessions 后统计其发送记录，避免旧副本重复计数。


## 按会话列表变化切换聊天

微信桌面端一次只显示一个聊天。调度器每轮先用原生 worker 的只读 `sessions` 操作读取左侧会话列表一次（`native/bot/Sessions.swift`），只在需要时才打开对应聊天：

- 当前打开的聊天每轮照常检查，且最先检查。
- 白名单联系人那一行的预览摘要变化（只比较文字和数字，忽略随选中高亮变化的 OCR 标点），或头像区红色像素比该联系人历史最小值多出 ≥20（未读角标；头像自带的红色不算），才切换过去。
- 启动后每个联系人检查一次；列表中看不到的联系人，或超过 `idle_check_seconds`（默认 60，范围 10..3600）未检查的联系人，兜底检查一次。
- 会话列表读取失败时断开重连，本轮按旧方式检查全部联系人。
- 切换优先由 worker 的 `open_session` 直接点击会话列表中唯一匹配的行：要求辅助功能权限、该点最上层窗口属于微信，点击后 2.5 秒内标题须完全一致；任一条件不满足即回退到 MCP `get_messages` 导航。实测切换约 2.2 秒（原 8～10 秒），日志 `chat_switched` 记录是否点击及回退原因。
- 多个会话同时有变化时按检测顺序处理；同一次检测中列表位置越靠下（越早变化）越先处理；已检测未处理的会话保留到下一轮。当前打开的聊天始终最先检查。
- worker 只返回白名单行的名称、行位置、预览哈希和红色像素数，不返回其他联系人或预览原文。

聊天区裁剪延伸到输入框分隔线（y=496）。旧裁剪到 y=490 时，滚动到底的最后一个链接卡片会被误判为“底部气泡不完整”而一直等待。

实测未读角标约 140 个红色像素（头像自带红色计入基线），阈值 20。限制：连续发送完全相同的消息时预览不变，只能依靠角标或兜底检查发现。

## 对话历史

设计：`docs/superpowers/specs/2026-11-05-conversation-history-design.md`。

- **存储**：`<state_dir>/history/history.sqlite3`（SQLite WAL，0600）；附件 `<state_dir>/history/media/`，按内容 SHA-256 命名。永久保留，无删除功能。
- **记录内容**：Bot 看到的每条可见消息，按联系人追加。`sender` 为 `customer`（客户）/ `bot`（Bot 自动回复，关联发送状态与回显）/ `self_other`（我方人工或无法区分）；`capture` 标明来源：`live`、`baseline`（首次可见历史）、`rebaseline_gap`、`ambiguous`（重复消息条数不确定）、`imported`（旧发送流水）。图片在出现时先存聊天缩略图，回复时另存为的原图（`source=original`）和模型理解要点（`media.understanding`，迁移 3）也挂到同一条消息；文件在回复时另存为原件，没有回复的文件在空闲时排队下载，已滚出屏幕的标记为 `gone`。只读接口（HTTP、MCP）返回附件的 `source` 与 `understanding`，搜索也匹配理解要点。
- **模型上下文**：该联系人最近 `context_messages` 条（默认 5，0..100）+ 本批消息。
- **会话目录**：`state_dir/sessions/<contact_id>/`。从旧版本升级先 `control stop`，再运行 `wechat-mcp control migrate-history`；存在旧哈希目录时 Bot 拒绝启动。

### 联系人名称与改名

名称按标准化键匹配（全角半角统一，忽略表情、装饰符号和不可见字符）。OCR 识别不稳定时为白名单条目加识别别名：

```json
{"recipient": "🍓小美丨鞋", "kind": "private", "aliases": ["小美丨鞋"]}
```

客户改备注后：把新名写进 `recipient`，旧名放进 `aliases`，历史不中断。历史已经分成两个联系人时用：

```
wechat-mcp control merge-contact --from 旧名或c_ID --into 新名或c_ID --note 改名
wechat-mcp control unmerge-contact --merge-id N
```

名称最长 64 个字符。`control history-status` 显示每个联系人的消息数、未匹配的识别名称，以及合并后建议加进目标联系人 `aliases` 的名称（`alias_suggestions`，照做不会冲突）；`control history-backup --to <新文件>` 在线备份。

### 查询接口（只读）

**MCP**（stdio）：

```json
{"mcpServers": {"wechat": {"command": "/absolute/path/wechat-mcp"}}}
```

工具：`history_list_contacts`、`history_get_contact`、`history_get_recent_messages`、`history_get_history`、`history_search_messages`、`history_get_message`、`history_get_media`。

**HTTP**：`control history-http install|start|stop|status`（launchd 标签 `local.wechat-rpa.history`）。默认 `127.0.0.1:8765`，配置 `history_http` 可改；令牌在 `<state_dir>/history/http-token`。

```
TOKEN=$(cat "<state_dir>/history/http-token")
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8765/v1/contacts?name=小美
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:8765/v1/contacts/<contact_id>/messages?limit=100"
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:8765/v1/search?q=上次那个款"
```

路由：`/v1/contacts`、`/v1/contacts/{id}`、`/v1/contacts/{id}/recent`、`/v1/contacts/{id}/messages`（`since`/`until` 毫秒、`cursor`、`limit`≤500、`order`）、`/v1/search`（`q`，少于 3 个字用模糊匹配）、`/v1/messages/{id}`、`/v1/media/{id}`。分页返回 `next_cursor`（还有下一页时非空）；正序查询总会返回 `tail_cursor`，稍后用它查询即可只拿到之后写入的新消息。排序按写入顺序，不受系统时钟回拨影响。意外错误返回 JSON `500 internal`，不暴露内部细节。

限制：只保存 Bot 实际看到的消息；时间为观察时间而非微信发送时间；`self_other` 不区分人工回复和未配对的 Bot 回显；名称依赖 OCR，需要时补别名。
