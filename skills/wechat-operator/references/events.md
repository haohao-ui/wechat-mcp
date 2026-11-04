# 会话变化与 Agent 接收

## 监听和消费

显式使用 `start_listener` 启动选定账号的界面观察，以 `get_listener_status`检查启用状态和health。参数间隔2–60秒不是端到端时延承诺；截图/OCR还会占用时间。

处理现有持久收件箱：

1. `wait_for_events` 等待可用事件；返回事件不代表已经领取。
2. `claim_events` 以稳定consumer领取租约，保存原event_id和lease_token。
3. 核对事件账号、source、kind、verified、payload及覆盖范围，再交给目标Agent。
4. Agent实际接收后，使用领取时的consumer和lease_token调用 `ack_event`。
5. 处理较慢时调用 `renew_event_lease`。租约失效后旧令牌不能确认；明确放弃且仍持有有效租约时可用 `release_event`。

转交给外部Agent的数据不包含lease_token。相同event_id的投递保持稳定delivery_id，接收端按这个标识去重；不同事件即使正文相同也不能合并。

## 可信度

`interface_change`、`source=desktop_ocr`、`verified=false`、`incoming_verified=false`表示会话界面观察。不能改称“已确认入站消息”，也不能据此认定发送人、完整正文或消息未丢失。

预览未变化的重复消息可能无法触发事件，滚动和布局变化也需要区分。完整气泡、方向、群成员、持久消息游标与补读尚未齐备；遇到歧义或缺口保留unknown。

## Agent桥接

安装/注册 MCP 只是让Agent能够调用工具，不等于它会被新事件唤醒。区分：事件已入库、通知已提交、Agent实际接收、业务处理完成。

当前可选接收器：

- 后台Codex执行器：已有单事件真实接收和结构化确认闭环；不代表当前桌面聊天被自主唤醒。
- Codex桌面排队：CLI退出0也可能没有恢复聊天，必须等实际消费确认。应用消息工具的投递复核不能算作CLI后台唤醒成功。
- 其他Agent/自建服务：尚未实接；沿用稳定事件与确认契约增加适配器，不替换微信驱动。

项目桥接模块是独立控制端程序，不是这个Skill内的后台服务。先发现项目和可执行入口，再按用户请求启动；当前验证器有时限且单次处理一个匹配事件，未安装开机常驻服务。进程退出不一定清除持久监听启用状态，结束后用 `get_listener_status`复查。

专用GUI宿主目前只允许一个MCP连接。桥接持有连接时，不另起连接竞争桌面。Agent确认只表示事件已接收；自动回复需要用户另有对应授权。
