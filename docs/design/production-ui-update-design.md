# D2：大脑界面与完整应用更新设计

日期：2026-11-10。状态：设计提案，待协调者审查；不是已实现 API 或项目验收。

## 1. 基线、证据与边界

派发和本次 HEAD：`32bb12e550b6739c5c7d6874bbd7388cd771ef5c`；分支 `work/prod-ui-update-design`；开始 `git status --short` 无输出。只新增本文及 `production-ui-wireframes/`，不改原始 checkout、生产配置或运行状态。权威输入为原始 checkout 的 `docs/superpowers/plans/2026-11-10-production-brain-ui-and-app-update.md`、任务总表及 I3–I6 任务书（只读）。

图发现使用 `search_graph` / `get_code_snippet`；图返回 `原始checkout根目录`，且 McpPage 索引行号落后于本 worktree，因此明确降级为本 worktree 源码核对，未以旧图行号作为实现证据，未刷新/写入原始 checkout 索引。

当前实际能力：

| 源码 | 已有能力及复用点 |
|---|---|
| `native/bootstrap/Shell.swift` | AppKit NSSplitView + NSTableView 动态 register；860×760，侧栏 140–180，行高 36；NSBox.setContent 的约束修复必须保留 |
| `native/bootstrap/main.swift:72` | 总览、会话、联系人、MCP、设置、日志注册；新增大脑/更新沿用同一注册机制 |
| `native/bootstrap/ConfigForm.swift` | maskConfig、buildUpdatedConfig 保留未知字段、校验/备份/原子保存；getSecureTempDirectory 使用真实 HOME，不能直接跑保存测试 |
| `native/bootstrap/McpPage.swift` | 内置/外部选择、外部超时/超时动作/排队、令牌及连接检查；有 control mcp-info/open-api-check 等真实控制命令 |
| `native/bootstrap/SettingsPage.swift` | 模型摘要、处理参数、权限检查与原设置保存；不是角色存储 |
| `native/bootstrap/SessionsPage.swift` | 会话状态、现有 handoff 恢复按钮；尚非完整轮次 provenance 浏览器 |
| `native/bootstrap/CoreSupervisor.swift:360` | 主动停止 PID、正常 exit 0、配置 exit 2 单独处理；其他异常进入 RestartPolicy |
| `native/bootstrap/Supervisor.swift:13` | 1h 滚动窗口第 5 次异常 giveUp；5/15/60 秒退避 |
| `scripts/deploy_host.sh` | 原位安装、多组件分别替换、本机专用钥匙串签名；不是事务式更新器 |

下文 P 表示 proposed，必须由 D1/I2 或 I4/I5 契约审查落实后才能接线；E 表示上述已存在能力。未提供可调用 HTTP 路由、CLI 命令或数据库表名的地方不得由 UI 自行创造实现。全部静态示意使用合成账号/联系人，不连接服务。

协调者回执 `msg_adefc92172f5`：MCP 保留模式、模型连接与超时的唯一编辑入口；大脑页只读摘要和跳转，编辑角色/知识/模板/绑定/版本；暂以 proposed 标记设计字段，不实现生产接口。已按协调者回执 `msg_3f52a10ab839` 只读核对 D1：`相邻worktree prod-brain-design 的 docs/design/production-brain-contract.md`；D1 全部新增字段仍 proposed，以下 §3.4 以其精确字段覆盖前文语义槽位，不代表接口/schema 已批准。

## 2. 导航、布局和交互

导航顺序：总览 / 会话 / 联系人 / **大脑** / MCP / 设置 / **应用更新** / 日志。协调者已批准 I3 对 main.swift 增加大脑页导航注册及必需局部接线，禁止改启动/信号/supervisor 生命周期；I6 必须在 I3 集成后串行增加更新页注册。Shell.register 已支持动态注册，Shell.swift 不需要为新增菜单预先修改；仅有实际缺口才修改，其他预列文件同理。

大脑页采用现有系统字体 13pt、标签 12pt、NSBox、系统按钮；不另建视觉系统。内容边距 20、分组间距 16、字段间距 8。窗口最小沿用 860×760；右区约 680：列表 180、编辑区余宽，窄窗口以“返回角色列表”替代挤压双栏。长内容单一 NSScrollView；底部操作条固定且不遮字段。焦点顺序随视觉顺序；错误用文本与图标、支持 VoiceOver，不只依赖颜色；按钮标实际动作。

```text
┌ 导航 160 ┬ 大脑   当前账号：合成账号 A                 ┐
│ 总览     │ 模式：外部 · 模型：已配置 [前往 MCP]        │
│ 会话     ├ 角色列表 180 ┬ 售后助手  草稿 r4 / 运行 r3  │
│ 联系人   │ 搜索/新增   │ 定义 知识 话术 绑定 版本      │
│ 大脑     │ 售后助手 ●  │ 名称/身份/目标/语言/语气…     │
│ MCP      │ 群聊助手    │ 校验摘要与错误定位            │
│ 设置     │ 已归档      │ [保存草稿] [合成测试] [启用…] │
│ 应用更新 │             │ 最近加载失败：运行仍为 r3      │
└──────────┴─────────────┴──────────────────────────────┘
```

列表显示名称、草稿/运行修订、绑定账号数、归档状态；新增空草稿，复制生成新 ID、未绑定，归档不删除历史；D1 要求无 active 引用才可归档，引用中返回 role_in_use，UI 引导先解除或改绑。首版无物理删除。切换角色/账号/页面有未保存输入时提供保存草稿、放弃、取消；保存失败保留输入。账号切换取消旧请求，响应按账号与请求代次核对，不能把旧账号数据绘到新账号。

编辑标签：定义（身份、目标、语气、范围、规则）；知识（文本/Markdown 导入、原文预览、摘要、版本、启用）；话术（逐模板触发条件、允许变量、冷却、预览）；绑定（账号默认、联系人/群覆盖）；版本（修订差异、校验、合成测试记录、激活/回退审计）。模型摘要始终只读，不显示凭据。Markdown 作为安全文本渲染，不执行 HTML/脚本、不自动抓远程图或链接。

首版话术严格限 D1 完整 map：greeting、insufficient_information、unreadable 各 reason、unreadable_missing_labels、unreadable_many_items。等待、转人工、模型超时模板及自由触发器/冷却/频率字段均不在 D1 首版契约，隐藏这些编辑入口，不以原方案愿景冒充支持能力；不让用户输入自由代码/表达式。事实原因、不能虚称已读、附件/转写语义及权限系统约束在只读“系统约束”摘要，不能删改。未配置模板显示“无自动文案，进入待处理/人工接管”，不偷偷发送默认业务话术。

合成测试侧面板：用户手填合成消息/附件元数据；默认仅确定性组装预览与 fixture 决策，标“未调用模型”。能力满足且另行授权时才有模型测试入口；本轮不调用。显示输入摘要、角色/知识/模型引用、决策和错误，绝不显示“已发送”。外部 simulation 协商不支持则禁用并说明原因，不借用真实 next_turn 队列，不消费真实轮次/历史/统计。

启用确认列出账号、受影响覆盖、目标修订、当前运行修订及“仅新轮次生效”；提交成功先显示“已请求”，读取运行态确认为目标后才显示“运行中”。保存/校验成功不等于启用。加载失败保留旧运行修订。回退是对历史修订执行同一 CAS 激活流程，生成审计，不覆盖历史文件。人工接管沿用 SessionsPage 的恢复动作；角色启用/应用升级均不能隐式清除 handoff、全局或会话暂停。

会话详情增加角色/知识修订、模型引用、输入快照摘要、决策、幂等键/执行阶段、错误、生成/触发/己方回显/送达未知独立标签。旧记录缺 provenance 显示“旧版未记录”，不推断角色或送达；不展示其他账号记录。保留现有会话恢复按钮，不重写发送状态机。

## 3. 字段唯一所有者与后端映射

### 3.1 既有配置（E）

| UI 字段 | 实际来源/写入 | 唯一所有者与约束 |
|---|---|---|
| 账号 | config `account_id` | 沿现有配置流程；大脑仅选择授权账号视图，不创建账号权限 |
| 模式 | `brain.mode` builtin/external | MCP 编辑；大脑/设置只读 |
| 外部超时 | `brain.external.timeout_seconds` | MCP；1..3600 秒 |
| 超时动作 | `brain.external.timeout_action` | MCP；skip/builtin/handoff，不由模板覆盖 |
| 排队时限 | `brain.external.queue_minutes` | MCP；1..1440 分钟 |
| 模型/连接引用 | `model_config.backend/model`，凭据仅状态 | 当前设置仅摘要，模型编辑能力未证实；MCP 为未来唯一编辑入口，未有后端能力前不加编辑按钮 |
| 令牌/端点/连接检查 | McpPage 现有 control 适配 | MCP；不复制 token 到角色/导出/日志 |
| 联系人授权范围 | config `allowlist`/`ignorelist` | 现有联系人页/配置唯一事实源；绑定只引用，不能添加授权项 |
| 附件大小/限流/冷却/上下文/窗口 | ConfigForm 当前字段 | 现有设置；角色不能扩大系统限额 |

### 3.2 新能力语义映射（全为 P，待 D1 对齐）

此表覆盖每个新增可编辑/展示字段；斜杠代表语义分组而非已实现 JSON path。请求和响应字段以 D1 批准版本为准，UI adapter 负责映射，Swift 不复制核心校验器。

| 屏幕字段 | 后端契约语义槽位 | 写入/校验与反馈 |
|---|---|---|
| 名称、身份、目标、语言、语气、业务范围 | profile revision 的 name/identity/goal/language/tone/scope | 保存草稿；名称必填、长度限制取核心返回 |
| 可回答范围、必须转人工、信息不足策略 | revision rules | 核心结构化规则/允许枚举；不可自由执行代码 |
| 角色 ID/schema/修订/基修订/内容摘要 | profile_id/schema_version/revision_id/base_revision/digest | 系统只读；保存携带 expected revision，拒绝覆盖冲突 |
| 归档状态/复制来源 | lifecycle archived / copied_from | 原子归档、复制新 ID；历史保留 |
| 知识名称/格式/正文/启用/顺序 | KnowledgeRevision title/text + 有序 knowledge_refs | txt/md仅作为text，启用为加入refs，顺序为refs顺序；无额外enabled字段，版本固定 |
| 模板名称/正文/启用/触发器 | D1 templates 固定键 → string/null | 名称/触发器不可编辑，启用仅映射null/正文；见§3.4 |
| 允许变量/渲染预览 | template variable schema + rendered preview | 核心校验未知变量、转义与必需变量；不靠前端替换得出生产文案 |
| 模板冷却/频率（愿景） | D1首版没有此字段 | 不展示入口；保留现有核心限流，不新增策略 |
| 模型引用摘要 | existing connection reference + masked status | 从现有模型配置读取，不新增每角色 API key/独立连接副本 |
| 账号默认角色 | account binding → profile/revision reference | account scoped，CAS binding revision；未配置明确显示 |
| 私聊/群覆盖 | authorized recipient reference + kind → profile/revision | 从当前授权范围选择，不复制白名单；同名歧义拒绝，改名/失效引用显示待修复，不自动模糊匹配 |
| 草稿/运行/加载失败 | saved revision / active revision / load error | 分别展示；不以本地缓存替代 runtime acknowledgement |
| 激活/回退目标 | activation target + expected active/binding revision | 原子校验与加载，新轮次快照；失败仍旧版，审计 actor/time/result |
| 合成输入/输出 | simulation capability + synthetic input + snapshot/decision/error | 显式 simulation，隔离 namespace、禁止真实附件路径，输出不进真实客户记录 |
| 校验与冲突 | structured errors field/code/message + current revision | 字段定位；冲突保留本地草稿，载入远端与并排差异，用户合并后重试 |
| 历史详情 | I2 turn provenance + execution outcome | account scoped 只读；缺值为 unknown，附件仍按原授权访问 |

待批准操作集合：读取能力/schema；列举/读取角色修订；保存草稿(CAS)；复制/归档；知识导入预览；模板校验渲染；读取/修改绑定(CAS)；校验/合成预览；激活/历史回退(CAS)；读取运行确认及轮次 provenance。这是能力需求清单，不是新增 route/CLI 承诺。I3 仅在 I2 已提供且批准的操作上接线，其余显示不可用及原因。

### 3.3 空态、超限、错误

| 情况 | 屏幕/行为 |
|---|---|
| 新安装无角色 | 空态“创建角色”，不隐式采用固定人设；内置无有效绑定时阻止启用；外部模式保持原有行为，按 D1 契约明确状态，不擅自切换 |
| 旧版本升级 | 显示“从旧版迁移”的显式可编辑修订及内容差异；迁移幂等；外部模式/连接/超时不变 |
| 无联系人授权 | 绑定区为空，跳转联系人页；不提供在绑定页添加白名单 |
| 草稿损坏/未知 schema | 展示来源与错误，可导出原始草稿供排查（不含凭据），禁用启用，不覆盖运行版 |
| 校验失败 | 汇总错误+字段定位；保存草稿是否允许无效内容按 D1 约定；无论如何不可激活 |
| 并发修改 | 显示本地基版本/服务器当前版本；不自动重试覆盖，保留输入 |
| 运行加载失败/失联 | 保留 last-known 标签和时间；无确认不显示已启用 |
| 不支持能力 | 不露出可操作按钮；保留原因，例如“外部客户端不支持隔离合成测试” |

### 3.4 与 D1 精确对齐（取代上述泛化槽位中的未支持项）

| UI / 操作 | D1 proposed 精确字段 / control 能力 | 首版处理 |
|---|---|---|
| 定义 | RoleRevision `name,persona,goals,tone,business_rules,model_ref` | name 1..80；persona≤8000；goals/rules≤20×500；tone≤2000；model_ref 固定 config.model_config。语言/身份/业务范围放 persona/rules，不增加 language/identity/scope 独立后端字段 |
| 修订摘要 | `role_id,revision_id,parent_revision_id,created_at,origin,content_sha256` | 只读，origin=user/legacy_builtin_v1；RFC8785 hash |
| 知识 | KnowledgeRevision `knowledge_id,revision_id,parent_revision_id,created_at,title,text,content_sha256,source_label`；Role `knowledge_refs[{knowledge_id,revision_id,content_sha256}]` | UTF8≤1MiB，每角色≤32；Markdown作为 text；启用/停用通过加入/移除有序 refs，不添加 enabled 字段；source_label 不授权抓 URL |
| 模板 | `templates.greeting/insufficient_information/unreadable.{unsupported_type,too_large,download_failed,snapshot_changed,extract_failed,empty,transcribe_failed,unknown}/unreadable_missing_labels/unreadable_many_items` | 完整必填 map，各条 null=无自定义表达；开关在 UI 映射 null/正文，不添加 enabled/trigger/policy；变量仅 label/labels/count/max_mb/supported_formats，无自定义模板名称/触发器 |
| 预算 | `context_budget.max_input_bytes/knowledge_bytes/history_bytes/overflow` | 默认49152/16384/16384，overflow固定 truncate_tail；总额4096..1048576，分区不超过总额；单位 UTF8 JSON 序列化 bytes，绝不说精确 token |
| 根与 CAS | `brain-bindings.json` 的 `generation,roles,accounts,migrations` | 统一 expected_generation，不使用每角色独立 CAS 代替根 CAS；parent_revision_id 也需匹配 |
| 绑定 | `accounts[].draft/active.{default,overrides}`；override `contact_id,chat_kind,role_id,revision_id` | 稳定 contact ID，不按名称保存；撤权 dormant；merge 多覆盖 binding_ambiguous；draft全量保存仅当前账号，activate整份draft，不能任意激活一个角色参数 |
| 角色操作 | `brain-list/get/save-role/save-knowledge/copy-role/archive-role` | copy新ID未绑定；archive有active引用拒绝；知识保存后再写角色 refs |
| 绑定/生效/回退 | `brain-save-bindings/brain-activate/brain-rollback` | rollback target_generation 重建该账号 draft+active并产生新generation，不倒退全局根；UI先显示绑定差异 |
| 合成预览 | `brain-preview` execution=assemble_only | 返回 preview_id/snapshot/decision:null/cost.incurred:false；按钮名“组装预览”，不能用假模型回复填充结果 |
| 真实执行预览 | execution=execute + cost_consent + 部署允许；外部还需 preview_isolation_v1 | 本轮和 I3 fixture 均禁用；不承诺免费，不把能力声明当计费隔离证据 |
| 详情 | `brain-turn-detail` | D1 provenance；旧记录 provenance:null/source:legacy；projection不完整显示 provenance_incomplete，不重新发送 |

所有 proposed 本地调用沿 D1 `control brain-* --config <explicit-path> --request-json <JSON>`，仅批准后实现；每请求 schema_version:1、当前 account_id、UUID request_id，写操作 expected_generation。成功 `{ok:true,data}`；失败 `{ok:false,error:{code,message,retryable,details}}`，退出码0/2/1分别成功/请求冲突/IO。request_id同内容重试取原结果，不同内容 request_conflict；UI冲突不自动覆盖。结构化字段错误位置未由 D1 冻结时只做错误汇总，不臆造后端 field path。

首版没有独立模板渲染/能力schema查询 endpoint，模板预览使用 brain-preview 的组装快照，不新增自定义操作；支持枚举由冻结契约和版本能力给出。草稿保存同样核心严格校验，无效输入留在内存并定位，不假称已保存。

预算按 D1：固定系统约束/角色/当前输入/失败事实先占额，不可裁剪；知识按 refs 顺序在分区内 Unicode 安全截尾，历史整项删最旧，最后复核完整 JSON bytes。**知识截断必须可见**：保存后/启用确认做 assemble_only 显示 original_bytes/included_bytes/truncated/omitted IDs 和算法 utf8-json-v1；用户明确看到差异才能确认启用。运行实际输入变化可能导致新的裁剪，详情保留同报告；必需部分超额 context_budget_exceeded，不调用模型/不发送，沿原失败/重试上限路径，不另造 handoff/wait。字节预算不保证 provider token 限额，provider超长沿旧错误路径，不自动换模型。

外部旧客户端仍可沿原 v1运行；自定义角色启用前明确显示 config_applied:unverified_legacy_client，不声称外部已应用角色，不强制切 builtin。协商 snapshot_v1 后显示 client_ack 仅证明确认版本，不证明遵守全部话术。新安装 role_required 和旧外部 inactive legacy fallback、migration_required 均按 D1，不自行推断迁移或绑定。迁移显示 origin 与种子来源，用户可审阅全部 unreadable 含 turns.ensure_unreadable_named 漏项补充模板。

## 4. 更新页与可见状态

更新页上方当前整套 release ID、组件版本一致性、架构/系统；中段候选版本、发布时间、发布说明、最低 OS、schema/回退限制、下载大小及需要空间；底部阶段/字节进度/恢复指引。设置仅放“应用更新”跳转，不重复存储源。分发源由批准配置提供，不给任意 URL 输入框，不内置 GitHub Token。

```text
应用更新                        当前 vA · arm64 · 组件一致
候选 vB · 发布时间 · 来源已验证/尚未验证（明确区分）
兼容：macOS ≥ X；配置 C；数据库 D；回退限制…
说明：可滚动纯文本                 [检查更新] [下载]
下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检
正在等待 1 个轮次；自动处理已暂停       [延期] [查看详情]
```

检查/下载失败可重试；下载时继续处理。安装按钮仅完整验证+兼容通过后可用，先确认目标/暂停/备份/重启。等待超时默认延期，保留暂停/在途状态；“停止处理”只请求安全停止并待确认，不强杀发送中任务。安装不可随意取消时说明阶段；断电恢复显示事务 ID、已完成阶段、当前版本、保留数据与允许动作。无分发方案时显示“更新源尚未配置”，可查看当前版本，不显示假可用版本。

更新读模型 P：current_release/components/architecture/os；candidate release/sequence/date/notes/compatibility/size；transaction_id/stage/progress/error/recovery_options；inflight_count/drain_reason；publisher_verification/local_signing/permissions 各自状态；backup_id/digest/schema/data_since_activation。动作 P：check/download/defer/prepare_install/install/recover；全部复用 I5 真状态与 I4 生命周期接口，不由 Swift 猜进度或改符号链接。

## 5. 发布清单与信任契约（P，I5 实施前 gate）

采用签名封套，明确签名字节为原始规范化清单字节（例如 RFC 8785 JSON；最终算法/序列化须冻结并有 golden vectors）；验签前只解析有大小上限的封套，不执行清单路径或地址。签名覆盖所有字段和组件摘要，不能仅签 version 字符串。

| 清单字段组 | 必需内容 |
|---|---|
| 身份 | manifest_schema、release_id、monotonic sequence、channel、published_at、expires_at、key_id、signature_algorithm |
| 目标 | os=macOS、arch=arm64 或 x86_64、最低/最高已验证系统范围、CPU/最低运行条件 |
| 包 | 固定源相对 artifact ID、原始字节数/展开上限、sha256、签名封套 |
| 组件 | host/core/worker/click/desktop_helper/layout/default resources/updater 的相对路径、原始摘要/size/mode、版本、协议兼容范围、完整集清单 |
| 数据兼容 | config/profile/knowledge/history schema read/write ranges、migration IDs、可否原地回退、最低 updater version |
| 说明 | 纯文本 release notes、已知限制；不渲染远程 HTML |

manifest 与包都必须绑定 channel/target，拒绝缺组件、混架构、摘要不符、不支持 schema、旧 updater。当前 deploy_host 的编译目标是 x86_64 macOS12，并不证明 arm64 或后续 OS 已验收；每目标单独构建、测试、签名。不能把 Rosetta 可启动当作原生支持。统一资源不包含目标机 layout 校准覆盖文件。

信任根内置发布公钥集合（公开材料）；生产私钥离线/CI 受控签名服务保存、不进仓库/客户端。轮换用当前受信根授权的新 keyset（keyset_version、阈值签名、有效期、撤销），过渡期双签并保留离线恢复路径。未知/撤销/过期 key 拒绝；跨算法变更需旧根授权的新客户端。时间回拨不能解除 sequence 下限；本机持久 highest accepted sequence/keyset_version，只有整体验证后原子提升，拒绝同 sequence 不同摘要与跨 channel 重放。首次安装离线包也受内置最低序列约束。紧急降级必须是新序列签名 rollback 授权，绑定目标版本/摘要/范围，不以勾选“忽略签名”实现。已安装 last-known-good 的本地事务恢复与下载旧版本区分；本机恶意管理员篡改全部信任状态不在此机制可防范围。

分发待定：公开签名产物或独立授权下载服务、服务身份/域名、权限与短期下载凭证、离线安装策略、CDN 过期/撤回、密钥托管与紧急轮换责任人；未冻结不能启动 I5 真实分发适配。不得打包开发者仓库 Token，日志/manifest 不含下载 bearer。签名/分发待定仅阻塞 I5/I6，独立大脑 I1–I3 不受阻。

## 6. 安全暂存、本机签名与原子激活

布局 P：稳定受控 launcher/宿主入口 → `releases/<release_id>/` 完整版本目录；`current` 原子指针；`previous`；`update-transactions/<id>/`；独立 `state/` 与用户数据。具体安装根、稳定 App 路径与 launcher 的 TCC 身份需 I5/I6 gate 冻结。worker 继续在 App 包外，但从单次解析且固定的 release root 启动；禁止逐组件查询 current 造成混版。自检验证全部组件报告同一 release。更新器不从待替换版本目录加载尚在执行的依赖。

下载到同卷私有暂存，目录 0700、文件 0600；锁定单一 updater 事务、磁盘空间预检覆盖原包/展开/备份/旧版保留并预留余量。先验证签名与原始包摘要，再安全解包：拒绝绝对路径、..、NUL、重复/大小写归一冲突、symlink/hardlink、设备/FIFO、越界权限位、未列组件、超条目/超展开大小/压缩炸弹；目标路径不跟随链接，拒绝越过根和 TOCTOU 替换。不执行包内安装脚本。验证逐文件原始摘要后将来源证据写入不可混淆的 transaction record。

顺序：原始来源验证 → 暂存树/组件摘要验证 → 本机钥匙串可用性及指定证书检查 → 按依赖内到外本机代码签名 → 逐组件 codesign 验证与签名身份核对 → 本机签后树摘要另存 → 进入激活候选。重签使字节变化，不能再以发布者原始摘要校验签后文件；保存 original digest 与 local signed digest/identity 两份证据。不能用本机签名替代发布者来源验签。钥匙串锁定、证书缺失、签名失败全部中止候选，旧版继续，禁止临时 ad-hoc 降级。UI 不收集钥匙串密码写入配置/日志。

激活前持久 journal、fsync 候选与父目录；同卷原子 rename 指针，记录 old/new release 与阶段；失败后依据指针实际值和校验过的 journal 恢复，禁止猜“最后写的版本号”。App bundle 和外部 worker 必须以整套版本被解析，固定入口自身更新采用独立 updater 的可恢复两阶段替换，具体实现不得宣称多处 rename 天然原子。updater 自升级采用前后兼容 bootstrap/双槽，不覆盖正在执行 updater；首版可固定稳定 updater，最低版本不足则停止并提示人工升级。

## 7. 排空、监督器和恢复

### 7.1 现有 deploy_host 的源码结论

`scripts/deploy_host.sh` 的 MCP 分支以 pgrep 找 core，直接 `kill -TERM` 并等待新 PID，没有调用 CoreSupervisor.stopProcess，因此绕过 manualStoppingPIDs 登记。App 分支先 osascript quit，等待宿主退出后 bootout；`main.swift` 已有 SIGTERM 走正常退出清理，必须保留。`CoreSupervisor.handleProcessTermination` 已排除主动停止和 `.exit/0`，配置错误 exit2 也不计崩溃；`src/core/serve.rs:20` 安装 SIGTERM handler 设置共享 stop，因此直接 TERM 仍可能正常 exit0。**源码只证明路径绕过，不能证明该路径导致历史崩溃计数，也不能证明每次 TERM 都安全排空发送。**本轮没有执行 deploy_host（含 dry-run）、launchctl、GUI 或真实 core。

I4 用隔离子进程记录 TERM 后 reason/status，以及 host 监测 tick、manualStoppingPIDs、重启时机；覆盖 direct TERM→exit0、direct TERM→signal、stopProcess→TERM、超时 KILL、配置 exit2、真实异常与 5 次阈值。先 RED 可复现竞争/错误后最小修复，禁止清空全部崩溃计数。状态机不重写核心调度/发送；通过既有暂停/维护接口扩展受审查的适配契约。

### 7.2 拟定生命周期握手（P）

1. updater 获取单事务锁并持久 intent，host 确认 transaction ID/目标 release；暂停新轮次，沿原全局/会话机制保留人工暂停，不另造第三套 handoff。
2. 核心报告在途轮次、发送副作用阶段、worker 活动；等待完成或确定安全暂停。超时返回 defer_required，未知发送保留未知幂等状态，不换键重发。
3. I4 lifecycle adapter 返回 drain token（账号/事务/代次/有效期）；新轮次入口必须实际被阻止，不能靠 UI 按钮禁用代替。
4. 完整备份验证后，由 host 走主动停止通路；同时安装持久更新抑制标志，防止 tick 或 launchd 把旧版拉起。确认 core/worker/helper 全退出并释放维护锁；不得在 core 持锁时恢复 DB。
5. 独立 updater 存活于 host 之外；同 UID 的受控 launchd job 管 updater 恢复，固定标识/参数，无任意 shell。host 的 launchd keepalive 在事务期启动只能进入维护界面，不能启动 core。需要 bootout/bootstrap 的具体授权与部署策略由 I4/I5 冻结，不在本任务执行。
6. 切换后启动候选维护态，自检版本/组件协议、配置/schema、备份、worker 非 GUI fixture 握手与权限只读状态；通过后显式清除 updater pause，保留人工 handoff/暂停；仅指定 lifecycle owner 重启，不让 updater 与 supervisor 双重启动。
7. 有限重试后进入 recovery_required，不循环安装/重启/发送。真实异常仍保留退避上限；计划更新状态与真实崩溃记录分离，不能滥用 reset。

### 7.3 断电恢复表

| journal 阶段 | 重启后确定行为 |
|---|---|
| downloading/staging | current 未变，续传需重新完整验证或丢弃候选；旧版可运行 |
| publisher_verified/local_signing | 重验原始包及签后结果，失败废弃；不把部分签名视为完成 |
| draining | 持久维护标志生效，核对存活进程与轮次账本；未知发送不重发，允许延期恢复旧版 |
| backup_started | 不完整备份不能用于恢复；验证失败不切换 |
| backup_verified/stopped | 确认进程已停/锁释放；按已验候选继续或恢复旧版 |
| switched/not healthy | 对照实际 current 和 journal，候选维护态自检；失败用完整旧 release 回退 |
| migration_started | 检查迁移事务/幂等标记，禁止旧程序直接读新 schema；进入迁移恢复流程 |
| healthy/committed | 原子写完成并保留 previous/备份；清理不能移除仍在使用版本 |
| journal 损坏/丢失/指针异常 | 暂停自动处理，展示恢复所需证据，不自动选最高目录启动 |

## 8. 数据、角色迁移与回退

沿现有备份恢复及 core.lock，不建立一套不一致的复制器。I2 提供角色/知识/绑定完整覆盖后 I5 接入；备份 manifest 包括 DB 一致快照、附件、配置、不可变角色修订/知识、绑定、迁移与 turn ledger/幂等状态、摘要和源 schema/账号范围；密钥依既有受限备份政策保存，角色导出不含凭据。日志/校准布局/诊断数据分别按原策略，用户资源不被默认资源覆盖。备份必须验证可读/摘要及恢复兼容，不能只判断目录存在。

新安装无固定人设；旧部署先确定性抽取原内置角色与 unreadable 客户文案生成显式修订、绑定与迁移标记；重复执行不复制；原外部模式、模型、连接和超时保持原值。系统能力约束仍由代码追加，不放可编辑模板。迁移检查字段/输入快照，不要求模型自然语言输出相同。

无不兼容数据迁移可原子回退完整 release，校验配置/profile/schema 读范围。不可兼容迁移须停服务、保留新数据副本、核对备份与数据差异后走明确恢复步骤；新版本产生业务记录后不得自动恢复旧 DB，UI 显示影响时间窗/记录范围/附件及未决发送。备份缺失、摘要不符、权限或自检失败时保持维护暂停，不能“恢复成功”或启用自动回复。回退不恢复真实消息外部副作用。

发布者验签、本机重签名、TCC 权限保持三项分别验收。本轮没有签名、权限授权或真机安装证据；签名身份/路径稳定不保证录屏/辅助功能免重授权。权限缺失时保持暂停，设置页沿原指引重新授权再复检。

## 9. 文件所有权及实现 gate

| 任务 | 独占/受控文件 | 交接与约束 |
|---|---|---|
| I3 | 新 BrainPage.swift；ConfigForm.swift、Shell.swift、StatusPanel.swift、McpPage.swift、SessionsPage.swift；新增 native/tests/BrainContractTests.swift 与 native/tests/fixtures/brain/（按D1提议，派发时冻结并先核实构建接入）；纯逻辑/fixture 测试 | D1/D2 批准且 I2 集成后；main.swift 仅大脑页注册及必需局部接线已由 msg_3f52a10ab839 批准，禁改启动/信号/supervisor；Shell.swift 现有register足够，不预定改动；不复制 Rust 校验/迁移，不改连接语义 |
| I4 | CoreSupervisor.swift、Supervisor.swift、Actions.swift、scripts/deploy_host.sh、生命周期 fixture 测试 | D2/Q0 后；先隔离复现；生命周期接口交 I5/I6，不并改 UI 公共文件 |
| I5 | 新更新模块/独立 updater（具体 Rust 模块路径由协调者冻结）；scripts/build*.sh、install.sh、release.yml、版本资源/测试 | D2、I2、I4 已集成且分发/信任契约批准；不修改 I4 文件，必要 Cargo/入口变更须明确文件清单批准 |
| I6 | 新 UpdatePage.swift；Shell.swift、StatusPanel.swift、SettingsPage.swift、main.swift、fixture 测试 | I3 公共文件集成后串行；调用 I4/I5，不实现第二 updater/supervisor |

I3→I6 对 Shell/StatusPanel/main 串行；I4 生命周期冻结后 I5 接入；I5 与 I6 状态模型单一来源。D1 的存储/schema/公共接口由协调者另行批准；本设计没有批准任何生产公共接口变更。

## 10. 测试矩阵与首版边界

| 层/责任 | 必要用例 | 环境/通过证据 |
|---|---|---|
| I3 表单 | 空态、旧版迁移显示、保存/运行分离、未知字段保留、CAS 冲突保留输入、加载失败、归档绑定、账号切换迟到响应 | 内存 adapter/临时路径；先 RED 后 GREEN；不调用真实 ConfigForm HOME 保存 |
| I3 绑定/知识/模板 | 白名单唯一来源、重名/失效引用拒绝、群/私聊规则不变、知识摘要固定、超预算/未知预算、未知变量/触发器 | D1/I2 合成 fixtures；Swift 与 Rust 契约 golden 对齐 |
| I2/I3 simulation/history | 不支持模拟禁用、不触手/真实队列/账本/统计、未调用模型标签、旧 provenance unknown、账号隔离 | 合成消息/附件；记录禁止副作用断言 |
| I4 生命周期 | 主动/正常/异常/config2、5 次真实崩溃、挂起/超时 kill、人工暂停、TERM 实际 reason/status、host tick 竞争、在途发送延期 | 独立 sleep/fixture 子进程，不 launchctl/GUI，不真实 core |
| I5 信任 | 篡改清单/包、未知/撤销/过期 key、轮换双签、旧序列/同序异摘要/时间回拨/跨渠道、最低 updater | 本地测试密钥/合成包，不用生产私钥或网络发布 |
| I5 安装 | 错架构/OS/schema/组件缺失、zip-slip/链接/权限/炸弹、磁盘不足、锁竞争、签名失败、每 journal 阶段中断 | 临时 state/config/DB/release root；故障注入 + 旧版摘要仍完整 |
| I5 备份恢复 | DB/附件/角色/知识/绑定摘要、迁移幂等、兼容回退、新记录后禁自动恢复、缺备份/损坏 journal | 临时 DB，验证数据与未知发送标记保留 |
| I6 展示 | 断网、未配置源、验签未完成、钥匙串锁定/缺证书、排空超时/延期、重授权、失败恢复动作 | fixture 页面；不操作系统钥匙串/安装目录 |
| Q1 集成 | 现有内外脑、白名单/忽略、幂等、handoff、群@、语音/附件、历史/备份；首条未读顶上/同时长语音定位 | 合成回归与另行授权真机分开；现有失败和 HEAD 基线比对 |
| 另行真机 | 两架构/目标 OS、TCC 录屏/辅助功能、launchd/updater 断电、连续运行 | 独立具体授权；单次测试不代替至少 7 天试点 |

最小首版：文本/Markdown 知识、结构化受支持模板、账号默认+授权联系人/群覆盖、草稿/CAS/激活/历史回退、确定性合成预览、provenance；更新为人工检查/确认、完整签名包、完整版本切换、独立 updater、备份/恢复。外部模拟无能力时禁用，不阻塞角色编辑。后续再考虑复杂检索知识库、在线模型测试、自动下载/静默安装、差分包、多节点编排；首版不提供这些入口。

## 11. 本任务验证记录

本次只新增设计与静态示意，没有生产行为变更，因此生产行为 RED/GREEN 不适用；不得把文档存在性检查称为功能测试。设计文档缺失检查 `test -f docs/design/production-ui-update-design.md` 在创建前 exit1（仅工件缺失证据）。本地静态检查及最终 commit 在交付回执记录；未运行 Rust/Swift 全量测试、ConfigForm 保存、真实 GUI、真实 DB、付费模型、签名或部署。原因：无生产源码变化，且现有保存代码会访问真实 HOME。Q0 负责既有门禁基线，此任务不复述未运行门禁为成功。

D1 备份对齐补充：当前 backup manifest v2 未覆盖角色或完整发送账本；I2 拟增 manifest v3，含 brain root/reachable revisions/激活历史、去密钥 config_snapshot、execution_state 与 history brain_turns。I5 只能依赖集成后 v3，不声称现状已覆盖。旧 v2 恢复对已存在角色/新记录必须 restore_incomplete，不混用新根与旧 DB；历史 migration 5 是否可用由协调者在集成基线重新分配。

## 12. 交付校准与实际检查

协调者补充 `msg_8d033aceb824`：D1 初稿已提交 `6e626930101d5721b7b817b4ce78a74da69a16ee`，仍待角色/知识账号所有权、完整根 schema（activation_history/request_results/legacy_fallback）、请求去重 scope 的补充审查。D2 **按账号隔离**：角色列表、知识、复制、版本、预览、绑定和详情都限定当前账号；跨账号对象返回 not_found，不能因全局 roles 根结构就展示其他账号角色。UI 不先行实现未冻结 ownership 字段；补充 commit 到位由协调者在 I3 gate 校准。I1 仅提供存储/迁移函数，不自动接入启动；启动接入由 I2 审查，D2 不增加启动行为。

本地检查实际记录：

- `git rev-parse HEAD` / `git status --short`：exit0，HEAD与派发一致、初始无diff。
- 创建前 `test -f docs/design/production-ui-update-design.md`：exit1，仅工件缺失，不是行为RED。
- Python标准库 HTMLParser 静态检查（只读两个新文件）：exit0；拒绝script/iframe/img/form、事件handler和外部src/href，所有button均disabled；关键设计映射词存在。没有访问 HOME、GUI、DB或网络。
- `git diff --check`：exit0。新增文件未被普通diff覆盖，提交暂存后还须执行cached检查。
- 未运行浏览器渲染/VoiceOver/原生UI测试；布局为静态设计示意，不能作为像素、可访问性或生产流程验收。未运行生产测试，RED/GREEN不适用；未对既有基线失败作新归因。

本任务交付不阻塞独立大脑开发；D1补充契约冻结、更新分发/信任根与安装路径选择、I4隔离复现、TCC真机验收仍为对应实现/上线gate。

提交检查记录：首次 commit exit1，隐私钩子拒绝两处本机绝对路径；已改为相邻 worktree/仓库相对引用，未绕过钩子。
