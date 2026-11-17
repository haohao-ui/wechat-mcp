# T7 应用程序回滚宿主协议（Refs #2）

本实现是离线程序版本回滚，不恢复业务数据库，不访问网络或发布渠道。
未改动 UpdatePage.swift。UI 二次确认、停止/重新启动服务由 T6 接入。

## 宿主命令

以下命令调用随宿主打包的 `wechat-updater`，不要通过 shell 拼接参数。
`--config` 使用实际 Bot 配置文件，需为现有 BotConfig 支持的私有配置。
配置中的 `install_root` 必须是绝对路径，并有 releases/current/previous 布局。
`state_dir` 使用配置值，不假设一定是 install_root/state。

1. 查询：`wechat-updater rollback-status --config <config.json>`
2. UI 展示 version 和 installed_at（二者可为 null），展示二次确认。
3. 用户确认后停止核心服务，禁止 supervisor 在此期间自动重启；重新查询。
4. 执行：`wechat-updater rollback --config <config.json> --expected-version <查询返回的version>`
5. 成功后宿主重新启动核心服务，检查实际运行结果。
6. 断电/进程被杀导致事务中断：保持停服，运行
   `wechat-updater rollback-recover --config <config.json>`，恢复回滚前的当前程序。
   该命令仅处理本协议的 RollbackStarted，不处理其他更新事务。

查询是只读的，不创建锁、journal 或元数据。服务运行时 `can_rollback=false`，
reason=stop_core_first，仍返回已知上一版本，UI 可先显示版本再走停服流程。
执行会重检所有条件，并同时持有 update.lock/core.lock，拒绝陈旧确认。

stdout 为单个 JSON 对象，stderr 不输出底层报错或配置内容。
查询成功（包括没有可回滚版本）退出 0；参数/配置错误退出 2；执行失败退出 1。
install_root 未配置返回下述查询结构，reason=install_root_unconfigured，退出 1。

```json
{"version":"1.0","installed_at":100,"can_rollback":true,"reason":null}
```

installed_at 是上一版本原安装时间，Unix 秒；version 是已提交 release_id，
不从高水位推断营销版本，也不编造缺失的时间。

成功执行：

```json
{"status":"rolled_back","version":"1.0","transaction_id":"tx-UUID","restart_required":true}
```

成功恢复：

```json
{"status":"current_restored","restart_required":true}
```

执行失败：

```json
{"error":"rollback_failed","details":{"version":"1.0","installed_at":100,"can_rollback":false,"reason":"backup_corrupt"}}
```

其他固定 error：invalid_arguments、invalid_config、expected_version_required、
rollback_recovery_failed。reason 的固定类别有 no_previous_version、
no_matching_completed_update、previous_release_corrupt、backup_corrupt、
rollback_not_supported_by_release、
database_schema_changed、database_checkpoint_required、stop_core_first、
recovery_required、rollback_state_invalid。未知底层错误统一映射为最后一类。
UI 应本地化固定类别，不能直接展示底层错误。

## 版本与备份边界

原有 rollback.rs 只切换软链接，previous 是最近一次切换前的程序目录；
它不核验版本元数据/文件摘要，也不提交权威记录，旧 RolledBack 日志会被启动守卫阻止。
原有 SQLite 在线备份包含一致数据库副本与 SHA-256；自动覆盖旧数据库仍被禁止。

新增 previous-release.json：在现有 executor 完成备份后、切换前保存上一版本
权威记录、完整程序文件摘要/权限、数据库 schema 指纹、备份证明及所属更新事务。
更新清单必须明确声明 in_place_rollback_supported=true。
只有最近一次已提交更新对应的上一版本可回滚；不支持任意历史版本或反复来回切换。
目录/软链接、摘要、备份完整性、当前提交及 journal 必须一致；缺证据就拒绝。
若备份损坏、程序被修改、数据库新增/删除或 schema/user_version 变化则拒绝。
存在未清空 WAL 时要求先由拥有该数据库的服务正常关闭/完成 checkpoint，查询不执行 checkpoint。
所有新业务行保留。schema 相同不是对任意版本业务语义兼容的完整证明，跨语义不兼容升级
仍需发布方兼容性约束和真实应用验收；本实现不声称自动迁移或逆向迁移数据库。

回滚前写 rollback-undo.json 和 RollbackStarted journal；失败恢复原指针、记录和
原 journal，另写 last-rollback.json 保存失败结果。成功提交绑定新 tx_id 的旧版本
记录和 Committed journal，使启动守卫可核验放行；独立回滚凭证的 highest_sequence
明确保留发布序列高水位（不扩展旧程序严格解析的 journal schema），不能借回滚重放旧发布包。落盘失败无法承诺可启动时保持
journal 的非终态，由 rollback-recover 恢复，绝不伪报成功。

## 集成限制与待验收

仓库目前真实 apply 入口仍禁用，SandboxUpdateExecutor 只由 fixture 驱动。
该 executor 现在会自动产生回滚证据；真实安装器以后沿用本协议时必须调用
manual_rollback::preserve_previous，不能手工伪造 previous-release.json。
旧部署脚本保存整个 .app 到 backups，未产生版本化 releases/current 布局、
权威版本记录和本协议凭证，因此这种安装查询会明确不可回滚；本任务没有
把旧 .app 备份导入为可信发布，也没有修改安装/发布脚本。协调方需要明确是否
另派安装器布局迁移/整包 .app 回滚任务。不能把版本化 fixture 成功称为已完成旧部署 .app 验收。

T6 仍需 UI 二次确认、停服/重启集成，以及在 MacBook 的真实 .app 上验收，附截图、录屏。
本地只提交，不推送、不开 PR，不修改发布工作流，不生成 tag。

旧程序本身的启动守卫可读取原 journal schema；旧版 updater/control 的高水位读取器
仍可能把显式回滚后的 sequence 差异显示为 recovery_required。宿主更新状态和后续
更新必须调用本次更新的 updater，不能把旧版读取器的状态当作本协议的完成结果。

## 本地验证（2026-11-17）

- Cargo 默认 target 指向沙箱不可写的缓存，以下 Rust 检查使用 `CARGO_TARGET_DIR=target`。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --all-targets --locked`：通过，有仓库既有警告和一项测试断言风格警告。
- `cargo test --locked --lib updater`：35 通过，1 忽略。
- `cargo test --locked --test swift_update_ui`：通过（独立记录）。
- `swiftc -target "$(uname -m)-apple-macos12" native/bootstrap/*.swift native/eyes/Screens.swift -o /tmp/host-t7`：通过。
- `cargo test --locked`：1016 通过、3 失败、1 忽略，未到后续 integration suites。
  两个 gateway 用例被 HOME 下桌面资源锁的沙箱权限阻止；对应源文件与 HEAD 相同。
  control history maintenance CLI 用例全量并发运行时报告 core.lock 被占用，单独原样复测通过。
  不把以上结果表述为全量测试通过。
- Orca orchestration 报告受阻：`no_active_sender_terminal`，当前环境没有
  `ORCA_TERMINAL_HANDLE`，没有冒用其他终端身份发送完成消息。
