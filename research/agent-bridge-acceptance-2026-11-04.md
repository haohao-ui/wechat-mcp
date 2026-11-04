# 微信 → Codex 首轮实测

日期：2026-11-04，Asia/Singapore。

## 结果

**通过：真实会话变化 → 持久收件箱 → 后台Codex执行器 → 匹配的结构化确认 → SQLite acknowledged。**

用户按提示从示例联系人甲发送`Agent推送测试1104`。既有监听器产生一条真实OCR界面变化事件，测试预览匹配。没有由本程序发送微信消息。

- event_id：`338e305e-3371-423e-91c3-5aebd9e5c275`。
- delivery_id：`f96c9c5bc0ce89c6b43bfd618500a411e011b0c069ee66c8f501a18e801766ec`。
- source=`desktop_ocr`，kind=`interface_change`，verified=false。
- Codex真实执行返回accepted=true、匹配的delivery_id/kind，incoming_verified=false。
- 桥接自动调用ack_event。正常连接关闭后，对已关闭的测试库以immutable只读方式核对，持久状态为acknowledged，source/kind/verified未被提升。
- MCP复查listener enabled=false、health.state=stopped，未留下本次后台监听。

## 当前桌面聊天路由

首次使用`codex queue --resume`返回排队成功但带警告输出，120秒内未收到消费确认。该路径没有通过后台自主唤醒验收。未把CLI退出0当作Agent收到。

随后使用应用正式消息工具投递相同delivery_id，当前聊天实际收到通知，读取并写出接收确认；这证明应用工具路由可达，属于人工发起的路由复核。未把这一步与后台自主唤醒混算。

自动闭环通过的是独立、非交互的官方Codex执行器。复用首次已入库且未确认的同一事件，用户无需重复发送；真实模型执行1次，没有为诊断重复请求模型。使用临时会话，不创建新的侧栏聊天；不加载用户MCP配置，禁用命令执行/应用/搜索工具，无微信回复授权。

## 验证与边界

15项Python桥接测试通过，覆盖真实确认前不ack、租约续期/丢失、账号隔离、跨线程确认拒绝、重启去重、证据冲突、未知投递不盲目重复，以及Codex执行确认不提升观察可信度。本轮修改未改变Rust业务逻辑或数据库结构。

这是单个真实界面变化事件的闭环，不是完整入站消息、群聊、P95实时性、无损接收或生产常驻验收。其他Agent和自建服务尚未实接；桌面聊天的CLI自主唤醒仍未通过。
