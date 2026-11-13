# 桌面改版基线核对

2026-11-14。工作树：`/path/to/desktop-ui-redesign/wechat-mcp`；分支 `codex/desktop-ui-implementation`。

## 基线选择与隔离

UI 实施基线为 `a1a7125`（ui-update-polish），已有 BrainPage、UpdatePage、会话追溯与相应控制契约。当前用户 checkout 是 `b7e3afb`，两者分叉：当前分支独有13个提交，UI基线分支独有115个提交。未将大量后端变更混入UI修改。

尝试在新建隔离工作树进行不提交合并以核实兼容性，发现 `.gitignore`、Cargo.toml/lock、McpPage.swift、history/store.rs 冲突；随后完整撤销该次合并，原 checkout 未变。

重大阻塞：当前分支 migration 5 为 pending_replies，UI基线 migration 5 为 brain_turns、migration 6 有历史补偿。该分歧涉及已升级安装的 schema_migrations 记录，需要单独兼容迁移设计与审查，不能用简单改号或任选一侧解决。因此本UI分支不是当前收发分支的可部署替代品，最终集成待该项解决。不得部署本分支覆盖当前运行版本。

## 八页实际功能

| 页面 | Controller | 核实结果 |
|---|---|---|
| 总览 | OverviewPageController | 三指标、检查、暂停恢复、八类诊断及原权限/重启/归位动作 |
| 会话 | SessionsPageController | 监听状态、恢复自动回复；此基线另外已有轮次ID查询及追溯详情，保留 |
| 联系人 | ContactsPageController | 五分类、同步三操作、四列表格、详情八字段、处理方式、待确认项 |
| 大脑 | BrainPageController | 已有六分区及角色增删复制、默认角色、知识、话术、绑定/版本/预览等；保留实际既有能力，不按图片删减 |
| MCP | McpPageController | 模式与连接策略、对外接口/令牌、端点/测试、配置复制及工具列表；此分支接口启停能力与当前主checkout不同，随最终集成核对 |
| 设置 | SettingsPageController | 五权限、账号模型只读、六运行参数、窗口管理、保存默认、更新页跳转 |
| 日志 | LogsPageController | 两文件夹、诊断复制、来源/级别/搜索、暂停继续、清屏 |
| 更新 | UpdatePageController | 状态与检查只读契约可用；真实下载/安装/延期仍禁用，UI不解锁能力 |

生产代码修改限制在 native/bootstrap 的十个界面/入口文件；无Rust、数据库、公共接口、Supervisor、签名、发布配置修改。

## 验证方式

- 修改前，以 `swiftc -target $(uname -m)-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift` 成功构建此基线。
- 修改后以相同编译入口构建；回归运行 swift_host、swift_brain_ui、swift_update_ui。
- 首次回归宿主76项和大脑1项通过；更新夹具99 PASS / 1 FAIL，由重排后标签不一致引起，保留原标签后复验。
- 一次编译因构建过程中源文件被修改而失败；该次不作为有效功能测试结果，停止修改后重新构建。
- `scripts/preview_desktop_ui.sh` 使用隔离Home与控制命令stub生成八页（大脑含六分区）×三尺寸×浅深色的78张离屏截图，不加载StatusPanel.view，不运行服务、不发送消息。权限/环境诊断中仍会读取本机权限和窗口元数据；截图用于布局，不作为实际运行状态或生产健康证据。
- 渲染入口拒绝不在截图目录内的控制命令stub，避免预览误用真实业务控制程序。

最终复验结果见交付报告；这里不将历史文档中的任务状态当作当前实现验收。
