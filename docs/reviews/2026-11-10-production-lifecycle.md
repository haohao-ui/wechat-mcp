# I4 本地生命周期适配交付

本轮结果：协调者收缩放行范围内的隔离复现和本地 Swift 维护适配完成；不是完整更新、核心排空或整个项目验收通过。历史部署故障根因仍为 **unknown**。

## 基线与范围

- 初始 HEAD `32bb12e550b6739c5c7d6874bbd7388cd771ef5c`，`git status --short`、`git diff --stat` 均空。
- 按顺序 cherry-pick Q0 `1ba20e2a988d95ec88cd102f8e195f3e185afa30` → `9cca815`，D2 `afed6b1134256988b3613f5ee475ea9fab8bd133` → `9e2dfe4`；无冲突。
- 原 checkout 的任务书、任务总表和方案只读；未编辑原 checkout。
- 公共本地 Swift 契约申请 `msg_66ed22087fe0`；协调者 `msg_e7db0022b0f8` 批准，仅 CoreSupervisor.swift、独立 LifecycleTests 与此报告。
- 图发现最初命中原 checkout，已核对本 worktree 源码，并为当前 worktree 建立 `wechat-prod-lifecycle` fast 索引（不持久化到仓库）；报告的 partial parse 不涉及本次 CoreSupervisor。未使用原图代替当前源码。
- 未改 Supervisor.swift、Actions.swift、部署脚本、Rust、main、IPC、数据库或 UI 文件；未扩大白名单/改变账号隔离、模型、外脑、发送、幂等、转人工和群规则。

## 隔离复现和结论

`native/tests/LifecycleTests.swift` 仅编译 Foundation/Darwin 和两个监督器文件。所有 supervisor 启用标志、配置和日志路径显式指向独立 UUID 临时目录，pythonLoadedChecker 为固定 false；只启动 `/bin/sh` 合成循环、exit fixture 和 `/usr/bin/true`，只给自己持有的子进程发信号。未运行真实 core、launchctl、AppKit/GUI、真实微信、业务 DB、付费模型或远端操作；没有调用 HostLogger。Swift 初始化器取得 HOME 路径对象，但测试不使用默认 HOME 文件路径、不读写其中状态。

现有机制验证：

- direct TERM + shell TERM handler → exit0；连续六次不 reset 也无崩溃退避。
- direct TERM + 无 handler → 异常 signal，进入退避；这仅证明此 fixture 的行为。
- stopProcess 主动 TERM 与超时 SIGKILL → 排除崩溃计数。
- exit2 → configError，不自动重启；真异常 5 次 → gaveUp；5/15/60/60 秒退避保持。
- SIGSTOP 挂起子进程在维护停止中可由超时 KILL 退出并确认；人工 disabled 开关在结束维护后仍保持。
- 先累计四次真异常，计划维护停止再结束，第五次真异常仍 gaveUp，证明维护没有清空崩溃历史。

D2 指出的 deploy_host direct TERM 绕过 manualStoppingPIDs 是源码事实，但有 SIGTERM handler 的子进程仍可 exit0；本轮没有复现历史真实 core 的失败，不能据此认定重启阈值错误。没有删改真崩溃保护，也没有运行 deploy_host（包括 dry-run）。按批准范围保留原部署路径；跨进程计划停止接线仍待审查。

## 提供 I5/I6 的本地 Swift API

| API | 契约 |
| --- | --- |
| `init(... maintenanceURL: URL? = nil, ...)` | 默认启用标志同目录 `core.maintenance`；没有维护文件时保留原行为；旧调用无需新增参数 |
| `maintenanceStatus` | `inactive`、`active(transactionID:)` 或 `recoveryRequired`；UI 必须单独读取，原 currentStatus 仍描述进程，不伪装排空 |
| `beginMaintenance(transactionID:) throws` | UUID 字符串恰好36字节；同ID幂等，不同ID冲突；损坏/不安全文件拒绝覆盖；持久抑制自动和直接启动，不停止/暂停/排空正在运行的核心 |
| `stopForMaintenance(transactionID:) throws` | 仅匹配事务使用现有主动 stop 通路；持有精确 Process 并观察不再运行才 `stopped`；无持有证据返回 `notConfirmed`；不承诺其他进程、锁或发送已排空 |
| `finishMaintenance(transactionID:) throws` | 当前匹配事务且此前确切子进程已确认退出才移除标志；否则 conflict/notConfirmed/recoveryRequired；不隐式启动、不 reset、不改 core.enabled；随后由既有 supervisor reconcile 按原保护和开关决定启动 |
| `CoreMaintenanceError` | invalidTransaction、conflict、recoveryRequired、notConfirmed、ioFailure(errno)；IO 失败不冒充成功 |

目录必须是当前 UID 所有的真实目录，group/other 权限为零；文件为当前 UID 的单链接 regular file、0600。O_NOFOLLOW 拒绝标志和父目录 symlink。先在受控同目录 O_EXCL 创建临时0600文件，写入并 fsync，再通过 exclusive link 发布、删除临时名并 fsync 目录；不覆盖已有事务。检查和发布标志与 startProcess 使用同一 NSLock；tick 初检后仍在 startProcess 内复检。stopLock 串行停止/完成，内部 stop 实现避免重入锁死。

硬链接发布中断可能留下双链接文件，读取会 recoveryRequired；不猜测成功或主动清除。reset 不删除维护标志。实例重建后仍抑制启动，但无原 Process 证据时 stop/finish 不能确认退出；I5 恢复方仍需受审查的其他进程/锁证据，不能用本 API 的空缓存冒充恢复成功。UUID 事务归属是本地调用契约，不是跨进程鉴权机制。

I5 应先完成已批准的真实暂停/排空/备份契约，再调用 stop；本轮完全没有提供 drain token 或 drain 成功状态。I6 需将维护抑制、进程状态、排空证据分开展示，未知就延期/恢复所需。完成接口不赋予 updater 第二套拉起逻辑；接线由后续 gate 串行处理。

## RED / GREEN 和实际命令

行为 RED 在生产修改前取得：先给旧监督器放置合法 UUID/0600 维护标志并启用 core，断言 reconcile 应 keep；旧代码仍返回 start，fixture exit **1**，失败为 `persisted maintenance blocks reconciliation after interruption`。实现后相同断言通过；随后补齐错误/权限/事务/并发/崩溃保留矩阵，最终 fixture 共88条 PASS 输出（含等待与循环断言，不是88个独立测试）。

| 命令 | 退出码 | 结果 |
| --- | ---: | --- |
| `swiftc native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests` + 执行（改前） | 1 | 上述行为 RED；编译成功，执行失败 |
| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests` + 执行（改后） | 0 | 最终88条PASS；损坏/symlink/父目录权限/实际ENOENT及EACCES、错误所有者、无证据确认、重建、人工暂停、挂起强杀与并发通过 |
| `cargo test --locked --test swift_host test_supervisor_ -- --test-threads=1` | 0 | 6 passed、0 failed、0 ignored；环境和日志权限、exit0、exit2、TERM、KILL、异步停止 |
| `cargo test --locked --test swift_host test_decision_ -- --test-threads=1` | 0 | 6 passed、0 failed、0 ignored；启停、互斥、退避、放弃与配置保护 |
| `cargo test --locked --test swift_host -- --test-threads=1 test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 5 passed、0 failed、0 ignored；原 RestartPolicy/ChildExitHandler 回归 |
| `cargo fmt --all -- --check` | 0 | 未修改/排除任何格式门禁 |
| `cargo clippy --all-targets --locked` | 0 | 既有1937 warnings（1439 duplicates），与Q0记录一致；未放松 lint |
| `bash scripts/test-gates.sh` | 0 | 原有静态门禁与负向夹具通过；tag拒绝提示为预期 |
| `bash scripts/test-gate-fixtures.sh` | 0 | Q0独立门禁夹具通过 |
| `git diff --check` | 0 | 无空白错误 |

确定性竞争测试让 tick 通过初检后停在注入 checker，主线程 begin 写维护标志，再放行 tick；断言没有创建启动日志、没有子进程，且不死锁。另有30组并发 tick/direct-start 均被抑制。竞争中的原 checkAndReconcile 日志输出一次 recoveryRequired 是预期启动拒绝，不是测试失败；返回决策 start 不应当被后续调用方当成启动成功证据，应读取实际进程和维护状态。

既有 swift_host 合成监督器入口已核对：路径来自临时 fixture 或 `/tmp/fake...`，注入 checker=false，不运行 launchctl/GUI/真实 HOME 配置；仅执行所列筛选，不运行包含配置保存或GUI的全套测试。

## 限制与后续

- Q0 基线 `core::open_api::compat_tests::ops_tools_list_unchanged` 已知失败（893 passed/1 failed），见同目录 production-baseline 报告；本轮未重跑 Rust 全量，不宣称该失败已修复。Rust和协议fixture无diff。
- 没有完成核心在途发送排空、全组件退出/锁释放、持久人工业务暂停、跨进程指定更新方协议、真实更新切换/回退、launchd/TCC 或断电真机测试。本地 fsync 与中断标志测试不等于物理断电验收。
- marker 的有效存在抑制启动；同 UID 的可信 updater 仍需遵守单事务和目录所有权契约。生产入口尚未接线，现有 deploy direct TERM 仍有前述限制。
- 没有部署、推送、合并、发布、真实微信发送、修改本机微信状态或远端诊断配置。仅本地独立分支提交本任务三文件，具体commit见worker交付回执。

## I4 补充：完成阶段 IO 失败保持维护（task_950622d99412）

补充基线为 `b3c1365cba57e6f4ba5e94843eb086024bfb3fab`，开始时工作区干净。协调者指出首轮未覆盖 unlink 成功后目录 open/fsync 失败：原 finish 抛错但 marker 已不存在，维护状态可变成 inactive。**首轮 IO 测试只有创建/读取等局部错误覆盖，不是完整 IO 失败矩阵；不能据首轮交付宣称完成阶段失败安全。** 本节记录此次修复，不覆盖上一节明确列出的其他未验证边界。

推荐并实现的最小语义：仅目录同步成功才完成 finish；unlink 或后续同步失败即在原同步锁内锁存 recoveryRequired，保留原错误，尽力恢复原事务标志。即使恢复标志成功，当前实例也不自动重试完成或解除维护。没有新增公共方法、公开状态或 production 文件；内部 syscall 闭包只为本地故障注入，正常路径继续调用 Darwin unlink/open/fsync，没有第二套 updater。图查询 finish/sync 未返回符号，已明确降级核对本 worktree 源码。

实现变化：

- `maintenanceCompletionFailed` 是实例内不可自动清除的故障锁存；maintenanceStatus、startProcess、checkAndReconcile 和 owner 检查共用该状态；reset 不清除。再次 begin（同ID/新ID）、finish、stop 均无法绕过恢复要求。
- unlink 失败时保留现有标志并尽力同步目录；unlink 成功后 open/fsync 失败时，复用既有受控0600临时文件/exclusive link/fsync 发布逻辑恢复原UUID标志，不覆盖其他事务或 symlink，不清除停止证据和崩溃保护。
- 内部只读诊断 `maintenanceRecoveryPersistenceConfirmed` 记录恢复操作是否走完同步；恢复无法确认则 stderr 记录 marker durability unconfirmed，但仍抛出最初的完成错误，不让后续 EACCES 等覆盖原 EIO/EMFILE。
- 该诊断是 syscall 结果证据，不是物理断电保证。持续 open/fsync 失败时可能已恢复可见标志但持久化未知；磁盘/目录不可写时甚至无法重建标志。此时当前实例仍 fail closed；实例销毁后内存锁存不可能持久存在，外部 journal/恢复入口必须在自动启动前提供独立证据。本轮不新增此跨进程恢复协议。

先 RED：仅加入默认行为不变的内部 syscall seams 和故障 fixture，未加恢复修复；完成时注入 EIO，验证已 unlink，再断言 recoveryRequired，执行退出 **1**：`FAIL: completion IO failure stays recoveryRequired: sync`。前88条历史断言保持通过，失败发生在新完成阶段用例。

后 GREEN：最终合成 fixture 输出 **169 条 PASS**（原88条保留，新增81条为5组故障及等待/循环断言，不是169个独立测试），退出 **0**。

| 故障 | 当前实例结果 | 可见文件与重建证据 |
| --- | --- | --- |
| unlink 返回 EACCES | 原 EACCES；锁存 recoveryRequired | 原标志保留，目录同步确认，重建 active |
| unlink 成功后目录 open 返回 EMFILE | 原 EMFILE；锁存 recoveryRequired | 尽力重建原标志，可见且重建 active；目录同步仍失败，持久化未确认 |
| unlink 成功后目录 fsync 持续 EIO | 原 EIO；锁存 recoveryRequired | 标志可见且重建 active；持久化未确认 |
| unlink 后 fsync EIO，再将目录变为0500使恢复写入失败 | 原 EIO；锁存 recoveryRequired | 无标志；恢复权限后新实例确实 inactive，明确暴露仅靠内存不能跨重启保证安全的边界；不调用其 start/reconcile |
| 仅首次目录 fsync EIO，恢复同步成功 | 原 EIO；锁存 recoveryRequired | 原标志重建且同步确认，新实例 active；当前实例仍不自动解除维护 |

每组都验证 reset 后仍 recoveryRequired、启用开关下 tick keep、直接 start 拒绝、移除故障注入后同事务/新事务 begin 和 finish 仍拒绝。存在标志的重建实例还验证 tick keep、空 Process 证据 stop 返回 notConfirmed、finish 拒绝以及新事务不能覆盖标志。各 open/fsync 注入点断言发生时 marker 已删除，避免把 begin 阶段错误当成完成阶段复现。首次 EIO 后真实 chmod0500/恢复0700只作用于本 fixture 的 UUID 临时目录。

本次实际命令（无真实HOME文件、GUI、部署、DB或微信操作）：

| 命令 | 退出码 | 结果 |
| --- | ---: | --- |
| `swiftc -warnings-as-errors native/bootstrap/Supervisor.swift native/bootstrap/CoreSupervisor.swift native/tests/LifecycleTests.swift -o /tmp/wechat-lifecycle-tests` 加执行 | 改前1 / 改后0 | 编译均成功；行为RED后GREEN；最终169条PASS，三个持久化未确认日志为预期故障证据 |
| `cargo test --locked --test swift_host -- --test-threads=1 test_supervisor_ test_decision_ test_backoff_sequence test_give_up_after_five_in_an_hour test_window_rolls test_reset_clears test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up` | 0 | 17 passed、0 failed、0 ignored，保留首轮回归集合 |
| `cargo fmt --all -- --check` | 0 | 未放松门禁 |
| `cargo clippy --all-targets --locked` | 0 | 既有警告保留，未修改Rust或lint |
| `git diff --check` / 暂存后同检查 | 0 | 无空白错误 |

修改仍限 CoreSupervisor.swift、LifecycleTests.swift 和此报告。没有清除真实崩溃保护、修改其他任务文件或主入口；首轮已知 OpenAPI 基线失败未修复未重跑，整体生产链路和完全不可写磁盘下的重启安全仍不在此次通过范围。新提交完整 hash 由此次 worker_done 回执给出。
