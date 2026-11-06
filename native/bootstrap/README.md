# 微信客服助手宿主程序

这个宿主解决“SSH 可连接，但 SSH 进程没有桌面权限”的执行上下文问题。它复用现有 Rust MCP 服务，通过本机 Unix socket 透传原有 JSON-RPC，不新增微信业务网关。

当前组成：

- `Supervisor.swift`：纯逻辑退避状态机（`RestartPolicy`：退避序列 5s、15s、60s、60s，滚动 1 小时 5 次上限后停止重试）。
- `Probe.swift`：只读取显示器、微信窗口元数据和当前进程权限，不保存聊天内容。
- `main.swift`：AppKit 环境检查、用户主动请求权限、启动桌面服务。
- `SocketWorker.swift`：当前用户专用的本机连接、MCP 子进程退避托管、退出日志记录与清理。
- `gateway.test.json`：单个 `test-desktop` 桌面账号路由；不是微信账号身份验证结果。
- `local.wechat-mcp.host.plist`：LaunchAgent 模板，用于登录自启及异常崩溃保活（`KeepAlive={SuccessfulExit=false}`，`ThrottleInterval=30`）。

## 执行和文件布局

GUI 应用：`~/Applications/微信客服助手.app`，bundle ID 为 `local.wechat-mcp.host`，可执行文件为 `wechat-cs-host`。

应用资源 `Contents/Resources/gateway.test.json` 保存测试路由配置。实际执行程序放在 `~/.wechat-rpa/bin/`，包含 `wechat-mcp`、`wechat_ocr`、`wechat_click`、`wechat_window`，避免业务二进制更新改变 GUI 宿主的签名。宿主自身更新仍可能需要 macOS 重新授权；当前开发版为本地 ad-hoc 签名。

`~/.wechat-rpa/` 必须归当前用户所有且权限为 0700，socket 权限为 0600；连接还检查对端 UID。worker 二进制必须是当前用户拥有、不可被组或其他用户写入的可执行普通文件。socket 不监听 TCP 端口，本机模式通过 Unix socket 直连，远程仍由 SSH 负责认证和传输。
子进程退出日志记录在 `~/.wechat-rpa/logs/host.log`，目录权限 0700。

宿主只有在屏幕录制和辅助功能均已授权时才启动桌面服务。**桌面应用的授权不会授予从 SSH 直接运行的同名程序。** 测试客户端应连接已授权 GUI 宿主创建的 socket，而不是从 SSH 再启动一份 MCP 服务。

数据保存在 `~/.wechat-rpa/test.sqlite3`。已有 inbox/outbox、账号隔离和桌面锁语义继续由 Rust 服务处理。退出宿主会终止当前 MCP 子进程；正在执行的动作结果仍应查询持久流水，不自动重试发送。

## macOS 12 编译与部署

宿主在专用 Mac（macOS 12，Intel）上用系统 Swift 5.7 编译，Rust 程序以 `MACOSX_DEPLOYMENT_TARGET=12.0` 和 `x86_64-apple-darwin` 构建。当前宿主明确使用 `WECHAT_MCP_DESKTOP_RUNTIME=legacy`。

编译宿主命令：

```bash
swiftc -O -target x86_64-apple-macos12 \
  native/bootstrap/Supervisor.swift \
  native/bootstrap/Probe.swift \
  native/bootstrap/SocketWorker.swift \
  native/bootstrap/main.swift \
  -o wechat-cs-host
```

部署脚本：

开发机上提供 `scripts/deploy_host.sh` 进行自动化部署与更新：
```bash
# 默认 dry-run 检查执行计划
scripts/deploy_host.sh <host>

# 确认执行部署（支持 --app, --worker, --bot 单独部署或全部部署）
scripts/deploy_host.sh <host> --yes
```

诊断 CLI `--json` 不会请求权限。`--transport-self-test` 启动本机连接并对真实 Rust 服务执行 initialize 和 tools/list，不访问微信桌面。GUI 运行且 socket 已占用时，自检会明确失败，不覆盖现有连接。

“系统设置已勾选”但 API 仍返回 false 时，刷新仅重新查询权限，不会修复旧签名记录。应先确认实际 bundle 路径与进程，再检查 TCC 日志是否出现 `Failed to match existing code requirement`。恢复时只重置本应用的对应服务记录，通过系统提示重新授权，并重启相同二进制验证。不要全局重置 TCC，不要把重复勾选或 SSH 进程的查询当作 GUI 宿主授权成功。

客户端可通过 Unix socket 直连（`nc -U ~/.wechat-rpa/worker.sock`）传输原始 MCP 协议。一个宿主同时只接纳一个连接；断开后 MCP 进程收尾，随后可重新连接。

## 当前能力边界

宿主提供执行环境，不新增消息、群成员或好友处理能力。现有 MCP 仍是 OCR 文字读取和发送触发接口；气泡级消息、发送内容读回/回显、群成员识别、添加好友及好友申请处理还需要分别实现和验收。
