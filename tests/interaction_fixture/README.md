# 手动桌面验收窗口

此程序仅用于人工选择运行的 GUI 验收。`cargo test`、构建、部署脚本都不会启动它。它会在本机弹出独立窗口，不访问网络、业务应用或业务文件。不要在无人知情时执行下面的启动命令。

仅编译（不会弹窗）：

```sh
mkdir -p target/manual-fixture/DesktopFixture.app/Contents/MacOS
swiftc tests/interaction_fixture/main.swift -o target/manual-fixture/DesktopFixture.app/Contents/MacOS/fixture
cat > target/manual-fixture/DesktopFixture.app/Contents/Info.plist <<'PLIST'
<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.desktop.acceptance-fixture</string>
<key>CFBundleExecutable</key><string>fixture</string>
<key>CFBundleName</key><string>DesktopFixture</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
```

明确决定进行本机 GUI 验收后，才手动启动：

```sh
open target/manual-fixture/DesktopFixture.app
```

通过 MCP 的 `desktop_windows` 选择该测试应用的 window ID，再调用 `desktop_observe`，bundle ID 为 `local.desktop.acceptance-fixture`。每次动作使用返回的新 `observation_id`：

- 观察按钮后点击，`expected_text` 使用 `Fixture completed 1`；下一次点击应显示 `Fixture completed 2`，避免把上一轮结果当作新成功。
- 对 `identifier=fixture-input` 的 AX 文本控件执行 `type_text`，同时带 `expected_text`，核对 `input_readback` 与 `verification` 两个独立字段。
- 手动令输入框出现闪烁光标，验证不因整窗像素变化就拒绝 AX 目标；真正移动窗口、编辑目标内容或改变目标身份应被拒绝。
- 将测试应用放到后台，再显式 `focus`。只有实际应用和窗口到了前台，才能返回 `activation=confirmed`；协作式激活未获准时应报 `activation_not_confirmed`。
- 测试结束后用测试窗口的关闭按钮或应用菜单退出，并清理 `target/manual-fixture`。不要使用会匹配其他应用的模糊进程终止命令。

本轮未运行此 GUI 验收。当前已知边界及上一次验收失败记录见 [通用眼手 MCP](../../docs/interaction/desktop-tools.md)。
