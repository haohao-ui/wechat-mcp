# 设计基线合并版在专用机的部署与只读验收记录

日期：2026-11-18，Asia/Singapore。专用机：Macmini7,1，Intel x86_64，macOS 12.7.6，控制台用户 `work`。

## 一、这次部署的是什么

- 分支 `integrate/design-base`（PR #15），基线是 11-14 的设计实现分支，并入 issue #2 的功能。
- 这次装机的是产品代码 `5590e04`（之后 `9bd1aa4` 只改了测试里的 OCR 比较，不影响 App 二进制）。部署命令：

  ```bash
  scripts/deploy_host.sh work@192.168.3.81 --app --worker --mcp --yes
  ```

- 部署脚本这次修了两处（属于部署链路缺陷，不是产品功能）：
  1. 写死的 `target/x86_64-apple-darwin/release/...` 改为读 `cargo metadata` 的 `target_directory`（本机 target 在 `~/.cache/cargo-target`）。
  2. `--mcp` 步骤以前只上传 `wechat-mcp`，**没有上传 `wechat-updater`**，导致机器上一直是 11-14 的旧 updater（没有 `rollback-status`），应用更新页的回滚卡片只能显示「暂不可用」。现在两个二进制一起上传、签名并保留 `.before-<ts>` 备份。
- 新增启动后自检：bootstrap 之后宿主如果立刻退出，脚本会打印 host.log / host.stderr.log / 最新 `.ips` / launchd 状态并非 0 退出（134 那类问题不再哑失败）。

## 二、只读真机验收

| 项 | 命令 | 结果 |
|---|---|---|
| 宿主与核心 | `ps`, `launchctl print gui/501/local.wechat-mcp.host` | `state=running`，`wechat-cs-host` + `wechat-mcp serve-core` 同时在线，无新增崩溃报告 |
| 权限 | 总览页实拍 | 「屏幕录制与辅助功能已授权」 |
| 今日已发送（AC4） | `wechat-mcp control status` | `stats.sent_today=0`，`triggered=72`，`client_echo_observed=70`；界面显示「今日已发送：0」 |
| 会话与转人工（AC5） | `wechat-mcp control sessions-list` | 返回真实会话及稳定 `contact_id`：`午夜咖啡 c_3a735fda2fee4eff`、`测试群 c_46653427f7874a2b`、`爱咖啡 …`；显示名唯一，转人工可按名唯一定位 |
| 历史版本（AC7） | `wechat-mcp control brain-history` | `generation=7`，但 `generations/versions` 为空、`roles` 为空；页面应显示空态，不编造 |
| 整包回滚（AC6） | `wechat-updater rollback-status --config …` | `{"version":"38755db-…","installed_at":null,"can_rollback":true,"reason":null}`，回滚卡片可启用；`installed_at` 为 null 与设计一致（不展示） |
| 五语 | 切 `defaults write local.wechat-mcp.host ui_language en` 后重启 | 窗口标题、9 项导航、总览内容全部英文；验证后已恢复 `zh` |

截图：`dist/issue-2-deploy-20261118/04-zh.png`、`05-en.png`。

## 三、暂停原因的真实性修正

`control status` 的健康原因是 `wechat_not_logged_in`，但旧界面统一显示「机器人已暂停（建议：点击恢复自动回复）」，会误导值班人。现在 `BotInput` 解析 `health.reason`，暂停时区分：

- `wechat_not_logged_in` → 「机器人已暂停：微信未登录或未运行（PID: …）」+「先启动并登录微信，再点恢复自动回复」
- 其它原因 → 原来的「已暂停」+ 恢复建议


## 五、破坏性但可回滚的真机验证（2026-11-18 14:00 SGT）

### 5.1 整包回滚（AC6）

| 步骤 | 证据 |
|---|---|
| 回滚前 | 磁盘 app `5590e04-20261118131512`；`rollback-status` → `{"version":"9c20bad-20261118131146","can_rollback":true}` |
| 错误 expected-version 试一次 | `{"error":"rollback_failed"}`，**没有发生替换**（expected 校验生效，失败安全） |
| 正确执行 | `wechat-updater rollback --config … --expected-version 9c20bad-20261118131146` → `{"status":"rolled_back","version":"9c20bad-20261118131146","restart_required":true}` |
| 回滚后磁盘 | `CFBundleVersion = 9c20bad-20261118131146` |
| 回滚后启动 | `launchctl kickstart -k` 后 host.log：`微信客服助手启动，版本: 0.1.0 (9c20bad-20261118131146)`，进程在线，无新增 `.ips` |
| 恢复生产版本 | `deploy_host.sh --app --yes` → 版本 `56bc98d-20261118141315`，宿主在线 |

结论：整包回滚在真机上确实能把 `.app` 换回上一版本，换回后能正常启动；生产版本可一键恢复。

### 5.2 会话转人工（AC5）

| 步骤 | 证据 |
|---|---|
| 初始 | 午夜咖啡 / 测试群 / 爱咖啡 均 `handoff=false, handling=auto_reply` |
| 转人工 | `control session-handoff --request-json {"session_id":"c_f1bb88779f584fa2","to":"human"}` → `{"handoff":true,"reply_mode":"human"}`；列表变为 `handoff=True, handling=handoff` |
| 转回机器人 | `to":"bot"` → `{"handoff":false,"reply_mode":"bot"}`；列表恢复 `handoff=False` |

用的是 0 条消息的「爱咖啡」，机器人当时处于暂停，验证后已转回自动回复。

### 5.3 main 分支状态

PR #15 已合并进 main（merge commit `fd4fd15`），**main 的 CI 全绿**（敏感信息扫描 / shellcheck / 构建与测试 macOS）——这是 10-19 之后 main 第一次绿。

## 六、仍然没有执行的部分（不构成验收通过）

1. **仍未执行**：真实微信收发（需要先启动并登录微信）、真实模型试探（会调用外部模型）、群聊/私聊区分与未读数采集（T10–T12 无可靠数据源）。整包回滚与会话转人工已在第五节实测通过。
2. 微信客户端未运行，机器人处于暂停：真实收发、群聊/私聊区分、未读数、送达状态都没有新的实机证据（T10–T12 仍缺可靠数据源）。
3. 视觉保真度只做到文本层（OCR + AX 控件树 + 设计说明逐条对齐），间距、字号、图标需要人眼确认。
4. 安装时间（`installed_at`）按决定不展示，回滚卡片只显示上一版本号。
