# I6 应用更新界面与宿主集成交付审查报告 (自适应布局与指引终版)

**日期**：2026-11-10  
**任务**：I6 应用更新界面与宿主集成 (`task_9d11e4ed9f91` / Dispatch `ctx_3b0c0ea47435`)  
**分支**：`work/prod-update-ui`  
**基线**：I3 界面基线 `9fc0cdf` + 集成 I5 更新核心 `d001a4e` (`d7355e9`)  
**前序提交**：`785c02d`  

---

## 1. 协调者本轮复审意见与整改闭环

针对协调者在 `task_9d11e4ed9f91` 中指出的具体问题，本轮完成了精确修正：

1. **720 宽与宿主最小宽 (680 宽) 自适应布局修复，彻底消除右侧裁切**：
   - **读模型值列受限宽度换行**：将 `currentReleaseLabel`、`currentSeqLabel`、`architectureLabel`、`consistencyBadge`、`supervisorStatusLabel`、`publisherVerificationBadge`、`localSigningLabel`、`compatibilityLabel` 全部切换为 `wrappingLabelWithString`，并显式配置 `lineBreakMode = .byWordWrapping` 与 `widthAnchor.constraint(lessThanOrEqualToConstant: 460)`。维护状态等长文本自适应折行，不再将卡片右边界撑出视口。
   - **操作按钮区分两行排布**：
     - 第一行（状态查询类）：`checkButton`（“检查更新”）、`refreshStatusButton`（“刷新状态”），总宽约 200pt。
     - 第二行（写入受控类，本阶段未支持）：`downloadButton`（“下载更新 (未启用)”）、`installButton`（“安装并重启 (未启用)”）、`deferButton`（“延期更新 (未启用)”），总宽约 460pt。
     - 彻底解决了原 5 个按钮单行挤占导致最右侧“刷新状态”被裁切的问题。
   - **严格宽度边界单测断言**：在测试套件中增加了 `subviews_within_680w_bounds` 断言，验证即使在 680pt 宿主最小宽度下，所有子视图的 `frame.maxX <= 680.0`，无任何子元素越界。
2. **恢复指引文案求实修正**：
   - 彻底删除了“原旧版本及客户会话未受污染”等无读模型证据的确定性保证；
   - 准确标注：**维护标志文件已记录；实际核心进程存活状态与业务数据完整性未知**；
   - 明确指出真实版本回退能力当前处于 unsupported 禁用状态，不诱导用户自行回退，说明需要专门受控恢复。
3. **状态刷新异步处理与结果如实呈现**：
   - `handleRefreshStatus()` 调整为异步调度 `client.fetchStatus()`；
   - 成功时更新模型并呈现“状态已刷新 (只读)”，失败时保留并显示具体的错误原因，绝不误报成功。
4. **离屏渲染全画幅证据 (720 宽与 680 宽真实双尺寸验证)**：
   - 生成 4 张不透明 Aqua 真实离屏渲染图片（画幅高度均为 1150pt）：
     - `/tmp/update_ui_unconfigured_full.png` (720x1150, 127KB)
     - `/tmp/update_ui_recovery_full.png` (720x1150, 187KB)
     - `/tmp/update_ui_unconfigured_680w.png` (680x1150, 125KB)
     - `/tmp/update_ui_recovery_680w.png` (680x1150, 185KB)
   - 经检查，所有按钮文字完整呈现，恢复卡片展开详尽指引，卡片流完全处于视口安全区域内。

---

## 2. 修改文件清单

- `native/bootstrap/UpdatePage.swift`：
  - 读模型标签启用换行与受限宽度约束；
  - 操作区按钮重构为两行排布；
  - 异步处理刷新状态并精准反馈；
  - 恢复指引删除伪确定性保证，真实标注维护标志记录与受控恢复说明。
- `tests/host_update_selftest/main.swift`：
  - 增加 720 宽与 680 宽双画幅渲染测试；
  - 增加 `subviews_within_680w_bounds` 宽度边界不越界断言；
  - 增加恢复指引“维护标志文件已记录”与不包含“未受污染”断言。
- `docs/reviews/2026-11-10-production-update-ui.md`：
  - 更新本轮审查记录与整改证据。

---

## 3. 测试命令与退出码

1. **更新专项与真实控制契约测试**：
   - 命令：`cargo test --test swift_update_ui --locked`
   - 退出码：`0` (PASS，耗时 19.63s)
   - 证据：41 项断言全量通过，包含真实 Rust control `update-status` / `update-check` 对接、写操作只读守卫、680w/720w 双画幅离屏渲染及子视图宽度不越界验证。
2. **专项回归验证**：
   - `cargo test --test swift_host --locked`：退出码 `0`，74 项测试全部 PASS。
   - `cargo test --test swift_brain_ui --locked`：退出码 `0`，PASS。
   - `cargo test test_control_update_status_and_check --locked`：退出码 `0`，PASS。
   - `swiftc ... LifecycleTests.swift`：退出码 `0`，PASS。
   - `bash scripts/test-gates.sh`：退出码 `0`，PASS（运行时与验证无 Python）。
   - `cargo fmt --check`：退出码 `0`。

---

## 4. 范围局限声明 (Partial 范围)

- **I6 目标在限定阶段保持 partial**：
  - 本任务严格限于只读更新状态呈现、自适应界面布局、本地检查与隔离 fixture 验证；
  - 真实更新下载、安装切换（`apply`）、回退（`rollback`）、排空协调及专用机钥匙串重签/TCC权限保持在底层保持 unsupported 状态，未接入生产运行；
  - 真实安装能力的落地须待专用机试点与信任公钥体系建立后专项推进。
