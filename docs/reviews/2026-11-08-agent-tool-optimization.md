# 微信 MCP 作为 Agent 工具的分析与优化方案

日期：2026-11-08。范围：当前工作区源代码、默认 stdio MCP 入口及本地隔离验证。不涉及真实微信收发、上线或公共接口变更实施。

## 结论

项目应定位为可靠的微信操作服务：负责观察、会话定位、消息轮次、发送核验和恢复；外部 Agent 决定业务处理和回复内容。继续复用 Rust core、Swift worker、SQLite 和 MCP，不建议另起一套 RPA 或把知识库塞入驱动。

当前底座已有价值：账号隔离、SQLite 发送幂等、持久事件及消费租约、桌面互斥、取消控制、OCR 数据来源及可信边界说明。首要工作是打通对外契约和运行路径，再提高工具易用性，最后优化性能和扩展性。

## 已验证问题与影响

| 优先级 | 证据 | 对 Agent 的影响 | 建议 |
| --- | --- | --- | --- |
| P0 | src/tools.rs:122 将 brain/history 工具加入 tools/list；src/gateway.rs:81 的 execute 没有对应分支。隔离进程实际列出 46 个工具，next_turn 与 history_list_contacts 均返回未知工具 | Agent 能发现却无法调用，客服接管与历史检索闭环中断 | 建立统一工具注册及分发，接通已有 brain/history handler；不可用依赖应返回明确的能力错误 |
| P0 | .github/workflows/ci.yml 仍调用 tests/protocol_smoke.py、tests/desktop_runtime_smoke.py；工作区实际为对应 .rs 文件 | CI 验证入口失效；模块测试通过也不能证明工具可用 | 更新 CI 与 README 命令，新增经过真实 stdio 入口的注册/路由契约测试 |
| P1 | src/gateway.rs:70 成功有 structuredContent，错误只有自然语言；发送路径还保留“未触发发送”的字符串匹配 | Agent 无法稳定判断能否重试，可能错误重复发送 | 使用类型化错误和一致结果模型，明确 retryable、side_effect、send_id、恢复动作 |
| P1 | src/storage.rs:143 允许已证明 not_sent 的相同幂等键重新 dispatch；src/tools.rs:154 却描述为重复键不会再次触发 | Agent 对幂等的理解与实际行为不一致 | 明确“同一业务意图去重；已证明未发送时允许重试”，未知结果始终禁止自动重发 |
| P1 | src/adapters.rs 使用 DesktopActor/旧 wechat 发送；src/core/scheduler.rs、worker_client.rs 又存在常驻调度/worker 路径 | 两套入口可能争用窗口或执行不同核验策略，需逐路径验证 | 单一常驻核心拥有桌面操作权；MCP 为薄客户端，临时操作也进入同一调度队列 |
| P1 | src/core/external_brain.rs 领取使用连接级 client_id；src/core/brain_tools.rs 每次调用新建 socket；reply 仅返回 accepted | 多 Agent 所有权、断线恢复和最终发送结果难以统一判断 | 审计跨连接领取语义，补稳定 consumer 身份、租约、结果查询；accepted 与最终发送分开 |
| P2 | 默认 46 个工具，含 5 个旧别名、CRM、监听、文章、历史及客服工具；本地 tools/list 中 23 个有 annotations、0 个有 outputSchema | 选工具和解析输出成本高；类似工具易混用 | 按用途提供工具集配置，补齐契约和注解，保持老客户端兼容 |
| P2 | history_get_history 描述为“完整历史”，而微信采集存在可见区域、启动时间和连续性限制 | Agent 可能把本地已采集历史当成微信全量历史 | 改为“本地已采集历史”，返回采集区间、缺口、来源及截断信息 |
| P2 | upload_file 用 base64；history_get_media 可能返回至多 10MB base64 | 文件经过模型上下文，成本和响应体偏大 | 支持独立附件传输/资源引用，工具默认只返回摘要与元数据 |

P0/P1 表示本方案处理顺序，不表示上述路径已发生真实误发。双路径互斥和跨连接领取属于需要补充集成验证的风险，不作为已复现事故。

## 建议的运行架构

```text
外部 Agent：Codex / Claude / 自定义业务大脑
          │ MCP（工具发现、调用、结果查询）
          ▼
薄 MCP 网关：统一工具注册 / 参数和输出校验 / 权限 / 请求追踪
          ▼
常驻 Rust Core
  ├─ 观察与消息轮次：采集来源、连续性、方向及置信边界
  ├─ 会话及桌面调度：唯一操作队列、优先级、超时、暂停与接管
  ├─ SQLite：事件、轮次、发送意图、尝试和结果
  └─ Swift Worker：唯一执行微信 UI 操作
```

本架构复用已有 serve-core 和 worker，不重新实现。先明确两条现有路径的所有权，再渐进收敛。SQLite 当前有多类存储，先厘清各类事实的唯一归属；不要求第一阶段合库或重构表结构。

外部知识库、模型提示词、客户业务规则留在 Agent。微信服务执行目标约束、发送核验、限流、暂停和幂等，不能仅靠 Skill 文案保证。

## 面向 Agent 的两种流程

### 临时操作

1. 查询运行健康、账号绑定及当前能力。
2. 解析会话候选；同名或身份不确定时返回歧义，不猜选。
3. 根据任务读取当前可见内容或本地历史，明确两者覆盖范围。
4. 用户已授权发送时，以同一业务意图的幂等键发送。
5. 查询发送状态并据实报告；未知结果进入核对流程。

建议新增的会话引用应绑定账号、目标证据和有效期；它只提高定位安全性，不宣称已验证微信真实身份。沿用现有授权，无需每条消息重复询问。

### 常驻自动回复

1. 服务负责监听、去除自身回显、聚合连续的一轮消息。
2. Agent 通过 next_turn 领取，获取上下文、附件引用、deadline 和覆盖边界。
3. Agent 通过 reply 选择 send / skip / handoff，收件人由服务绑定。
4. 服务记录接受状态并执行发送，Agent 可查询最终结果。
5. Agent 断线、超时、服务重启后通过稳定轮次身份恢复，不能靠再次生成正文去重。

常驻客服场景优先推荐轮次接口；显式 send_message 保留给用户要求的临时发送，两者遵守各自授权并共用发送执行层。

## 工具契约优化

1. 建立单一 ToolSpec 注册表，覆盖 name、input schema、handler、输出契约、注解、能力依赖和副作用类型。tools/list 和 tools/call 从同一来源生成。
2. 工具集采用连接启动配置，例如 operator、responder、history、crm、article、legacy；默认精简，legacy 作为兼容配置逐步退场，不能直接删除现有工具。
3. 补齐机器可读错误：INVALID_ARGUMENT、CORE_UNAVAILABLE、TARGET_AMBIGUOUS、UI_BUSY、PERMISSION_REQUIRED、SEND_NOT_TRIGGERED、SEND_OUTCOME_UNKNOWN 等。Agent 依据 code 和实际副作用状态决策，不解析中文报错。
4. 输出包含 schema_version、request_id、account、data 或 error；error 提供 retryable、side_effect 和恢复建议。保留现有文本输出供旧客户端使用；兼容迁移后再收紧输出结构。
5. 发送区分“请求已接受”“未触发”“已触发”“观察到客户端回显”“结果未知”。UI 回显不是网络送达，不能升级为 delivered。
6. 输出添加覆盖信息，例如 visible_only、collected_history、observed_at、continuity、truncated 和 unknown 字段；unknown 不能默认为 false。
7. 补齐工具 annotations，但这些只是客户端提示；实际授权和防护在服务端执行。读取聊天会改变 UI 焦点，不能因为名字是 get 就认定完全无副作用。
8. 历史默认分页和摘要，媒体按需取详情；输入端设置行大小、正文、附件和批量条数限制，避免无界读取及大响应。
9. 提供只读 health/capabilities 入口，聚合 core、worker、权限、窗口、OCR、监听和队列状态，区分配置存在、进程在线、界面可操作。

以上涉及新增公共接口的部分属于待评审设计，本轮不实施。第一阶段可以先接通现有接口、更新描述和修复测试入口。

## 分阶段交付与验收

| 阶段 | 范围 | 主要文件 | 验收条件 |
| --- | --- | --- | --- |
| A：先修可调用性 | 接通 brain/history；纠正 CI；修正幂等文案；建立工具注册契约测试 | src/tools.rs、src/gateway.rs、src/main.rs、.github/workflows/ci.yml、tests/protocol_smoke.rs、README.md | 所有已公布工具都有 handler；缺服务返回明确依赖错误而非未知工具；真实 stdio 验证 next_turn、reply、历史查询；CI 不引用不存在脚本 |
| B：稳定 Agent 契约 | 统一错误；发送结果查询闭环；工具集；健康状态；补齐 schema/annotations | src/mcp.rs、src/gateway.rs、src/core/brain_tools.rs、src/core/history/mcp_tools.rs、skills/wechat-operator/ | 未触发/未知结果的重试行为可验证；同键不同载荷被拒；Agent 无需解析自然语言报错；历史覆盖不被误报 |
| C：统一运行与恢复 | MCP 路径收敛到 core；桌面唯一调度；稳定消费者及租约；轮次和回执恢复 | src/core/serve.rs、scheduler.rs、external_brain.rs、worker_client.rs、src/adapters.rs | 多客户端并发操作不交叉；轮次领取/回复断线可恢复；发送前后故障注入不引发未知结果自动重发；用户切换窗口时安全停止 |
| D：性能与实机推广 | 附件引用、预算、指标、回放样本和实机矩阵 | 历史/媒体工具、src/core/replay.rs、安装及发布脚本、research/ | 有可重复的 P50/P95 数据；长轮询不阻塞健康查询；覆盖亮暗主题、缩放、窗口变化、OCR 超时和恢复；未知结果有待处理清单 |

阶段 C 风险最高，应在阶段 A/B 基础上按单条操作链渐进替换，保留可回滚开关。工期需按真实部署和实机矩阵拆解后估算，不按文件数量承诺。

## 测试策略

- 契约层：tools/list 与实际分发一致；成功及失败均校验结构；对条件不可用工具用假服务验证路由。
- 故障层：回车前失败、回车后断线、回执落库失败、进程重启、重复请求、同名目标、租约过期、两消费者竞争。
- 调度层：监听、临时读取、人工切换、发送同时发生时，操作权明确；取消请求不会留下半完成的副作用。
- Agent 场景层：历史查询、明确目标发送、未知发送核对、收到消息后回复、人工接管及恢复；记录工具选择正确率、无效调用次数和完成率。
- 实机层：使用授权测试联系人与固定样本，记录操作耗时和证据。单元测试不替代微信实机验收。
- 指标：请求排队时长、采集耗时、轮次等待、发送核验耗时、未知结果比例、恢复积压、每轮工具调用数及输出体积。未具备可靠入站真值前，不宣称每条消息零丢失。

## 本轮验证与边界

- cargo build --locked --quiet：通过，存在编译警告。
- 隔离 SQLite 和账号配置启动 target/debug/wechat-mcp：tools/list 返回 46 个工具；23 个含 annotations，0 个含 outputSchema。
- 同一真实 stdio 入口调用 next_turn(wait_seconds=0)、history_list_contacts：均返回 isError=true，内容为未知工具。未调用发送工具，未启动监听。
- cargo test --locked --quiet：退出码 0，759 项通过、1 项忽略。测试过程包含 Swift 辅助程序自测；不等于真实微信收发通过。
- 未执行真实微信操作、部署和远端 CI。保留工作区原有修改，不修改业务实现。
- 当前会话没有 Orca Task/Dispatch ID，无法把本方案绑定到一个已有 Orca 调度任务完成回执；不编造任务 ID，也不创建额外开发任务。
