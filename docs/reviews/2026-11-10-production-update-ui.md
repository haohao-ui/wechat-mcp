# I6 应用更新界面与宿主集成交付审查报告 (复审修订版)

**日期**：2026-11-10  
**任务**：I6 应用更新界面与宿主集成 (`task_f9af41a8a5ae` / Dispatch `ctx_fc8bc80ee8de`)  
**分支**：`work/prod-update-ui`  
**基线**：I3 界面基线 `9fc0cdf` + 集成 I5 更新核心 `d001a4e` (`d7355e9`)  
**前序提交**：`5e976bf` (复审未通过，已闭环修复)  

---

## 1. 协调者复审意见与整改闭环

针对协调者在 `msg_d921c7402998` 和 `msg_b93d4e5d2ee7` 中指出的明确缺口，本轮进行了彻底重构与整改：

1. **彻底移除生产假下载、假延期与模拟路径**：
   - 移除了 `UpdatePage.handleDownloadUpdate` 中使用 `asyncAfter` 将进度 10%→65%→100% 伪造下载验签完成的代码。
   - 移除了 `handleDeferUpdate` 中擅自修改本地内存伪造“已取消排空恢复业务”的代码。
   - 移除了安装确认弹窗中的可执行暗示。
   - **严格只读守卫**：所有下载、安装重启、延期控件在生产代码中始终禁用（`isEnabled = false`），按钮文案明确标注 `(本阶段未支持)`。若任何代码路径调用其 handler，统一触发只读守卫，输出 `unsupported` 提示，**绝对不修改读模型、进度条或业务状态**。
2. **纯真反映读模型，坚决不编造健康或版本**：
   - 当 `maintenanceActive == false` 时，界面严格显示 `“未处于维护抑制态 (运行健康度未经验证)”`，坚决不宣称“守护服务正常运行”或“健康”。
   - 未知架构、组件未校验等状态如实展示为 `未知`、`未校验 (未知)`、`未标记 (null)`，不擅自拼接硬编码的 `macOS 12.0+` 或组件一致假象。
   - 恢复卡片提供详实的只读指引，说明当前处于只读受控阶段，禁止通过界面清除维护标志，回退需由管理员在终端依据 journal 执行。
3. **补充真实 Rust Control + Swift Client 隔离契约测试**：
   - 在 `tests/swift_update_ui.rs` 中构造隔离的临时 state/config 环境，通过真实编译的 `target/debug/wechat-mcp` 二进制执行真实控制命令。
   - 验证了 `wechat-mcp control update-status` 真实输出的解码与状态字段映射。
   - 验证了 `wechat-mcp control update-check` 真实返回 `{"status":"unconfigured","reason":"distribution_source_not_configured"}` 的解析与无虚假候选版本断言。
4. **重做完整高度不透明 Aqua 离屏渲染截图**：
   - 将渲染画幅设为完整高度 `720 x 1150`，保证整个 `contentStack` 从顶部运行环境、候选说明、流水线进度、展开的故障恢复卡到底部只读禁用操作按钮完整呈现，无任何截断。
   - 生成真实离屏 PNG 证据：
     - `/tmp/update_ui_unconfigured_full.png` (720x1150, 126KB)
     - `/tmp/update_ui_recovery_full.png` (720x1150, 182KB)
   - 经 `sips` 图像分辨率与实际结构检验，元素排布清晰自适应，NSBox 随内容自动撑高。

---

## 2. 代码变更与文件清单

- `native/bootstrap/UpdatePage.swift`：严格只读更新界面实现，移除所有假模拟路径，写操作永久禁用，读模型真实求实。
- `native/bootstrap/SettingsPage.swift`：应用更新引导跳转与版本展示。
- `native/bootstrap/main.swift`：按 D2 规范在设置和日志之间注册应用更新页。
- `tests/host_update_selftest/main.swift`：包含真实 Rust 契约测试、只读防御、断网、验签失败、完整高度离屏渲染等 40 项测试。
- `tests/swift_update_ui.rs`：Rust cargo 集成测试入口，注入真实 `wechat-mcp` 环境。

---

## 3. 测试与验证记录

1. **专项更新 UI 与契约测试**：
   - 命令：`cargo test --test swift_update_ui --locked`
   - 退出码：`0` (PASS)
   - 证据：`test result: ok. 1 passed; 0 failed; finished in 20.49s`，内部 40 项断言（含真实 Rust control 契约与全高度离屏 PNG）全部通过。
2. **相关回归与硬门禁**：
   - `cargo test --test swift_host --locked`：退出码 `0`，74 项全部 PASS。
   - `cargo test --test swift_brain_ui --locked`：退出码 `0`，PASS。
   - `cargo test test_control_update_status_and_check --locked`：退出码 `0`，PASS。
   - `swiftc ... LifecycleTests.swift`：退出码 `0`，PASS。
   - `bash scripts/test-gates.sh`：退出码 `0`，PASS。
   - `cargo fmt --check`：退出码 `0`。
   - 产品运行时与测试脚本中均**无 Python**。

---

## 4. 范围局限与未验证项声明 (Partial 范围)

1. **原 I6 目标依然为 partial**：
   - 本阶段仅放行只读状态界面、本地检查与隔离 fixture；
   - 真实更新下载、安装切换（`apply`）、回退（`rollback`）、排空协调与钥匙串重签在底层保持 unsupported，未接入生产运行；
   - 不能将真实排空、安装或 TCC 权限保持视为在 Q1 自动达成，真实安装能力须待专用机与信任根建立后另行专项开发与真机试点。
2. **全仓并发测试缺口**：
   - 全量 `cargo test --locked` 在外部并发环境下存在历史长耗时缺口，本次采用专项针对性测试与回归覆盖，不凭空推断全仓无关。
