# I3 大脑配置与对话追溯界面实现 技术审查报告 (V6 修订版)

- **日期**：2026-11-11
- **任务编号**：I3（当前派发：`task_c2bbf88549ee` / `ctx_9b7e694ae888`，前序：`task_c9618a210184`、`task_02daa1ecaee1`、`task_65419b44ff9f`、`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：依据协调者本轮复核意见，全面修复核心读取/解码吞错隐患、kind 限制及 Swift 在途 handler 保护与完整边界测试：
  1. 彻底消除 `if let Ok` / `rows.flatten()` 吞错误隐患，任何 prepare / query / 解码错误均严格返回 `unavailable` + `[]` + `access_resolution_failed`，丢弃部分结果，绝不能空/部分授权投影假装完整 ready；
  2. 聊天类型严格限定为 `private` 或 `group`，任何非 private/group 的非法 kind（如 `channel`、`unknown`、空串等）立即拒绝并返回 `unavailable`，杜绝未知 kind 默认 Private；
  3. `ControlContactAccess::permits` 在 `brain_access.is_none()` 时返回 `false`，杜绝未加载时放行任何 contact；
  4. 补齐 4 项真实 Rust control 单元测试，覆盖缺列但授权可成功路径、某行 kind/null/类型损坏、跨账号/同名/群私聊映射、白名单撤销后刷新；
  5. Swift 端实现在无法核验时禁用新增与编辑控件，但可展示既有覆盖并支持安全删除；
  6. 真实触发 `handleSaveBindingsDraft` 的在途 CAS 保护测试，验证在途新输入绝不被旧回调覆写；
  7. 实际核验 680px 宽度离屏 PNG（680x760），表单垂直分行从容排布，无截断裁切；
  8. 全量专项测试与宿主测试 100% 通过；提交本地技术审查，非自行宣布整个项目验收通过。

---

## 1. 协调者意见与核心整改落地

针对前序版本的核心实现与测试覆盖，协调者明确指出并完成了以下技术整改：

### 1.1 核心读取与解码错误零吞错 (`src/core/control.rs`)
- **问题根因**：原先使用 `if let Ok(stmt) = c.prepare(...)` 和 `rows.flatten()`，若 SQL prepare 失败、缺列、缺表或某行数据类型不匹配，错误被静默吞掉，最终代码直接走到 `(json!(list), "ready", None)`，把 DB 错误伪造成了合法的“无可授权项”或部分 ready 状态；
- **整改落地**：
  - 重写为独立的 `compute_allowlist_contacts_projection` 函数；
  - 针对 `conn.prepare`、`stmt.query`、`rows.next()` 以及每一行 `row.get(0..2)` 逐一执行严格错误匹配；
  - 一旦发生任何错误，**立刻丢弃全部已收集结果**，精确返回：
    `(json!([]), "unavailable", Some("access_resolution_failed"))`；
  - 只有在全量数据读取、解码与授权校验 100% 成功时，才输出 `status: "ready"`；当无可授权项时输出 `status: "ready", allowlist_contacts: []`。

### 1.2 严格限制 `chat_kind`
- 字段 `kind` 严格仅接受 `"private"` 与 `"group"`：
  ```rust
  let chat_kind = match kind_str.as_str() {
      "private" => crate::core::brain_profiles::ChatKind::Private,
      "group" => crate::core::brain_profiles::ChatKind::Group,
      _ => return (json!([]), "unavailable", Some("access_resolution_failed")),
  };
  ```
- 杜绝将未知/非法字符串默认当作 Private 处理。

### 1.3 `ControlContactAccess` 零放行安全加固
- 当 `brain_access.is_none()` 时，`permits` 必须返回 `false`，杜绝在授权未加载或解析失败时放行 contact。

### 1.4 Swift 绑定页在途 CAS 保护与白名单不可用状态 (`native/bootstrap/BrainPage.swift`)
- **白名单不可用（`unavailable`）状态**：
  - 禁用添加按钮、更新按钮与联系人下拉框；
  - 标红提示错误信息（如 `history_store_unavailable` 或 `access_resolution_failed`）；
  - 保留表格展示，当表格中存在覆盖规则时，删除按钮保持启用，支持用户安全删除既有覆盖规则；
- **真实 CAS 在途防护**：
  - `handleSaveBindingsDraft` 在触发异步操作前分配自增 `token`；
  - 在异步回调主线程分支中，比对 `guard self.currentSaveToken == token else { return }`；若在途期间用户产生了新编辑并触发新操作，旧异步结果直接丢弃，绝不覆盖新草稿；
  - 暴露 `onSaveBindingsFinished` 完成回调钩子供宿主自检测试真实验证。

---

## 2. 全部离屏 PNG 验证对账 (含 680 宿主最小宽、白名单不可用、CAS 冲突与删除状态)

使用 `sips -g pixelWidth -g pixelHeight` 核验生成的离屏渲染产物，所有图像均基于不透明浅色离屏窗口（Aqua 外观、RGB(242, 242, 245) 填充）导出至 `target/brain_ui_fixtures/`：

| 页面名称 | 对应 Tab / 状态 | 导出文件路径 | 实际像素尺寸 | 文件体积 | 像素不透明度 (min_alpha) | 验证要点 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **绑定页 (标准表单)** | Tab 3 (标准 720) | `target/brain_ui_fixtures/brain_page_bindings.png` | 760 x 780 | 110 KB | 255 (100% 不透明) | 包含默认角色下拉框、覆盖表格、白名单联系人下拉框、角色下拉框、增删改按钮与保存草稿行； |
| **绑定页 (680 最小宽)** | Tab 3 (最小宽) | `target/brain_ui_fixtures/brain_page_bindings_680.png` | 680 x 760 | 108 KB | 255 (100% 不透明) | 宿主最小宽 680px 下，垂直流式分行表单（90px 标签 + 填满行宽下拉框）在 460px 内容区从容对齐，右侧无截断； |
| **绑定页 (不可用状态)** | Tab 3 (不可用 680) | `target/brain_ui_fixtures/brain_page_bindings_unavailable_680.png` | 680 x 760 | 105 KB | 255 (100% 不透明) | 验证白名单 unavailable 时，680px 下新增/编辑按钮置灰，展示显式错误提示，已有覆盖可删除； |
| **绑定页 (CAS 冲突)** | Tab 3 (错误状态) | `target/brain_ui_fixtures/brain_page_bindings_conflict.png` | 760 x 780 | 120 KB | 255 (100% 不透明) | 呈现代次冲突错误提示，且保留了用户编辑中的表单输入与草稿列表，未发生冲刷覆盖； |
| **绑定页 (删除状态)** | Tab 3 (删除后) | `target/brain_ui_fixtures/brain_page_bindings_deleted.png` | 760 x 780 | 116 KB | 255 (100% 不透明) | 呈现删除某条覆盖规则后的最新草稿表格状态与提示； |
| **定义页** | Tab 0 | `target/brain_ui_fixtures/brain_page_definition.png` | 760 x 780 | 100 KB | 255 (100% 不透明) | 角色名称、人设、目标、系统能力约束说明折行可见；底部操作栏双行显示，三按钮完整； |
| **知识页** | Tab 1 | `target/brain_ui_fixtures/brain_page_knowledge.png` | 760 x 780 | 89 KB | 255 (100% 不透明) | 业务知识说明折行无截断；两项已引用知识及 SHA256 完整呈现；导入/新建卡片布局严整； |
| **话术页** | Tab 2 | `target/brain_ui_fixtures/brain_page_templates.png` | 760 x 780 | 93 KB | 255 (100% 不透明) | 通用话术模板说明折行无截断；欢迎语、信息不足、格式不支持、文件超限输入框完整对齐； |
| **版本管理页** | Tab 4 | `target/brain_ui_fixtures/brain_page_version.png` | 760 x 780 | 90 KB | 255 (100% 不透明) | CAS 与原子激活/回退说明折行无截断；版本与代次规则列表清晰，回退版本按钮完整无切边； |
| **合成测试页** | Tab 5 | `target/brain_ui_fixtures/brain_page_preview.png` | 760 x 780 | 101 KB | 255 (100% 不透明) | 标明【未调用模型】；展示模拟客户消息与组装快照预览结果； |
| **会话追溯页** | 会话页 | `target/brain_ui_fixtures/sessions_page.png` | 740 x 760 | 57 KB | 255 (100% 不透明) | 会话列表与轮次追溯区展示 turn_id、角色修订、模型引用、幂等键与独立送达状态未知标签； |

---

## 3. 测试套件与契约执行证据

### 3.1 Rust 控制投影单元测试套件
- **测试命令**：`cargo test --bin wechat-mcp core::control::tests::test_control_brain_list`
- **实际退出码**：**0**（PASS，耗时 0.06s）
- **覆盖用例（4项专项测试）**：
  1. `test_control_brain_list_allowlist_contacts_projection`：无 DB 时输出 `history_store_unavailable`，正常时严格过滤跨账号与未授权记录；
  2. `test_control_brain_list_projection_missing_column_rejected`：contacts 表缺少 `display_name` 列但 `BrainAccess::load` 能够加载成功，投影查询被严格拒绝，输出 `unavailable` + `access_resolution_failed` + `[]`，证明绝不吞成空 ready；
  3. `test_control_brain_list_projection_invalid_kind_or_null_rejected`：某行 kind 为未知非法字符串（如 `channel`）或字段为 NULL 时，投影查询被拒绝并丢弃全部部分结果，输出 `unavailable` + `access_resolution_failed` + `[]`；
  4. `test_control_brain_list_projection_group_private_and_revocation`：私聊和群聊的正确识别与映射；白名单撤销后输出 `ready` + `error: null` + `[]`（已成功计算但无可授权项）。

### 3.2 Swift 隔离契约与 UI 自检测试套件
- **测试命令**：`cargo test --test swift_brain_ui`
- **实际退出码**：**0**（PASS，耗时 22.04s）
- **测试断言统计**：**全部 102 项断言 100% 通过**（Total passed: 102, failed: 0）。
- **核心断言覆盖**：
  - `ui_bindings_unavailable_add_disabled`：白名单 unavailable 时禁用新增按钮；
  - `ui_bindings_unavailable_update_disabled`：白名单 unavailable 时禁用更新按钮；
  - `ui_bindings_unavailable_delete_enabled`：白名单 unavailable 且存在覆盖时启用删除按钮；
  - `ui_bindings_unavailable_safe_delete`：触发真实 `deleteOverrideDraft` 安全删除选定覆盖；
  - `ui_bindings_unavailable_delete_disabled_after_all_deleted`：全部覆盖删除后自动置灰删除按钮；
  - `ui_bindings_inflight_preserved`：真实触发 `handleSaveBindingsDraft`，在异步在途期间产生新草稿输入与令牌自增，旧回调返回后断言新草稿未被冲刷覆写；
  - `real_control_list_allowlist_status`：真实控制通道白名单投影状态为 ready；
  - `real_control_list_allowlist_id_exact`：真实控制通道联系人 ID 精确为 `c_contract`；
  - `real_control_list_allowlist_id_ne_name`：验证 `contact_id != displayName`，绝不凭姓名构造 ID；
  - `real_control_reject_unauthorized_override`：保存非法联系人（`illegal_contact_999`）时控制命令严格拒绝报错。

### 3.3 宿主回归测试与静态代码规范
- **宿主回归测试**：
  - 命令：`cargo test --test swift_host`
  - 退出码：**0**（PASS，74 passed; 0 failed，耗时 19.76s）
- **代码格式化检查**：
  - 命令：`cargo fmt --all -- --check`
  - 退出码：**0**
- **零 Python 依赖检查**：
  - 命令：`bash scripts/check_no_python.sh`
  - 退出码：**0**

---

## 4. 修改文件清单

- **`src/core/control.rs`**：
  - 实现独立的 `compute_allowlist_contacts_projection` 函数，针对 prepare/query/row.get 及非法 kind 全面杜绝吞错，任何错误严格返回 `unavailable` + `[]` + `access_resolution_failed`；
  - 加固 `ControlContactAccess::permits` 在 `brain_access.is_none()` 时返回 `false`；
  - 补充 4 项真实控制单元测试（缺列、非法 kind、NULL 字段、群聊/私聊与白名单撤销）。
- **`native/bootstrap/BrainPage.swift`**：
  - 增加测试辅助接口（`isAddOverrideButtonEnabled`、`isUpdateOverrideButtonEnabled`、`isDeleteOverrideButtonEnabled`、`bumpSaveTokenForTest` 等）；
  - 在 `handleSaveBindingsDraft` 的主线程成功/失败分支中支持 `onSaveBindingsFinished` 完成回调；
  - 在 `updateOverridesListUI` 中同步联动更新 `deleteOverrideButton.isEnabled`。
- **`tests/host_brain_selftest/main.swift`**：
  - 补充 Section 12.4.5 白名单 unavailable 状态下真实控件状态与安全删除断言；
  - 补充 Section 12.4.6 真实触发 `handleSaveBindingsDraft` 的在途 CAS 保护测试断言。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 编写 V6 技术审查报告，记录各项缺陷修复、测试精确断言数与实际退出码。

---

## 5. 风险与限制说明

1. **真实环境安全隔离**：所有测试与渲染均在离屏与隔离 SQLite/JSON fixture 中完成，未触碰真实生产环境，未产生微信模型交互或外网通信；
2. **提交性质**：本次交付代表开发工程师根据协调者对核心读取零吞错、kind 严格限制及在途 CAS 保护的核查意见完成代码与测试整改，提交本地技术审查，非自行宣布整个项目验收通过。
