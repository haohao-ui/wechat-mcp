# 生产大脑配置与对话契约（D1）

日期 2026-11-10；基线 `32bb12e550b6739c5c7d6874bbd7388cd771ef5c`；本 worktree 初始 `git status --short` 为空。状态：**proposed；协调者同意作为设计交付提交，I1/I2 gate 保持；不是公共接口/DB实施批准或生产验收结论**。本交付只新增设计与合成 JSON；不改源码、真实 DB、微信、远端或模型连接。本地分支 `work/prod-brain-design`。

## 1. 源码事实与接入点（existing）

首次查询的 `wechat-mcp` 图过时：Turn 行号与源码不符、BuiltinBrain 无关系。随后以本 worktree 建立非持久 `wechat-prod-brain-design` fast 索引；Rust 相关源码均可索引，四个 Swift 文件有局部解析缺口，docs 被有意排除。图 trace `BotService.process_turn` 指向 `run_generation_and_send`、history context 等；图关系存在同名误连（如 Response.json），以下以本地源码逐处核对为准。

| 现有路径/符号 | 真实行为；设计接入 |
|---|---|
| `src/core/config.rs` BotConfig/BrainConfig | brain 缺省 builtin；external timeout/action/queue 与 model_config 独立；未知字段校验严格。保留 config schema 和所有旧字段，不向其写角色正文 |
| `src/core/turns.rs` process_turn → run_generation_and_send | 先规则/历史检查，取当前附件、context，再 understand_images，再构造 BrainTurn；有注入 BrainFn 则调用，否则 BuiltinBrain；新增快照固定点必须在 understand_images **之前**，不能到 build_prompt 才读 active |
| `src/brain/builtin.rs` build_prompt → generate | 混合角色、事实约束、unreadable hint；generate 经 model_api 选择 codex-exec/API；文件去 path/bubble；新实现只替换输入组装，不改连接实现 |
| `src/core/external_brain.rs` brain_fn → handle_turn | builtin 直连；external 放 pending/order，经 next_turn/reply 或 claim_next/reply_strict 回传 oneshot；timeout 按原 Skip/Builtin/Handoff；新增快照随轮次传递，超时 builtin 仍用原快照 |
| `src/core/open_api/http.rs` initialize / tools/list / tools/call | 当前 api_version=1，经 claim_next → contract::turn_v1，reply_strict 和 get_turn_status；不可给旧 next_turn 强加必填参数 |
| `src/core/brain_tools.rs` / external_brain socket | 旧 MCP → core.sock；send/skip/handoff，claim_token 可选；其旧 wire 输出保持 |
| `src/core/open_api/access.rs` BrainAccess | effective_account_id + contacts/contact_names/merge family；唯一 allow、全部 ignore 候选扣除；授权仍以它为准 |
| `src/core/open_api/history.rs` RestrictedHistory | 每次按账号/contact/message/media ownership 校验，路径不对外暴露；新配置引用不得越过此边界 |
| `src/core/turns.rs` ensure_unreadable_named | 回复漏附件 label 时追加“打不开，麻烦重发一下”，超过 200 字用计数摘要；这也是生产话术，不能漏迁移 |
| `src/core/turns.rs` run_generation_and_send 后半段 | skip 写 journal；handoff 写既有 handoff.json；send validate → unreadable 补充 → journal generated → stop/verify_before_send → history.record_bot_reply → try_send_once；triggered 标 delivery unknown。不修改幂等键/发送动作算法 |
| `src/core/history/store.rs` migrations 1..4 | contacts/messages/sends/media 等；sends.model 已保存 metadata、input_message_ids；skip/handoff 没有完整 sends 行，故仅往 model 塞配置不足以追溯每轮 |
| `src/core/history/query.rs`, `serve.rs`, `mcp_tools.rs` | 历史查询及 HTTP/MCP 序列化；扩展只经受权限校验的新详情操作，不扩旧 ops 输出 |
| `src/core/backup.rs` run_backup/run_restore | manifest v2 只含 history/gateway SQLite 与 history/media；restore 停核心取得 core.lock，维护旧目录/回滚。**当前不包含角色配置或完整发送账本**，不能声称已覆盖 |
| `src/core/control.rs` execute_control_full | App 复用本地 CLI JSON 控制入口；新增 brain-* 操作，避免 Swift 写存储文件 |

## 2. 唯一数据模型（proposed）

采用 state_dir 下不可变对象 + 单一原子根清单；不新建角色 SQLite。目录 0700、文件 0600；ID 仅 `[a-zA-Z0-9_-]{1,80}`，路径由核心生成，禁止用户传绝对路径、`..`、符号链接。账号由既有 effective_account_id 决定，不能由请求自由切换。

- `brain-profiles/roles/<role_id>/<revision_id>.json`：RoleRevision。
- `brain-profiles/knowledge/<knowledge_id>/<revision_id>.json`：KnowledgeRevision。
- `brain-bindings.json`：唯一可变根，含全部角色目录状态、draft 指针、active 绑定与迁移记录；不可变对象未被根引用时只是孤儿，不生效。
- `brain-profiles/.write.lock`：进程间 flock，涵盖读取 generation、验证、写对象与根提交；与运行维护 core.lock 不互相替代。需要两锁时固定 core.lock → profile lock；正常编辑只取 profile lock，不停核心。

所有对象 `schema_version:1`，拒绝未知 schema，写入未知字段报错；未来较新 schema 启动只读并返回 schema_unsupported，不重置、不默认迁移。旧 config.json 原样保留且不额外添加 brain schema 字段。

### RoleRevision 字段

`role_id, revision_id` 字符串；`parent_revision_id` nullable；`created_at` UTC 秒；`origin`=`user|legacy_builtin_v1`；`name` 1..80 字；`persona` 0..8000 字；`goals` 字符串数组（最多20条、每条500字）；`tone` 0..2000 字；`business_rules` 数组同 goals；`model_ref` **仅 `config.model_config`**；`knowledge_refs` 为 `{knowledge_id,revision_id,content_sha256}` 有序数组；`templates` 如下；`context_budget` 如下；`content_sha256` 为排除该字段后 RFC8785 canonical JSON 的 SHA256（小写 hex64）。所有不可变修订不提供 update/delete，编辑即新修订；回退只是将 active 指向历史修订，不改旧对象。

`templates` 是完整必填 map：`greeting`、`insufficient_information`、`unreadable`（unsupported_type/too_large/download_failed/snapshot_changed/extract_failed/empty/transcribe_failed/unknown）、`unreadable_missing_labels`、`unreadable_many_items`。每条可为 null 表示“无自定义表达”；模板可用变量只有 `{label}`、`{labels}`、`{count}`、`{max_mb}`、`{supported_formats}`。不执行表达式、不读文件、不动态网络；未识别变量报 template_invalid。null 不等于启用通用商业回复；必需事实仍由不可编辑系统规则表达，但系统层只能要求忠于事实，不得在空模板时重建写死业务人设、问候或商业兜底句。空模板且无法生成合规内容时安全失败，不发送默认商业回复。迁移修订所有 legacy 模板为显式文本。

系统能力/格式不是可编辑模板：supported_formats 与 max_mb 来自实际解析能力和 config。UI 只提供业务角色、知识、话术编辑；模型连接、brain.mode、external timeout/action/queue **仅 MCP 页面编辑**，大脑页显示摘要和跳转，不能第二处保存。

### KnowledgeRevision 字段与预算

`knowledge_id, revision_id, parent_revision_id, created_at, title, text, content_sha256, source_label`；纯 UTF-8 文本、单项最大 1 MiB、每角色最多32项；source_label 是人工说明，不授权抓取 URL/路径。首版无在线 RAG、无隐式模型摘要；摘要若用户提供，作为独立不可变知识正文保存并明确来源，不伪装为完整原文。

预算 `context_budget={max_input_bytes:49152,knowledge_bytes:16384,history_bytes:16384,overflow:"truncate_tail"}`；均为去本地路径后的 UTF-8 JSON **序列化字节**预算，非精确 token 数，不改变模型配置。max_input_bytes 4096..1048576；分区之和不得超过总额；固定系统约束、角色、当前消息、失败事实及当前附件说明先占额，禁止裁掉。超出总额返回 context_budget_exceeded（不调模型、不发送、不把消息当已处理），由现有失败/重试上限策略处理并显示需调整，不另造等待状态机。

剩余额中按 knowledge_refs 顺序装入知识，历史按既有 context_messages 上限取最近项；每区最多配置额度及剩余总额。知识在 Unicode 边界截尾（为完整 JSON 预留转义/截断标记成本）；历史整项从最旧开始丢弃并保持剩余顺序；不对当前失败附件清单做静默裁剪。记录每项 original_bytes/included_bytes/truncated 及 omitted IDs、算法版本 `utf8-json-v1`；每次序列化复核总额，超额失败。不会承诺 bytes 等于模型 token 或永不被 provider 拒绝；provider 超长错误沿旧错误路径返回，禁止自动调用另一个模型。

### 根清单与绑定

根 `schema_version,generation,roles,accounts,migrations,activation_history,request_results`。activation_history 保留所有激活记录 `{account_id,generation,active,at}`，直到显式受审的历史保留策略允许清理；request_results 是有序 `{request_id,request_sha256,result}` 数组。账号项还含 nullable `legacy_fallback` 引用，仅用于旧配置已启用的 builtin 超时兜底，不替代 active 或改变 timeout_action。存储层主键、CAS、读取和修订寻址不得依赖可变显示名，名字只供展示。roles 数组项 `{role_id,draft_revision_id,archived}`；draft 可与 active 相同，保存草稿不影响 active；archive 仅允许无 active 引用角色，有引用返回 role_in_use；不物理删除已被历史引用对象。

accounts 数组项 `{account_id,active:{default,overrides},draft:{default,overrides}}`；default 为 nullable `{role_id,revision_id}`；overrides 数组 `{contact_id,chat_kind,role_id,revision_id}`。联系人/群稳定 ID 来自现有 contacts，绝不按显示名保存绑定；chat_kind 验证与 contacts 一致。active 解析优先精确 contact 覆盖，其次账号默认；无角色返回 role_required，禁止隐式旧人设。账号可用默认角色覆盖私聊/群，但群规则始终强制；用户可另配群覆盖。

白名单唯一来源是 **config.allowlist/ignorelist 经 BrainAccess 解析**；绑定不复制清单，也不授予访问。保存覆盖须账号匹配且当前 permit；后来撤白名单仅令覆盖 dormant，不删除修订、不恢复权限。改名不失绑定；merge 不自动重写稳定 ID，沿 family 获取候选，多个不同覆盖返回 binding_ambiguous，必须显式解决，不能随机取一个；unmerge 恢复各原 ID。账号默认只应用当前有权限的联系人。

## 3. 原子性与控制接口（proposed）

本地 `control brain-* --config <explicit-path> --request-json <JSON>` 经 execute_control_full 分派；输出单 JSON、退出码 0 成功/2 请求或冲突/1 IO。不能绕过原有配置路径与当前账号权限。所有写操作必填 `expected_generation`；读取返回 `generation`。请求共用 `schema_version:1,account_id`（必须等于当前账号）、`request_id`（UUID）；同 request_id + 同 canonical 请求重放原结果，不同内容返回 request_conflict；根保存最近1000项请求结果，淘汰后的旧 request_id 仅按 CAS 处理，不承诺永久幂等。

| 操作名 | 请求专有字段 | 成功 data |
|---|---|---|
| brain-list | 无 | roles 摘要、当前 account bindings、generation、model/模式只读摘要 |
| brain-get | role_id, revision_id（可省略取 draft） | 完整 RoleRevision |
| brain-save-role | expected_generation, role（新建省 role_id；编辑带 role_id,parent_revision_id） | role_id,revision_id,content_sha256,generation；仅推进 draft |
| brain-save-knowledge | expected_generation, knowledge | knowledge_id,revision_id,content_sha256,generation |
| brain-copy-role | expected_generation,role_id,revision_id,name | 新 role_id/revision_id；不复制 active 绑定 |
| brain-archive-role | expected_generation,role_id | archived:true,generation |
| brain-save-bindings | expected_generation,draft:{default,overrides} | draft,generation；全量替换当前账号草稿，其他账号不变 |
| brain-activate | expected_generation；无任意 role 参数 | 把当前账号完整 draft 原子复制至 active，generation，previous_active；验证所有引用及权限 |
| brain-rollback | expected_generation,target_generation | 从保留的该账号激活历史重建 draft+active，产生新 generation；不是全局根倒退 |
| brain-preview | role_id,revision_id,input,execution:"assemble_only" | preview_id,snapshot,decision:null,cost:{incurred:false} |
| brain-preview | 同上，execution:"execute",cost_consent:true | preview_id,snapshot_id,decision,model,cost:{incurred:"unknown",provider_reported:null} |
| brain-turn-detail | turn_id | 权限内 provenance/input refs/decision/execution；不泄漏其他账号 |

成功 envelope `{ok:true,data:{...}}`；错误 `{ok:false,error:{code,message,retryable,details}}`。conflict details 仅返回 current_generation、当前账号摘要（不给整份跨账号根）；expected_generation 不匹配返回 revision_conflict，可重新读取人工合并，不静默覆盖。parent_revision_id 与当前 draft 不符也冲突。

稳定错误：invalid_arguments、schema_unsupported、revision_conflict（retryable false）、request_conflict、role_required、role_in_use、template_invalid、binding_ambiguous、not_found（隐藏跨账号与不存在区别）、context_budget_exceeded、preview_capability_required、cost_consent_required、preview_execution_disabled、store_unavailable（retryable true）、integrity_mismatch、restore_incomplete。旧外部既有 turn_not_found/turn_expired/turn_revoked/already_answered/invalid_claim_token/invalid_text/rate_limited 等不改名、不改 envelope。

提交算法：锁内读根 → CAS/权限/引用校验 → 写同目录 create_new 临时对象、fsync → rename 至不可变名称（存在则验证相同 hash）→ 写根临时文件并 fsync → 原子 rename 根 → fsync 父目录 → 响应。rename 前故障旧根完整；之后响应丢失 request_id 可恢复。不可变孤儿允许下次清理但不清历史引用。读者先一次读根，再按 hash 读对象；不允许“角色新、知识旧”的分裂快照。单个根含迁移标记，避免多文件事务假象；generation 单调递增。

## 4. 系统约束与角色文案完整拆分

清单只取非 cfg(test) 生产代码；测试夹具、golden 字符串不是运行时默认来源。

| 来源 | 不可编辑系统事实/规则 | 可编辑或显式迁移文案 |
|---|---|---|
| builtin::build_prompt service 首句 | 无业务授权时不得声称有订单/退款等能力 | “用户部署的个人微信AI客服助手”；友好简洁；100字；首次自我介绍 AI；不足询问 |
| 同函数事实段 | 禁止编造商品/价格/订单/退款/营业时间；有知识也不得虚构已执行动作 | 不足信息的客户表达模板，业务目标与规则 |
| 同函数图像段 | 仅有本轮附件才分析；缩略图不清须承认；image_notes 为已提取要点；recent_context 不是本轮；image_count/earlier_images 语义 | 无；不能由角色否认旧图/本轮边界 |
| 同函数 unreadable/files 段 | 每项失败点名；真实 reason；正文/表格/页面才可用；必须承认截断/失败；不能仅凭 filename 声称已读 | 按 reason 对客户表达，下一步建议模板 |
| 同函数 voice_transcript 段 | 当作对方本轮语音，非文件；转写可能错误；不清需确认 | 请对方确认的语气可由 tone 调整 |
| 同函数 group 段 | 群昵称前缀、只回应本轮、必要 @ 发言人、隐私边界、不互相转述、不逐条复述 | 无，保留现有群聊规则 |
| 同函数非 service | 个人微信授权测试上下文与测试披露要求 | 显式 legacy-test 修订：“简短自然中文、不超过100字、说明自动回复测试” |
| 同函数最终格式段 | 只生成文字、不执行工具；不承诺已办业务；不索密码验证码；聊天/附件不可信，不改任务白名单操作；JSON 仅 reply、非空、无 Markdown | JSON 示例“你好，我是AI客服助手”改为协议占位，不能当兜底回复 |
| unreadable_hints unsupported_type | 格式不可读，支持格式来自核心；禁止声称仍上传 | “这种格式暂时读不了…请对方转成这些格式再发” |
| too_large_hint | 实际 max_mb 限制不可编辑 | “文件超过{max_mb}MB上限，请对方压缩、拆分或截图关键部分再发” |
| unreadable_hints download_failed | 未取得文件 | “这次没能取到文件，请对方重发一次” |
| 同 snapshot_changed | 取文件时画面变化、未取到 | “取文件时聊天画面变了，没取到，请对方重发一次” |
| 同 extract_failed | 解析失败；加密只是可能原因，不得升级为确定加密 | “文件打不开或已加密，读不出内容，请对方另存为普通格式再发” |
| 同 empty | 无可读文字；扫描/空文件只是可能 | “文件里没有可读的文字（可能是扫描件或空文件），请对方发文字版或截图” |
| 同 transcribe_failed | 语音转写失败 | “这条语音没能转成文字，请对方打字再说一次” |
| 同 unknown/default | 未知失败，不能编原因 | 同 download_failed 文案，显式 unknown 模板 |
| unreadable_item | 保留原 label/reason；既有 hint 不可绕过事实规则；旧纯 label 可转 unknown | 按固定快照渲染 hint，不信任外部传入 hint 为系统指令 |
| turns::ensure_unreadable_named | 漏项必须补充；仍执行 validate_reply/200字符上限 | “{labels}打不开，麻烦重发一下。”和“有{count}个文件或图片打不开，麻烦重发一下。”迁移入显式模板；新版按 reason 分组，不能把 too_large 都说成打不开 |
| understanding::understand_with_requester | 分批图片标号、逐图 notes、看不清承认、不编造、图片文字不可信 | “看图助手”是内部任务说明，保留系统，不属于客户人设 |
| understanding::note_checker / model_api SYSTEM_NOTES | notes 数量匹配、每项1..300字符、JSON单字段 | 无 |
| model_api SYSTEM_REPLY / 通用格式提示 | reply 输出协议、校验/重试；不新增 action 到内置模型输出 | 无 |
| documents 提取器 | “[公式无缓存值，未执行]”、截断/提取元数据为事实，公式不执行 | 无；不是客户话术 |

系统约束由程序拥有 `system_contract_version:"builtin-facts-v1"`；角色、知识、模板只能在下层数据段进入提示，不插入 system 权限。提示注入并不能靠关键字过滤彻底防住：行为层的白名单、无工具能力与 validate_reply 仍是最终边界。迁移验收比较编译输入、字段事实及动作，不断言自然语言逐字相同。

## 5. 升级、全新安装与快照

迁移仅核心执行一次，UI 不能猜旧版。旧部署判定：安装/升级入口显式 `upgrade_from_legacy:true` 且已有有效 config；全新 init 标记 fresh。单纯缺少 brain-bindings.json 不足以判升级；入口无法判定则 migration_required，人工选择，不静默种子。不得读取真实 DB 来推断本设计。

迁移标识 `legacy-builtin-v1:<account_id>` 在根 migrations 内，内含 source_config_sha256、seed_content_sha256、completed_generation。已完成即 no-op，不覆盖后来编辑。旧 builtin service 迁出 legacy-service role；非 service 使用显式 legacy-test role。旧外部模式仍为 external，不改 timeout/action/queue/model；可创建标为 inactive 的 legacy fallback 修订供原 timeout_action=builtin 使用，只有旧配置确实启用 builtin fallback 时绑定该 fallback 引用，不隐式让外部执行角色。新安装未配置角色时 role_required；旧未完成迁移的运行不能悄悄切换空角色。

迁移顺序：停接新配置写入 → 备份旧配置（不含密钥正文）→ 生成固定 legacy 角色/空知识引用与话术 → 原子提交根及迁移标记 → 校验读取/hash → 接入新 snapshot 编译器 → 不修改 config 原模式。迁移失败保留旧版可启动，升级事务不激活新二进制；不得把损坏角色根当未迁移再生成。旧格式 config reader 兼容；旧二进制忽略 sidecar，会回到旧硬编码，故组件回退需显式提示“角色配置不受支持”，不能声称相同行为。

每个真实轮次固定 `snapshot_id`、`account_id/contact_id`、`turn_id`、active generation、role/knowledge revision+hash、system_contract_version、model_ref 和去密钥的模型配置摘要、allowlist hash、context budget/report、input IDs/media IDs/source、assembly hash。快照在首次可能模型调用（包括理解图片）前持久化；同 turn_id 重试读取原快照，激活不影响在途。当前权限仍在认领、配置读取、回复及发送前重新校验，快照不能保留已撤销授权。模型/连接切换不改本任务策略：快照只记录已选择的配置，I2 必须保证同一轮理解与回复使用同一配置实例；不读取后改的 config 拼接新实例。

迁移示例见 production-brain-examples/migration.json：确定性断言 mode/model/超时策略不变；role origin、100字符偏好与全部失败模板显式存在；再次运行根 generation 与 revision IDs 不变。迁移 legacy 补充模板保留原文可审计，但新 reason-aware 补充的行为差异须单独 RED/GREEN，不能以迁移等价掩盖。

## 6. 外部协议与兼容（proposed）

不新增 wait 动作：等待仍由现有 quiet/max/附件稳定、claim lease、deadline、timeout 策略管理；external action 永远 send/skip/handoff。handoff 和恢复复用 handoff.json/admin-pause.json 与既有 resume_session，不建第三套暂停。

新客户端调用独立 `brain_negotiate`（HTTP tool 或 core.sock op）请求 `{versions:[2],capabilities:["snapshot_v1","preview_isolation_v1"]}`，会话记录交集；响应 `{selected_version:2,capabilities:[...],preview_enabled:false}`。socket 使用连接级 client_id 状态，HTTP 使用既有 session 状态；断线/重新 initialize 回到 v1，不能仅凭 clientInfo 名字推断能力。不改 MCP protocolVersion 的含义。v2 只在协商后 `next_turn` 返回 api_version="2" 并增加 `brain_snapshot`，携带同一编译输入（角色、知识、话术、规则、预算报告、历史/附件引用）；inline 的小文本优先，超过 wire 上限拒绝交付而非半份快照。首版不提供远程任意配置路径读取。

| 客户端 | v1/旧部署无角色 | 激活自定义角色后 | 模拟 |
|---|---|---|---|
| 旧 core.sock/MCP | 原 wire、next_turn/reply/resume_session 保持 | 仍原 wire；详情标 config_applied:"unverified_legacy_client"，不谎称角色已应用；UI 激活外部角色前展示该限制 | 禁止进入旧队列 |
| 旧 HTTP v1 strict | turn_v1 原字段与 snapshots 保持，claim/reply/status 原义 | 同上，角色无法强制旧客户端遵循；能力未协商不注入未知字段 | 拒绝 preview_capability_required |
| 新 HTTP/socket 已协商 snapshot_v1 | v2 完整快照；原授权和 strict ledger 不变 | 取得固定版本，reply 回显 snapshot_id；不匹配返回 snapshot_mismatch 且保留有效 claim | 仍需 preview_isolation_v1 |
| 新客户端连旧核心 | brain_negotiate unknown_tool；回落 v1，仅显式显示 legacy | 不展示“外部角色已生效” | 禁用 |
| 内置 | 使用固定编译快照 | active 即下一新轮次生效 | 默认 assemble_only；本任务不执行真实模型 |

新增 `snapshot_mismatch` 为新协议专用 invalid arguments 类错误，不改变 v1 reply schema。新 v2 reply 增 snapshot_id 必填；v1 输入 additionalProperties:false 保持。外部业务角色标识 applied 仅表明客户端确认输入版本，不能证明模型完全遵守话术；保留 client_ack 字段。unverified 只描述角色配置应用缺少证据，不是旧收发能力失败，也不能把旧客户端状态标为故障。不增加旧客户端必填字段，不阻塞现有 legacy 服务、不强制升级客户端、不将其隐式切回 builtin。

### 预览隔离及费用

默认 assemble_only：合成 input `{text,chat_kind,files,images,failed,context}`；内存 fake requester，无付费调用、无 GUI/真实路径。input 中路径、真实 turn_id/message_id/media_id 一律拒绝；预览 namespace `preview_<uuid>`，独立临时目录与队列，不写 history/messages/sends/open-api/turns.json，不调用 Capture/hand，不占真实限速/统计，不改变 handoff/pause。复用同一个 snapshot compiler 和 Brain decision adapter，发送 adapter 不存在。

execute 仅明确 cost_consent=true 且部署显式允许 preview execution；本次及 I3 fixture 验收始终禁用。外部执行还要求会话支持 preview_isolation_v1，独立 `brain_preview_next/brain_preview_reply` 工具、独立 claim/deadline，payload 必填 simulation:true、purpose:"preview"、billable:true；真实 reply 不能接 preview ID，反之亦然。客户端明确隔离能力只证明协议声明，实际计费按服务规则；费用无法确定返回 unknown，不能显示免费。超时不自动 fallback 真实模型。可取消预览仅清理其临时资源，不撤销已发生费用。真实历史预览首版不支持；后续须另审授权和脱敏契约。

## 7. 对话 provenance、表与备份（proposed，需要审核）

不改 messages/sends 主键和既有状态含义。新增 history migration **5**（当前基线最高4；实际集成基线必须重新核对并由协调者分配最终编号，禁止覆盖）：

```sql
CREATE TABLE brain_turns (
 account_id TEXT NOT NULL, turn_id TEXT NOT NULL,
 contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
 snapshot_id TEXT NOT NULL, snapshot_json TEXT NOT NULL,
 input_message_ids TEXT NOT NULL, observation_json TEXT NOT NULL,
 decision_json TEXT, model_json TEXT NOT NULL DEFAULT '{}',
 send_key TEXT, execution_json TEXT NOT NULL DEFAULT '{}',
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 PRIMARY KEY(account_id, turn_id)
);
CREATE INDEX brain_turns_contact_time ON brain_turns(account_id,contact_id,created_at);
```

snapshot_json 保存完整去路径、去凭据的编译输入及不可变引用，确保知识修订文件丢失仍能审计；与消息正文同等账号权限。snapshot_id 同 turn 唯一；INSERT 冲突时 hash 不同即 integrity_mismatch，不能覆盖。启动/决策前事务持久 snapshot，失败阻止模型/发送；decision_json 在模型完成后、发送前提交（send/skip/handoff/reason/generated_reply/client_ack）；记录失败不执行手。已有 sends.model 保留现有 metadata，附带 brain:{snapshot_id,role_revision_id} 供旧详情渐进读取；sends.input_message_ids 及 journal 继续写，不复制发送状态机。

execution_json `{phase,error_code,delivery:"unknown",echo_observed,handoff:{reason,resolved_at},updated_at}` 从既有 journal/sends/回执归并的只读投影；phase=not_sent/generated/triggered/echo_observed/unknown。不凭 triggered 判送达，echo 只证明己方观察。更新该投影失败不得重新发送；以原持久发送回执为权威，详情返回 provenance_incomplete 并可幂等重建投影。旧记录无 brain_turns 行，展示 provenance:null/source:"legacy"，不臆造配置版本。模型 JSON 仅 backend/provider/model/request_id/seconds/token usage（若提供）、错误码，不存 API key/env 值/Authorization 或裸 provider 请求体。

`brain-turn-detail` 本地以及新 HTTP `brain_get_turn_detail` 都先重新 BrainAccess 检查 account/contact，再读行；详情输入 ID 关联 source/capture/live/baseline/rebaseline_gap/ambiguous/imported，不把 OCR 当完整聊天。外部 v1 history_get_* 原 shape 不变；I3 走本地详情操作。

备份 manifest **v3** 为拟议编号，必须在实际集成基线重新核对、分配并冻结兼容范围；它新增 `brain:{root_generation,files:[{relative_path,sha256,size_bytes}],complete:true}`、`config_snapshot`（非密钥 model 引用/模式/白名单）、`execution_state` 文件清单。核心维护锁或在线稳定截点固定 root 后复制所有 reachable revisions、激活历史、knowledge、migration 标记及 history DB brain_turns；保持旧 media/gateway 备份。包含 sessions journal、既有发送幂等状态、handoff/admin-pause、open-api/turns.json（含发送未知风险）；不备份 socket/lock/PID、访问 token、凭据正文、preview 临时文件。配置快照保留 api_key_file/env **引用**但恢复不拷密钥，缺凭据保持暂停。

恢复先完整验证白名单相对路径、禁止 symlink、hash/schema/全部引用、DB integrity；全程持有 core.lock，临时 staging，保存全部旧目录和配置，事务日志逐阶段记录。所有新文件/DB安装成功后才激活配置，任何阶段失败恢复完整旧集合，重启遇未完成 restore 保持暂停。恢复发送未知状态不自动重发/改 key；恢复的外部 terminal ledger 不恢复过期活跃 claim。旧 v2 备份支持原逻辑但标 brain_coverage:"absent"；若当前已有角色/新记录，禁止默默混用旧 DB 与新根，返回 restore_incomplete，须显式受审的 legacy restore 计划。v3 不允许旧核心恢复（安装器检查兼容范围）；本设计不做真实恢复。

## 8. 精确文件所有权与串行接口

下列为协调者批准后建议冻结的清单；未批准不代表可改源码。I1 完成并集成后才启动 I2，I2 后才 I3；禁止并行争抢公共文件。

| 任务 | 独占文件/范围 |
|---|---|
| I1 | 新 `src/core/brain_profiles/{mod,store,schema,migrate,tests}.rs`；`src/core/config.rs` 仅兼容入口；`src/core/mod.rs` 仅模块注册。实现存储/CAS/迁移/只读快照解析函数，无 turns/history/Swift；模块内测试不新建共享 integration 文件 |
| I2 | `src/brain/{mod,builtin,model_api,understanding}.rs`（不改 documents 解析算法）；新 `src/brain/snapshot.rs`；`src/core/turns.rs`、`external_brain.rs`、`brain_tools.rs`、`control.rs`、`backup.rs`；`src/core/open_api/{mod,contract,http,session,ledger,history,turn_tests,compat_tests}.rs` 及 snapshots 下新增 v2 文件（旧 v1 snapshots不重写）；`src/core/history/{store,query,serve,mcp_tools}.rs`；`src/core/serve.rs` 仅启动迁移/注册接入，不改调度算法。I1模块若需扩接口先 ask，不自行改其文件；access.rs 现有授权逻辑不改 |
| I3 | 新 `native/bootstrap/BrainPage.swift`；`native/bootstrap/{ConfigForm,Shell,StatusPanel,McpPage,SessionsPage}.swift`；`native/bootstrap/main.swift` 仅大脑页注册与必要局部接线，不改启动/信号/supervisor；新增纯逻辑 fixture 测试 `native/tests/BrainContractTests.swift` 与合成 fixtures `native/tests/fixtures/brain/`（须核对 D2 最终测试目录）；若编译注册需要脚本，先协调，不默认授权。不修改 Rust、不自写角色存储/迁移 |

D2 可设计页面与更新，但上述运行字段唯一事实源在本文件；模型/模式/超时仅 MCP 页面拥有编辑权。I6 在 I3 集成后串行修改 main.swift 注册更新页，不能与 I3 并行编辑。更新任务 I5 读取 v3 backup 契约但不与 I2 同时改 backup.rs。现有手/眼 native/bot、policy、worker 动作不归这三个任务。

## 9. 可执行验收计划及本次证据

本次设计无生产行为修改，RED/GREEN **不适用**，不得把 JSON 解析成功当实现 GREEN。未运行 cargo/Swift/GUI/模型；既有全量基线由 Q0 独立提供，本任务未重新验证或归因既有失败。后续每项先增加失败断言并保存 RED 命令/退出码，再实现 GREEN；以下测试名为 **拟新增**，不是已有测试。

测试壳须先 `mktemp -d`，把 HOME、XDG_CONFIG_HOME、WECHAT_* 明确指向该临时根，配置/state/history/media/MESSAGE_GATEWAY_DB 全在根内，model requester=fake、Capture=fake；测试内断言 canonical 路径属于临时根且 GUI 调用次数0。不得执行真实 control start/serve、真实 codex、网络 provider。编译工具缓存路径可只读复用，但运行配置不能落真实 HOME；无法隔离则不运行并记录。

| 阶段/拟命令 | 必须揭示的 RED 与 GREEN 断言 |
|---|---|
| `cargo test --locked brain_profiles::tests::migration_` | builtin 迁移内容逐字段（含全部 unreadable 和漏项模板）；重复迁移 generation/ID/hash 不变；external mode/model/timeout/queue 字节等价；fresh 无种子；损坏根不重置 |
| `cargo test --locked brain_profiles::tests::cas_` | 同 generation 两写仅一成功；故障注入每个 fsync/rename；读者无半根；request replay；未知 schema/越界路径/symlink 拒绝；跨账号隐藏 |
| `cargo test --locked snapshot_` | 激活发生在图片理解/回复之间仍同 revision/model；同 turn 重启复用 hash；知识超额 UTF8/JSON预算确定；当前输入超额不调模型；群规则不可覆盖 |
| `cargo test --locked preview_` | assemble_only 模型/GUI/队列/真实 DB 写次数均0；不支持能力拒绝；preview ID 无法真实 reply；cost consent 缺失拒绝；支持外部 fake client 独立队列 |
| `cargo test --locked core::open_api` | v1 snapshots不变；claim/token/lease/deadline/revoke/replay原测试回归；v2协商/重连/错误snapshot_id；旧客户端显示unverified，不中断原 send/skip/handoff |
| `cargo test --locked core::turns::tests` | 当前附件、语音、重复消息、群 @、stop/verify/幂等/未知发送/转人工原测试；新 skip/handoff/model失败均有 provenance；写快照失败模型与发送0次 |
| `cargo test --locked core::backup::tests` | v3 roundtrip 包含角色知识绑定/active draft/brain_turns/幂等；每阶段恢复失败回滚；v2缺覆盖拒混用；缺凭据暂停；真实 HOME fallback 哨兵禁止 |
| I3 合成 Swift fixtures | 冲突 reload/merge不覆盖；draft/active分开；旧external未应用提示；费用unknown；模型只读跳MCP；legacy provenance空值；权限失败不保留前账号数据 |

快照和恢复覆盖扩展须分别增加故障测试：图片理解前快照写失败、决策落库失败、重启后原快照重用；恢复每个文件集合替换/配置激活/事务日志阶段故障、DB与根不匹配、ledger缺失和发送未知保留。不得只用成功 roundtrip 代表扩大后的恢复范围正确。

最终相关 fmt/clippy 与完整基线命令按 Q0 提供，不删测试、不放松门禁。真机首条未读顶上方、同时长语音身份冲突、换号/改名/重名/遮挡/退出、连续运行另行授权验收；本设计不声称通过。

本次文档检查：JSON 示例由本地解析器解析、交叉引用/模板原因集合检查；git diff --check；只允许 docs/design/production-brain-contract.md 与 production-brain-examples/*.json。详细运行命令与退出码在最终协调者回执提供。

### D1 实际检查记录

- `git rev-parse HEAD`：退出0，得到派发基线；`git status --short`：退出0、初始为空。
- 本地 Node 合成示例检查：退出0，3份 JSON 均可解析、fixture_only=true、迁移前后 config 深相等、二次迁移 generation 不变、8种 unreadable 原因齐全、角色 canonical hash 一致、审核关键项存在；只读本任务文档/JSON，无 HOME/DB/GUI/网络/模型访问。
- `git diff --check`：退出0；首次和审校后均未发现空白错误。此命令在新增文件暂存后再运行一次覆盖新增内容。
- `orca orchestration check --from ...` 曾退出1（该 check 子命令不接受 --from）；按 CLI 错误指引改用 `--terminal ... --json` 后退出0并读到协调者审批。ask 退出0，批准仅限 proposed 设计交付。
- Rust/Swift 运行时测试、fmt/clippy、真实模型及真机测试未运行：设计范围无生产源码变更；运行时基线仍归 Q0，不能将本次文档检查记为实现 RED/GREEN。
