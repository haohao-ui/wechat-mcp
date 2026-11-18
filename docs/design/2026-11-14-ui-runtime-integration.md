# 桌面UI与最新收发分支集成

日期：2026-11-14。隔离分支 `codex/desktop-ui-implementation`。

## 范围与确认

将当前收发分支 `b7e3afb` 集成到已有UI提交 `587540b` 上。用户明确确认追加幂等迁移7，详见 `2026-11-14-branch-migration-compatibility.md`；不包含真实数据库升级、部署或消息发送。

## 集成结果

- 保留当前收发分支的待回复队列、观察基线/重绘处理、附件下载和旧Excel解析修复。
- 保留UI基线的大脑、更新能力和八页改版，不回退至旧六页宿主。
- MCP同时保留“大脑页跳转”和“对外接口启停”；配置保存沿用最新分支的受监督核心重启路径，联系人修改保持无需重启的原语义。
- Cargo.toml保留两侧依赖；Cargo.lock离线追加calamine依赖树，无既有包版本删除或替换。
- `.gitignore`合并两侧规则，不忽略src/bin程序。
- 历史SQL 5/6逐字保持，迁移7仅补pending_replies及两个原索引；所有升级使用临时数据库验证，未读取真实聊天库。
- 修复自动合并未报告的编译缺口：恢复待回复轮次构造TurnData时显式初始化fixed_snapshot=None；模型重试的既有固定快照逻辑继续保留并回归。

## 兼容矩阵

新增测试覆盖fresh、v4、v5-pending、v5-brain、v6-brain、v6-both六种形态，保留原待回复文本/时间、大脑快照、账号归属和已有迁移时间，验证两表和索引、重开升级、重复升级以及升级后入队/结算和快照读取。

定向历史存储回归83项通过。全量验证的最终结果及日志见本报告后续记录。

## 验证入口

- `cargo test --locked`：全量Rust及集成测试；日志 `target/ui-redesign/integration-full-tests.log`。
- `swiftc -target $(uname -m)-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift -o target/ui-redesign/integrated-host`：集成宿主构建通过。
- `scripts/preview_desktop_ui.sh`：78张截图及视口检查通过，包含重新接入的MCP接口开关；日志 `target/ui-redesign/integrated-preview.log`，预览 `target/desktop-ui-preview/index.html`。
- `bash scripts/test-gates.sh`：通过，包含发布tag负例与Python文件边界夹具，不执行发布。
- `cargo fmt --all -- --check`：初次发现新合入documents/scheduler/worker_client格式差异，仅作格式整理后复验。

## 边界

本报告只描述隔离工作树集成；不自动改写原checkout的分支指针，不推送、部署或更改真实状态。真实更新安装仍沿原只读边界禁用，生产安装/恢复与真机持续运行未验收。

Orca完成回报仍受当前会话没有ORCA_TERMINAL_HANDLE限制；不借用其他会话身份。已有本地代码与测试报告可审查，不以Orca或Git状态宣称整个项目验收通过。

## 最终本地结果

- `cargo test --locked` 退出码0：26个测试目标汇总2310次通过、0失败、3次忽略；其中库/主程序重复运行同一组测试，不能将2310视作互不重复用例数。
- 忽略项：真实发布包验证测试在lib/bin各忽略一次（需专门构建测试发布包）；图片气泡phash夹具缺少已脱敏素材忽略一次。均未记作通过。
- 历史存储83项定向回归通过；历史migration 5/6 SQL与UI基线逐字比较相同。
- 全量测试后仅格式化新合入的三个Rust文件；`cargo fmt --all -- --check`及`cargo check --locked`复验通过，无业务逻辑改动。
- 项目门禁、集成宿主编译、78张离屏视口检查、`git diff --check`通过。
- 原跨分支数据库版本5冲突已在隔离分支解决；这是本地集成验证，不是对真实安装数据库的升级证明或生产验收。
