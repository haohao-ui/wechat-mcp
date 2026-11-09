# 微信事件到 Agent 的桥接

旧版 Python Agent Bridge 已移除，原有验证命令不再可用；目前没有等价的 Rust Agent Bridge CLI，不应将它视为仍受支持的功能。

现有 Rust MCP 提供持久收件箱和事件领取、续租、确认工具，消费者需按事件身份及租约处理，执行结果不明时不能自动确认或重复调用付费模型。收件确认只表示消费者处理了事件，不表示微信送达、已读或已回复。

协议及服务测试使用 `cargo test --locked --test protocol_smoke` 和 `cargo test --locked service_tests`；这些隔离测试不代表真实微信或 Codex 接收器已完成端到端验证。
