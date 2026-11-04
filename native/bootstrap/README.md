# 专用 Mac 测试宿主

这个宿主解决“SSH 可连接，但 SSH 进程没有桌面权限”的执行上下文问题。它复用现有 Rust MCP 服务，通过本机 Unix socket 透传原有 JSON-RPC，不新增微信业务网关。

当前组成：

- `Probe.swift`：只读取显示器、微信窗口元数据和当前进程权限，不保存聊天内容。
- `main.swift`：AppKit 环境检查、用户主动请求权限、启动桌面服务。
- `SocketWorker.swift`：当前用户专用的本机连接、单个 MCP 子进程托管和退出清理。
- `gateway.test.json`：单个 `test-desktop` 桌面账号路由；不是微信账号身份验证结果。

## 执行和文件布局

GUI 应用：`~/Applications/WeChat RPA Test.app`，bundle ID 为 `local.wechat-mcp.desktop-probe`。

应用资源 `Contents/Resources/gateway.test.json` 保存测试路由配置。实际执行程序放在 `~/.wechat-rpa/bin/`，包含 `wechat-mcp`、`wechat_ocr`、`wechat_click`、`wechat_window`，避免业务二进制更新改变 GUI 宿主的签名。宿主自身更新仍可能需要 macOS 重新授权；当前开发版为本地 ad-hoc 签名。

`~/.wechat-rpa/` 必须归当前用户所有且权限为0700，socket权限为0600；连接还检查对端UID。worker二进制必须是当前用户拥有、不可被组或其他用户写入的可执行普通文件。socket不监听TCP端口，SSH仍负责远程认证和加密。

宿主只有在屏幕录制和辅助功能均已授权时才启动桌面服务。**桌面应用的授权不会授予从SSH直接运行的同名程序。** 测试客户端应连接已授权GUI宿主创建的socket，而不是从SSH再启动一份MCP服务。

数据保存在 `~/.wechat-rpa/test.sqlite3`。已有inbox/outbox、账号隔离和桌面锁语义继续由Rust服务处理。退出宿主会终止当前MCP子进程；正在执行的动作结果仍应查询持久流水，不自动重试发送。

## macOS 12 测试模式

宿主和三个旧驱动助手在测试机用Swift5.7编译，Rust程序以`MACOSX_DEPLOYMENT_TARGET=12.0`和`x86_64-apple-darwin`构建。当前宿主明确使用`WECHAT_MCP_DESKTOP_RUNTIME=legacy`，未宣称现有要求macOS14的stream实现已适配macOS12。

编译宿主：

```bash
swiftc -O -target x86_64-apple-macos12 \
  native/bootstrap/Probe.swift native/bootstrap/SocketWorker.swift native/bootstrap/main.swift \
  -o wechat-desktop-probe
```

签名和启动前须按上述目录准备App bundle、Info.plist、路由配置及worker程序。不会自动安装依赖、启动登录项或修改系统授权。

诊断CLI `--json`不会请求权限。`--transport-self-test`启动本机连接并对真实Rust服务执行initialize和tools/list，不访问微信桌面。GUI运行且socket已占用时，自检会明确失败，不覆盖现有连接。

“系统设置已勾选”但API仍返回false时，刷新仅重新查询权限，不会修复旧签名记录。应先确认实际bundle路径与进程，再检查TCC日志是否出现`Failed to match existing code requirement`。本次已证实ad-hoc更新造成此类旧记录；恢复时只重置本应用的对应服务记录，通过系统提示重新授权，并重启相同二进制验证。不要全局重置TCC，不要把重复勾选或SSH进程的查询当作GUI宿主授权成功。

远程客户端可通过SSH调用系统`nc -U ~/.wechat-rpa/worker.sock`传输原始MCP协议。一个宿主同时只接纳一个连接；断开后MCP进程收尾，随后可重新连接。目前没有多客户端排队和系统服务托管。

## 当前能力边界

宿主提供执行环境，不新增消息、群成员或好友处理能力。现有MCP仍是OCR文字读取和发送触发接口；气泡级消息、发送内容读回/回显、群成员识别、添加好友及好友申请处理还需要分别实现和验收。
