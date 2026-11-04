# 微信事件到 Agent 的桥接

首轮验证接收端为Codex。控制端要求Python3.11及以上、已登录的Codex CLI；旧Mac测试机继续使用已授权GUI宿主，不修改其bundle或签名。现有MCP参数及SQLite表结构保持不变。

## 模块

- `scripts/agent_bridge/mcp.py`：持久MCP连接、串行请求、账号核验；取消或响应错误时丢弃连接。
- `pump.py`：等待现有持久收件箱、领取事件、续租、等待Agent确认后ack。它只依赖接收器的`notify(account,event)`与`receipt(delivery_id)`，便于后续替换平台。
- `codex_exec.py`：官方非交互执行器接收器；只读、临时会话、不加载用户MCP配置、禁用应用/命令执行与搜索工具，返回结构化确认。不是桌面聊天的自动唤醒。
- `codex.py`：桌面排队接收器与私有事件文件。CLI退出0只代表排队调用结束，仍须当前聊天实际写入消费确认。
- `__main__.py`：有时限、单个匹配事件的验收入口。退出时停止由本次开启的监听；不接管已经启用的监听。

## 事件契约

事件信封包含version、delivery_id、账号、原事件来源/可信度/载荷；不会向接收器传递lease_token。delivery_id由平台、账号、持久event_id共同计算。同一正文的不同event_id仍是不同事件，不按正文集合去重。

Codex只有返回完全匹配的delivery_id、kind及incoming_verified，才产生接收确认；之后使用原领取令牌调用ack_event。领取或续租失败时不确认。处理较慢时续租。接收端超时或执行结果不明时保留证据与待恢复事件，不把CLI退出0当作完成，也不自动重复执行付费模型调用。当前需要人工复核未知执行后再决定重试。

事件文件目录0700、文件0600，保留重启后的投递身份与确认。收件箱仍是既有SQLite，不另建第二个消息数据库。发起消费确认只表示Agent读取了事件，不表示微信送达、已读或已回复。

## 验证入口

```bash
python3 -m scripts.agent_bridge validate-codex \
  --mcp-command '["/absolute/path/wechat-mcp"]' \
  --account local --receiver exec \
  --spool /absolute/private/spool \
  --recipient 测试联系人 --expect-preview 唯一测试正文 \
  --duration 300 --receipt-timeout 240
```

远程测试时mcp-command可配置为SSH连接GUI宿主私有socket的参数数组。命令以参数数组执行，不展开shell文本；SSH远端命令字符串仍需由操作者提供正确引用。`ready`表示至少一轮会话观察成功，此后再发送测试文字。

桌面排队验证使用`--receiver desktop --thread <现有聊天UUID>`。本机Codex0.166.0的CLI排队没有成功唤醒当前桌面聊天；应用正式消息工具可以投递到当前聊天，但这属于单独的应用工具路由，不能据此宣称后台CLI已实现自主唤醒。官方说明也明确指出queue的resume失败可能只打印警告而仍退出成功：[Codex开发命令](https://learn.chatgpt.com/docs/developer-commands)。

## 目前限制

- 输入事件仍为desktop_ocr/interface_change、verified=false、incoming_verified=false。仅检测可见会话预览变化，尚无完整气泡、方向/群成员、消息ID与完整补读；重复相同预览可能不产生事件，不能保证每条消息不漏。
- 2秒监听参数不是2秒到达保证；截图/OCR和Agent执行另有耗时。本轮没有测P95或丢失率。
- 目前是有时限、单事件验证器，尚未安装开机常驻服务、自动重连监督器、重试/死信控制或多接收端路由。
- 专用机GUI宿主一次只允许一个MCP连接。桥接等待期间占用该连接，后续生产架构需要共享会话路由或独立采集服务；不要让Agent另起连接竞争。
- 验证用过滤器领取最多100条再释放不匹配事件，不确认其他事件。大积压可能阻塞目标，不能当作生产订阅过滤器。
- 若进程被强杀或MCP连接失效，finally不一定能清除持久监听开关，需要复查get_listener_status并显式停止。常驻服务生命周期仍待实现。
- 其他Agent平台/自建服务尚未实接；未来沿用事件及接收确认契约实现适配器，不把它们耦合进微信驱动。

测试：`python3 tests/test_agent_bridge.py -q`。测试使用本地假接收器，不触发真实Codex或微信；已加入CI配置，尚无远端CI结果。
