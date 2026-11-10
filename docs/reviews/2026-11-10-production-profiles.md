# I1 角色修订存储与显式迁移

状态：本地纯存储实现；未接入运行时、界面或升级入口，不代表项目/生产验收通过。

## 基线、批准与文件边界

- 开始 HEAD `32bb12e550b6739c5c7d6874bbd7388cd771ef5c`，分支 `work/prod-profile-storage`，工作区干净，未覆盖其他修改。
- 经协调者回复 `msg_66e2fdeaf687`，依次集成 Q0 `1ba20e2a988d95ec88cd102f8e195f3e185afa30`、D1 `6e626930101d5721b7b817b4ce78a74da69a16ee`、补充 `b03fa846af003100066bfc409ca1a0abea70bac4`；无冲突，本地对应 `56bb646`、`952b7ce`、`5d2eacc`。
- 原 checkout 的方案/任务书仅只读；先读本 worktree AGENTS.md；codebase-memory-mcp 对本 worktree fast 非持久索引，graph/snippet 定位 builtin 文案并核对源码。docs/config/Cargo 等非代码直接读取；未打开图索引排除的业务 DB。Swift 有局部图解析缺口，本任务不修改 Swift。
- 允许文件：`src/core/brain_profiles/{mod,schema,store,migrate,tests}.rs`，`src/core/mod.rs` 仅注册，本文。未改 config.rs、strict reader、serve、turns、history、手/眼、Swift 或现有 DB。
- `msg_62e8766180ab` 批准 `serde_json_canonicalizer = "=0.3.2"`；临时 manifest 离线解析确认只增加 canonicalizer 0.3.2、ryu-js **1.0.2** 和依赖边，没有升级原包。canonicalizer MIT、无 build script，ryu-js Apache-2.0 OR BSL-1.0；其 serde_json `float_roundtrip` 是共享特性，已做兼容回归。
- `msg_11b001597181` 批准 `libc = "=0.2.189"`（锁中原已存在，仅新增本包直接依赖边），以及设计文档/合成 schema 的**迁移 ID 最大274 UTF-8字节**修订；原账号仍最多256字节，保留完整原值。Cargo.toml 合计仅新增上述两行，Cargo.lock 仅新增 canonicalizer/ryu-js 两包及相应依赖边，未执行 cargo update。

## 交付内容

- 闭合 schema；全部 nullable 字段也必须出现；未知 schema/字段拒绝。角色/知识完整内容与 owner 使用 RFC8785 + 现有 SHA256；黄金向量验证 UTF16 键序、负零、指数及数字边界。正文、模板变量、预算、对象/根大小、账号/目录/覆盖数量与安全整数限制独立检查。
- 角色与知识修订只新增；父修订必须属于同账号，修改推进 draft，不改变 active。知识正文/人工摘要固定修订和摘要，source_label 不读取路径或 URL。复制只允许本账号；被 active/draft/legacy fallback 引用的角色不能归档。
- 唯一可变根原子提交，generation CAS、账号/操作/UUID 作用域去重，完整请求（包含 expected_generation）参与规范化摘要；缓存前复核磁盘对象、父修订和知识归属。跨账号/损坏 owner 对外同为 not_found，磁盘完整性失败只记录不含正文的 integrity_mismatch。缓存紧凑结果，与根同时提交；保留每账号1000结果/100激活历史，不删除修订或其他账号记录腾空间。
- 账号默认 + 稳定联系人覆盖；权限与合并 family 全部由注入 ContactAccess 提供，不查真实 contacts/history DB。撤销权限的覆盖 dormant；冲突 family 返回 binding_ambiguous，群/私聊覆盖分别匹配；回退按目标账号的保留激活历史同时重建 draft + active；不倒退全局根或修改其他账号（本轮修订纠正原“保留 draft”的契约偏差）。
- Store::open 只接受调用者提供的现存私有 state 根；不会发现 HOME/config 路径。AccountContext 必须由可信调用者按 authenticated effective account 构造，不能由 wire 请求选择账号。本模块不提供 HTTP/MCP/control 接口，也不声称替调用者完成账号认证。
- 文件0600、目录0700；基于持有的目录 FD 和 openat(O_NOFOLLOW/O_DIRECTORY)、mkdirat/linkat/renameat/unlinkat 遍历/发布/清理，防 FIFO 阻塞和路径穿越。unsafe 仅集中在 store.rs 的 unix 小封装，说明 FD 所有权和单组件路径不变量；File/stream/临时文件 guard 在错误时释放。根替换或父目录替换拒绝；不可变对象用原子 no-clobber linkat 发布，防 rename 覆盖旧修订；对象与根均 fsync、父目录 fsync，根用 renameat 原子替换。
- 同一 flock 覆盖校验/对象/根；提交前失败保留旧根，允许无效力的孤立对象；完全相同孤立对象可重试复用且配额不重复计数。根 rename 后响应/目录同步失败则报告错误，原 request_id 可恢复成功结果，不能以错误断言根未提交。
- 显式 Fresh 不创建角色或根；LegacyUpgrade 才迁出 service/test 人设和完整话术。迁移标记/source config byte hash/seed hash 与根同事务。重跑不改后来角色；失败后重试保留已有 seed 的初始时间戳。external mode 始终不改，只有原 timeout_action=builtin 时设置固定 inactive legacy fallback；skip/handoff 不设置 fallback。旧配置字节、model/timeout/queue 不写回。新安装 resolve 无配置返回 role_required；旧隐式默认仍在原运行链路内，等 I2 显式接入后再切换，I1 没有让升级运行停摆。

## 测试环境与 RED/GREEN

所有夹具仅临时合成 state/config，没有真实账号、DB、网络模型、GUI 或微信发送。运行入口 `/tmp/i1-evidence/run` 每次创建 `/tmp/i1-isolation.*`，设置 HOME、XDG_CONFIG_HOME、WECHAT_MCP_HOME、MESSAGE_GATEWAY_DB，前置拒绝真实 osascript/screencapture/launchctl/open/微信 helper/codex/claude 的退出125替身，清除常见模型 key，Cargo/Rustup 只复用编译工具缓存。测试 Store 根另建随机临时0700目录并检查路径属于临时目录；Drop 清理夹具。最初一次脚手架编译未设置隔离 HOME，未执行测试（编译错误101）；随后实际 RED/GREEN 均通过隔离入口。实际纯存储模块不读取 HOME，也没有 GUI/模型依赖。

- 脚手架编译先出现 SHA 输出格式及字节切片比较错误，退出101，修正后才记录行为 RED，未将编译失败当行为证明。
- `cargo test --locked --bin wechat-mcp brain_profiles::tests -- --test-threads=1`：初始行为 RED **1通过/3失败**（退出101）；扩展后 **3通过/10失败**（退出101）。首轮 GREEN 13/13（退出0）。
- 故障扩展 RED **18通过/2失败**（退出101）：已写对象/未提交根的保存重试错误地冲突，以及迁移时钟变化导致 seed 不匹配；修正后 GREEN **20/20**（退出0）。这里20是该阶段20个测试函数，不是20种完整生产场景或20项验收；其中两项分别循环10个故障点。
- 缓存前磁盘 owner RED **22通过/1失败**，父修订读取 owner RED 1失败；修正后通过。
- 256 UTF-8字节账号迁移 RED 1失败；批准274字节迁移标识后通过，ASCII/多字节256成功、257拒绝、重跑幂等。
- 第一版目录替换测试因目标子目录缺失而未揭示漏洞（退出0），没有拿它当有效 RED；改成替换最终角色目录并在 object_write 观察后，真实 RED 退出101：发现根外临时文件。FD 锚定后 GREEN，外部目录无临时文件/写入。另测根目录替换、symlink 根文件/目录/锁、权限及无路径穿越。
- 孤立对象配额 RED 1失败（退出101），发现重试重复计数；修正后重试成功、下一对象超额拒绝且旧根逐字节不变。
- 最后补诊断时出现两处机械替换编译错误101，已修正并重跑最终模块测试/clippy；不计为产品行为 RED。
- 最终模块共 **31个测试函数**，包括两个独立打开锁描述符的并发写线程仅一成功、10阶段保存/迁移故障、错误后锁/FD释放（30次错误后统计FD）、摘要/知识归属、父修订/缓存校验、活跃/草稿/回退/归档、合并歧义、保留上限、账号/覆盖/安全整数/配额限制，以及完整迁移字段与设计 fixture 比较。

## 实际检查记录

临时日志 `/tmp/i1-evidence/` 为本机证据，不保证跨机器/重启永久存在。

| 命令 | 退出码 | 结果 |
| --- | ---: | --- |
| 初始 git rev-parse/status | 0 | 派发 HEAD，干净工作区 |
| 依赖顺序 cherry-pick | 0 | 三提交集成，无冲突 |
| 临时 manifest 首次离线解析 | 非0 | ryu-js 未缓存；cargo info 下载1.0.2后离线 check成功，逐字 diff核对无关包未变 |
| 隔离 cargo test --locked --bin wechat-mcp -- --test-threads=1 | 101 | **924通过、1失败、0 ignored**；原893通过 + 新增31通过；唯一失败同Q0 |
| Q0/I1失败断言payload比较 | 0 | ops_tools_list_unchanged 的 left/right 两个字符串逐字节相同，未改旧fixture；baseline-comparison.log |
| 隔离 cargo test --locked --test protocol_smoke --test swift_common_layer --test swift_time_labels --test swift_voice --test swift_bounded -- --test-threads=1 | 0 | 协议2项 + Swift纯逻辑14项通过 |
| rustfmt --edition 2024 --check (已跟踪Rust排除原src/interaction.rs，再显式加入新模块) | 0 | 保留既有检查范围及新文件；没有放宽排除 |
| bash scripts/test-gate-fixtures.sh | 0 | 18个门禁夹具通过 |
| bash scripts/test-gates.sh | 0 | 本地静态门禁通过 |
| 首次 cargo clippy --all-targets --locked | 101 | 真实磁盘空间不足，并非代码通过；只用 cargo clean 清理本任务编译产物/临时manifest target后重跑 |
| 隔离 cargo clippy --all-targets --locked（恢复磁盘后） | 0 | 硬规则保留；有既有和新模块 pedantic 警告，不声称零警告 |
| 最终隔离模块测试、clippy | 0 / 0 | 模块31通过、0失败；clippy完成，test target 1980警告（1472 duplicates），未放松规则 |
| git diff --check | 0 | 无空白错误 |

编译磁盘恢复后使用 CARGO_INCREMENTAL=0、CARGO_PROFILE_DEV_DEBUG=0、CARGO_PROFILE_TEST_DEBUG=0 控制本任务缓存占用；未更改仓库构建配置/检查规则。曾有直接 rm 风格清理被自动审批拒绝，改用 cargo clean 成功，只清理本任务可再生编译产物，未请求绕过。

## 风险与未验证

- 本任务不接入 serve/turns/control/外部协议、运行快照、知识上下文裁剪、备份、UI 或自动迁移。I2 必须注入真实可信账号与原授权 resolver、先用现有 strict reader 验证配置并明确 Fresh/LegacyUpgrade；不得由 sidecar 缺失推断升级。保存内部 API 接收完整 sealed 修订/固定ID；请求字段适配与账号获取属于 I2，不直接把网络请求反序列化成可信上下文。
- 纯存储 active 解析已测试；没有声称真实微信新轮次采用角色、外部旧客户端支持配置、真实升级/回退或发送链路成功。原 config/外部 mode 的兼容为源文件未改、字节不写回和既有合成测试证据。
- 基线唯一 ops_tools_list_unchanged 失败继续归 I2/Q1；整体回归为 partial，未更新旧快照掩盖。
- unsafe FD封装在本机 macOS 实际运行验证；未跨编译/运行 Linux，未做真实断电、磁盘耗尽/硬件fsync失败试验。故障注入模拟调用边界，不等于真实断电保证；同用户恶意进程可直接篡改任何0700用户文件，不以本模块替代OS账号安全。
- 1 GiB账号配额的拒绝/重试算法使用相同路径、缩小测试阈值验证，未生成1 GiB实体数据；128 MiB根阈值未制造巨型文件。没有自动清理孤立/历史对象；磁盘容量不足会安全报错，维护清理需另行设计。
- 未运行完整原生App构建、安装/升级、远端、真实模型、GUI/屏幕权限、微信发送或连续运行；未发布、推送、合并或修改原 checkout。

最终源码诊断/重复读取简化后已再跑31项模块测试与全target clippy；完整925项回归、协议/Swift在此无返回语义变化的收尾前执行。实现本地commit见Orca完成回执（不把待填hash写入提交自身）。

## 协调者复核修订：rollback 同步 draft + active

本轮起点 `7e83069e83728b1cd50aa78585c8b16ccc737872`，`git status --short` 为空；只修改 store.rs、tests.rs、本文及获准的归档保护设计短语。重新读取 AGENTS.md；图查询未返回相关新节点，已刷新本 worktree 非持久索引，仍未命中该过滤查询，明确以当前源码核对。未修改运行接入、公共协议、数据库、配置 reader 或其他 worktree。

协调者指出的偏差成立：D1 §3 已要求从目标账号保留激活历史重建 **draft + active**，旧实现及本文原叙述却保留旧 draft。最小修复仅在 `Mutation::Rollback` 分支把目标 BindingSet 同时赋给同一 AccountEntry 的 draft 和 active，然后仍走原根提交、CAS、授权和请求去重流程；普通 Activate 不改行为，不修改角色目录 draft_revision_id，不倒退全局 generation，不改其他账号。

新增2个测试均先在旧实现上退出101（行为 RED），不是编译失败：

- `cas_rollback_restores_draft_and_active_without_touching_other_account`：构造目标历史、不同的现行 active 与空 draft，以及另一个真实存在于合成根的账号；RED 精确失败于 draft 应等于目标。测试还覆盖权限拒绝、跨账号 target_generation 拒绝、旧 CAS 拒绝且根字节不变；成功回退后另一个账号的绑定/角色/迁移/激活历史/请求缓存保持不变；同请求重放不写根；撤权限后缓存也不能绕过授权；再次 Activate 不会跳回先前草稿。
- `cas_concurrent_rollbacks_restore_both_bindings_once`：两个独立打开 Store/锁描述符的线程竞争同 expected_generation，只有一个提交，另一个 revision_conflict；generation 只增加1，draft/active 均为目标。旧实现的并发/CAS断言通过，但最终 draft 断言失败，RED退出101。

沿用已核对的 `/tmp/i1-evidence/run`：独立临时 HOME/XDG_CONFIG_HOME/state/DB、GUI/模型命令拒绝替身、合成账号/内容、无真实微信/数据库/付费模型。日志 `rollback-red.log`、`rollback-concurrent-red.log`、`rollback-green.log`、`rollback-clippy.log` 均在 `/tmp/i1-evidence/`，仅为本机临时证据。

本轮实际命令与结果（均使用上述隔离入口运行Rust命令）：

| 命令 | 退出码 | 结果 |
| --- | ---: | --- |
| cargo test --locked --bin wechat-mcp brain_profiles::tests::cas_rollback_restores_draft_and_active_without_touching_other_account -- --test-threads=1（实现前） | 101 | RED，0通过1失败，精确命中draft未回退 |
| cargo test --locked --bin wechat-mcp brain_profiles::tests::cas_concurrent_rollbacks_restore_both_bindings_once -- --test-threads=1（实现前） | 101 | RED，0通过1失败，并发后draft仍非目标 |
| cargo test --locked --bin wechat-mcp brain_profiles::tests -- --test-threads=1（修复后） | 0 | GREEN，33通过0失败0 ignored，含原31项全部回归 |
| cargo clippy --all-targets --locked | 0 | test target 1981警告（1472 duplicates）；未放宽规则，新增长测试带来1项pedantic警告 |
| rustfmt --edition 2024 --check src/core/brain_profiles/store.rs src/core/brain_profiles/tests.rs | 0 | 无格式差异 |
| git diff --check | 0 | 无空白错误 |

本轮未重复全量Rust、协议/Swift和真机检查：仅存储分支有5行新增/1行替换，原31项及两项新增定向覆盖均已通过；上一轮唯一 Q0 ops_tools_list_unchanged 失败仍为既有未解决项，不将本轮33项GREEN宣称为全项目全绿。本次本地提交的完整hash见Orca回执，未推送/合并/部署。

### I2/UI list 与归档约束

当前 `list` 先按可信账号筛选，再校验该账号目录指向的磁盘角色、父修订和知识；损坏对象会被过滤出 roles，其读取失败或 owner/hash 完整性错误记录不含正文的 `integrity_mismatch` 日志。**成功返回空 roles 不等于无配置或全新安装**，尤其根仍可能保留账号绑定和角色目录；当前 list 没有逐条错误字段，I2/UI必须保留既有状态并结合明确读取/诊断展示加载失败，不能因空列表重置、重新迁移或隐式写默认角色。根本身损坏/未知schema仍返回错误，不变为空根。

## I1 增量：显式执行模式与根 schema 2

状态：纯存储增量交付；未接入运行时/serve/control，保持既有边界。

### 设计与决策冻结（依据 msg_012a8b1f9166 与 msg_1301e6be92d1）

1. **Schema 2 闭合升级**：
   - `AccountEntry` 新增必填字段 `execution_mode: ExecutionMode`（枚举 `legacy | managed`，无 serde 默认值）。
   - `Root.schema_version` 升级为 2；不可变对象 `RoleRevision` 与 `KnowledgeRevision` 保持 `schema_version: 1` 及哈希算法不变。
   - 闭合 JSON 反序列化拒绝未知根字段、未知账号字段、未知执行模式及未来 schema。
2. **模式流转与上下文**：
   - `Store::execution_mode(ctx) -> Result<ExecutionMode>`：无根文件或根中无该账号时返回 `Ok(ExecutionMode::Legacy)`；损坏根或未知 schema 报错。
   - `ensure_account` 自动创建的草稿账号设为 `ExecutionMode::Legacy`。
   - `SaveRole`、`SaveKnowledge`、`SaveBindings`、`CopyRole`、`ArchiveRole` 保持现有模式不变。
   - `Activate` 与成功的 `Rollback` 在同一原子事务内将当前账号设为 `ExecutionMode::Managed`。
3. **显式初始化与迁移**：
   - 新增 `Store::initialize(ctx, installation, validated_config_bytes, service, at, upgrade_schema_from)` 持进程间 flock：
     - **普通读取/初始化拒隐式升级**：`upgrade_schema_from: None` 时，遇 V1 根返回 `schema_unsupported`，不静默改写磁盘。
     - **Fresh 初始化**：原子建立当前账号为 `Managed`，无种子、无角色，后续 resolve 返回 `role_required`；若遇已有 `Legacy` 配置账号，拒绝冲突返回 `revision_conflict`，绝不擦除已有草稿；重复 Managed 初始化幂等返回当前 generation。
     - **LegacyUpgrade 初始化**：生成种子角色并原子提交根，设当前账号为 `Managed`；已有 Legacy 账号且无匹配迁移记录时返回 `revision_conflict`；重复 Managed 初始化校验配置 hash 与磁盘可达对象一致性，通过则幂等，损坏则报错。
     - **显式 V1 升级（upgrade_schema_from: Some(1)）**：完整校验 V1 闭合结构及所有可达角色/知识对象的磁盘存在、所有权与哈希；保留全部其他账号数据并明确标为 `Legacy`；当前账号按初始化请求升级为 `Managed`；根升为 2，单次 generation +1；损坏/未知字段/冲突均拒绝且不修改原 V1 根。
     - 其它未支持版本（如 `Some(3)`）返回 `schema_unsupported`。

### 实际测试命令与回归记录

| 命令 | 退出码 | 结果 |
| --- | ---: | --- |
| cargo test --locked --bin wechat-mcp brain_profiles -- --test-threads=1 | 0 | **40 通过、0 失败、0 ignored**；旧 33 项全部回归通过，新增 7 项测试全部 GREEN |
| git diff --check | 0 | 无空白错误 |

40 项测试覆盖：
- `execution_fresh_managed_without_seed_and_repeat`：初始 Legacy、Fresh Managed、无种子 role_required、重复初始化幂等、缺席账号 Legacy。
- `execution_drafts_stay_legacy_activate_and_rollback_manage`：草稿保持 Legacy、已有草稿拒 Fresh/Upgrade 冲突、Activate 切 Managed、Rollback 保持 Managed。
- `execution_legacy_initialize_repeat_checks_source_and_reachable_objects`：LegacyUpgrade 重复校验配置 hash、对象损坏后重复调用拦截报错。
- `execution_v1_requires_explicit_upgrade_and_preserves_other_account`：普通读 V1 报 schema_unsupported、显式升级保留其他账号原字段与对象、原账号显式标记 Legacy。
- `execution_v1_current_migration_upgrade_no_reseed_or_draft_loss`：V1 当前账号升级保留 draft 与 active、不重种、重复调用幂等。
- `execution_v1_rejects_unknown_closed_fields_and_corrupt_objects`：V1 根未知字段、账号未知字段、损坏对象均拒绝升级且原根字节不变。
- `execution_initialization_root_faults_and_unknown_schema`：root_create/write/sync/rename/dir_sync 故障恢复及 Some(3) schema_unsupported 校验。
- 原 33 项所有 CAS 隔离、所有权校验、并发抢锁、联系人覆盖、配额截断及迁移回归测试均 100% 保持通过。

归档保护已在设计原条款最小明确：draft 的 default 和 overrides 引用均会阻止归档并返回 role_in_use，须先显式解除草稿引用；active 和 legacy_fallback 的原保护保持。没有为归档自动清空绑定，也不新增 UI 或放松账号权限。
