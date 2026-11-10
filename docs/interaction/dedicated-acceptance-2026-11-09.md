# 专用机通用眼手验收（2026-11-09）

状态：**blocked**。用户授权在专用机启动验证；本轮仅专用 Mac 弹出独立测试窗口，结束后已退出，未操作微信消息或业务数据。

## 部署核对

- 目标 macOS 12.7.6（Intel）。现有 MCP 位于 `~/.wechat-rpa/bin/wechat-mcp`，工具列表没有 `desktop_*`；标准目录起初不存在 `desktop_helper`。不能将“机器已更新”当作本功能已部署完整。
- 本轮上传当前 Swift 源码到 `~/.wechat-rpa/desktop-acceptance`，在目标机编译 helper 和独立 fixture，使用已有 `WeChat CS Local Signing` 签名。
- 首次使用隔离目录 helper，随后将签名 helper 安装到原本不存在的 `~/.wechat-rpa/bin/desktop_helper`，复核 companion 与 helper 同目录的行为。未替换 MCP、核心或 worker，未重启现有服务。

## 真实结果

- 原生 helper 读取测试窗口：AX 可用、未截断，按钮与文本输入框及 OCR 可见。
- 原生 `focus` 返回 `activation=confirmed`（标准目录复测耗时 0.23 秒）。这不是后台切换或 macOS 14 协作式激活的完整验收。
- 原生 `click` 在执行前返回 `window_occluded`（0.04 秒）；没有实际点击，后续输入与读回验证停止。
- 窗口列表显示系统 Dock 的窗口 ID 21、layer 20、alpha 1、bounds `(0,0,1280,1024)` 覆盖整个屏幕，排在测试窗口上方，与按钮区域相交。缩小遮挡区域仍不足以避开这类系统窗口；没有通过忽略所有 Dock 窗口的硬编码绕过检查。
- 原始结构化动作结果副本：`target/desktop-acceptance/dedicated-results.json`；远端保存在 `~/.wechat-rpa/desktop-acceptance/results.json`。

## 未通过与后续

遮挡判定仍是发布阻塞，需区分系统合成/输入窗口与真正阻挡目标的交互窗口。真实点击、光标闪烁输入、独立读回字段、总预算、overlay 相交及 MCP 全链路均不能记为通过；应在修正并更新完整组件后重新验收。测试窗口已正常退出，临时验收源码与结果保留供复核。
