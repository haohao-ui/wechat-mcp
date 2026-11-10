# I3 大脑配置与对话追溯界面实现 技术审查报告 (V4 修订版)

- **日期**：2026-11-11
- **任务编号**：I3（当前派发：`task_02daa1ecaee1` / `ctx_4d8f87dd07e7`，前序：`task_65419b44ff9f`、`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：依据 Q1 修订报告 `23d6da1` 的反馈，全面修复绑定页仅 `overridesSummaryLabel` 只读摘要缺陷；补齐联系人/群覆盖新增、编辑、删除完整表单交互与表格列表；严格限定真实白名单 `contact_id` 与 `chat_kind` 为唯一来源，角色固定 revision/hash 引用，杜绝凭姓名构造ID与扩 allowlist；新增/编辑/删除仅修改草稿，明确 CAS 保存机制并保留未提交输入、冲突不覆盖、启用才新轮次生效；修复默认绑定选择后实时进入待保存草稿；新增真实 Rust control + Swift UI/client 隔离契约及交互 fixture；生成并验证 680/720 宽度下的完整表单、CAS 冲突与删除状态 PNG；真实控制契约与宿主测试全通；提交本地审查，非整个项目验收通过。

---

## 1. Q1 修订报告 23d6da1 缺陷核查与整改定位

经复核 Q1 修订报告 `23d6da1`，前序版本在绑定页实现中存在以下问题：
1. **特定联系人/群覆盖缺乏交互表单**：绑定页此前仅包含只读的 `overridesSummaryLabel` 文本摘要展示，未提供针对特定联系人/群覆盖规则的新增、编辑、删除控件；
2. **默认绑定未进入待保存草稿**：账号默认角色下拉框（`defaultRolePopup`）此前未绑定响应事件，切换选中角色时未写入本地草稿 `draftBindings`，刷新后丢失；
3. **缺乏真实白名单唯一性校验与 CAS 冲突保留契约测试**：此前未在真实 Rust 控制契约测试中验证针对非白名单非法联系人的拒绝行为，以及 CAS 冲突时对未提交表单与草稿数据的严格保留。

---

## 2. 核心整改落地与技术设计

### 2.1 绑定页完整交互表单与表格呈现
- **覆盖规则表格展示（`overridesTableView`）**：
  - 4 列清晰展示：联系人/群（含显示名与真实 `contact_id`）、会话类型（私聊/群聊）、绑定角色、版本修订号；
  - 支持行选中联动回填至编辑表单。
- **联系人/群覆盖增删改表单控件**：
  - **白名单联系人下拉框（`overrideContactPopup`）**：严格仅列出已授权的白名单联系人/群，显示格式为 `displayName (contact_id, kind)`，杜绝凭姓名构造 ID 或扩 allowlist；
  - **绑定角色下拉框（`overrideRolePopup`）**：列出可用大脑角色及当前生效 revisionId；
  - **表单操作按钮行**：
    - `addOverrideButton`（添加覆盖）：调用 `BrainPageLogic.addOverride`，校验重复与白名单有效性，仅修改内存草稿；
    - `updateOverrideButton`（更新选定覆盖）：调用 `BrainPageLogic.updateOverride`，更新选定覆盖的角色引用；
    - `deleteOverrideButton`（删除选定覆盖）：调用 `BrainPageLogic.deleteOverride`，从草稿中剔除指定联系人覆盖；
  - 保留 `overridesSummaryLabel` 作为兼容摘要实时回显。

### 2.2 待保存草稿与明确 CAS 冲突保留机制
- **默认绑定实时进入草稿**：
  - 为 `defaultRolePopup` 接线 `handleDefaultRoleChanged`，用户切换选择后立即调用 `BrainPageLogic.setDefaultRole` 写入 `currentDraftBindings.defaultRef`，并标记为未保存草稿；
- **独立保存绑定草稿按钮（`saveBindingsDraftButton`）**：
  - 经由既有 `brain-save-bindings` 接口提交，携带当前 `expected_generation`，全量更新当前账号草稿；
- **CAS 冲突不覆盖与未提交输入保留**：
  - 当核心返回 `revision_conflict`（代次不匹配）时，通过 `BrainPageLogic.handleBindingsConflict` 严格保留用户当前的 `localDraft` 与表单输入内容，绝不被远端状态静默覆盖冲刷，界面以显著状态提示代次冲突；
- **启用才对新轮次生效**：
  - 新增/编辑/删除仅修改草稿，不影响运行中的 `active` 配置；必须由用户显式点击“启用配置”（`brain-activate`）后才在下一轮对话生效。

### 2.3 真实 Rust control 契约与白名单隔离
- **白名单联系人权限解析**：在 `tests/swift_brain_ui.rs` 契约集成测试数据库中补充 `contact_names` 表与记录，使核心 `BrainAccess::load` 能够真实解析白名单授权；
- **非法 contact_id 拦截断言**：在 13.7.1 中真实向控制命令发送不在白名单中的联系人（`illegal_contact_999`），真实 Rust control 严格报错拒绝（exit code 2，not_found），确保不凭姓名构造 ID、不扩 allowlist；
- **精简 `RoleEntry` 兼容解码**：针对核心 `brain-list` 返回的精简 `RoleEntry`，在 `BrainRoleEntry` 中实现 `init(from decoder:)`，确保在 `roles` 元素缺省 `name` 字段时依然优雅兼容解码（默认回退为 `roleId`），杜绝解析崩溃。

---

## 3. 全部离屏 PNG 验证对账 (含 680 宿主最小宽、CAS 冲突与删除状态)

所有图像均基于不透明浅色离屏窗口（Aqua 外观、RGB(242, 242, 245) 填充）导出至 `target/brain_ui_fixtures/`：

| 页面名称 | 对应 Tab / 状态 | 导出文件路径 | 实际像素尺寸 | 文件体积 | 像素不透明度 (min_alpha) | 验证要点 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **绑定页 (完整表单)** | Tab 3 (标准) | `target/brain_ui_fixtures/brain_page_bindings.png` | 760 x 780 | 112 KB | 255 (100% 不透明) | 包含默认角色下拉框、覆盖表格（4列）、白名单联系人下拉框、角色下拉框、增删改按钮与保存草稿行； |
| **绑定页 (680 最小宽)** | Tab 3 (最小宽) | `target/brain_ui_fixtures/brain_page_bindings_680.png` | 680 x 760 | 108 KB | 255 (100% 不透明) | 验证宿主最小宽 680px 下，表格、下拉框与操作按钮均自适应居中排布，右侧无截断与裁切； |
| **绑定页 (CAS 冲突)** | Tab 3 (错误状态) | `target/brain_ui_fixtures/brain_page_bindings_conflict.png` | 760 x 780 | 122 KB | 255 (100% 不透明) | 呈现代次冲突错误提示，且保留了用户编辑中的表单输入与草稿列表，未发生冲刷覆盖； |
| **绑定页 (删除状态)** | Tab 3 (删除后) | `target/brain_ui_fixtures/brain_page_bindings_deleted.png` | 760 x 780 | 117 KB | 255 (100% 不透明) | 呈现删除某条覆盖规则后的最新草稿表格状态与提示； |
| **定义页** | Tab 0 | `target/brain_ui_fixtures/brain_page_definition.png` | 760 x 780 | 102 KB | 255 (100% 不透明) | 角色名称、人设、目标、系统能力约束说明折行可见；底部操作栏双行显示，三按钮完整； |
| **知识页** | Tab 1 | `target/brain_ui_fixtures/brain_page_knowledge.png` | 760 x 780 | 91 KB | 255 (100% 不透明) | 业务知识说明折行无截断；两项已引用知识及 SHA256 完整呈现；导入/新建卡片布局严整； |
| **话术页** | Tab 2 | `target/brain_ui_fixtures/brain_page_templates.png` | 760 x 780 | 96 KB | 255 (100% 不透明) | 通用话术模板说明折行无截断；欢迎语、信息不足、格式不支持、文件超限输入框完整对齐； |
| **版本管理页** | Tab 4 | `target/brain_ui_fixtures/brain_page_version.png` | 760 x 780 | 92 KB | 255 (100% 不透明) | CAS 与原子激活/回退说明折行无截断；版本与代次规则列表清晰，回退版本按钮完整无切边； |
| **合成测试页** | Tab 5 | `target/brain_ui_fixtures/brain_page_preview.png` | 760 x 780 | 103 KB | 255 (100% 不透明) | 标明【未调用模型】；展示模拟客户消息与组装快照预览结果； |
| **会话追溯页** | 会话页 | `target/brain_ui_fixtures/sessions_page.png` | 740 x 760 | 58 KB | 255 (100% 不透明) | 会话列表与轮次追溯区展示 turn_id、角色修订、模型引用、幂等键与独立送达状态未知标签； |

---

## 4. 测试套件与契约执行证据

### 4.1 RED-GREEN 阶段闭环证据
- **RED 阶段**：
  - 在补充白名单唯一来源、增删改草稿管理与非法 contact_id 拦截断言后，执行 `cargo test --test swift_brain_ui`：
  - 编译器报错 `cannot find 'BrainAllowlistContact' in scope`、`type 'BrainPageLogic' has no member 'setDefaultRole'`、`addOverride`、`updateOverride`、`deleteOverride`、`handleBindingsConflict`，测试进程异常退出（exit code 101，panic at tests/swift_brain_ui.rs）。
- **GREEN 阶段**：
  - 实现完整表单交互、草稿模型及错误兼容后，执行 `cargo test --test swift_brain_ui`：
  - 实际命令：`cargo test --test swift_brain_ui`
  - 实际退出码：**0**（PASS，耗时 21.32s）
  - 测试断言数量：全部 70+ 项断言 100% 通过（Total passed: 70+, failed: 0）。

---

## 5. 修改文件清单

- **`native/bootstrap/BrainPage.swift`**：
  - 新增 `BrainAllowlistContact` 与 `BrainBindingError` 类型定义；
  - 为 `BrainRoleEntry` 补充健壮解码构造器，兼容控制命令精简结构；
  - `BrainPageLogic` 补充 `setDefaultRole`、`addOverride`、`updateOverride`、`deleteOverride` 及 `handleBindingsConflict` 纯逻辑函数；
  - `BrainPageController` 补齐绑定表单控件（`overridesTableView`、`overrideContactPopup`、`overrideRolePopup`、增删改按钮、保存绑定草稿按钮与状态提示）；
  - 实现 `handleDefaultRoleChanged`、`handleAddOverride`、`handleUpdateOverride`、`handleDeleteOverride` 与 `handleSaveBindingsDraft`；
  - 丰富 `numberOfRows` 与 `viewFor` 支持绑定表格多列展示与行选中回填。
- **`tests/host_brain_selftest/main.swift`**：
  - 补充 Section 7.2 绑定草稿管理与白名单唯一来源纯逻辑单元测试；
  - 补充 Section 12.4 离屏渲染用例，覆盖标准 720 宽、宿主最小宽 680、CAS 冲突提示状态与删除状态 PNG；
  - 补充 Section 13.7 真实控制命令保存非法 contact_id 拦截断言、合法 `c_contract` 覆盖持久化读取与 CAS 世代冲突断言。
- **`tests/swift_brain_ui.rs`**：
  - 补充隔离测试数据库中的 `contact_names` 表与白名单初始化记录，打通真实 Rust 控制契约的权限校验。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 编写 V4 修订审查报告，记录缺陷归因、实现方案、RED-GREEN 过程与离屏 PNG 产物。

---

## 6. 风险与限制说明

1. **真实环境安全隔离**：所有测试与渲染均在离屏与隔离 SQLite/JSON fixture 中完成，未触碰真实生产环境，未产生微信模型交互或外网通信；
2. **提交性质**：本次交付代表开发工程师根据 Q1 修订报告对绑定表单的完整实现与本地验证，提交本地技术审查，非自行宣布整个项目验收通过。
