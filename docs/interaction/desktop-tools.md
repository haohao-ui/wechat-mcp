# 通用眼手 MCP

这组工具为 macOS 12+ 提供应用无关的观察与动作入口，复用 `native/action` 的拟人算法。它与现有微信客服工具并存；尚未把原微信发送链迁移到新接口，也不自动学习功能或生成流程规范。

## 调用流程

1. `desktop_apps {}`：列出普通桌面应用的名称、bundle ID、PID、激活状态。
2. `desktop_windows {"bundle_id":"应用的真实 bundle ID"}`：列出该应用可见普通窗口。多个窗口时必须选择明确的 window ID。
3. `desktop_observe {"bundle_id":"…","window_id":123,"include_screenshot":true}`：读取 AX 控件、OCR 文字与窗口截图，生成 `observation_id`。不激活或点击应用。
4. 从结果选择控件 ID，调用 `desktop_act`。应用在后台时先显式执行 `focus`；使用它返回的新观察再进行下一步。
5. 动作响应携带新观察、新编号，以及独立的验证状态。下一步使用新的编号。

```json
{
  "observation_id": "上一步返回的编号",
  "target_id": "上一步返回的控件编号",
  "action": "click",
  "expected_text": "预期出现的文字",
  "timeout_seconds": 5
}
```

公开观察通过截图及本机 Vision 读取界面；可选截图以 MCP `image/png` 内容块返回，结构化结果不重复携带 base64。AX 动作前的内部检查只读取 AX 目标状态，不重复整窗 OCR。图像用于观察和诊断，不保存截图文件。MCP 客户端会收到所选窗口的文字；调用方应按任务选择应用和窗口。

## 观察的含义

- `source=accessibility` 是 AX 控件；`role`、`identifier`、`actions`、`enabled`、`focused`、`states`、`value` 是应用暴露的信息。
- `source=ocr` 是可见文字区域，并附识别置信度；它不是已确认的可点击按钮，不据此声称理解了功能。
- 坐标是 CG 全局屏幕逻辑点，原点在主屏左上，副屏可以为负；截图可能为 Retina 像素。OCR 坐标按实际窗口大小换算，不使用微信固定尺寸。
- `accessibility_available` 表示是否取得了可对应窗口的 AX 树；`accessibility_truncated` 表示遍历因节点、深度或时间预算停止。
- `coverage=visible_window_only`：没有读取整个文档、整个应用或全部历史。AX 安全文本框不进入控件结果。截图仍是应用实际渲染的窗口，应由调用方决定是否请求图片。
- 本机无需由模型猜坐标即可定位 AX/OCR 目标，但按钮功能与流程语义仍需上层判断和验证。

## 动作

| action | 输入 | 说明 |
|---|---|---|
| focus | 窗口内任一有效 target_id | 请求激活并抬起对应 AX 窗口，等待实际应用和窗口都在前台才返回 `activation=confirmed`；未获激活则明确报错 |
| click / right_click / double_click | target_id | 在目标有效区域内采样落点；双击复用同一落点，发送原生 clickCount |
| drag | target_id、destination_id | 源与目的属于同一次观察；按原轨迹算法拖拽，途中停止也释放鼠标 |
| scroll | target_id、非零 lines、ticks | 在目标内滚动，保留动作间自然节奏 |
| key | target_id、keys | 当前获得焦点的 AX 控件；支持 cmd/shift/alt/ctrl 组合、方向键、Tab、Home/End 等。字母是物理快捷键位置，不承担任意 Unicode 文本输入 |
| type_text | 可编辑 AX 文本控件、text | 拟人点击后核对焦点，保留/恢复剪贴板，以粘贴输入，按 AX 值读回核验。不自动按回车 |

输入长度、滚动次数和等待预算受限。工具拒绝未定义字段和与动作无关的参数。没有裸坐标动作接口，控件必须来自服务端保存的观察。

## 有效性、停止和结果

- 观察保存在当前 MCP 进程内，数量有界，不写入业务数据库；断线重建进程后需重新观察。
- 每个编号只能使用一次，失败也不会恢复。重复调用不会重复触发动作；过程异常时必须查看新画面再决定下一步。
- 执行前核对应用、PID、窗口 ID、窗口标题和几何。AX 目标按唯一 identifier（若有）或 AX 路径重新定位，并核对角色、标签、位置、值和状态；不以整窗像素或焦点环变化作为拒绝条件。整窗 `fingerprint` 仅保留作诊断。
- OCR 目标必须保持唯一的同文同位置候选，并通过目标文字局部像素校验；不把其他区域的时钟或 OCR 置信度抖动当成目标变化。目标文字区域本身的动画、截断或识别框漂移仍可能被拒绝，不能声称任意自绘应用已经可用。
- 除显式 focus 外，动作要求绑定窗口在前。遮挡检查针对操作目标区域（拖拽包括源与目的），不再覆盖整个窗口；与目标区域无交集的菜单栏、Dock 不阻断。仅当进程实际执行路径对应当前安装目录的 `wechat_overlay` 时，按本项目 click-through companion 合约排除它，不能凭同名进程或浮动层级放行其他窗口。执行中继续检查取消、父进程存活、前台应用及窗口状态。
- macOS 14+ 使用协作式激活；`activate()` 的返回值不是到前台的证明。当前 focus 在总预算内轮询并验证实际前台，失败报 `activation_not_confirmed`。无法获得宿主让出激活权时仍可能失败，未做强抢前台或启动新应用的回退。[Apple 官方说明](https://developer.apple.com/documentation/appkit/passing-control-from-one-app-to-another-with-cooperative-activation)
- 与微信流程共用 `message-gateway/desktop.lock`，不同进程也不能同时操作桌面。
- `desktop_stop {}` 取消本 MCP 会话的通用动作并使观察失效，不停止微信机器人。返回 `cancellation_requested` 不代表已停止；必须等待原动作返回。鼠标和修饰键在中断路径释放。
- `status=executed` 仅表示动作函数完成。`input_readback=passed/timeout/not_applicable` 独立记录输入读回，不会被 `expected_text` 覆盖。
- `verification=passed/timeout/unverified/not_requested` 单独记录预期文字检查。出现指定文字不是“界面发生了指定业务变化”的证明；后续观察失败时以 `observation_error` 表达，不把已经执行的动作倒写成未执行。
- `timeout_seconds` 是一次原生动作的总预算（默认及上限 30 秒），包含动作前检查、激活、动作、读回、观察和预期文字检查；所有阶段共享同一单调时钟截止点。截图等待、AX 查询、Vision 识别都受剩余预算约束，不各自追加 30 秒。动作后预算耗尽时可能没有新观察编号，应重新观察。Rust 为该预算额外保留 10 秒 IPC/退出宽限，单独观察命令仍有 90 秒上限以容纳 Vision 冷启动。
- `business_success=unknown`：是否发出消息、保存文件或完成交易，由上层结合应用证据判断。异常响应同样不能当作“完全没产生副作用”。

## 构建与当前验证边界

`native/interaction` 与 `native/action` 编译为常驻的 JSON 行协议 `desktop_helper`，由每个 MCP 会话复用同一进程，保留 Vision 的模型预热状态。首次识别使用冷启动预算，后续识别使用正常预算。默认从 `wechat-mcp` 同目录寻找，可通过 `DESKTOP_HELPER_BIN` 指定受信任的安装路径。常规构建、CI、发布包及 `deploy_host.sh --worker` 都包含它；专用 Mac 部署时使用已有签名证书。未改动证书策略。

需要 macOS 屏幕录制权限；AX 控件和动作需要辅助功能权限。没有权限时返回明确错误，不自动请求或假装成功。ScreenCaptureKit 用于新系统截图，旧系统使用兼容 API。

自动测试使用事件记录 sink 和合成 helper，不操作真实桌面。覆盖拟人轨迹兼容、双击计数、拖拽和按键中断释放、观察绑定、单次消费、取消、参数校验和 MCP 分发；另外编译真实 helper。真实应用上的 TCC、OCR 精度、动态控件和多显示器交互仍需实机验收，不能由编译成功推断。

### 本轮验收记录

- 原生 helper 已通过真实 MCP 进程完成应用清单读取；未输出用户应用名称作为测试报告。
- 独立合成窗口实测读取到了 AX 按钮与 OCR。首次测试发现 Vision 冷启动超过正常预算，因此改为常驻 helper，并采用首次/后续两种识别预算，沿用微信 worker 的预热思路。
- 合成窗口点击前返回 `stale_observation`，该次没有执行点击；不能记为真实点击验收通过。用户指出本机出现测试窗口后，已确认窗口和测试进程退出、临时应用清理，并停止后续弹窗测试。
- 后续验证只使用无真实事件的测试 sink 与合成 backend。真实应用动作验收仍未完成。

前一轮定向验证（不是本次复核结果）：通用接口 8、网关 18、服务流程 23、原拟人动作 29、Swift 边界 3、通讯录 Swift 9、通用 Swift 2、桌面锁 2，共 94 项测试通过；Intel 与 Apple Silicon 的 macOS 12 helper 编译通过，Shell 语法和 `git diff --check` 通过。常驻 helper 引入后曾遇到重复调用的锁争用失败；其真实成因未查明。保留租约析构显式解锁，并用手动 `try_clone` 的测试验证该释放语义，但此测试没有复现历史失败，也不能证明子进程 exec 后继承了描述符。未提交、推送或部署。


## 本次复核后的发布阻塞与范围

- 上一次“整窗 SHA256 不同就拒绝动作”导致的可用性风险列为发布阻塞。当前已改 AX 语义绑定及 OCR 局部绑定，并补合成回归测试；没有完成新的真实应用点击验收，因此不能标为端到端已解决。
- macOS 14+ 激活权交接、companion overlay 的实际识别、多显示器、含动画的 OCR 控件仍待手动验收。当前仅完成代码检查、编译及不发真实事件的测试，本轮没有启动测试窗口。
- 输入读回与 `expected_text` 同传、总预算耗尽、目标变化和焦点环变化必须在自动测试中分别覆盖；测试通过不等于真实桌面可用。
- 手动窗口源码的用途、构建、启动与退出说明见 [manual fixture](../../tests/interaction_fixture/README.md)。它不是自动测试入口，也不随构建或部署启动。

本次复核实际修改范围：`native/interaction/{Model,Runtime}.swift`、`src/interaction.rs`、`tests/interaction_selftest/main.swift`、`tests/interaction_fixture/{main.swift,README.md}` 与本文档。当前仓库已存在联系人提交 `addaae0` 和通用眼手基线提交 `42ef72a`；这些是工作区现有历史，不是本次复核创建的提交。未跟踪的 `docs/reviews/2026-11-09-deep-audit.md` 属于另一项审计，本轮未修改。本轮未暂存、提交、推送或部署。

本次实际验证：`cargo test --bin wechat-mcp interaction -- --quiet` 9 项通过；`cargo test --test swift_interaction -- --quiet` 2 项通过（含无真实事件的 Swift 合成断言与本机架构 helper 编译）；Intel（x86_64、macOS 12 目标）helper 交叉编译通过；`git diff --check` 通过。没有运行手动 GUI fixture。
