# T8 会话级主动转人工

范围：Issue #2 AC5，分支 `issue-2/handoff-api`，基于 T4 `1e4edd5`。已读取 Issue 正文及 16 条评论。

## 命令与字段

```sh
wechat-mcp control session-handoff --config /path/to/config.json \
  --request-json '{"session_id":"c_...","to":"human"}'
wechat-mcp control session-handoff --config /path/to/config.json \
  --request-json '{"session_id":"c_...","to":"bot"}'
```

`session_id` 与当前历史库的 `contact_id` 相同；必须从 `sessions-list` 获取，不能使用显示名。
可选 `account_id` 必须匹配当前有效账号。不存在、跨账号、已合并的会话均拒绝，且不创建目录。

成功：`{"ok":true,"data":{"account_id":"...","session_id":"c_...","contact_id":"c_...","handoff":true,"reply_mode":"human"}}`。
`to=bot` 返回 `handoff=false, reply_mode="bot"`。重复切换允许，最终状态不变。
失败只返回固定的 `invalid_arguments` / `not_found` / `store_unavailable` 类别和安全说明。

`sessions-list` 的 `data.sessions[]` 和 `session-detail` 的 `data.session` 均包含：

- `session_id`、`contact_id`、`handoff`、`reply_mode`（`human` / `bot`，状态读不到时为 null）。
- `auto_reply_enabled` 表示配置允许；`auto_replying` 表示有在线和健康证据的实际运行状态。
- 转人工时 `auto_replying=false`；恢复 bot 不绕过离线、暂停、联系人规则或群聊限制。
- `unread`、`delivery` 无底层依据时继续返回 null，不编造。

列表/详情使用 T2 分支现有的只读投影代码和相关测试。本分支基线没有 T2 实现，因此只复用了其会话部分；没有引入角色历史、MCP 或统计改动。调度方集成 T2 时应保留其其余能力，合并这两个命令的路由和投影字段。

## 实现

复用 `state_dir/sessions/<contact_id>/handoff.json`（0600，原子写入并同步目录），不改数据库结构。核心每次处理新消息及发送前读取标记；生成中发生的转人工阻断发送，重试同样检查。重建服务保留状态。已经进入桌面发送调用的操作无法撤回，确认弹窗说明了此边界。

会话页两个方向都二次确认；等待期间禁用重复操作，成功后更新按钮并刷新列表/详情。失败只提示无法确认结果、建议刷新，不展示进程输出或原始错误。新增文案集中在 `L10n.swift`，包含五种语言。没有安全会话 ID 的旧版状态回退只展示，不按名字发起操作。

主要文件：

- `src/core/control.rs`：命令路由、请求解析、安全错误。
- `src/core/control_handoff.rs`：按账号校验身份和持久化切换。
- `src/core/control_readonly.rs`：复用 T2 会话投影、状态字段及 Rust 测试。
- `src/core/turns.rs`：发送前拦截及运行时测试。
- `native/bootstrap/SessionsPage.swift`、`L10n.swift`：确认、请求、刷新及文案。
- `tests/session_handoff_cli.rs`：独立进程间持久化测试。
- `tests/swift_session_handoff.rs`、`tests/swift_host.rs`、`tests/host_t4_selftest/main.swift`：源码断言及 Swift 客户端行为断言。

## 验证与交接

实际结果（本地）：

- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `cargo clippy --all-targets --locked`：退出码 0，仍有仓库现有 warning 和复用投影代码的风格 warning。
- `cargo test --locked --lib core::control::readonly`：6/6；`--lib handoff`：6/6（两组有重叠）。
- `--test session_handoff_cli --test swift_session_handoff`：各 1/1。
- `--test swift_host`：77/77；同步更新了一条识别旧按名字恢复调用的源码断言。
- 独立 Swift 客户端自测通过，覆盖 human/bot 请求、结果校验、错误脱敏和五种语言。
- `swiftc -target "$(uname -m)-apple-macos12" native/bootstrap/*.swift native/eyes/Screens.swift -o /tmp/t8-host`：通过（额外指定可写模块缓存）。
- `cargo test --locked`：lib 为 1014 通过 / 2 失败 / 1 忽略；随后 `--no-fail-fast` 全量为 2247 通过 / 19 失败 / 3 忽略，含 lib/bin 重复执行的用例。该轮中的宿主源码断言失败已修正并复跑 77/77。
- 两项 gateway 测试及两项 protocol smoke 在隔离 HOME 后全部通过，原失败涉及不可写的默认桌面锁路径。
- 尚有 `swift_action` 5 项桌面目标不可达、`swift_native_frames` 5 项 Vision warm-up 失败；未将这些视为通过。
- `swift_brain_ui` 和 `swift_update_ui` 编译失败：其 HEAD 编译清单就缺少 `L10n.swift`，而 HEAD 的 `StatusModel.swift` 已依赖它；本任务未修改这两个测试。完整宿主编译及 T4 自测正常。

测试使用工作目录内 `CARGO_TARGET_DIR`，避免默认缓存目录写权限限制。输出位于 `/tmp/t8-*.log`，未写入仓库。

`.git` 为只读，没有尝试提交或绕过权限；由调度方提交。未 push、开 PR、合并、部署或修改 workflow。

真实微信会话 / 打包 `.app` 的按钮交互、截图录屏及 CI 尚未验收。本地宿主编译成功不表示这些验收已通过。Orca `run-current` 返回 runtime 元数据不可读；实际 `worker_done` 发送返回 `no_active_sender_terminal`（缺少 `ORCA_TERMINAL_HANDLE`）。没有冒用其他 terminal handle；完成报告未发送，本文件作为可复核交接记录。
