# 对话历史存储与查询设计

日期：2026-11-05
状态：待用户确认

## 1. 目的

白名单客服 Bot 目前每个联系人只保存“当前一屏”游标（`cursor.json`）和发送流水（`send-*.json`），模型上下文是当前画面最后 12 个气泡。客户说“还是上次那个款”“什么时候发的货”时，Bot 和后端 Agent 都没有依据。

订单和发货信息目前只出现在聊天里（以后可能接 ERP）。因此工具端要**永久保存每个人的原始对话**，并给后端 Agent 开发者提供完整的历史查询能力，让后端能像真人一样“记得”每个客户。

## 2. 目标

1. 按联系人追加保存所有观察到的消息，原始记录只增不改，永久保留。
2. 每条消息标明方向与来源：客户发来 / Bot 自动回复 / 我方其他发出（人工或无法区分）。
3. Bot 回复关联到发送流水（幂等键、发送状态、回显是否确认）。
4. Bot 调用模型时默认带该联系人最近 5 条消息，可配置。
5. 提供只读的历史查询，**MCP 工具和 HTTP 接口两种形式**，返回同一种数据结构。
6. 联系人用稳定内部 ID；备注名改名时可手动合并；名称中的表情符号、特殊符号可正确保存和匹配。

## 3. 非目标

- 不从对话中提炼“买过什么、偏好、发货单号”等记忆或结构化事实——这是后端 Agent 的职责。
- 不接入 ERP、订单系统。
- 不提供删除、修改消息的功能（暂不需要；见 §11）。
- 不处理群聊（Bot 本身不处理群聊）。
- 不承诺完整历史：只能保存 Bot 实际看到的可见消息（见 §13 限制）。
- 不修改远端微信电脑上的 Rust MCP 及其现有 SQLite 客户档案。

## 4. 反目标（即使能跑也算失败）

- 消息被记到错误的人名下。
- 同一条消息被重复记录，或 Bot 回复与其回显被记成两条。
- 查询接口能写入、修改或删除数据。
- HTTP 接口默认对局域网开放或无鉴权。
- 存储失败导致 Bot 发出回复后没有记录（必须先落库再进入模型/发送流程，与现有“先写流水”原则一致）。
- 存储增长导致 Bot 轮询明显变慢。

## 5. 约束

- 运行环境：控制端 macOS，Python 3.12，**仅标准库**（现有 Bot 无第三方依赖）。SQLite 3.54，FTS5 trigram 可用。
- 唯一写入方是 Bot 协调器进程（`scripts/wechat_bot/coordinator.py`，串行处理各会话）。
- 现有状态目录：配置项 `state_dir`（当前为 `~/Library/Application Support/WeChatRPABot/state/`），目录权限 0700、文件 0600（沿用 `private_directory` / `write_private`）。
- 联系人身份只有微信界面显示的**备注名**（OCR 识别），没有微信 ID。
- 微信界面上的消息时间当前识别不到，只有“首次观察到的时间”。

## 6. 已确定的决策

| 决策 | 来源 |
|---|---|
| 工具端只保存原始数据，不做记忆提炼 | 用户 |
| 模型上下文默认最近 5 条，可配置 | 用户 |
| 提供完整历史查询，MCP 工具和 HTTP 接口都要 | 用户 |
| 联系人按备注名识别，改名时手动合并 | 用户 |
| 名称含表情、特殊符号，按标准化键 + 识别别名处理 | 用户确认 |
| 永久保留，暂不做删除 | 用户 |
| 附件全部存档：所有看到的图片缩略图和**所有文件原件**都保存 | 用户 |
| 使用 SQLite（WAL），存储层接口独立以便将来换 PostgreSQL | 设计者决定 |
| 存储与接口放在控制端，与 Bot 同一 Python 包 | 设计者决定 |
| 附件存磁盘文件，按 SHA-256 内容寻址，数据库只存引用 | 设计者决定 |
| 查询接口全部只读；合并联系人只通过 `control` 命令行 | 设计者决定 |
| 会话状态目录改按 `contact_id` 命名，并迁移现有目录 | 设计者决定（§8.1） |
| 远端 worker 只做“名称属于候选列表”的精确比对，`name_key` 只在 Python 实现 | 设计者决定（§8.2） |
| 去重改用数据库内的“已记录画面”水位，与消息同一事务 | 设计者决定（§9.1） |
| Bot 回复在 `attempting` 时即写入，状态随流水更新；启动时按流水回补 | 设计者决定（§9.2） |
| 迁移顺序：停服务 → 导入旧流水 → 写当前画面基线 → 启动 | 设计者决定（§9.3） |

## 7. 数据结构

数据库文件：`<state_dir>/history/history.sqlite3`；附件目录：`<state_dir>/history/media/<sha256前2位>/<sha256>.<ext>`。打开时 `PRAGMA journal_mode=WAL; foreign_keys=ON; busy_timeout=5000`。时间统一用 UTC Unix 毫秒整数。

### 7.1 `contacts` —— 稳定联系人

| 列 | 类型 | 说明 |
|---|---|---|
| contact_id | TEXT PK | 生成的稳定 ID，如 `c_` + 16 位随机十六进制 |
| account_id | TEXT NOT NULL | 配置中的账号绑定，如 `test-desktop` |
| kind | TEXT NOT NULL | 当前仅 `private` |
| display_name | TEXT NOT NULL | 当前生效的备注名（原样） |
| merged_into | TEXT NULL | 被合并时指向目标 contact_id；查询自动跟随 |
| created_at / updated_at | INTEGER | 毫秒 |

### 7.2 `contact_names` —— 名称与别名

| 列 | 类型 | 说明 |
|---|---|---|
| id | INTEGER PK | |
| contact_id | TEXT FK | |
| account_id | TEXT | |
| raw_name | TEXT | 原样名称（配置名或 OCR 识别名） |
| name_key | TEXT | 标准化匹配键（§8） |
| source | TEXT | `config`（白名单名称）/ `alias`（配置的识别别名）/ `observed`（OCR 实际识别到） |
| first_seen_at / last_seen_at | INTEGER | |

唯一约束：`(account_id, name_key)` 只能指向一个未合并的联系人。冲突时拒绝自动处理（见 §8）。

### 7.3 `messages` —— 原始消息（只追加）

| 列 | 类型 | 说明 |
|---|---|---|
| message_id | TEXT PK | `m_` + 随机 ID |
| contact_id | TEXT FK | 记录时归属的联系人（合并不改此列，查询时跟随 `merged_into`） |
| account_id | TEXT | |
| seq | INTEGER | 该联系人内单调递增序号（按写入顺序），分页用 |
| direction | TEXT | `inbound` / `outbound` |
| sender | TEXT | `customer` / `bot` / `self_other`（我方人工或无法区分） |
| kind | TEXT | `text` / `image` / `file` / `link` / `other` |
| text | TEXT | OCR 文字（原样） |
| observed_at | INTEGER | 首次观察到的时间；Bot 回复为触发发送的时间 |
| capture | TEXT | `live`（正常增量）/ `baseline`（首次启动时的可见历史）/ `rebaseline_gap` / `ambiguous`（重复锚点无法确定条数）/ `imported`（从旧流水导入） |
| bubble_digest | TEXT | 现有气泡指纹 `digest`，用于去重与回显匹配 |
| send_key | TEXT NULL | Bot 回复的幂等键（`bot-service-...`），关联 §7.4 |
| media_id | TEXT NULL | 附件引用（§7.5） |
| batch_key | TEXT NULL | 入站消息所属的处理批次幂等键 |
| meta | TEXT | JSON：OCR 坐标、文件名、是否截断等次要字段 |

索引：`(contact_id, seq)` 唯一；`(account_id, observed_at)`；`send_key`。

### 7.4 `sends` —— Bot 发送记录

| 列 | 说明 |
|---|---|
| send_key PK | 与 `send-*.json` 的 `idempotency_key` 相同 |
| contact_id | |
| status | `generated` / `attempting` / `triggered` / `unknown` / `superseded` / `skipped_by_rule` 等（沿用流水状态） |
| reply_text | 模型回复原文 |
| model | JSON：后端、模型名、耗时、token |
| echo_observed | 0/1：界面上是否看到回显 |
| input_message_ids | JSON 数组：本批次入站消息 ID |
| created_at / updated_at | |

`send-*.json` 流水继续作为发送幂等的权威来源；`sends` 是供查询的镜像，随流水状态变化更新。

### 7.5 `media` —— 附件

| 列 | 说明 |
|---|---|
| media_id PK | 内容 SHA-256 |
| kind | `image` / `file` |
| filename | 原文件名（若有） |
| extension / size | |
| source | `chat_thumbnail` / `original` |
| path | 相对 `history/media/` 的路径 |

### 7.5.1 附件存档流程

- **图片缩略图**：从当前截图裁出（worker 现有能力，无额外界面操作），随消息同一轮写入。
- **文件原件**：全部下载存档，包括不触发回复的消息、基线和重建基线中可见的文件。下载需要远端界面操作（`prepare_file` 填写保存对话框，每个数秒），因此：
  - 新表 `media_jobs(message_id PK, contact_id, status, attempts, last_error, updated_at)`，状态 `pending` / `done` / `failed` / `gone`。
  - 回复优先：协调器只在**所有会话都没有待处理的入站批次**时，才在当前打开的聊天里处理下载任务，每轮最多 1 个，避免拖慢回复（反目标“轮询明显变慢”）。Bot 回复本身需要的文件仍按现有流程立即下载，并标记该任务 `done`。
  - 只能下载当前画面中仍可见的文件（`prepare_file` 要求画面快照一致）。文件已滚出可见区域时标 `gone`，消息仍保留文件名等元数据。
  - 失败重试最多 3 次，间隔递增；worker 现有的“同一进程内同一文件只尝试一次”保护保持不变（结果未知时不重复点保存），跨进程重启后才重试。
  - 下载的文件按内容 SHA-256 存入 `history/media/`，单个上限沿用 10MB；超限标 `failed` 并记录原因。

### 7.6 `messages_fts` —— 全文检索

FTS5 虚拟表，`tokenize='trigram'`，内容为 `messages.text`，以 `messages` 的 rowid 关联（external content 表 + 触发器同步）。搜索词少于 3 个字时退回 `text LIKE '%词%'`（限定联系人或时间范围时代价可接受）。

### 7.7 `merges` —— 合并日志

`id, from_contact, into_contact, operator_note, created_at, undone_at`。合并只设置 `contacts.merged_into`，不改动消息；撤销清空 `merged_into` 并写 `undone_at`。

### 7.8 `schema_migrations`

版本号表，启动时按序执行迁移脚本。

## 8. 名称标准化与匹配

`name_key(raw)`：
1. Unicode NFKC（全角/半角统一）；
2. 去掉首尾空白、所有空白字符、零宽字符（U+200B–U+200F、U+2060、U+FEFF）和变体选择符（U+FE00–U+FE0F）；
3. 去掉 Unicode 类别为 `So`（符号，含绝大多数表情）、`Sk`、`Cf` 的字符；
4. 若结果为空，退回步骤 2 的结果（名称全是符号时仍可用）。

白名单配置扩展（向后兼容）：
```json
{"recipient": "🍓小美丨鞋", "kind": "private", "aliases": ["小美丨鞋", "@小美鞋"]}
```
- 联系人匹配：OCR 标题或会话行名称的 `name_key` 等于该联系人任一 `config`/`alias` 名称的 `name_key`。
- 每次实际识别到的名称以 `observed` 记录，`control status` 和日志显示，方便补充别名。
- 冲突：两个白名单联系人标准化后 `name_key` 相同，配置校验失败并报错；运行中识别名同时匹配多个联系人时，该会话暂停并提示，不写入消息。

### 8.1 联系人与会话状态

- 会话状态目录从“target 字典哈希”改为 `state_dir/sessions/<contact_id>/`。首次升级时按现有 `binding.json` 的 recipient 找到/创建联系人，把旧目录原样重命名过去（保留游标、流水、文件上下文），旧路径写一个指向新目录的说明文件。
- `routing.decision` 与 coordinator 改为按联系人解析：白名单条目 →（`recipient` + `aliases` 的 `name_key`）→ contact_id。
- 改名流程（推荐）：客户在微信改备注后，把新名写进 `recipient`、旧名留在 `aliases`。两者解析到同一 contact_id，游标与历史不中断，无需合并。
- `merge-contact` 用于历史已经分裂的情况（例如先建了新条目才发现是同一人）：合并后被并入的联系人不再单独轮询；其会话目录保留备查，coordinator 使用目标联系人的目录。被并入的名称自动加为目标联系人的 `alias` 名称记录（不改配置文件，`history-status` 提示把它写进配置）。

### 8.2 远端 worker 的名称比对

worker 不实现 `name_key`。Python 把该联系人全部候选原始名称（`recipient` + `aliases` + 已确认的 `observed` 名称）作为列表传给 worker；worker 在去掉首尾空白和零宽字符后，判断 OCR 标题/会话行名称是否**精确等于列表中某一项**（`capture`、`read_media`、`prepare_file`、`sessions`、`open_session` 均同）。返回实际识别到的原始名称，由 Python 用 `name_key` 记录 `observed` 并在未匹配时提示补别名。这样发送前核验仍是精确比对，两端不会因 Unicode 版本不同而规则不一致。需要重新部署远端 worker。

注意：Bot 现有的“标题必须与白名单名称完全一致”（原生 worker `allowlisted_title_mismatch`、`Policy.check_frame`、会话列表探测按名称精确筛选）要同步改为按 `name_key` 或别名匹配。这一步影响发送前的身份核验，需要单独测试。

## 9. 写入流程（Bot 侧）

在 `BotService.cycle` 中接入，原则是**先落库再进入模型/发送**：

1. 首次建立基线：当前可见气泡以 `capture=baseline` 写入（按方向：入站 `customer`，出站 `self_other`），不回复。
2. 正常增量：`new_bubbles` 得到的新气泡逐条写入。入站 `sender=customer`。出站气泡先尝试与该联系人最近未匹配回显的 Bot 回复（`sends.reply_text`，用现有 `echo_matches` 比较）配对：配对成功则不新增消息，只把对应 Bot 消息的 `bubble_digest` 补上、`echo_observed=1`；配对失败则写为 `sender=self_other`。
3. 入站批次写入后、调用模型前，写入/更新 `sends`（`generated` 前的状态）。
4. 触发发送后，以 `sender=bot`、`send_key` 写入 Bot 回复消息，`observed_at` 为触发时间。
5. 重复锚点歧义（`ambiguous_bubbles_skipped`）：被跳过的气泡**也要保存**，`capture=ambiguous`，`meta.note` 说明条数不确定。
6. 连续性缺口重建基线：当前可见气泡以 `capture=rebaseline_gap` 写入，`meta` 记录“可能与已有记录重复”。
7. 写库失败：该会话本轮停止，不进入模型/发送，健康状态报 `history_write_failed`，下一轮重试（游标未推进，消息不会丢）。

### 9.1 去重：已记录画面水位

不用 digest 去重（客户连发两条相同的“好”必须记两条）。改为每个联系人在数据库里保存一个 `recorded_frame`（已写入的最后可见气泡指纹序列，即 `cursor.json` 的 bubbles 指纹），新表 `contact_cursor(contact_id PK, frame_fingerprints JSON, updated_at)`。

每次写入：在**同一个事务**里用 `new_bubbles(recorded_frame, current)` 求出新气泡、追加消息、更新 `recorded_frame`。崩溃后重放时，水位已经包含已写的气泡，不会重复。`cursor.json` 仍控制 Bot 的回复流程；两者分开推进，但写库只看数据库内的水位。歧义与缺口按 §9 第 5、6 条以 `ambiguous` / `rebaseline_gap` 写入并把水位设为当前画面。

### 9.2 Bot 回复的记录时机与补偿

- 状态变为 `attempting`（即将调用发送）前，写入 `sender=bot` 消息，`send_key` 关联，`sends.status` 同步；之后每次流水状态变化（`triggered` / `unknown` / `superseded` / `stopped_before_send`）更新 `sends`。查询时 `send.status` 让后端区分“已触发”与“结果未知”；`superseded`、`stopped_before_send`、`skipped_by_rule` 的 Bot 消息在 `get_recent_messages`/模型上下文中排除，在完整历史中保留。
- 回显配对（§9 第 2 条）只与 `triggered` / `unknown` 状态的 Bot 消息配对。
- 补偿：协调器启动时扫描各会话的 `send-*.json`，凡数据库 `sends` 缺失或状态落后的，按流水回补（流水仍是权威来源）。这覆盖“触发后写库失败”的情况。

### 9.3 迁移步骤（`control migrate-history`，服务停止时执行）

1. 创建数据库；按现有会话 `binding.json` 建联系人并迁移目录（§8.1）。
2. 按 `created_at` 顺序导入所有 `send-*.json`（`capture=imported`），保证旧消息 seq 在前。
3. 把各会话当前 `cursor.json` 的可见气泡以 `capture=baseline` 写入，并设置水位。与第 2 步可能重复的部分不做跨来源去重，在 `meta` 标注 `may_duplicate_imported`。
4. 之后启动服务，增量写入。服务启动时若发现未迁移的会话目录，拒绝启动并提示运行迁移命令。

### 模型上下文

新配置项 `context_messages`（默认 5，范围 0..100）。调用模型时上下文改为：该联系人（含已合并联系人）最近 `context_messages` 条消息（不含本批次入站），按 seq 升序，`customer` → `user`，`bot`/`self_other` → `assistant`；再加本批次入站。替换现有的“当前画面最后 12 个气泡”。

### 旧数据导入

见 §9.3。导入只覆盖 Bot 参与过的批次，按 `send_key` 幂等。

## 10. 查询接口

两种形式共用同一个只读查询层 `scripts/wechat_bot/history/query.py`，以只读方式（`mode=ro` URI）打开数据库。

### 10.1 统一返回结构

消息对象：
```json
{"message_id":"m_…","contact_id":"c_…","seq":128,"direction":"inbound","sender":"customer",
 "kind":"text","text":"还是上次那个款","observed_at":1793812345678,"capture":"live",
 "send":{"send_key":"bot-service-…","status":"triggered","echo_observed":true},
 "media":{"media_id":"…","kind":"image","filename":null}}
```
`send`、`media` 无关时为 `null`。分页返回 `{"items":[…],"next_cursor":"…"|null}`，游标是不透明字符串（编码 seq）。

### 10.2 操作

| 操作 | 参数 | 说明 |
|---|---|---|
| `list_contacts` | `query?`（名称模糊/标准化匹配）, `cursor?`, `limit?≤200` | 返回 contact_id、当前名称、全部别名、最后消息时间、消息数 |
| `get_contact` | `contact_id` 或 `name` | 名称按 `name_key` 解析；合并过的返回目标联系人 |
| `get_recent_messages` | `contact_id`, `limit?`（默认 `context_messages`，≤200） | 最近 N 条，升序 |
| `get_history` | `contact_id`, `since?`, `until?`（毫秒）, `cursor?`, `limit?≤500`, `order?=asc|desc` | 完整历史分页 |
| `search_messages` | `q`, `contact_id?`, `since?`, `until?`, `cursor?`, `limit?≤100` | 全文检索，返回命中消息及其前后各 2 条上下文 ID |
| `get_message` | `message_id` | 单条详情，含 `meta` |
| `get_media` | `media_id` | MCP：返回元数据和 base64（≤10MB）；HTTP：直接返回文件 |

### 10.3 MCP 工具

新增 stdio MCP 服务：`python3 -m scripts.wechat_bot.history.mcp_server`，标准库实现 JSON-RPC 2.0（`initialize`、`tools/list`、`tools/call`），工具名与上表一致，前缀 `history_`（如 `history_get_history`）。

### 10.4 HTTP 接口

`python3 -m scripts.wechat_bot.history.http_server`（或由 `control` 以 launchd 常驻，标签独立于 Bot）。标准库 `ThreadingHTTPServer`：
- 默认监听 `127.0.0.1:8765`；改监听地址需显式配置 `history_http.bind`。
- 鉴权：请求头 `Authorization: Bearer <token>`，令牌存 `<state_dir>/history/http-token`（0600，首次启动生成）。
- 只接受 GET：`/v1/contacts`、`/v1/contacts/{contact_id}`、`/v1/contacts/{contact_id}/recent`、`/v1/contacts/{contact_id}/messages`、`/v1/search`、`/v1/messages/{message_id}`、`/v1/media/{media_id}`。名称只作查询参数（URL 编码），路径中只出现 ID。
- 错误：JSON `{"error":{"code":"…","message":"…"}}`，401/404/400。

## 11. 管理命令（`scripts.wechat_bot.control`）

- `history-status`：库大小、消息总数、各联系人消息数、最近写入时间、`observed` 名称中未匹配别名的提示。
- `merge-contact --from <名称或ID> --into <名称或ID> [--note]` 与 `unmerge-contact --merge-id`。
- `migrate-history`（§9.3）。
- `history-backup --to <路径>`：SQLite 在线备份 API，不停服务。
- 永久保留：无删除命令。将来需要删除时另行设计（需处理 FTS 与附件）。

## 12. 规模与性能

估算上限：200 活跃客户 × 50 条/天 ≈ 1 万条/天、约 365 万条/年、约 2GB/年（不含附件）。按 `(contact_id, seq)` 索引的最近 N 条和分页查询与总量无关。Bot 每轮写入为少量行的单事务。将来出现多写入方或跨机器访问时，替换 `history/store.py` 的实现为 PostgreSQL，查询层和接口不变。

## 13. 已知限制

- 只能保存 Bot 实际看到的可见消息；Bot 停机期间对方连发且超过一屏的消息会丢失（重建基线后只存可见部分）。
- 消息时间为观察时间，不是微信发送时间。
- `self_other` 无法区分“人工客服回复”和“Bot 回显未配对”。
- 链接卡片、图片的 OCR 文字不完整；图片只有聊天缩略图，不是原图。
- 文件只能在仍可见时下载；Bot 停机或消息被刷出屏幕后无法补下载。
- 名称识别依赖 OCR，标准化不能保证所有表情/符号情况都能匹配，需靠别名补充。

## 14. 留给实现者的决定

- 模块拆分细节（建议 `scripts/wechat_bot/history/{store,query,names,mcp_server,http_server}.py`）。
- 迁移脚本组织方式、随机 ID 生成方式。
- HTTP 接口是否由 `control` 管理 launchd 常驻（建议是，独立标签）。
- 测试组织；需覆盖：去重、回显配对、名称标准化与冲突、合并跟随、只读打开、鉴权、分页游标、FTS 短词回退、写库失败不发送。
