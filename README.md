# WeChat MCP Server (Rust)

基于 Rust 编写的微信自动化 MCP (Model Context Protocol) 服务端，专为 macOS 桌面端设计。

通过 macOS 原生无障碍辅助（Accessibility）与系统事件（System Events）实现对微信桌面版的安全自动化控制，**零封号风险**，保护账号安全。

---

## 核心特性

- **安全无封号**：不注入动态库、不篡改内存、不触碰微信私有协议，完全基于系统级合法辅助操作。
- **保护用户剪贴板**：执行粘贴前自动暂存当前系统剪贴板，并在发送操作完成后自动恢复，不影响日常复制粘贴。
- **标准 MCP 协议**：严格遵循 Model Context Protocol 标准（JSON-RPC 2.0 over `stdio`），开箱即用于 Claude Desktop、Cursor、Antigravity 等各类 AI 客户端。
- **纯净高性能**：基于 Rust + Tokio 异步 I/O，极小二进制体积与内存占用。

---

## 提供的 MCP Tools

1. **`send_wechat_message`**
   - **描述**：向指定联系人、群聊或文件传输助手发送文本消息。
   - **参数**：
     - `recipient` (string)：接收人昵称、备注名或群聊名称（如：`文件传输助手`、`张三`、`技术交流群`）。
     - `message` (string)：要发送的文本内容。
2. **`focus_wechat`**
   - **描述**：激活并置顶显示微信窗口。

---

## 编译与构建

确保本地已安装 Rust（1.75+）：

```bash
# 编译 Release 版本
cargo build --release
```

编译生成的可执行文件位于：
`target/release/wechat-mcp`

---

## 接入配置指南

### 1. Claude Desktop 配置

编辑 Claude Desktop 配置文件（路径通常为 `~/Library/Application Support/Claude/claude_desktop_config.json`）：

```json
{
  "mcpServers": {
    "wechat": {
      "command": "/Users/example/tools/code/Rust/target/release/wechat-mcp"
    }
  }
}
```

### 2. Cursor 配置

在 Cursor 的 `Settings -> Features -> MCP Servers` 中添加：
- **Name**: `wechat`
- **Type**: `command`
- **Command**: `/Users/example/tools/code/Rust/target/release/wechat-mcp`

### 3. Antigravity 配置

在 MCP 配置中添加：

```json
{
  "mcpServers": {
    "wechat": {
      "command": "/Users/example/tools/code/Rust/target/release/wechat-mcp"
    }
  }
}
```

---

## macOS 权限说明

由于本工具通过 AppleScript 控制微信桌面端，首次触发操作时，macOS 系统可能会弹出权限请求提示：
1. **辅助功能 (Accessibility)**：前往 `系统设置 -> 隐私与安全性 -> 辅助功能`，确保调用该程序的宿主应用（如 终端 / Claude / Cursor）已获得勾选授权。
2. **自动化 (Automation)**：前往 `系统设置 -> 隐私与安全性 -> 自动化`，允许宿主应用控制 `System Events` 和 `WeChat`。
