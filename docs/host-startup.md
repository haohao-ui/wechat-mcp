# 宿主启动与 Gatekeeper 恢复说明

日期：2026-11-13

本说明针对专用 Mac 上“微信客服助手 App 双击没反应 / 起不来”的已知情况。

## 现象

- Finder 双击 `微信客服助手.app` 后没有窗口，Dock 图标可能短暂出现。
- `pgrep -f "微信客服助手.app/Contents/MacOS/wechat-cs-host"` 能看到进程，但状态为 `T`（stopped），`sample` 只显示 `_dyld_start`。
- `launchctl print gui/$(id -u)/local.wechat-mcp.host` 显示 `state = exited` 或 `not running`。
- 系统日志中 `syspolicyd` 报 `rejecting due to lack of matching active rule`。
- `spctl -a -vvv -t exec "$HOME/Applications/微信客服助手.app"` 可能返回 `rejected`。

## 原因

宿主 App 使用本机自签名证书 `WeChat CS Local Signing` 签名，没有 Developer ID 签名和公证。

- 通过 LaunchAgent 直接执行 App 可执行文件时，签名有效、TCC 权限保持，可以正常运行。
- 通过 Finder / LaunchServices 双击启动时，Gatekeeper 会评估该 App；未公证的自签名包会被拒绝，并可能留下一个停在 dyld 的 stopped 进程。

这不是 core、配置文件或数据库损坏。

## 推荐启动方式

### 方式一：双击启动脚本

将仓库里的 `scripts/start_host.command` 放到专用机桌面（或任意目录）：

```bash
cp scripts/start_host.command "$HOME/Desktop/启动微信客服助手.command"
chmod +x "$HOME/Desktop/启动微信客服助手.command"
```

双击该文件，它会：

1. 检查 LaunchAgent 是否存在；
2. 必要时 `launchctl bootstrap`；
3. 执行 `launchctl kickstart -k`；
4. 等待宿主进程出现；
5. 清理未被 LaunchAgent 管理的残留宿主进程（例如 Finder 双击后卡在 dyld 的实例）。

### 方式二：手动命令

```bash
launchctl kickstart -k "gui/$(id -u)/local.wechat-mcp.host"
```

如果 LaunchAgent 未加载：

```bash
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/local.wechat-mcp.host.plist"
```

### 方式三：登录时自动启动

`local.wechat-mcp.host.plist` 已设置 `RunAtLoad=true`。正常情况下登录后会自动启动。

注意：`KeepAlive.SuccessfulExit=false`，所以用户主动退出 App 后 LaunchAgent 不会反复拉起；需要手动 kickstart 或重新登录。

## 卡住进程的处理

如果进程停在 `T` 且没有窗口：

```bash
pgrep -fl "微信客服助手.app/Contents/MacOS/wechat-cs-host"
kill -9 <pid>
launchctl kickstart -k "gui/$(id -u)/local.wechat-mcp.host"
```

不要同时保留多个实例。

## 微信未登录

core 的 `health.json` 可能显示：

```json
{"state":"paused","reason":"wechat_not_logged_in"}
```

此时需要：

1. 在专用机打开微信并登录；
2. 等待 core 恢复正常轮询；
3. 若仍为 paused，重新执行一次 LaunchAgent kickstart。

## 日志

- 宿主启动/退出：`$HOME/.wechat-rpa/logs/host.log`
- 宿主 stderr：`$HOME/.wechat-rpa/logs/host.stderr.log`
- core 日志：`$HOME/.wechat-rpa/logs/core.log`
- 健康状态：`$HOME/Library/Application Support/WeChatRPABot/state/health.json`

## 长期修复

要恢复 Finder 双击启动，需要对 App 使用 Developer ID 签名并公证；本机自签名不能替代。未完成签名/公证前，请使用 LaunchAgent 或上述启动脚本。
