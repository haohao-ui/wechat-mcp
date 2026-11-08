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

截图以 MCP `image/png` 内容块返回；结构化结果不重复携带整段 base64。未请求截图时仍会截图并用本机 Vision 识别，图像仅用于感知和指纹，不保存截图文件。MCP 客户端会收到所选窗口的文字；调用方应按任务选择应用和窗口。

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
| focus | 窗口内任一有效 target_id | 激活绑定应用并尝试抬起对应 AX 窗口 |
| click / right_click / double_click | target_id | 在目标有效区域内采样落点；双击复用同一落点，发送原生 clickCount |
| drag | target_id、destination_id | 源与目的属于同一次观察；按原轨迹算法拖拽，途中停止也释放鼠标 |
| scroll | target_id、非零 lines、ticks | 在目标内滚动，保留动作间自然节奏 |
| key | target_id、keys | 焦点控件或窗口；支持 cmd/shift/alt/ctrl 组合、方向键、Tab、Home/End 等。字母是物理快捷键位置，不承担任意 Unicode 文本输入 |
| type_text | 可编辑 AX 文本控件、text | 拟人点击后核对焦点，保留/恢复剪贴板，以粘贴输入，按 AX 值读回核验。不自动按回车 |

输入长度、滚动次数和等待预算受限。工具拒绝未定义字段和与动作无关的参数。没有裸坐标动作接口，控件必须来自服务端保存的观察。

## 有效性、停止和结果

- 观察保存在当前 MCP 进程内，数量有界，不写入业务数据库；断线重建进程后需重新观察。
- 每个编号只能使用一次，失败也不会恢复。重复调用不会重复触发动作；过程异常时必须查看新画面再决定下一步。
- 执行前重新观察并核对应用、PID、窗口 ID、窗口几何、像素及 AX 状态指纹。动态内容也会使观察失效，这是保守保护，不能绕过后继续点。
- 除显式 focus 外，动作要求绑定窗口在前且未被其他窗口遮挡；执行中继续检查取消、父进程存活、前台应用及窗口状态。
- 与微信流程共用 `message-gateway/desktop.lock`，不同进程也不能同时操作桌面。
- `desktop_stop {}` 取消本 MCP 会话的通用动作并使观察失效，不停止微信机器人。返回 `cancellation_requested` 不代表已停止；必须等待原动作返回。鼠标和修饰键在中断路径释放。
- `status=executed` 仅表示动作函数完成。`verification=passed/failed` 表示指定文字在后续观察中是否出现；不是“界面发生了指定业务变化”的证明。输入读回单独报告 `input_readback_passed`。
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

最终定向验证：通用接口 8、网关 18、服务流程 23、原拟人动作 29、Swift 边界 3、通讯录 Swift 9、通用 Swift 2、桌面锁 2，共 94 项测试通过；Intel 与 Apple Silicon 的 macOS 12 helper 编译通过，Shell 语法和 `git diff --check` 通过。常驻 helper 引入后，重复调用测试暴露了桌面租约随复制描述符延续的问题，已改为租约析构时显式解锁，并增加回归测试。未提交、推送或部署。
