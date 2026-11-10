# I3 大脑配置与对话追溯界面实现 技术审查报告

- **日期**：2026-11-10
- **任务编号**：I3（当前派发：`task_23a5854df5b2` / `ctx_64c713f1f84b`，前序：`task_86a24d0cb23c` / `ctx_4ddd423d2dc5`、`task_48f9bde2b511` / `ctx_a9280fad2d32`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：撤回之前证据不足的“PNG无裁切/清晰”结论；全面修正隔离渲染 fixture（Aqua 外观、不透明系统浅色窗体、装配丰富合成数据）；修正会话页提示文本；真实 Rust control + Swift client 契约集成测试持续通过；提交本地审查，非整个项目验收通过。

---

## 1. 协调者反馈与证据不足结论撤回

### 1.1 结论撤回与原因核查
在上一轮交付中声称的“PNG 证明清晰无裁切”结论**在此明确撤回**。经核实：
1. **背景透明导致暗色隐没**：原 fixture 仅在裸视图上执行 `cacheDisplay`，未挂载于设置了不透明背景与具体外观的 `NSWindow`，导出的 PNG 背景包含透明 alpha 通道。在暗色查看器下背景呈黑色，深色文字与边框完全隐没；
2. **缺乏真实合成数据呈现**：原 fixture 仅在空状态或未完成数据绑定的初始视图下截帧，没有呈现完整的角色列表、人设正文、知识摘要、绑定覆盖以及结构化轮次追溯内容，不足以作为视觉无裁切与文字可读性的充分证据。

### 1.2 本轮修正与改进对账
1. **不透明 Aqua 离屏窗口承载与渲染**：
   - 编写 `renderOpaquePNG`，使用显式 `NSAppearance(named: .aqua)` 与不透明系统浅色窗口背景（`RGB(242, 242, 245)`）的离屏 `NSWindow` 承载视图；
   - 显式在位图上下文中以 100% 不透明浅色底色填充画布后再执行视图绘制，确保导出的 PNG 每个像素的 `min_alpha == 255`（完全不透明），文字与控件具有极高的明暗对比度（动态范围 0~255，平均 RGB 约 240~244）；
2. **载入丰富完整的合成数据（绝不触碰真实业务或启动宿主）**：
   - **大脑配置页**：载入“VIP 售后专属顾问”与“售前导购助手”等多角色、草稿（`rev_v2`）与运行中（`rev_v1`）双版本标记、人设正文、语气、多条目标与规则、系统能力约束说明、两项知识引用及 64 位 SHA256 摘要、话术自定义模板、联系人白名单绑定与覆盖；
   - **分 Tab 独立渲染输出**：分别切换并导出了定义页（`brain_page_definition.png` / `brain_page.png`，99K）、知识页（`brain_page_knowledge.png`，88K）、绑定页（`brain_page_bindings.png`，68K）、版本页（`brain_page_version.png`，62K）；在 760px 宽度下，6 个分段标签、各输入框、表格列宽及底部操作按钮均布局工整，横向约束无裁切；
   - **会话页（`sessions_page.png`，34K）**：载入多条不同健康状态（listening、handoff）的会话，顶部精准输入框填有 `turn_contract_real_101`，下方结构化详情完整呈现真实格式合成 provenance（Turn ID、联系人、角色修订、模型引用、幂等键、阶段独立标签“模型已生成 · 发送已触发 · 已发送”、大脑决策与回复）；
3. **会话提示文本更正**：
   - 原文案仍残留“点击会话行查看”，已同步修改为：“暂无选中轮次。请在上方输入框手动输入有效 turn_id 并点击查询，以查看详细快照、角色修订与发送状态。”；
4. **并发测试说明纠正**：
   - 撤回“已知事实是 socket 端口抢占”的不严谨表述，客观记录为：并发多线程测试在特定用例处停顿超过 60 秒后手动终止，具体挂起机理未经逐一排查比对，不作为已知定论，亦不作为无回归依据。

---

## 2. 测试执行与验证证据（RED / GREEN）

### 2.1 契约集成与 UI 离屏渲染测试（`tests/swift_brain_ui.rs`）

- **RED 阶段**：
  - 更新 `host_brain_selftest` 增加丰富合成状态与分 Tab 渲染后，针对渲染方法、会话状态输入类型进行了严格编译与类型校验（记录修复过程）。
- **GREEN 阶段**：
  - 执行 `cargo test --locked --test swift_brain_ui`：
    - 退出码：0
    - 耗时：18.70s
    - 结果：`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.70s`
    - 包含 13 大组全部通过：
      1. `budget_defaults`：字节预算默认 49152/16384/16384 与 truncate_tail；
      2. `templates_reasons_coverage` & `templates_greeting`：话术 8 项原因完整覆盖与 null 映射；
      3. `brain_list_parse` & `execution_mode_managed` & `roles_count`：草稿与运行态分离、角色解析；
      4. `cas_conflict_preserves_local_name` & `cas_conflict_reports_error`：CAS 冲突时完整保留本地输入；
      5. `copy_role_new_id` & `in_use_role_cannot_archive`：复制生成新 ID、active 引用角色禁止归档；
      6. `role_knowledge_ref_fixed_sha`：知识引用摘要与版本固定；
      7. `preview_decision_null` & `preview_cost_incurred_false`：组装预览标记未调用模型；
      8. `binding_default_role` & `binding_override_allowlist_valid`：联系人白名单唯一来源验证；
      9. `legacy_provenance_null` & `legacy_display_text` & `modern_provenance_present` & `modern_role_id` & `modern_phase_labels`：旧记录与新记录阶段标签拆分展示；
      10. `shell_select_page_by_id`：导航选择与跨页面跳转；
      11. `isolated_ui_view_loaded` & `isolated_ui_view_size` & `isolated_ui_table_configured`：隔离 UI Fixture 视图层次与视图树装配校验；
      12. **新离屏渲染与视觉验证**：
          - `brain_page_definition_written` & `brain_page_png_written` & `brain_page_png_size_valid`（文件大小 99KB，像素 760x780，min_alpha=255，mean_RGB 约 240）；
          - `brain_page_knowledge_written`（文件大小 88KB，呈现导入区域与已引用列表）；
          - `brain_page_bindings_written`（文件大小 68KB，呈现默认角色与覆盖项）；
          - `brain_page_version_written`（文件大小 62KB，呈现版本与操作按钮）；
          - `sessions_page_png_written` & `sessions_page_png_size_valid`（文件大小 34KB，像素 740x760，呈现会话列表与结构化轮次详情）；
      13. `real_control_init` ~ `real_control_conflict_caught`（16 项真实控制契约断言）：角色新建/保存/读取、知识保存引用、绑定/激活/合法回退、assemble_only 组装、真实轮次详情各字段解码与 CAS 冲突拦截。

### 2.2 既有宿主功能回归（`tests/swift_host.rs`）

- **执行命令**：`cargo test --locked --test swift_host`
- **退出码**：0
- **耗时**：17.98s
- **结果**：`test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.98s`
- **范围限制说明**：该套件仅覆盖宿主状态、控制命令封装、Supervisor 生命周期、配置表单与联系人同步等 74 项宿主回归用例，通过仅代表该范围有效。

### 2.3 门禁与代码质量

- **`bash scripts/test-gates.sh`**：退出码 0，PASS；
- **`rustfmt --edition 2024 --check tests/swift_brain_ui.rs`**：退出码 0，PASS；
- **`cargo clippy --test swift_brain_ui --locked`**：退出码 0，PASS（0 警告）。

### 2.4 并发测试手动终止记录

并发多线程测试在 `gateway::tests::brain_tools_dispatched_via_gateway` 等处停顿超过 60 秒后手动终止。具体挂起机理未经逐一排查比对，不作为已知定论，亦不作为无回归依据，按照指令不重复启动整套并发测试。

---

## 3. 修改文件与所有权清单

严格遵循预定所有权范围，无越界修改：

- **`native/bootstrap/BrainPage.swift`**：
  - 增加 `selectTab(index: Int)` 与 `applyFixtureState(payload:role:accountId:)`，支持视图加载合成数据与切换 Tab 进行离屏渲染；
  - 保持已修复的 generation CAS 解析与小写 UUID 标准化；
  - 保持 Codable 显式 `encodeNil` 补齐字段。
- **`native/bootstrap/SessionsPage.swift`**：
  - 修复提示文案，将“点击会话行查看”更为“请在上方输入框手动输入有效 turn_id 并点击查询”；
  - 增加 `applyFixtureState(sessions:selectedTurnDetail:)` 与公开 `displayTurnDetail` 方法。
- **`tests/host_brain_selftest/main.swift`**：
  - 重写第 12 组渲染逻辑，引入 `renderOpaquePNG`（Aqua 外观、不透明浅色窗体、递归渲染）；
  - 注入合成角色、版本、话术、知识与结构化轮次详情数据，分 Tab 导出并校验 5 个关键页 PNG。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 更新技术报告，撤回原不充分结论，详细说明不透明 Aqua 渲染与合成数据证据。

---

## 4. 风险、限制与未验证项

1. **真实微信与付费模型未验证**：本任务严格遵循安全约束，未连接真实微信进程，未发送真实微信消息，未调用付费模型 API；
2. **全仓并发测试缺口**：全仓并发测试存在停顿缺口，具体挂起机理未排查定论；
3. **真机 Retina 动态交互抽检**：虽然离屏 PNG 在 760px 宽度与 Aqua 浅色外观下证实了不透明度（`alpha==255`）与各控件/文字的高对比度排版无裁切，但高分屏真实用户的鼠标悬停、文本框动态输入及滚轮交互体验需在生产前置验收阶段抽检。
