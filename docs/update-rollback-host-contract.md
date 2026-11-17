# T7：整包 .app 回滚宿主契约

本次按 2026-11-17 的产品决定，沿用现有安装位置，整包替换上一份
`微信客服助手.app`。不迁移为 releases/current 布局，不恢复数据库，不联网，
不修改发布工作流。UpdatePage.swift 由 T6 接入，本任务没有修改界面。

## 宿主命令和 JSON

1. 查询：`wechat-updater rollback-status --config <config.json>`。
2. `can_rollback=false` 时禁用按钮，直接展示中文 `reason`。
3. 用户二次确认后执行：
   `wechat-updater rollback --config <config.json> --expected-version <version>`。
   后端关闭核心运行开关、等待退出并持有核心锁；宿主不要同时发起更新、启停或退出。
4. 成功后按 `restart_required` 重启宿主应用，继续使用原有启动方式。
5. 操作中断或存在维护标记时执行：
   `wechat-updater rollback-recover --config <config.json>`。
   未完成事务恢复回滚前的整包应用；已经提交的事务只完成清理，不撤销成功回滚。

查询成功退出码 0，固定字段（无法确定时 version / installed_at 为 null）：

```json
{"version":"abc123-20261117103000","installed_at":null,"can_rollback":true,"reason":null}
```

执行成功退出码 0：

```json
{"status":"rolled_back","version":"abc123-20261117103000","transaction_id":"UUID","restart_required":true}
```

恢复成功退出码 0，同样保留四个字段。恢复旧当前包时 status 为
`current_restored`、version 为回滚前版本；完成已提交事务清理时 status 为
`rolled_back`、version 为回滚目标版本。重复恢复不会再次交换应用。

执行失败退出码 1：

```json
{"error":"rollback_failed","details":{"version":null,"installed_at":null,"can_rollback":false,"reason":"没有找到上一版本的应用备份。"}}
```

`details` 是失败后的最新可用性查询，不能据此推断上次执行成功。
恢复失败返回 `{"error":"rollback_recovery_failed"}`，退出码 1。
缺少 `--expected-version`、参数错误、配置不可读取返回固定 error 类别，退出码 2。
所有回滚输出均不包含底层原始报错、密钥或备份路径。

## 上一份应用的识别

实际部署规则来自 `scripts/deploy_host.sh`：

- 当前应用：`$HOME/Applications/微信客服助手.app`。
- 备份：`$HOME/.wechat-rpa/backups/微信客服助手.app.before-<Unix秒>.bak`。
- 只接受准确的前后缀和整数时间戳，选择时间戳最大的那份。
  最新备份损坏时不悄悄跳过它去选择更早版本。
- version 从备份 `Contents/Info.plist` 的 **CFBundleVersion** 读取。
  部署脚本在这里写入 git revision 和构建时间；不是读取固定的
  CFBundleShortVersionString，也不从目录名或外部 VERSION 文件猜测。
- installed_at 为 null：现有备份没有旧版本的准确安装时间，不能把备份时间
  冒充安装时间。备份名的 Unix 秒仅用于选择最近的一份；UI 按原契约处理未知时间。
- 使用当前用户 HOME 下的现有整包布局；不依赖 install_root，不更改该配置。

没有备份、版本读取失败、资源缺失、签名损坏、版本与当前一致，以及存在未完成
维护/更新时均禁用。reason 是固定中文说明，不回显系统工具输出。
核心运行本身不禁用按钮：执行入口负责停服；无法在 60 秒内获得核心锁则拒绝替换。

## 前置检查、事务和失败恢复

执行前持有 updater 锁并重新选择和验证备份，校验 expected-version，拒绝过期确认。
校验应用标识、主可执行文件、三个部署资源文件、执行权限和
`codesign --verify --deep --strict`。不重新签名，不调用网络签名服务。

`state_dir/app-rollback-journal.json` 保存每一步和原应用目录的 device/inode、
原核心开关状态、transaction_id。独立 journal 避免让既有启动 guard 把整包事务误认
为需要 releases/current 的版本化事务。既有 update-journal.json 和发布序列不变。

顺序如下：先持久化意图，关闭 `$HOME/.wechat-rpa/core.enabled`，等待核心退出并
取得核心维护锁，再写宿主已支持的 `core.maintenance` UUID 标记以阻止自动重启。
将备份用 ditto 完整复制到当前应用同目录的 `.rollback-current-<UUID>.app`，再次
校验副本，然后使用 macOS `renameatx_np(RENAME_SWAP)` 原子交换两个目录。
交换后该隐藏目录保存**回滚前当前应用的完整副本**，不删除它；原始备份也不移动。
交换不产生当前 .app 路径缺失的中间状态。副本的文件和目录在交换前完成 fsync。

交换后再次验证目标包并持久化提交；恢复原运行开关、移除本事务维护标记。
清理完成标志也持久化，重复恢复不会覆盖用户之后主动关闭的运行开关。
成功返回要求宿主重启。任何执行阶段失败都按 device/inode 判断是否交换过；如已
交换，原子换回原包，写 RolledBack journal，再恢复开关和清理标记。
恢复失败保持非成功状态和恢复证据，不声称当前包已恢复。
断电后 recover 同样通过原目录身份识别交换前后，不依赖「交换后 journal 已写入」
这个有中断窗口的假设。备份应用不能覆盖外部 `~/.wechat-rpa/bin` 辅助程序，也不改变
历史数据库或其 schema；本功能保证的是宿主 .app 整包回滚，外部组件兼容性须实机验收。

## 保留代码与验收边界

原 manual_rollback.rs 和版本化 sandbox updater 的 preserve_previous 接入保留，
供旧有 sandbox 测试及 sequence guard 使用；生产三个 rollback CLI 子命令全部改为
app_rollback.rs，没有版本化安装迁移，也不依赖 previous-release.json。

Rust 测试使用临时 HOME、真实 plist、临时 ad-hoc 签名 .app，覆盖正常交换、无备份、
损坏/版本不可读、最新备份不跳过、过期确认、互斥、交换中途失败恢复、交换前后中断
恢复，以及真实 CLI JSON 和错误脱敏。这些不是完整产品 .app 的运行验收。
T6/QA 仍需在 MacBook 的实际部署包上验证停服、回滚后宿主重启和外部组件兼容性，
并附截图、录屏与 CI 结果。
