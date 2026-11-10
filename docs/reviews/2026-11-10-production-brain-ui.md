# I3 大脑配置与对话追溯界面实现 技术审查报告

- **日期**：2026-11-10
- **任务编号**：I3（`task_48f9bde2b511` / `ctx_a9280fad2d32`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：阶段实现与隔离测试完成；提交本地审查，非整个项目验收通过。

---

## 1. 任务范围与设计集成（D2）

依照原 checkout 权威方案《生产落地执行方案：大脑配置界面、对话闭环与应用版本更新》（只读）及任务书《I3 大脑配置与对话追溯界面实现》、D2 详细设计（commit `afed6b1134256988b3613f5ee475ea9fab8bd133`），严格在派发 worktree 文件所有权边界内实施：

1. **角色列表与管理**：
   - 角色列表显示角色名称、草稿版本与运行版本标签、绑定账号数、归档标记；
   - 支持空草稿角色新增、复制角色（生成新 `role_id`、未绑定、记录父修订）、安全归档（当被 active 引用时拦截，UI 引导先改绑）。
2. **角色定义与系统约束**：
   - 名称（1..80 字符）、语气（≤2000 字符）、身份与业务范围 Persona（≤8000 字符）、目标与规则结构化列表；
   - 系统约束保持只读，不向用户暴露删改入口，不与角色人设混淆。
3. **知识与话术管理**：
   - 知识导入 Markdown / 纯文本，自动固定版本与内容摘要，通过角色有序引用管理启用；
   - 话术严格限定 D1 契约的固定模板映射（`greeting`、`insufficient_information`、`unreadable` 8 类原因等），留空/关闭映射为 null，显示“无自动文案，进入待处理或人工接管”。
4. **模型连接引用与页面分工**：
   - 严格遵循 MCP 页面为模型连接、大脑模式与超时的唯一编辑源；大脑页仅只读显示连接摘要与跳转按钮；
   - `McpPage.swift` 增加明确跳转指引（`McpPageLogic.brainPagePointerText()`），两页字段不重复存储。
5. **账号与联系人绑定**：
   - 支持账号默认角色引用与私聊/群覆盖（Overrides）；
   - 联系人唯一真实来源为当前配置白名单（allowlist），绑定页仅引用不复制联系人清单。
6. **版本管理、CAS 并发保护与激活/回退**：
   - 草稿保存与运行中版本明确分离；保存草稿携带 `expected_generation`，遇并发冲突（CAS mismatch）完整保留用户当前输入，提示并发冲突；
   - 激活配置弹出模态确认框，明确“仅新轮次生效，在途轮次不受影响”；
   - 回退支持原子回退至目标代次；加载失败显示具体错误，禁用非法激活。
7. **合成测试（组装预览）**：
   - 仅执行确定性组装预览（`execution: "assemble_only"`），明确标注“【未调用模型】，仅组装预览”，不产生费用，绝不发送真实微信消息。
8. **对话 Provenance 追溯（`SessionsPage.swift`）**：
   - 在会话页面增加轮次追溯面板与查询入口；
   - 旧记录缺少 provenance 时明确显示“旧版未记录 (provenance: null)”；
   - 完整轮次分别展示：角色修订、模型引用、输入快照摘要、决策回复、幂等键 `send_key`、阶段独立标签（“模型已生成”、“发送已触发”、“己方回显”、“送达未知”）；
   - 严格遵循账号隔离，保留既有“恢复自动回复”按钮。
9. **导航注册与生命周期**：
   - `main.swift` 仅按标准顺序注册大脑页面：`总览 -> 会话 -> 联系人 -> 大脑 -> MCP -> 设置 -> 日志`；未改动任何启动、信号或 supervisor 生命周期。

---

## 2. 测试执行与验证证据（RED / GREEN）

### 2.1 新增测试套件（`tests/swift_brain_ui.rs`）

- **RED 阶段**：
  - 在未编写 `native/bootstrap/BrainPage.swift` 前执行测试，确认编译失败（RED 证据）：
    `<unknown>:0: error: error opening input file '.../BrainPage.swift' (No such file or directory)`
    `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s`。
- **GREEN 阶段**：
  - 实现代码后，执行 `cargo test --locked --test swift_brain_ui`：
    - 退出码：0
    - 耗时：17.54s
    - 结果：`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.54s`
    - 内含测试用例覆盖（1 个测试套件，执行自测涵盖 11 组共 24 项断言全部 PASS）：
      1. `budget_defaults`：字节预算默认 49152/16384/16384 与 truncate_tail；
      2. `templates_reasons_coverage` & `templates_greeting`：话术 8 项原因完整覆盖与 null 映射；
      3. `brain_list_parse` & `execution_mode_managed` & `sales_role_draft_vs_active_different`：草稿与运行态分离、迁移角色标识识别；
      4. `cas_conflict_preserves_local_name` & `cas_conflict_reports_error`：CAS 冲突时完整保留本地输入；
      5. `copy_role_new_id` & `in_use_role_cannot_archive`：复制生成新 ID、active 引用角色禁止归档；
      6. `role_knowledge_ref_fixed_sha`：知识引用摘要与版本固定；
      7. `preview_decision_null` & `preview_cost_incurred_false` & `preview_marked_uncalled_model`：组装预览标记未调用模型；
      8. `binding_override_allowlist_valid`：联系人白名单唯一来源验证；
      9. `legacy_provenance_null` & `legacy_display_text` & `modern_provenance_present` & `modern_phase_labels`：旧记录未记录标识、新记录阶段标签拆分展示；
      10. `shell_select_page_by_id`：导航选择与跨页面跳转；
      11. `isolated_ui_view_loaded` & `isolated_ui_view_size` & `isolated_ui_table_configured`：隔离 UI Fixture 视图层次与视图树装配校验（720x760，子视图正常挂载，未连真实微信）。

### 2.2 既有宿主功能回归（`tests/swift_host.rs`）

- **执行命令**：`cargo test --locked --test swift_host`
- **退出码**：0
- **耗时**：18.16s
- **结果**：`test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.16s`
- **结论**：既有 74 项宿主测试 100% 通过，未发生任何功能回归。

### 2.3 门禁与代码质量

- **`bash scripts/test-gates.sh`**：退出码 0，PASS（产品运行时无 Python，CI/发布门禁保持）；
- **`rustfmt --edition 2024 --check`**：退出码 0，通过；
- **`cargo clippy --all-targets --locked`**：退出码 0，通过（无新增警告）。

### 2.4 全量并行测试记录说明

此前执行默认多线程并行 `cargo test --locked` 时，进程在 `gateway::tests::brain_tools_dispatched_via_gateway` 与 `test_mcp_handle_request_through_gateway_to_core_sock_contract_and_mutex` 上停留超过 60 秒（未归因，无退出码，已手动终止），未作为测试通过依据。

---

## 3. 文件修改与所有权清单

严格遵循预定所有权范围，无越界修改：

- **`native/bootstrap/BrainPage.swift`**（新增）：大脑配置页面主控制器、数据模型、`BrainPageLogic` 纯逻辑工具、`BrainClient` 接口封装。
- **`native/bootstrap/Shell.swift`**（修改）：补充 `selectPage(id: String)` 支持页面间解耦跳转，保持既有布局尺寸与 NSBox setContent 约束。
- **`native/bootstrap/McpPage.swift`**（修改）：在回复决策区域增加指向大脑页面的指引文案与跳转按钮，不重复维护角色/知识/话术字段。
- **`native/bootstrap/SessionsPage.swift`**（修改）：增加“轮次追溯”按钮、独立 turn 查询输入框及结构化 Provenance 详情展示区，保持既有 handoff 恢复功能。
- **`native/bootstrap/main.swift`**（修改）：在标准顺序注册大脑页面，不改动启动、信号或 supervisor 生命周期。
- **`tests/host_brain_selftest/main.swift`**（新增）：纯逻辑与隔离 UI Fixture 自测入口。
- **`tests/swift_brain_ui.rs`**（新增）：Rust 端驱动 Swift 编译与执行的集成测试。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**（新增）：本技术审查报告。

---

## 4. 风险、限制与未验证项

1. **视觉渲染验收**：本任务所有 UI 验证均在隔离内存/合成 fixture 下完成，已验证视图树和控件装配；真机 Retina 像素对齐与交互抽检需在另行授权的专用机试运行阶段完成。
2. **执行预览**：合成测试严格限定为 `assemble_only`（组装预览），避免触发付费模型或外部副作用。
3. **已知既有失败**：Q0/I2 基线中的 `ops_tools_list_unchanged`（由历史桌面工具改动引起）严格保留，未在本任务中伪造或覆盖快照。
