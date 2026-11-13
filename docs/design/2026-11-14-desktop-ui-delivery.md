# 桌面界面改版：本地交付记录

日期：2026-11-14。分支 `codex/desktop-ui-implementation`，基线 `a1a7125`。

## 已实现

- 外壳统一为八项导航：总览、会话、联系人、大脑、MCP、设置、日志、应用更新；系统图标、绿色选中背景、页面标题、196pt侧栏。默认窗口1180×800，最小窗口900×640。
- 原有NSBox内容嵌入方式保留，普通分组统一为带可见标题、细边框和圆角的原生卡片；使用语义颜色支持浅深色。
- 总览：指标与操作分开、诊断说明分行，保留八类检查与原操作。
- 会话：状态可换行，保留恢复自动回复和此基线已有轮次追溯；追溯区不再用640pt固定宽度撑大窗口。
- 联系人：保留现有同步、分类、四列、处理与详情，调整行高，沿用原有动态详情布局。
- MCP与设置：保留原字段、范围及动作；主要字段改中文标签，统一表单间距。账号模型信息仍只读。
- 大脑：保留既有六分区及完整业务操作；长表单在内容区滚动，底部保存/默认角色/启用操作固定可见；收紧角色侧栏和异常回复标签，预览输入与操作分行，避免窄窗口被撑大。
- 日志：保留来源、级别、搜索及原动作，调整等宽文字和输入宽度，日志区不再强制500pt最小高度。
- 更新：使用单卡片。默认显示当前版本、候选/空态说明、检查/刷新和下载入口；安装/延期、可信来源/序列/组件状态等保留于可展开技术详情。事务/恢复信息只在对应状态出现，未知进度使用不定进度，不伪造百分比。真实写操作仍禁用。

修改文件集中于 `native/bootstrap/{Shell,main,OverviewPage,SessionsPage,ContactsPage,BrainPage,McpPage,SettingsPage,LogsPage,UpdatePage}.swift`。未修改Rust业务、数据库、Supervisor或公共控制契约。

## 本地验证

1. 宿主构建：`swiftc -target $(uname -m)-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift -o target/ui-redesign/wechat-cs-host`，通过。
2. `cargo test --locked --test swift_host --test swift_brain_ui --test swift_update_ui`：76项宿主、1项大脑、1项更新集成测试通过。更新Swift夹具另含原有状态/控制契约及新增技术详情展开/收起断言；不把内层断言数当作Rust测试数。
3. 更新进度展示完成后定向复验 `cargo test --locked --test swift_update_ui`，结果记录于 `target/ui-redesign/update-final.log`。
4. `scripts/preview_desktop_ui.sh`：八页（大脑额外五个分区），三尺寸900×640、1180×800、1440×900，浅深色，共78张截图。每张强制检查实际视口尺寸，窗口被内部约束撑大即失败。最终全部通过。
5. `git diff --check`、`bash -n scripts/preview_desktop_ui.sh`，通过。

实际预览：`target/desktop-ui-preview/index.html`，逐图数据见同目录 `screenshots.json`。截图为离屏原生渲染，不是生成模型设计图；不启动业务服务，控制调用使用隔离stub。环境诊断仍读取本机权限/窗口元数据，不将这些样例状态作为生产健康证据。已人工查看八页代表图及大脑窄窗口分区；尺寸断言不证明每个控件内容、交互或对比度全部验收。

检查过程发现并修复：卡片标题自绘遗漏；会话固定宽度撑窗；大脑分区长表单导致最小高度过大；日志500pt最小高度导致标题裁切；更新重排后的标签断言不匹配。一次构建中修改源文件导致Swift拒绝编译，停止编辑后重跑通过。所有最终结论以最终日志为准，不用失败前旧截图替代最终证据。

## 任务状态和未完成部分

U0–U7 的本地UI范围已实施；U8完成受影响的本地回归与离屏布局检查，真机动态交互、VoiceOver逐项操作、生产运行及完整更新闭环未验收。不宣称整个项目通过。

**最终合入阻塞**：`b7e3afb` 当前收发分支与本UI基线对迁移版本5的含义不同。UI分支不得直接覆盖生产或合并时任选一侧；需另行明确兼容迁移并复核最新13项收发改动。细节见 `implementation-baseline-2026-11-14.md`。本次已撤销试验性合并，不含数据库改号或结构修改。

**更新能力边界**：继承原只读阶段；未启用下载、真实安装、重启、延期或恢复写操作。简化是展示变化，不代表完整更新能力已实现。

**Orca报告限制**：按AGENTS要求加载了orchestration技能并尝试 `orca orchestration run-current --json`，返回 `no_active_sender_terminal`：当前Codex会话未提供 `ORCA_TERMINAL_HANDLE`。未借用其他终端身份，未伪造worker完成回执；暂以本文件及Git提交记录交付。

未推送、部署、修改真实配置或发微信消息。
