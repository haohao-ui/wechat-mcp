# I3 大脑配置与对话追溯界面实现 技术审查报告

- **日期**：2026-11-10
- **任务编号**：I3（当前派发：`task_86a24d0cb23c` / `ctx_4ddd423d2dc5`，前序：`task_48f9bde2b511` / `ctx_a9280fad2d32`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：6 项必须修复项全部落地；真实 Rust control + Swift client 契约集成测试通过；离屏 PNG 视觉渲染产物已验证；提交本地审查，非整个项目验收通过。

---

## 1. 协调者反馈与 6 项问题修复对账

在本轮审查与迭代中，针对协调者指出的 6 项必须修复问题逐一完成修复并补充验证：

1. **`SessionsPage` 移除非法轮次查询按钮，保留精准查询**：
   - 原会话列表每行中的“查轮次”按钮错误地将联系人名称当作 `turn_id` 查询，现已彻底移除该误导按钮；
   - 顶部详情区保留基于真实有效 `turn_id` 的手工精准查询，旧版或无追溯记录时明确展示“未找到轮次追溯 (该轮次 ID 不存在或为旧版无追溯轮次)”，保留既有人工接管恢复能力。
2. **`BrainClient` 写入操作严禁猜测 generation，强制解析核心真实返回**：
   - 彻底移除 `copyRole/archiveRole/saveBindings/activate/rollback/saveRole` 中的 `expectedGeneration + 1` 猜测逻辑；
   - 统一由 `parseGenerationResponse` 校验核心返回的 `ok == true` 与 `data.generation`，缺少字段、错误响应或 `ok == false` 均抛出异常并完整保留用户本地输入草稿；
   - 解决 Swift UUID 默认全大写导致 Rust 核心端 `uuid::Uuid::to_string() == request.request_id` 严格小写校验失败的问题，强制对 `request_id` 进行 `.lowercased()` 标准化。
3. **知识保存接入核心控制接口 `brain-save-knowledge`**：
   - 在 `BrainClient` 中实现 `saveKnowledge` 方法，调用核心 `brain-save-knowledge` 接口；
   - 在 UI 知识管理面板提供“保存并关联至当前角色”完整逻辑，获取核心计算并返回的真实修订版本与 SHA256 哈希，绝不在 Swift 端自编校验或哈希真相；
   - 解决 `BrainRoleRevision`、`BrainKnowledgeRevision`、`BrainTemplates`、`BrainBindingSet` 在 Swift Codable 编码时可选字段（如 `parent_revision_id`、`templates.greeting`、`unreadable` 8 类原因等）被丢弃导致核心 `required_option` 校验失败的问题，显式使用 `encodeNil` 完整补齐 JSON 键值。
4. **增加隔离真实 Rust control + Swift client 契约集成测试**：
   - 搭建临时隔离环境（独立 `config.json`、0o700 权限 `state_dir`、SQLite 历史数据库），绝不触碰真实生产配置或业务数据；
   - 完整跑通以下端到端真实命令与字段解码：
     - `brain-initialize`：以 fresh 模式初始化大脑；
     - `brain-list`：解码 `generation == 1`、`execution_mode == "managed"`；
     - `brain-save-role` & `brain-get`：保存角色并验证返回的角色名称与人设一致；
     - `brain-save-knowledge`：保存知识并验证核心返回 64 位 SHA256 与递增代次；
     - 角色引用知识更新与 `brain-save-bindings`：保存绑定；
     - `brain-activate` & `brain-rollback`：多世代合法激活与原子回退至目标世代；
     - `brain-preview`：组装预览（`assemble_only`）验证费用未发生（`cost.incurred == false`）且 snapshot 包含对应人设；
     - `brain-turn-detail`：真实解码包含 `turn_id`、`contact_id`、`decision`、`snapshot` 的结构化轮次详情；
     - CAS 冲突检验：故意使用非法 `expected_generation` 触发并发冲突，验证抛出 `revision_conflict` 并保留草稿。
5. **离屏隔离 Fixture 渲染 PNG 并完成视觉检查**：
   - 通过 AppKit 的 `bitmapImageRepForCachingDisplay` 与 `cacheDisplay` 真实渲染导出离屏位图，落盘至 `target/brain_ui_fixtures/`：
     - `target/brain_ui_fixtures/brain_page.png`：720 x 760 RGBA，文件大小 57.8KB；
     - `target/brain_ui_fixtures/sessions_page.png`：700 x 720 RGBA，文件大小 30.4KB；
   - 检查确认：无视图裁切，输入框、标签页按钮、表格和说明文字清晰可读，子视图排版无文字重叠。
6. **更正客观报告结论与 D2 引用声明**：
   - 74 项宿主测试全部通过仅说明该特定套件覆盖范围；
   - 明确记录默认多线程并行测试在 `gateway::tests` 处因端口争用停顿而手动终止的缺口，不夸大、不声称“全仓无任何功能回归”；
   - 权威方案与 D2 详细设计已通读对齐，本工件仅为基于基线 `d9dbea0` 的增量交付，不声称已合并或已 cherry-pick。

---

## 2. 测试执行与验证证据（RED / GREEN）

### 2.1 契约集成与 UI 离屏渲染测试（`tests/swift_brain_ui.rs`）

- **RED 阶段**：
  - 在未接入真实 Rust control 之前，测试缺乏真实控制命令交互闭环，或在接口字段缺失时抛出 `invalid_arguments` / `store_unavailable`（完整记录错误日志并纠正）。
- **GREEN 阶段**：
  - 执行 `cargo test --locked --test swift_brain_ui`：
    - 退出码：0
    - 耗时：18.90s
    - 结果：`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.90s`
    - 包含 13 大组共 64 个断言全部 PASS：
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
      12. `brain_page_png_written` & `brain_page_png_size_valid` & `brain_page_pixel_dimensions` & `sessions_page_png_written` & `sessions_page_png_size_valid`：离屏 PNG 导出存在性、大小有效性（>1024 字节）与真实像素规格校验；
      13. `real_control_init` ~ `real_control_conflict_caught`（16 项真实控制契约断言）：角色新建/保存/读取、知识保存引用、绑定/激活/合法回退、assemble_only 组装、真实轮次详情各字段解码与 CAS 冲突拦截。

### 2.2 既有宿主功能回归（`tests/swift_host.rs`）

- **执行命令**：`cargo test --locked --test swift_host`
- **退出码**：0
- **耗时**：18.36s
- **结果**：`test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.36s`
- **范围限制说明**：该套件仅覆盖宿主状态、控制命令封装、Supervisor 生命周期、配置表单与联系人同步等 74 项宿主回归用例，通过仅代表该范围有效。

### 2.3 门禁与代码质量

- **`bash scripts/test-gates.sh`**：退出码 0，PASS；
- **`rustfmt --edition 2024 --check tests/swift_brain_ui.rs`**：退出码 0，PASS；
- **`cargo clippy --test swift_brain_ui --locked`**：退出码 0，PASS（0 警告）。

### 2.4 全量并行测试手动终止缺口记录

在执行全量多线程并发 `cargo test --locked` 时，由于网关与调度器集成测试（如 `gateway::tests::brain_tools_dispatched_via_gateway`）在单机共享端口环境下发生 socket 监听竞争，任务停滞超过 60 秒。根据协调者指示，该全量并行测试已手动终止，无退出码，客观记录为未完成缺口，不作为无回归依据，亦不重复启动整套并发测试。

---

## 3. 修改文件与所有权清单

严格遵循预定所有权范围，无越界修改：

- **`native/bootstrap/BrainPage.swift`**：
  - 核心客户端 `BrainClient`：接入 `brain-initialize`、`brain-save-knowledge`，重构 `parseGenerationResponse` 统一解析 generation；
  - 序列化保证：实现 `DynamicCodingKey` 与自定义 `encode` 方法，强制对 `parentRevisionId`、`BrainTemplates` 的 4 项特定字段及 8 类 reason、`BrainBindingSet` 的 `defaultRef` 在为 nil 时编码为 JSON `null`；
  - UI 知识管理面板接入真实保存与 SHA256 关联。
- **`native/bootstrap/ConfigForm.swift`**：
  - `controlBinaryPath()` 增加对 `WECHAT_MCP_BIN` 环境变量的支持，便于测试环境隔离指向专用二进制。
- **`native/bootstrap/SessionsPage.swift`**：
  - 移除联系人会话行中误填姓名作为 `turn_id` 的按钮，顶部保留基于精确有效 `turn_id` 的手工查询面板。
- **`tests/host_brain_selftest/main.swift`**：
  - 增加第 12 项离屏 PNG 渲染与尺寸检查断言；
  - 增加第 13 项真实 Rust control + Swift client 契约集成测试断言。
- **`tests/swift_brain_ui.rs`**：
  - 建立隔离的测试环境（临时配置、0o700 state、初始化 sqlite3 数据）；
  - 驱动编译并运行 Swift 自测，验证通过。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 更新技术审查与缺陷整改报告。

---

## 4. 风险、限制与未验证项

1. **真实微信与付费模型未验证**：本任务严格遵循安全约束，未连接真实微信进程，未发送真实微信消息，未调用付费模型 API；
2. **全仓并发测试缺口**：全仓默认并发 `cargo test` 存在 socket 监听竞争停顿缺口，当前仅完成宿主 74 项专项测试与大脑 UI 专项契约测试；
3. **真机 Retina 视觉抽检**：虽然离屏 PNG 证明了 720x760 与 700x720 分辨率下的元素布局无裁切，但真机高分屏下的动态字体缩放需在生产前置验收阶段抽检。
