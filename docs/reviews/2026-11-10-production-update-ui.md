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

---

## 5. Q1 缺陷整改与 I5 生产者契约适配 (task_3476eac7b178 / ctx_e1e8040bbd70)

### 5.1 背景与根本原因
在 Q1 审查中发现，原有 `control` 控制接口误将单二进制的编译版本当做整套安装版（将编译版本拼接填入 `release_id`，并将防降级高水位填补到已安装 `sequence`），导致 Swift 界面呈现不求实的“假已安装”状态。
已批准 I5 生产者（owner `ctx_9d52c93c838d`）精确契约：
```json
{
  "current_release": {
    "release_id": null,
    "sequence": null,
    "status": "unknown",
    "source": "unknown",
    "core_version": "CARGO_PKG_VERSION",
    "architecture": "ARCH",
    "components_consistent": null
  },
  "guard": {
    "highest_accepted_sequence": null,
    "maintenance_active": false
  }
}
```
其中 `guard.highest_accepted_sequence` 仅为防降级高水位线，不是整套已安装序列。

### 5.2 Swift 消费端模型与界面适配
1. **模型重构 (`UpdateReleaseInfo`)**：
   - `releaseId` 调整为 `String?`，`sequence` 调整为 `UInt64?`；
   - 增加 `status: String?`、`source: String?`、`coreVersion: String?`；
   - 当前整套版本无法核验时如实展示“整套版本无法核验 (未知)”，绝不把 `null` 误当未安装或解码失败；
   - 绝不使用 `coreVersion` 或 `guard.highestAcceptedSequence` 填补 `releaseId` 或 `sequence`。
2. **读模型标签清晰分离**：
   - 「当前整套版本」：展示 `userFacingReleaseDescription`（null 时为 `整套版本无法核验 (未知)`）；
   - 「核心编译版本」：独立标签！展示 `userFacingCoreVersionDescription`（例如 `0.1.0 (核心编译版)`），明确标明为独立核心编译版；
   - 「整套安装序列」：展示 `userFacingSequenceDescription`（null 时为 `未知 (未标记)`），不拿高水位冒充；
   - 「防降级高水位」：独立呈现 `highestAcceptedSequence`，注明 `(防降级高水位，非安装版本)`；
   - 「发布状态与来源」：展示 `userFacingTrustDescription`。兼容缺失 status/source 的旧 shape 且标记为旧源不可信；遇到未知枚举或 `unknown` 明确标为不可信，绝不默认正常；
   - 「宿主运行架构」、「组件一致性」、「维护抑制状态」按实际读模型准确映射。
3. **安全防线守卫**：
   - 写操作（下载、安装、延期）保持全部物理禁用 (`isEnabled = false`)，点击时触发只读保护拦截提示；
   - 异步刷新与查询遵循真实响应，失败保留具体错误。

### 5.3 离屏渲染与验证证据
- 新增 1b (批准的新 shape)、1c (防降级高水位隔离)、1d (未知枚举不可信防御) 单元测试；
- 渲染生成真实离屏 PNG 证据：
  - `/tmp/update_ui_approved_720w.png` (720x1250, 145KB)
  - `/tmp/update_ui_approved_680w.png` (680x1250, 144KB)
  - `/tmp/update_ui_unconfigured_full.png` (720x1250, 141KB)
  - `/tmp/update_ui_unconfigured_680w.png` (680x1250, 140KB)
  - `/tmp/update_ui_recovery_full.png` (720x1250, 202KB)
  - `/tmp/update_ui_recovery_680w.png` (680x1250, 201KB)
- 经实际图像排查与 `subviews_within_680w_bounds` 单测验证，680 宽下所有卡片子视图 `frame.maxX <= 680.0`，排版对齐无溢出截断。
- 测试结果：`cargo test --test swift_update_ui`，69 项断言全部 PASS (0 FAIL)。

---

## 6. I5 生产者 commit 集成与端到端隔离联测闭环 (bc8812e)

### 6.1 提交集成与版本溯源
- **生产者提交 (Producer Commit)**：`bc8812e75c37debc79091558230b497380af107c` (在 `work/prod-update-ui` 分支 cherry-pick 为 `b71f54e`)。仅涉及 `src/core/control.rs` 与 `docs/design/production-updater-contract.md`，无冲突。
- **消费者提交 (Consumer Commit)**：`b3f8eb8` (`fix(update-ui): 适配nullable release与防降级高水位隔离，独立核心编译标签与安全读模型`)。

### 6.2 真实编译 Core + Swift Client 隔离联测实测结果
通过真实编译的 `target/debug/wechat-mcp` 与 Swift `UpdateClient` 在隔离临时环境中完成端到端联测：
1. **整套版本与核心编译版隔离**：真实 `control update-status` 返回 `release_id: null`、`sequence: null`、`status: "unknown"`、`source: "unknown"`、`core_version: "0.1.0"`。Swift 客户端正确映射，界面显示「当前整套版本: 整套版本无法核验 (未知)」，独立标签显示「核心编译版本: 0.1.0 (核心编译版)」，安装序列号显示「未知 (未标记)」，发布状态显示「状态未知 / 来源未知 (不可信)」，`isTrustedInstalledRelease == false`。
2. **防降级高水位隔离断言**：在 `state_dir` 中写入持久高水位 `highest_accepted_sequence = 1001`。真实调用返回 `guard.highest_accepted_sequence = 1001`，Swift 呈现「防降级高水位: 1001 (防降级高水位，非安装版本)」，整套安装 `sequence` 仍为 `nil`，`release_id` 仍为 `nil`，绝不回填或冒充安装版本。
3. **故障恢复与未知来源不可信**：
   - 写入损坏的 `highest_accepted_sequence` 文件：真实调用返回 `status: "recovery_required"`，错误包含 `corrupt_sequence_file`，Swift 界面显示受控恢复指引与不可信状态。
   - 写入损坏的 `update-journal.json` 文件：真实调用返回 `status: "recovery_required"`，错误包含 `corrupt_or_unreadable_journal`，Swift 界面展开恢复指引，状态明确标注不可信。
4. **伪造 VERSION 免疫**：在 `state_dir` 与父级目录分别伪造 `VERSION` 文件（`8.8.8` / `9.9.9`），真实调用返回的 `release_id` 与 `sequence` 坚守为 `null`，核心版本不被篡改，绝不信任无签名保护的非受控文件。
5. **离屏渲染与界面呈现人工检查**：
   - 导出真实 720 宽与 680 宽全高度 PNG 图像（`/tmp/update_ui_approved_720w.png`、`/tmp/update_ui_approved_680w.png`、`/tmp/update_ui_recovery_full.png`、`/tmp/update_ui_recovery_680w.png` 等）；
   - 人工核查确认文字折行完整、两行按钮排布整洁、恢复卡指引详尽、所有卡片均严格处于 680 宽以内（`subviews_within_680w_bounds` 校验通过）；
   - 写操作（下载、安装、延期）物理禁用，只读拦截保护生效。
6. **最终测试执行**：
   - 执行命令：`cargo test --test swift_update_ui`
   - 测试结果：`test test_swift_update_ui_selftest ... ok`，92 项集成断言全量通过，0 失败。
