# 外部大脑对外开放接口 —— 设计

日期：2026-11-09　分支：feat/wechat-bot-history　基线：`42ef72a`　状态：已确认（2026-11-09 用户确认"只对大脑接口开放内网 HTTP，运维仍走 SSH"；第一期 A–F，Webhook 最后）。实施计划：`docs/superpowers/plans/2026-11-09-external-brain-open-api.md`
关联：修订 `2026-11-07-rust-core-pluggable-brain-design.md` 中的非目标"在局域网上开放 TCP 端口"（见第 10 节）。

修订记录：
- v1：初稿。
- v2：按评审补充媒体权限、认领与终态、会话生命周期、重启恢复、独立的 v1 数据契约、Webhook 投递规则和错误分层。所有被引用的现状均已对照代码核实（见第 2 节）。

## 1. 为什么做

- 产品定位：本机负责"眼、手和安全规则"，**回复内容由外部开发的客服大脑决定**。外部大脑接口是产品对外的核心接口。内置大脑（`brain.mode=builtin`）只是测试回复，不用于生产。
- 现状：外部大脑和运维共用 `~/.wechat-rpa/worker.sock` 上的同一个 MCP 连接，大脑也能调用 `send_message`、旧工具和通用桌面工具，可以给任何人发消息，违反 2026-11-07 设计中的 G4 和反目标。唯一的接入方式是 SSH 加 `nc -U`，外部开发者必须拿到机器的 SSH 账号。

## 2. 现状核实（2026-11-09，对照代码）

| # | 现状 | 位置 | 后果 |
|---|---|---|---|
| F1 | `get_media` 只按 `media_id` 查文件，不校验所属联系人；`media` 表没有归属字段，归属只能经 `messages.media_id` 反查 | `src/core/history/query.rs:730`、`store.rs:167` | 拿到任意媒体 ID 就能读文件，绕过联系人过滤 |
| F2 | `reply` 只有在"轮次已保存 token **且**请求带了 token"时才比较；轮次未认领或请求不带 token 都会直接通过 | `src/core/external_brain.rs:346` | 未认领的轮次可以被任何连接回复 |
| F3 | `reply` 接受后立即从 `pending` 删除该轮；`answered` 检查实际走不到；入口不检查 `deadline_at`（只有 `next_turn` 认领时检查） | `external_brain.rs:338-366` | 重复回复返回 `unknown_turn`，不是 `already_answered`；过期轮次仍可能被回复 |
| F4 | `reply` 返回的 `accepted` 只表示回复已交给调度器，不表示消息已发出 | `external_brain.rs:384` | 调用方不能把它当成"已发送" |
| F5 | 待回复轮次只保存在内存中；启动时把状态为 `preparing_media/generating/generated` 的发送日志改成 `not_sent`，这不等于执行了配置的 `skip/handoff` | `external_brain.rs:41`、`src/core/turns.rs:284` | 重启后进行中的轮次丢失，也无法报告哪些轮次已过期 |
| F6 | 内部 `Turn` 用 `text` 而不是消息数组；联系人只有 `name/kind`，没有 `contact_id`；`images` 和上下文媒体是本机路径 | `src/brain/mod.rs:13-80` | 远程大脑无法使用路径，也无法稳定地标识联系人 |
| F7 | 历史查询的允许名单按 `name_key` 在全部账号里匹配（没带 `account_id`），也不应用忽略名单 | `src/core/history/mcp_tools.rs:241` | 换号后可能带出其他账号的同名联系人；忽略名单中的会话可被查询 |
| F8 | `resume_session` 按名字删除 `handoff.json`，范围不限 | `external_brain.rs:390` | 大脑可以恢复任何会话的自动回复 |
| F9 | `HistoryQuery` 的允许名单只在 `list_contacts`、`get_contact` 生效；`get_history`、`get_recent_messages`、`search_messages`、`get_message`、`get_media` 都不过滤 | `src/core/history/query.rs:330,416` | 现有历史工具实际可读全部聊天，范围比 F1 更大 |
| F10 | `turn_id` 就是调度器的批次键，发送日志为 `send-{turn_id}.json`；重启后调度器重新产生的同一批消息有相同的 `turn_id` | `src/core/turns.rs:1202,1425` | 重启恢复可以依靠持久终态记录，见 6.4 |

## 3. 目标 / 非目标 / 反目标

**目标**
- G1 对外接口只包含大脑能力；发给谁、能不能发、能读什么，一律由服务端判断。
- G2 外部开发者不需要 SSH 账号：通过内网 Streamable HTTP（MCP 2025-06-18 传输）接入，带访问令牌。不提供原始 TCP 上的 JSON-RPC。
- G3 接口有独立于内部结构的 v1 契约：数据格式、状态机、错误分层、重试语义写进对外文档，并附示例大脑。
- G4 对现有功能零影响：运维入口（`worker.sock`）的工具列表、参数和行为保持不变，并用兼容测试证明（见 9.3）。

**非目标（本期）**
- 多个大脑同时接入、按开发者分配权限、公网或云端部署、HTTPS 证书管理。
- 回复附带文件。
- 通用桌面工具。

**反目标**
- 对外接口能调用可以指定收件人的工具、桌面工具或客户管理写操作。
- 对外接口能读到当前账号以外、允许名单以外、忽略名单之内的联系人、消息或媒体，包括通过媒体 ID 读取。
- 同一轮次被发送两次，或者"已接受"被当作"已发送"报告给大脑。
- 修复对外入口时改变了运维入口的行为。

## 4. 架构

```
外部大脑（内网） ──HTTP+令牌──▶ serve-core
                                 ├─ OpenApi 入口（新）：认证 → 会话 → 工具白名单 → BrainAccess 权限上下文
                                 ├─ ExternalBrain（扩展：轮次状态机 + 持久终态）
                                 ├─ HistoryQuery（新增"受限查询"包装，不改原函数）
                                 └─ 调度 / 发送前检查 / Swift worker（不变）
运维：SSH → nc -U worker.sock → 完整工具（不变）
```

- **对外入口放在 serve-core 里。** 大脑工具不碰桌面，不需要桌面授权；serve-core 常驻运行，持有轮次和历史库。
- **权限上下文 `BrainAccess` 只给对外入口使用。** 它由服务端在每个请求开始时根据当前登录账号、允许名单、忽略名单构造。受限查询和受限回复都只接受 `BrainAccess`。运维入口仍调用原有函数，所以第 2 节的修复不会改变运维行为。

## 5. 权限规则（BrainAccess）

**一个判定函数，所有读写都经过它**：`permit(contact_id) = 属于当前登录账号 ∧ 在允许名单内（按 contact_id）∧ 不在忽略名单内（忽略优先）`。

| 对象 | 规则 |
|---|---|
| 联系人 | `permit(contact_id)` |
| 消息 | `permit(message.contact_id)` 且 `message.account_id = 当前账号` |
| 媒体 | 至少存在一条引用该 `media_id` 的消息，且该消息通过消息规则；**没有任何消息引用的媒体一律拒绝**（修 F1） |
| 搜索、列表 | 先按 `permit` 过滤再分页，不能先分页再过滤 |
| `resume_session` | 参数改为 `contact_id`，并要求 `permit(contact_id)`（修 F8） |

- 允许名单原来按名字配置，改为启动时和配置变化时把名字解析成"当前账号下的 `contact_id` 集合"，解析时带上 `account_id`（修 F7）。名字解析不唯一时，按"不允许"处理，并在面板上提示。
- 被拒绝时返回 `not_found`，而不是 `not_allowed`，避免大脑借此探测某个 ID 是否存在。

## 6. 轮次状态机与终态（修 F2、F3、F4、F5）

### 6.1 状态
```
queued ──next_turn──▶ claimed ──reply 校验通过──▶ accepted ──调度器执行──▶ sent | not_sent | unknown
   │                     │                                                  （来自现有发送记录）
   └──────超过 deadline──┴──▶ expired ──按 timeout_action──▶ skipped | handed_off | builtin
claimed ──认领租期到期且未过 deadline──▶ queued（可被重新领取，旧 claim_token 失效）
```

### 6.2 `reply` 的校验（在同一把锁内一次完成）
1. 轮次存在于活动表中，状态是 `claimed`；
2. `claim_token` **必填**，并且和当前认领一致；
3. 现在时间 < `deadline_at`；
4. 当前登录账号仍然等于轮次的账号；
5. 该联系人仍然满足 `permit`。换号、移出允许名单、加入忽略名单，都会让已有认领在下一次校验时失效，并返回 `turn_revoked`；
6. `send` 时文本合法。

全部通过后转为 `accepted`，写入终态记录，然后交给调度器。任何一步失败都不改变状态（文本不合法时轮次保持 `claimed`）。

### 6.3 终态记录与幂等
- 每个轮次结束时（`accepted`、`expired`、`revoked`）写一条**持久终态记录**：`turn_id`、状态、动作、回复文本及其 SHA-256、时间。保存文本是为了重启后能把同一个回复交还调度器（见 6.4）；文件权限 0600，和发送日志同级保护。按条数（默认 1000 条）和时间（默认 7 天）设上限，保存在 state 目录下，权限 0600。
- 重复调用 `reply`：
  - 同一 `turn_id`、同一动作、同一文本哈希：返回**和第一次相同的结果**，标记 `duplicate: true`。用于"响应丢了、大脑重试"的情况，不会再次发送；
  - 动作或文本不同：返回 `already_answered`，并附上已接受的动作；
  - 轮次已过期：返回 `turn_expired`。
- 新增只读工具 `get_turn_status(turn_id)`：返回状态，以及 `accepted` 之后的发送结果 `send.status`，取自现有发送日志 `send-{turn_id}.json`：`triggered`（已按回车，送达未知）、`not_sent`、`pending`（准备中）、`unknown`（没有日志或无法判断）。不使用"sent"这个词，因为现有发送链只能确认"已触发"。"已接受"和"已发送"在接口上分开表达（修 F4）。

### 6.4 重启恢复（修 F5）
**采用的方案（计划阶段确定）：只持久化终态记录，不持久化活动轮次。**
- 活动轮次背后是调度器在等待的内存通道，无法跨重启保存；而 `turn_id` 是确定性的批次键（F10），调度器重启后会对同一批消息重新调用大脑。
- 启动时加载终态记录。调度器再次交来某个 `turn_id` 时：
  - 已有 `accepted` 记录：直接把记录中的动作和文本交还调度器，不再交给外部大脑。调度器自己的发送日志负责避免重复按回车；
  - 已有 `expired` 或 `revoked` 记录：按记录的结果直接结束；
  - 没有记录：正常入队。
- 重启前已被认领、但还没回复的轮次，旧 `claim_token` 自然作废；大脑用旧 token 回复时会得到 `turn_not_found` 或 `invalid_claim_token`，需要重新 `next_turn`。
- 和现有启动逻辑（`turns.rs:284` 把发送日志改成 `not_sent`）的关系：那段只处理"调度器自己"的发送日志，不能代替上面的轮次恢复。实施时要确认两者不会冲突，比如一个已经 `accepted` 的轮次，它的发送日志被改成了 `not_sent`，那么终态应报告 `not_sent`，而不是重发。
- 必须补重启测试：分别在 `queued`、`claimed`、`accepted` 未发送、已过期四种状态下重启。

## 7. 会话生命周期（单个大脑）

- 遵循 MCP Streamable HTTP：`initialize` 时分配 `Mcp-Session-Id`；同一会话允许并行的 HTTP 请求（长轮询 `next_turn` 的同时可以调用 `reply`）。
- 会话租期：任何请求都会续期，默认 90 秒没有请求就判定会话失效。客户端可以用 `DELETE` 显式结束会话。
- 只允许一个**活动**会话。已有活动会话时，新的 `initialize` 返回 `409 brain_already_connected`；旧会话失效或被显式结束后，就可以重新建立。大脑崩溃后，最多等一个租期就能重连。
- 认领租期独立于会话：默认 60 秒，每次 `next_turn` 或 `reply` 都会续期。会话失效时，它持有的认领全部回到 `queued`（修"认领响应丢失要等到超时"的问题）。
- 令牌轮换：面板上重新生成令牌后，旧令牌立即失效，现有会话被终止，认领回到 `queued`。

## 8. 对外数据契约 v1（独立于内部 `Turn`，修 F6）

```json
{
  "api_version": "1",
  "turn_id": "t_…",
  "account": {"account_id": "…"},
  "contact": {"contact_id": "c_…", "display_name": "…", "kind": "private|group"},
  "messages": [
    {"message_id": "m_…", "kind": "text|image|file", "text": "…", "media_id": "md_…", "filename": "…",
     "observed_at": "2026-11-09T08:00:00Z", "source": "desktop_ocr"}
  ],
  "context": [{"role": "customer|assistant", "message_id": "m_…", "kind": "text|image|file", "text": "…", "media_id": "md_…"}],
  "deadline_at": "2026-11-09T08:02:00Z",
  "claim": {"claim_token": "…", "lease_expires_at": "2026-11-09T08:01:00Z"}
}
```
- 时间一律用 RFC 3339 UTC。
- `message_id` 可以为 `null`：内部 `Turn` 只有合并后的 `text` 和图片路径，不是每一项都能对应到历史库中的消息。能对应的给出 ID，对应不上的给 `null`，不编造。`media_id` 同理，对应不上的图片放进 `unavailable` 列表，并说明原因。
- 不出现本机路径。图片和文件只给 `media_id`，通过受权限保护的 `history_get_media` 读取（按大小上限截断或分块）。
- 由一个独立的转换函数从内部 `Turn` 生成，并配快照测试。内部结构变化时，快照测试会失败，从而提醒对外契约要不要升版本。
- 文档必须写明数据局限：内容来自屏幕 OCR，`message_id` 是本机分配的，不是平台 ID；收发方向和群成员可能不准；相同预览重复出现时可能不产生新轮次；`text` 是客户输入，属于不可信数据，客户可能借消息诱导大脑。

## 9. 接口细节

### 9.1 工具白名单
| 工具 | 说明 |
|---|---|
| `next_turn(wait_seconds)` | 长轮询认领，返回第 8 节的结构 |
| `reply(turn_id, claim_token, action, text?)` | 见 6.2、6.3 |
| `get_turn_status(turn_id)` | 新增，只读 |
| `resume_session(contact_id)` | 见第 5 节 |
| `history_*`（7 个） | 全部经过 `BrainAccess` |

其他任何工具名都返回 JSON-RPC 错误，并且不会转发到网关。

### 9.2 错误分层
| 层 | 形式 | 例子 |
|---|---|---|
| HTTP | 状态码，不带 MCP 正文 | `401` 无令牌或令牌错误、`403` Origin 不允许、`404` 会话不存在或已失效、`409` 已有活动会话 |
| JSON-RPC | 标准 `error.code` | `-32601` 未知方法或工具、`-32602` 参数无效 |
| 工具业务 | `isError: true`，`structuredContent: {error: {code, message}}` | `turn_not_found`、`turn_expired`、`turn_revoked`、`already_answered`、`invalid_claim_token`、`not_found`、`invalid_text`、`rate_limited`、`brain_mode_not_external` |

### 9.3 运维入口兼容（G4 的证明方式）
- 新增测试：快照对比 `worker.sock` 上 `tools/list` 的完整输出，以及 `reply`、`history_*` 在运维入口上的现有行为。改动前后必须一致。
- 第 5、6 节的修复都放在受限包装和 `BrainAccess` 路径里。原函数如果必须修改（例如修 F3 的"删除后无法识别重复"），运维入口的返回值要保持兼容，并在兼容测试中覆盖。

### 9.4 监听与认证
- 配置 `open_api`：`enabled`（默认 false）、`bind`（只能是回环地址或本机网卡的具体地址，拒绝 `0.0.0.0`）、`port`、`token_file`（权限 0600，由面板生成）。
- 令牌比较用常数时间；校验 `Origin`，拒绝浏览器跨站请求。

## 10. Webhook（最后一个阶段，可选）
- 与 MCP 拉取**互斥**：同一时间只启用一种投递方式，避免两边各生成一遍回复。
- 每次投递有唯一的 `delivery_id`。签名覆盖时间戳、`delivery_id`、请求体：`X-Signature: sha256=HMAC(key, ts + "." + delivery_id + "." + body)`。重放窗口 5 分钟，接收方按 `delivery_id` 去重。
- 失败（超时、5xx、连接失败）时，在 `deadline_at` 之前按退避重试，同一轮次的 `delivery_id` 不变；4xx 不重试。重试可能导致大脑重复生成内容，文档要求接收方按 `delivery_id` 幂等处理；服务端用 6.3 的终态记录保证最多发送一次。
- 只允许内网 `http://` 或 `https://`，不跟随重定向。

## 11. 助手面板
- MCP 页新增"对外接口"一栏：开关、监听地址和端口、令牌（复制或重新生成）、活动会话（是否有、最近一次请求时间）、近期轮次的状态统计、示例配置。
- 工具按"对外接口 / 运维"分组显示，通用桌面工具单列；修正 `control.rs` 中 `mcp-info` 把未知工具都归入"微信操作"的问题。
- "测试连接"用真实令牌完成 `initialize` 和 `tools/list`，结束时 `DELETE` 会话。已有活动会话时显示"已有大脑连接中"，不去挤占。
- 运维 SSH 配置里的用户名、主机和远端目录，从实际运行环境（当前用户、`HOME`、本机地址）生成，不写死。

## 12. 分阶段计划
| 阶段 | 内容 | 依赖 |
|---|---|---|
| A | `BrainAccess` 权限上下文和受限查询（第 5 节），包括媒体经由消息授权；运维兼容快照测试（9.3） | — |
| B | 轮次状态机、持久终态和活动轮次、重启恢复、`get_turn_status`（第 6 节） | A |
| C | 对外数据契约 v1 的转换函数和快照测试（第 8 节） | — |
| D | Streamable HTTP 入口：认证、会话租期、白名单、错误分层（第 7、9 节） | A、B、C |
| E | 对外文档 v1 和示例大脑 | D |
| F | 助手面板（第 11 节） | D |
| G | Webhook（第 10 节） | B、C |

每个阶段单独提交、单独测试；D 和 G 必须在专用机上用示例群做实机验收后，才能对外宣称可用。

## 13. 验收（除各阶段的单元测试外）
- 权限：用另一联系人的 `media_id`、忽略名单中的 `contact_id`、其他账号的 `message_id` 调用历史工具，全部返回 `not_found`；搜索结果中不出现被禁止的联系人。
- 认领：不带 `claim_token`、token 错误、未认领、过期、换号后、移出允许名单后调用 `reply`，分别返回对应的错误，且发送记录中没有新增条目。
- 幂等：同一回复连续调用两次，只发送一次，第二次返回 `duplicate: true`；内容不同的第二次返回 `already_answered`。
- 重启：6.4 列出的四种状态下重启 serve-core，结果符合规则，且没有重复发送。
- 会话：大脑进程被强杀后，一个租期内可以重新连接，被它持有的认领重新入队。
- 兼容：运维入口快照测试和现有全部测试通过。
- 实机：在示例群跑通 `next_turn` → `reply(send)` → `get_turn_status` 返回 `sent`。

## 14. 需要确认的决定
1. 修改 2026-11-07 设计中的非目标"在局域网上开放 TCP 端口"，改为"只对大脑接口开放内网 Streamable HTTP，运维仍走 SSH"。（推荐）
2. 部署假设：内网中的单个大脑。如果以后面向多个客户或第三方、部署在云上，需要另立"多令牌 + HTTPS"设计。
3. 第一期范围：A 到 F；Webhook（G）放最后。（推荐）
4. 终态记录的保留上限（默认 1000 条 / 7 天）、会话租期（90 秒）和认领租期（60 秒）是否合适。
