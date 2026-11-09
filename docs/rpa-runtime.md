# 模块化桌面 RPA 运行底座

对应设计：[个人微信 RPA 技术方案](../research/personal-wechat-rpa-2026-10-27.md)。本文记录实际实现，设计中的后续功能不视为已完成。

## 已实现的模块

| 模块 | 职责 | 不承担的职责 |
|---|---|---|
| `domain/observation` | 归一化矩形、窗口/快照证据、客户文字视口与校验 | 不调用系统、数据库或模型 |
| `desktop/actor` | 复用进程内互斥和跨进程桌面资源锁 | 不按账号另建可并发操纵同一桌面的锁 |
| `desktop/runtime` | 原生进程生命周期、有限帧IPC、串行请求与取消清理 | 不猜测发送成功，不自动重试动作 |
| `desktop/protocol` | 版本、request ID、窗口和快照DTO校验 | 不把控件或OCR标成服务端消息 |
| `desktop/scheduler` | 截止时间调度与错过tick跳过 | 不产生无界读取任务 |
| `messaging/reconcile` | 纯数据的保序对齐、重复正文计数、缺口与歧义 | 不点击、写库或发消息 |
| `messaging/history` | 基于快照证据的连续性判断、乱序与同帧冲突拒绝 | 不依赖原生驱动或SQL |
| `storage/chat_context` | 客户快照和比较结果同事务写入已有事件日志 | 不运行OCR，不推进消息级游标 |
| `wechat/navigation` | 原生标题直达、搜索查询读回、唯一结果定位与标题复核 | 不按固定搜索结果坐标发送 |
| `native/macos/Layout.swift` | 相对区域模型与逐帧像素变换 | 不保存屏幕绝对坐标 |
| `native/macos/Capture.swift` | ScreenCaptureKit窗口流、图像摘要与窗口epoch | 不推断联系人身份 |
| `native/macos/Recognition.swift` | Vision OCR与区域摘要缓存 | 不返回未经识别的收发方向 |
| `native/macos/Diagnostics.swift` | 权限、进程、窗口及AX角色只读探测 | 不输出联系人、聊天正文或截图 |
| `native/macos/Protocol.swift`、`main.swift` | Helper协议与装配 | 不持有CRM业务规则 |

现有 `MessageAdapter`、Gateway、SQLite schema和公开MCP工具参数保持兼容。会话列表、标题核验和聊天读取在开启stream后委托原生Helper；旧读取仍为默认路径。读取结果保留 `text_lines`，stream模式另附文字矩形、置信度和快照证据。

原生采集的聊天正文与核验标题取自同一帧。像素内容、尺寸、行步幅参与摘要；不依赖PNG文件字节是否相同。区域摘要相同则复用OCR结果。窗口几何或进程/窗口身份改变时重建epoch；同一窗口仅刷新流时保持epoch和单调序号。旧流的迟到回调被丢弃。

## 构建与诊断

```bash
bash scripts/build.sh
WECHAT_MCP_DESKTOP_RUNTIME=stream ./target/release/wechat-mcp --desktop-diagnostics
```

诊断在加载Gateway和数据库之前执行，只统计权限与AX角色，不激活或恢复窗口。微信退出或窗口不可见时也返回明确状态。`ax_settable_text_controls`只说明当前快照找到多少个可写文字控件，不代表已经实现AX输入或消息读取。

```bash
WECHAT_MCP_DESKTOP_RUNTIME=stream ./target/release/wechat-mcp
```

配置MCP时将上述环境变量设置在该服务的env中即可。`WECHAT_MCP_DESKTOP_RUNTIME`只接受`legacy`（默认）或`stream`。未通过本机实机验收前保持默认；明确开启stream后，缺失Helper、权限不足、OCR超时等均报错，不静默回退到旧驱动。

`WECHAT_MCP_DESKTOP_HELPER`可指定原生Helper路径；默认寻找主程序旁的`wechat_runtime`，再查找工作目录下`bin/wechat_runtime`。构建、CI和通用二进制打包均包含该Helper；安装脚本同时兼容不包含它的旧Release。

stream采集需要macOS14及以上，使用Vision识别。旧驱动仍保留原有Vision/Tesseract行为。新路径尚未实现Tesseract替代，不把旧路径的兼容结果当作stream验收结果。

## 自定义区域

通过`WECHAT_MCP_DESKTOP_PROFILE`指定JSON文件。坐标为微信窗口内部0–1归一化比例，左上角为原点。未覆盖的区域按当前默认布局计算；窗口过小、未知区域名、越界、空区域或不支持的schema会明确失败。

```json
{
  "schema_version": 1,
  "regions": {
    "sessions": {"x": 0.07, "y": 0.09, "width": 0.25, "height": 0.90},
    "chat": {"x": 0.34, "y": 0.10, "width": 0.64, "height": 0.65},
    "title": {"x": 0.28, "y": 0.00, "width": 0.70, "height": 0.08}
  }
}
```

这些比例仅用于解释格式，不是本机实测标定。Profile在Helper启动时读取；修改后重启该MCP服务。可拖动区域的标定界面、账号/联系人身份绑定仍未实现。

还可覆盖`search`区域。默认覆盖左侧顶部搜索栏；点击点来自识别到的“搜索/搜尋/Search”占位词或已确认查询词的文字矩形中心。原生路径先尝试当前标题匹配，再使用唯一会话名称候选；搜索时必须读回完整查询词，再点击唯一同名结果并复核标题。查询不一致、同名歧义或整个标题/会话区无可读文字时停止，不进入旧路径的固定首个结果点击。

## 错误、取消与连续性

IPC单帧上限4MiB，每次交换最长20秒。请求序号、版本、结果信封、文字矩形和置信度均校验；失败或取消的未完成交换不能留在连接里供后续请求读取。交换取消会销毁持有该交换的Helper；下一次读取重新创建。Helper本身没有发送微信消息的操作。

文字轮询使用保序suffix/prefix匹配，保留完全相同正文的多次出现；纯内容集合不再用于判断新增文字。窗口epoch变化、缺少重叠、重复锚点不足会报告连续性缺口/歧义，不返回猜测出的入站消息。文字行仍不是气泡级消息，方向保持unknown。

后台会话列表观察继续产生原有`interface_change`，不会声称已经持久捕获每条新消息。监听能力按`background_observations`门控，调度使用固定截止时间并跳过积压tick；状态增加`last_observation_duration_ms`。现有2–60秒监听参数范围和事件租约语义保持不变。

## 客户上下文的持久比较

现有`capture_customer_context`现会在返回事件的`payload.text_continuity`中附加比较结果。复用已有`chat_snapshot`事件保存基线，不新增表、工具参数或第二套消息队列。首次为`baseline`；后续为`unchanged`、`anchored`、`gap`或`ambiguous`，并携带`previous_event_id`、原因和保留重复次数的`new_lines`。这些新增行是文字观察，始终`message_continuity_verified=false`、`incoming_verified=false`、`direction=unknown`。

SQLite写事务内读取同账号、同客户的最后一次已提交快照，再写入本次快照及比较结果；以内部rowid确定提交顺序，避免同秒时间戳和随机UUID排序造成分叉。重启后继续读取持久快照。并发乱序帧、同一帧的冲突证据、正文与证据不一致、客户recipient在读取后变化或非法矩形会报错，不推进基线；写事务失败也保留原基线。

窗口、区域、目标或epoch变化会记录缺口；Helper重启一般产生新epoch，不能直接连接旧观察。旧路径或旧记录缺少帧证据时，快照仍可保存，但不会仅凭正文宣称连续。多行重复锚点及重叠重复也判为歧义。已有事件保持`recorded`，不会因手动采集自动变成待回复的入站消息。

该功能只在明确调用客户上下文采集时运行。后台hot会话采集、气泡级持久消息、滚动补读和缺口恢复尚未接入。客户表中的recipient是本地配置的目标名称，不能据此宣称微信账号或联系人身份已验证。

## 验证

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
./target/release/wechat_runtime --self-test
cargo test --locked --test desktop_runtime_smoke
cargo test --locked --test protocol_smoke
cargo test --locked service_tests
bash scripts/test-install.sh
```

`--self-test`检查区域变换与非法区域，不访问桌面。`desktop_runtime_smoke.py`使用独立副本、假系统命令和假Helper，验证进程复用、重复行、客户上下文连续采集入库、错误request ID、Helper错误、超长帧、非法矩形和窗口变化。点击、剪贴板、截图和AppleScript均隔离；以上通过不能证明真实消息收发或P95性能达标。

首批本地验证：85项Rust单元测试通过，严格Clippy、格式检查、原生构建与Helper自检通过；原生Helper的arm64及x86_64编译通过。隔离的桌面运行时、MCP协议、CRM服务和安装脚本冒烟测试通过。CI配置已更新，但尚未在远端运行，也没有创建或发布Release。

第二批增加持久上下文比较后，95项Rust单元测试、严格Clippy、格式检查、debug/release构建、桌面运行时/MCP协议/CRM冒烟测试通过。新增测试覆盖重启、重复行、账号/客户隔离、多连接并发、事务失败、乱序及非法证据；均不替代真实微信验收。

## 实施进度与下一道验收

2026-10-27已实现P0只读诊断和P1的窗口流/IPC/文字观察底座，并接入现有读取；保序对齐与调度作为可复用模块交付。

后续批次已接入客户上下文快照的事务比较、持久前序引用及缺口记录，属于P2的文字证据部分。完整消息级游标仍未实现。

尚未完成：真实聊天态采集/OCR性能验证、气泡与发送者识别、hot独立窗口、多信号身份绑定、消息级持久游标/缺口补读、通用发送回显校验、Agent常驻消费者、Windows worker。没有新增数据库表或声称完整方案已验收。

本机诊断曾返回微信4.1.15、屏幕录制与辅助功能权限均为true、AX节点5且没有可写文字控件、`window_state=wechat_window_not_observable`。这是当次窗口状态，不是永久能力结论。进入实机采集和真实消息验收前，需要可观察窗口及明确选定的测试联系人/群和测试正文。

后续实机测试已获指定对象及正文授权，窗口采集和OCR曾返回真实文字；随后窗口切换后采集持续为白屏，目标仍无法核验。未触发发送，未完成收发及实时性验收。已加入搜索定位和白屏阻断回归；详见[实机验收记录](../research/rpa-acceptance-2026-10-27.md)。诊断中的`window_state=observable`仅指窗口几何及系统可见性，`content_readability=not_probed`明确表示没有检查内容是否可读。


## 专用测试机进展（2026-11-04）

macOS12 Intel测试机通过稳定GUI宿主托管旧驱动。两项系统权限恢复后，业务程序在bundle之外更新；无需因每次Rust更新重新授权。详见[专用机实测记录](../research/dedicated-host-2026-11-04.md)。

旧驱动增加当前标题优先、搜索查询读回/唯一候选核验。发送增加粘贴正文读回与再次标题核验；空OCR成功与OCR引擎失败分开处理。默认聊天区排除底部160pt工具栏及输入区，小窗口不再扩大裁剪到窗口外。该默认布局仍需后续标定，不保证所有输入框高度。

已向授权对象示例联系人甲真实发送一次指定正文，并人工核验右侧绿色气泡。持久状态仍为triggered、delivery=unknown；尚未实现自动气泡级回显与送达确认。随后已观察到用户实际回复“接收到测试一二三”（与约定文字不同）；OCR读取命中，并人工确认左侧气泡，程序方向仍为unknown。群聊和好友操作未验收。最新101项Rust测试及桌面/MCP/CRM冒烟通过；安装脚本隔离检查因Cline配置路径断言失败，不能列为通过。


## 操作可视化

构建产物`wechat_overlay`提供独立AppKit浮层，默认在同目录存在该程序时启用。`WECHAT_MCP_OPERATION_OVERLAY=0`关闭它；不改变公开MCP参数及数据库结构。

- `src/desktop/overlay.rs`负责子进程、事件确认、超时和截图隔离；`native/overlay/main.swift`只绘制，不执行点击或发送。
- 可见微信窗口定位后，在左上方显示“微信 RPA · 已定位”角标；靠近屏幕边缘时限制在所在屏幕可见区。这里表示窗口几何定位，不表示联系人身份已核验。
- 青色指针以60Hz跟随实际系统鼠标位置；旧点击助手沿约180ms路径移动，完成点击后显示橙色扩散光圈。移动/点击事件与实际驱动调用连接，不播放虚构的操作步骤。
- 浮层窗口不成为key/main窗口，忽略鼠标事件；不抢焦点、不拦截微信点击。截图前等待隐藏确认，截图结束后恢复；约2秒无事件自动隐藏，父进程断开后退出。
- 浮层不保存聊天内容；子进程失败或2秒未确认时记录警告并关闭该进程，本次业务动作仍按原驱动结果返回。取消未完成视觉交换会丢弃子进程，避免旧确认误用于后续隐藏。

macOS12 Intel测试机编译及浮层自检通过；经GUI宿主执行真实只读调用，系统窗口列表采样观察到2个可见浮层窗口。实际读取命中用户回复“接收到测试一二三”，未读到输入框提示。本轮没有新增发送或实机点击。移动/点击事件接入与截图隐藏顺序由隔离冒烟覆盖；实机点击光圈尚未单独验收。

新增协议确认/错误响应测试后，共102项Rust测试通过；严格Clippy、原生构建/自检、桌面运行时与MCP协议冒烟通过。浮层坐标自检覆盖负坐标及纵向屏幕排列，尚未完成真实双屏/不同缩放比例的视觉验收。


2026-11-04后续已完成单屏实机点击验收：平滑移动、实际点击穿透、前台保持、点击光圈自身视图、隐藏/恢复、空闲消失和退出均通过；正常MCP恢复后仍能读取测试回复。详见[浮层实机验收记录](../research/overlay-acceptance-2026-11-04.md)。以上后续结果取代前文“实机点击光圈尚未单独验收”的当时状态，双屏限制仍保留。


## Agent桥接首轮

已增加独立Python桥接模块，复用现有持久事件及租约。2026-11-04实测单个真实会话变化事件由后台Codex执行器接收并返回确认，原事件已acknowledged；当前桌面聊天的CLI自主唤醒未通过。当前入口为有时限单事件验证，不是开机常驻服务。详见[桥接说明](agent-bridge.md)与[实测记录](../research/agent-bridge-acceptance-2026-11-04.md)。


## 白名单模型客服

新增单联系人常驻Bot，在控制端通过launchd运行，微信仍由专用Mac GUI宿主操作。当前只允许示例联系人甲私聊，真实Codex模型生成正文；群聊和好友申请关闭。视觉候选不等于平台消息已验证，布局不匹配或发送结果不明时暂停。详见[客服Bot运行说明](allowlist-bot.md)及[实机记录](../research/bot-acceptance-2026-11-04.md)。
