已只读审查提交 d783e81 的 serve-core、scheduler、turns、worker/MCP 客户端、外部大脑、备份、历史 HTTP、帧录制、control 和 Swift 宿主管理，并对照 Python 实现；当前 checkout HEAD 为 81ca610 而非目标提交，全部代码证据均取自 git show d783e81，未改文件、未运行会写入状态的测试、未提交或推送、未连接 192.0.2.10。

发现如下，按严重度排序（P1 为安全边界失效或核心功能不可用，P2 为条件触发的故障或可恢复性/可诊断性缺陷；“已确认”只代表代码证据确认，不代表真机复现）：

1) P1【已确认】外部大脑工具只注册、未分发
- 证据：src/tools.rs:131 将 brain_tools::tools 加入 tools/list；src/gateway.rs:81-170 没有 is_brain_tool/execute 分支，最终进入 :170 的“未知工具”；src/core/brain_tools.rs:100-107 的 execute_at 是孤立实现
- Python 对照：scripts/wechat_bot/reply.py:73-90 是直接调用内置大脑，Python 无此新增外部大脑 MCP 功能，不能以“Python 本来没有”替代 Rust 接线验收
- 真机后果：客户端能看见 next_turn/reply/resume_session，却全部 tools/call 失败；external 模式只能超时 skip/handoff/builtin，正式外部大脑无法工作
- 建议：在 Gateway 通用账号工具匹配之前分发 brain_tools，并从真实 handle_request → Gateway → core.sock 做契约测试，不能只测 execute_at 或 tools/list

2) P1【已确认】默认 codex-exec 大脑生产实现缺失
- 证据：src/core/config.rs:316-336、:680-683 接受并默认 codex-exec；src/core/turns.rs:1417-1420 → src/brain/builtin.rs:129-149 无条件调用 model_api::generate_api；src/brain/model_api.rs:360-374、:471-472 无条件 requester → request_completion，:263-280 使用 API key 和 base_url/chat/completions，没有 codex 子进程分支
- Python：scripts/wechat_bot/reply.py:73-97 对 openai-compatible 和 codex exec 明确分支，后者带只读 sandbox、禁工具、输出 schema、180 秒超时
- 真机后果：config-check 接受的默认配置在首轮回复时因缺 key/空 base_url 失败，重试后暂停；external timeout_action=builtin 同样受影响
- 建议：补齐 Codex 执行后端及 Python 安全参数，或在配置加载时明确拒绝该后端并调整默认；验证实际 BuiltinBrain，不只注入 BrainFn

3) P1【已确认】SIGTERM/SIGINT 未实时传入发送前 stop 检查
- 证据：src/core/serve.rs:17-20 handler 只写 STOP_SIGNAL；:196-200、:212 在 sweep 前才拷贝到 coordinator.stop，sweep await 期间无同步；src/core/scheduler.rs:554 child.stop 共享的是 coordinator.stop；src/core/turns.rs:1049、:1466 只读 child.stop
- Python：scripts/wechat_bot/coordinator.py:141、:493-494 信号直接设置所有 child 共用的 self.stop；service.py:597-607 在生成后及发送前读取它
- 真机后果：宿主 stopProcess 发 SIGTERM 后，正在等模型/外部大脑的一轮仍可发送，且本次 sweep 后续会话也可继续处理，直到整轮返回；240 秒宽限期使窗口很长
- 建议：信号任务立即设置同一个共享取消标志，并使外部等待/延迟可取消，保留发送前最终检查；测试真实 OS 信号在生成期间到达

4) P1【已确认】发送前关键持久化失败仍继续真实发送
- 证据：src/core/turns.rs:1071 忽略 attempting journal 写入错误后 :1076 发消息；:1489-1498 忽略 record_bot_reply 错误；:660-664 忽略 replied_after 查询失败并继续；这些都在安全决策链上
- Python：service.py:614、:676-679 的写入会抛错阻断发送；:432 replied_after 失败也向上传播
- 真机后果：磁盘满、SQLite 故障或权限变化时，无可恢复 attempting 证据仍触发 UI；人工/先前回复检查失败可重复回应；历史中缺失发送关联
- 建议：try_send_once 返回 Result，在任何 UI 调用前成功持久化 attempting；record_bot_reply、replied_after 错误必须阻断本轮，journal→history 镜像更新可降级但必须记录并修复

5) P1【已确认，已知且另一 worker 正修】附件生产适配器返回全空
- 证据：src/core/scheduler.rs:328-344 的 CaptureAdapter::attachments 恒为三个空 Vec；:803-805 正式 visit 使用它；src/core/turns.rs:1332 因而从未拿到附件或 failed
- Python：scripts/wechat_bot/native_capture.py:283-349 执行 prepare_image/prepare_file、下载、正文/页面提取、逐项失败标记；service.py:585-587 向模型传递
- 真机后果：图像和文件只剩聊天占位文本，失败项也不提示重发；测试 MockCapture 实现掩盖生产缺口
- 建议：由负责 worker 完成生产 CoordinatorCapture→WorkerClient→文档处理全链，验收不能只补 MockCapture

6) P1【已确认】control start/stop/calibrate 仍操作 Python launchd，而生产由 core.enabled + CoreSupervisor 管理
- 证据：src/core/control.rs:12、:449-455、:602-704 固定 local.wechat-rpa.bot；:913-915 以这个 Python job 是否 loaded 决定能否校准；native/bootstrap/CoreSupervisor.swift:225、:271-275 读取 core.enabled，:154-155 在 Python loaded 时 conflict
- Python：scripts/wechat_bot/control.py:211-225 的 launchd 管理适用于旧 Python 部署
- 真机后果：control stop 可返回 stopped 但 Rust 仍自动回复；control start 若旧 plist 存在会启动 Python 并触发宿主停 Rust；Rust 正常运行时 calibrate 报“机器人未运行”
- 建议：Rust control 对接 core.enabled/宿主生命周期，status 区分 Python 与 Rust；不应默默管理旧 Python 服务；启动/停止成功需核实目标进程终态

7) P1【已确认】校准/接受校准是伪成功
- 证据：src/core/scheduler.rs:1437 获取 Swift calibrate 原始测量；:1452-1467 仅拼 accepted=true；:1470-1483 虚构 native/layout/candidate 文件路径、空 differences/missing，从未调用已有 control::run_calibrate/calibrate_accept
- Python：coordinator.py:471-486 调 run_calibrate/calibrate_accept；native/bot/Worker.swift:386-420 只测量并返回 layout/measured/PNG，不会写 candidate 或 active
- 真机后果：即使绕过第 6 项 loaded 检查，也显示无差异/接受成功而根本未生成或激活布局，重新定位永远不生效
- 建议：接上 control.rs:256-339、:342-429 的候选写入、校验、备份和原子替换，并返回真实绝对路径；测量有差异的集成用例核实文件内容变化

8) P1【已确认】未知发送回显检查丢弃标题校验结果
- 证据：src/core/turns.rs:1574-1578 的 policy.check_frame 返回值被 let _ 丢弃；识别回显后 :1607 保存该帧，:1679-1685 对 after_frame=Some 的路径跳过检查；正常成功后 :1235-1237 的延迟回显帧也无 policy 检查
- Python：service.py:650-664 在同一 try 内 check_frame，错误算不可验证；:828 也检查最终帧
- 真机后果：错误会话/无效帧中的同文出站气泡可能被当成本联系人的成功回显，错误帧可推进 cursor，破坏后续连续性与人工接管判断；当前 Swift capture 有标题门禁降低概率，但 Rust 独立防线确实失效
- 建议：所有用于回显或推进游标的帧必须通过 policy，失败保持 unknown，禁止将校验失败当作 NotSeen 或 Seen

9) P2【已确认】运行时临时故障全部 exit 2，宿主永久按配置错误停机
- 证据：src/main.rs:75-80 将 run_serve_core 任意 Err 映射 exit(2)；src/core/serve.rs:96、:103、:116 之前的锁/socket/DB 初始化错误均可产生 Err；CoreSupervisor.swift:382-386 对 exit 2 设置 configError 且 :173-174 不自动重启
- Python 对照：coordinator.py:503-513 对循环故障保持运行重试；Python 无此 Rust 宿主退出码契约
- 真机后果：短暂 socket bind/锁争用/数据库 I/O 故障与配置错同等对待，环境恢复后仍停机
- 建议：结构化区分配置错误与运行错误，仅配置错误 exit 2，其余非零退出触发宿主退避；启动依赖未 ready 做有限重试

10) P2【已确认】coordinator 事件只存测试用内存 Vec，无生产输出且无限增长
- 证据：src/core/scheduler.rs:373、:499-505 log_event 只 push logged_events；service_started/session_probe_failed/history_http_failed/backup_failed 等都走这里；未见生产消费或清空
- Python：service.py:31-33 logger.info 带时间结构化输出；coordinator.py:339、:499-511 调用同一日志函数
- 真机后果：core.log 看不到这些关键失败，长期运行事件堆积内存；“服务活着但未接线”更难定位
- 建议：结构化输出到 tracing/stderr，Vec 限定 cfg(test) 或有界；将错误原因和时间写入生产日志

11) P2【已确认】历史文件后台下载/归档错误逃出局部任务，不增加重试计数
- 证据：src/core/scheduler.rs:1245、:1247-1254 直接 ?；上层 :1370 使 sweep 失败，serve.rs:212-224 以 routing_config_failed 关闭连接；history/store.rs:913-929 只有 finish_media_job(failed) 才增加 attempts/退避
- Python：coordinator.py:293-310 将 prepare、download、attach 整段捕获，finish failed 并保留机器人服务
- 真机后果：同一损坏附件可每轮重复下载/关闭重连，不进入三次上限；无关会话也受影响，错误被误报路由配置失败
- 建议：覆盖整个后台任务的错误边界，确保失败更新 job attempts，只有核心状态损坏才传播到 sweep

12) P2【已确认】Python 历史维护命令未接入 Rust control
- 证据：src/core/control.rs:589 支持列表及 :1055-1057 unknown 分支，无 migrate-history/history-status/merge-contact/unmerge-contact/history-backup；scheduler.rs:414 却要求用户运行 control migrate-history；history/migrate.rs 等库存在不等于 CLI 可达
- Python：scripts/wechat_bot/control.py:128-136、:179-180 注册并分发这些命令
- 真机后果：遇 legacy session directories 按错误提示无法恢复；切 Rust 二进制后不能完成既有联系人维护流程
- 建议：接入现有 migration/history store 实现，或错误提示明确仍需哪个可用工具，CLI 级验证而非函数单测

13) P2【已确认】外部大脑 MCP bridge 每次调用断开连接，导致领取立刻被回收
- 证据：brain_tools.rs:213-247 每次建立局部 UnixStream，返回即 drop；external_brain.rs:277-291 将 claim 绑定 client_id；:528-529 断开即 on_client_disconnect，:423-429 清除 claimed_by
- Python：无外部领取功能对应，Python service.py:585-590 是单轮直接等待生成
- 真机后果：A 调 next_turn 得到一轮后，其 MCP 调用返回即释放领取，B 下一次 next_turn 可领取同一轮；浪费模型调用、不同代理抢答，迟到答复收到 unknown_turn；服务端 oneshot 仍限制只接受一份，不等同于必然重复发送
- 建议：复用持久连接或给 claim 独立 lease/token，并验证 reply 归属；通过真实 Gateway MCP 连续 next_turn 测试多客户端互斥

14) P2【已确认】内置/外部大脑附件数据边界回退，文件 path/bubble 被整体序列化
- 证据：turns.rs:1401 对 AttachmentFile 全字段 serde；AttachmentFile :87-94 包含 path 和完整 bubble；brain/builtin.rs:90-97 原样放入 files prompt
- Python：service.py:56-58 的 model_view 明确删除 path/bubble，:587 调用它
- 真机后果：附件链修好后，本机路径和视觉几何/元信息会随文件正文发往模型；当前附件全空使此问题暂时潜伏
- 建议：建立独立文件模型 DTO，只允许 filename/正文/截断/提取状态，外部大脑按需求明确字段，不直接透传内部结构

15) P2【已确认】模型 provenance 在实际回复路径被丢弃
- 证据：turns.rs:1423-1427 仅从 BrainReply.metadata 读 action，:1460-1464 journal 无 model；:1494 用空对象入库；brain/builtin.rs:134-153 确实生成 backend/provider/model/seconds metadata
- Python：service.py:589-595 保存 model_completed、record.model；:678 持久化 model 信息
- 真机后果：无法核对实际用了哪个模型、provider 和耗时，备份/历史中也失去这部分证据
- 建议：过滤掉控制 action 后把 metadata 写 journal 和 record_bot_reply，并保留 timings

16) P2【已确认】生产媒体 digest 校验残留测试后门
- 证据：worker_client.rs:660 sha256_expected == "dummy" 时直接跳过摘要校验
- Python：native_capture.py:270-272 无条件验证 SHA256；Swift Worker.swift:588-594 返回真实摘要
- 真机后果：合法 Swift 当前不会返回 dummy，正常链路不触发；但异常或替换 worker 可将损坏内容作为成功下载，生产代码不再满足协议校验契约
- 建议：移除特殊字符串，测试生成真实 fixture digest；补损坏分块契约测试

需运行/真机验证的疑点（不作已复现结论）：
A) 外部模式热加/重命名会话在首个 sweep 缺 BrainFn：serve.rs:202-208 仅给已有 sessions 注入，scheduler.rs:1319 会在 sweep 中创建 child，turns.rs:1417 会退到 builtin；若该 child 从磁盘恢复已到期 turn，首次 visit 即可能越过外部大脑；建议把 brain factory 注入 session 创建而非下一轮补齐，测试恢复 pending turn 后热加目标
B) 启动未检查其他 observer：Python coordinator.py:203-204 检查 get_listener_status(enabled) 后拒绝并行观察，Rust serve.rs:118-138 无等价检查；需在同机 listener enabled 场景验证 UI/游标相互干扰，补生产准入检查；CoreSupervisor 的 Python job 冲突检测不能代替 gateway listener 检测
C) admin pause 在模型生成中到达：scheduler.rs:1262 仅 sweep 开头读 marker，turns.rs:1037-1055 发送前只检查 stop/routing；Python coordinator.py:396 起的 sweep 亦仅轮前暂停，为沿袭缺陷；若 pause 的产品语义要求立即禁止尚未触发的发送，需要将 marker 纳入发送前门禁并真机验证
D) sessions 名称规模不匹配：config 允许 200 会话及每个 10 aliases，scheduler.rs:1013-1020 一次发送全部候选；Swift Worker.swift:457-458 限 names<=50；超过时 probe 必然失败且退化全量访聊，需验证长 allowlist 的轮询延迟和窗口干扰；Python 也有同批探测，应分批或统一限制
E) 自动备份失败后的空目录被当成最新成功备份：backup.rs:208-224 先建目录再 backup，:251-270 只按目录 mtime 恢复 last_backup_at；重启可能推迟下一次成功备份 24h；需用 DB 只读/锁冲突模拟，成功 manifest/临时目录原子改名后才计时
F) 备份的 gateway DB 依赖部署环境：backup.rs:181-189 默认 ~/.wechat-rpa/test.sqlite3，而 Gateway::load 默认 Library/Application Support/message-gateway/service.sqlite3；当前宿主 SocketWorker.swift:149 明确注入 test.sqlite3，故宿主部署的默认备份目标是匹配的，但单独 CLI/自定义 gateway 配置可能备错库；需从真正 worker 的活跃配置导出 DB 路径，不能把 core 进程环境当 worker 环境

已经正确的关键路径（仅代码接通结论）：
- main serve-core → run_serve_core → 真实 Arc<McpSocketClient> 注入 tool_client（serve.rs:118-138），scheduler Arc<dyn DynToolClient> 的 ToolClient 实现真实转发（scheduler.rs:64-70），已没有对全工具返回空对象的生产假客户端
- close_capture 保留注入 capture 对象并清 active_name（scheduler.rs:664-674），connect 对 disconnected 执行 prepare_target 后 start（:714-719），WorkerClient::start 拒绝空 recipient（worker_client.rs:252-258），quiet relaunch 的 get_messages 错误会 warn（:329-333）
- 白名单/ignorelist 在每轮 rules、visit、生成前、发送前复核；默认配置 host=local 强制本机；标题检查与 Swift capture/scroll/read_media 额外门禁接通，错误回显分支例外见第 8 项
- Worker 请求 protocol_version/id/operation/params 与 Swift 对齐（worker_client.rs:422-479，Worker.swift:383-385、:690-691）；sessions 的 rows/name/y/preview_digest/key/time/red_pixels 字段与 Sessions.swift:91-98 匹配；read_media 的 64KB 分块/offset/size/extension/eof/sha256 与 Rust 解析匹配，dummy 例外见第 16 项
- MCP 初始化、initialized、ID 核对、isError、structuredContent/text 解包、account 校验和 data 取值已接上；出错丢弃 transport，下次自动重连（mcp_client.rs:127-249）
- gateway begin_send/finish_send 持久化与相同 idempotency_key 重放（gateway.rs:252-329）是真实链；未知结果后 Rust 虽再次调用同 key，gateway 对 unknown/triggered 重放不会再次触发 UI，只有明确 not_sent 可重新调度；不能将任何 core 侧重试一概报告为必然重发
- turns 收集/quiet/max/上传等待、历史先于模型记录、历史读取失败保留 turn、人工出站拦截、停止前 turn 恢复、handoff marker 和 resume-session 的 core/file 路径均有实现；关键错误吞掉的例外见第 4 项
- 历史 HTTP 实际在 serve-core 启动，默认 loopback，Bear token 验证、GET-only、参数限制、no-store 接通；历史端口占用仅影响辅助查询；HTTP 每连接线程/无读取超时与 Python ThreadingHTTPServer 类似，本次未把这种继承行为列为 Rust 新回归
- frame-dump.enabled → WorkerClient capture/scroll → sessions/<id>/frames-YYYYMMDD.jsonl 链接通，0600、50MB/day、旧日清理和失败 warn 均实现（worker_client.rs:65-129、:510-549）；它记录结构化 OCR 帧，不应声称是原始屏幕视频
- 备份使用 SQLite online backup + integrity_check + 临时文件原子 rename（backup.rs:59-141），不是直接复制活跃 WAL 数据库；自动备份空闲门禁实际调用，失败目录计时疑点见 E
- Swift CoreSupervisor 以 core.enabled 管启停，冲突时停 Rust，SIGTERM→240s→SIGKILL、退避和日志文件权限接通；Rust 的 exit 2 与信号传递问题分别见第 9/3 项
以上为完整静态报告，未将单元测试全绿或静态接线作为真机端到端成功证据。

剩余工作是由协调者分配修复并验证上述问题，尤其先覆盖真实 Gateway→core.sock、实际 BuiltinBrain、OS 信号发送前阻断、control 宿主管理和校准落盘集成，附件空实现由现有另一 worker 继续，真机疑点需另行授权在专用 Mac 验证。