# I6 应用更新界面与宿主集成交付审查报告

**日期**：2026-11-10  
**任务**：I6 应用更新界面与宿主集成 (`task_60895b5defcd` / Dispatch `ctx_368455949ec1`)  
**分支**：`work/prod-update-ui`  
**基线**：I3 界面基线 `9fc0cdf` + 集成 I5 更新核心 `d001a4e` (commit: `d7355e9`)  

---

## 1. 任务范围与所有权边界

根据任务书与 D2 设计规范（`docs/design/production-ui-update-design.md` §4、§7、§9、§10）：
- **独占新增文件**：
  - `native/bootstrap/UpdatePage.swift`
- **公共协作文件**：
  - `native/bootstrap/SettingsPage.swift`（增加应用更新引导跳转）
  - `native/bootstrap/main.swift`（在“设置”与“日志”之间串行注册“应用更新”页）
- **测试套件文件**：
  - `tests/host_update_selftest/main.swift`（包含 39 项状态机、断网、签名、排空延期、恢复与离屏渲染测试）
  - `tests/swift_update_ui.rs`（Rust cargo test 集成入口）
- **严格遵循的生产约束**：
  - 不默认静默安装，不把配置启用称为应用更新。
  - 生产公钥根未配置时，明确显示“尚未验证（无可信根公钥配置）”，坚决禁用安装。
  - 真实更新入口处于协调者限定受控阶段（unsupported），真实反映系统接入状态，不篡改真实版本或伪造安装成功。
  - 复用 I4/I5 接口（`CoreSupervisor` 维护标志与停机协调、`control update-status` / `update-check`），不自行另写 updater 或 supervisor。

---

## 2. 核心实现说明

### 2.1 UpdatePage.swift (界面与控制器)
- **运行环境卡片 (Current Environment Box)**：
  - 完整呈现当前整套 Release ID、Sequence、架构（`aarch64` / `x86_64`）及最低系统版本（macOS 12.0+）。
  - 展示整套组件一致性状态（一致 / 不一致 / 未校验）。
  - 实时反映守护服务状态（正常运行 / 处于维护抑制态 / 崩溃保护中）。
- **候选版本与发布说明 (Candidate & Notes Box)**：
  - 严格支持未配置更新源（`unconfigured`）状态，不呈现虚假可用版本，禁止任意未经验证的网络下载 URL 输入。
  - 明确区分发布者验签状态：区分“来源已验证”与“尚未验证（无可信根公钥配置）”，来源未验证时坚决禁用安装操作。
  - 展示目标 Release ID、Sequence、发布时间、包大小格式化显示（KB/MB）及兼容性说明。
  - 提供可滚动文本区域展示发布说明（Release Notes）。
- **更新流水线与阶段 (Pipeline & Progress Box)**：
  - 呈现 8 步流水线状态：`检查 → 下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检`。
  - 下载阶段提供进度条展示与已下载百分比。
  - 排空阶段（`Draining`）显示在途轮次等待警示，并提供[延期更新]操作，延期后立即清除排空等待并恢复正常业务处理。
  - 本机代码签名状态核验：钥匙串锁定或本地证书缺失时明确显示警示。
  - 提示专用机更新后可能触发系统权限重新授权的注意事项。
- **故障恢复指引 (Recovery Required Box)**：
  - 当底层 `UpdateJournal` 处于 `RecoveryRequired` 或损坏时自动展开醒目卡片。
  - 展示事务 ID、中断阶段及底层错误信息。
  - 提供回退到旧版本、日志排查与安全清理的操作指引。
- **操作与维护 (Actions Box)**：
  - 提供[检查更新]、[下载更新]、[安装并重启]、[延期更新]及[刷新状态]按钮。
  - 安装前触发用户确认模态弹窗，提示业务暂停、数据备份与重启风险；在当前阶段如实反馈 unsupported 契约状态。

### 2.2 宿主与设置页联动
- `SettingsPage.swift` 新增“应用版本与更新”卡片，展示当前版本并提供 `jumpToUpdateButton`，触发 `onNavigateToUpdate` 回调。
- `main.swift` 按 D2 导航规范将 `UpdatePageController` 注册在 `settings` 与 `logs` 之间，并完成跳转闭环。

---

## 3. TDD 验证与测试矩阵

### 3.1 RED 阶段记录
- **命令**：`cargo test --test swift_update_ui --locked`
- **执行结果**：退出码 `101` (FAILED)
- **失败原因**：`<unknown>:0: error: error opening input file '/.../native/bootstrap/UpdatePage.swift' (No such file or directory)`，准确揭示了实现缺失。

### 3.2 GREEN 阶段记录
- **命令**：`cargo test --test swift_update_ui --locked`
- **执行结果**：退出码 `0` (PASS)，运行 39 项断言全部通过：
  - `status_current_release_id` - PASS
  - `status_current_sequence` - PASS
  - `status_architecture` - PASS
  - `status_components_consistent` - PASS
  - `status_candidate_release_id` - PASS
  - `status_candidate_sequence` - PASS
  - `status_transaction_status` - PASS
  - `status_guard_maintenance` - PASS
  - `check_unconfigured_status` - PASS
  - `check_unconfigured_reason` - PASS
  - `check_unconfigured_has_no_fake_candidate` - PASS
  - `check_unverified_status` - PASS
  - `check_unverified_candidate_release` - PASS
  - `check_unverified_candidate_size` - PASS
  - `check_unverified_publisher_verified_false` - PASS
  - `check_unverified_install_blocked` - PASS
  - `check_already_installed_status` - PASS
  - `check_already_installed_install_blocked` - PASS
  - `check_incompatible_status` - PASS
  - `check_incompatible_reason` - PASS
  - `check_incompatible_install_blocked` - PASS
  - `error_network_offline` - PASS
  - `error_command_failed` - PASS
  - `signing_keychain_locked` - PASS
  - `signing_failure_description` - PASS
  - `drain_active` - PASS
  - `drain_deferred` - PASS
  - `drain_cleared_after_defer` - PASS
  - `recovery_status_flag` - PASS
  - `recovery_status_tx_id` - PASS
  - `recovery_status_stage` - PASS
  - `recovery_status_error` - PASS
  - `recovery_guide_contains_instructions` - PASS
  - `update_view_loaded` - PASS
  - `offscreen_render_unconfigured` - PASS
  - `offscreen_png_exported` (`/tmp/test_update_ui_unconfigured.png`, 90KB) - PASS
  - `offscreen_recovery_png_exported` (`/tmp/test_update_ui_recovery.png`, 100KB) - PASS
  - `shell_select_update_page` - PASS
  - `settings_page_jump_to_update` - PASS

### 3.3 回归测试与门禁验证
1. `cargo test --test swift_host --locked`：74 项测试全部 PASS。
2. `cargo test --test swift_brain_ui --locked`：全部 PASS。
3. `swiftc native/bootstrap/CoreSupervisor.swift native/bootstrap/Supervisor.swift native/tests/LifecycleTests.swift -o /tmp/lifecycle_tests && /tmp/lifecycle_tests`：全部 PASS。
4. `cargo test test_control_update_status_and_check --locked`：PASS。
5. `bash scripts/test-gates.sh`：门禁测试 PASS。
6. `cargo fmt --check`：退出码 0，无格式差异。
7. `cargo clippy --all-targets --locked`：退出码 0。

---

## 4. 风险、限制与未验证项声明

1. **未授权真实部署与安装**：
   - 本任务遵守协调者限定放行范围，真实安装入口（`apply` / `rollback`）在底层保持 `unsupported`。UI 准确呈现该提示，不伪造更新成功。
2. **生产信任公钥未就绪**：
   - 生产公钥根尚未配置，未配置信任根时 UI 正确标识 `尚未验证（无可信根公钥配置）` 并阻断安装。
3. **真实真机钥匙串与 TCC 权限**：
   - 专用机钥匙串解锁与 macOS 录屏/辅助功能权限绑定需要在目标机实测，本任务仅完成了合成测试与故障注入验证。
