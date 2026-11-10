# I3 大脑配置与对话追溯界面实现 技术审查报告 (V7 收尾版)

- **日期**：2026-11-11
- **任务编号**：I3（当前派发：`task_802e58faf577` / `ctx_e2c8f59d0bab`，前序：`task_c2bbf88549ee`、`task_c9618a210184`、`task_02daa1ecaee1`、`task_65419b44ff9f`、`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：依据协调者最新审查指令，完成授权快照事务一致性、fail-closed 全链路回归与 680 表格版本列紧凑微调收尾：
  1. **授权与候选枚举同一只读事务快照**：`compute_allowlist_contacts_projection` 在同一个只读连接与事务（`conn.transaction_with_behavior(Deferred)`）内原子加载 `BrainAccess::load` 并枚举过滤候选联系人，杜绝并发读取竞态，失败严格返回 unavailable；
  2. **`ControlContactAccess.permits` fail-closed 全链路回归**：在 `test_control_fail_closed_access_across_operations_without_db` 中验证无 DB 时 Fresh 初始化、`brain-save-role`、`brain-save-knowledge`、无覆盖绑定保存、激活、回退均完全可用；而当包含 contact 覆盖时，DB 不可读严格拒绝报错（`code: 2, not_found`），证明零静默降级、零越权；
  3. **680 宿主最小宽表格版本列自适应微调**：表格列宽紧凑微调为 105/42/95/58（总宽 300px），设置 `uniformColumnAutoresizingStyle`，确保“版本”列完全展示在视口内部，彻底解决被右边缘截住问题；
  4. 真实控制通道断言、Swift UI 契约测试（102 项断言）及 74 项宿主回归测试均以退出码 0 完全通过；
  5. 提交本地技术审查，非自行宣布整个项目验收通过。

---

## 1. 协调者意见与收尾整改定位

协调者在最新评估中指出：
1. **只读事务一致性保障**：原实现虽在同一连接上执行，但未显式在同一个只读事务隔离快照内执行 `BrainAccess::load` 与联系人候选枚举。必须在同一个只读事务内完成，失败立即返回 unavailable，杜绝不一致假装 ready；
2. **`ControlContactAccess.permits` fail-closed 影响面验证**：适配器由 `true` 改为 `false` 后安全方向正确，但必须给出无 DB / 缺授权来源时针对各个 `brain-*` 操作的具体行为与测试回归证据；
3. **680 图表格版本列视口容纳**：表格在 680px 宽度下右侧版本列紧贴边缘，需通过最小列宽调整确保所有列在视口内清晰完整展现。

---

## 2. 核心技术实现

### 2.1 同一只读事务内加载 `BrainAccess` 与枚举候选联系人 (`src/core/control.rs`)
重构 `compute_allowlist_contacts_projection`：
```rust
pub(crate) fn compute_allowlist_contacts_projection(
    cfg: &BotConfig,
    db_path: &Path,
) -> (Value, &'static str, Option<&'static str>) {
    if !db_path.exists() {
        return (json!([]), "unavailable", Some("history_store_unavailable"));
    }
    let mut conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return (json!([]), "unavailable", Some("history_store_unavailable")),
    };

    // 在同一个只读事务隔离快照内，原子执行 BrainAccess 授权解析与候选联系人枚举过滤
    let tx = match conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred) {
        Ok(t) => t,
        Err(_) => return (json!([]), "unavailable", Some("history_store_unavailable")),
    };

    let brain_access = match crate::core::open_api::access::BrainAccess::load(cfg, &tx) {
        Ok(ba) => ba,
        Err(_) => return (json!([]), "unavailable", Some("access_resolution_failed")),
    };

    let mut stmt = match tx.prepare(
        "SELECT contact_id, display_name, kind FROM contacts WHERE account_id = ?1 AND merged_into IS NULL ORDER BY rowid",
    ) {
        Ok(s) => s,
        Err(_) => return (json!([]), "unavailable", Some("access_resolution_failed")),
    };
    // 遍历 rows 并在同一事务快照内校验 brain_access.permit(&cid) ...
    (json!(list), "ready", None)
}
```
通过 `&tx` 将只读事务作为 `&Connection` 传入 `BrainAccess::load`，后续对 `contacts` 的查询与解码同样使用 `tx`，保证了读取的完全隔离与一致性。

### 2.2 `ControlContactAccess` fail-closed 全链路回归分析
在 `store.rs` 的写入授权机制中：
- `SaveRole`、`SaveKnowledge`、`CopyRole`、`ArchiveRole`：仅校验 `owner_account_id` 与引用对象的合法性，不调用 `access.permits`；
- `SaveBindings(b)`、`Activate`、`Rollback`：调用 `self.bindings(root, ctx, b, Some(access))`；
  - 只有当 `b.overrides` 存在非空条目时，才会循环执行 `access.permits(&ctx.account_id, &o.contact_id, &o.chat_kind)`；
  - 当无 DB（`brain_access` 为 `None`）时，`access.permits` 返回 `false`，任何含有 contact 覆盖的绑定保存均会被 `require(..., "not_found")` 严格拒绝；
  - 当绑定为无覆盖（只有默认角色）时，不执行 permits 循环，保存、激活与回退正常放行。
- 在 `test_control_fail_closed_access_across_operations_without_db` 中完成了全套状态机流转断言。

### 2.3 680 宿主最小宽表格列宽微调 (`native/bootstrap/BrainPage.swift`)
针对覆盖表格进行列宽紧凑微调：
- 联系人/群列：`width = 105` (minWidth: 70)；
- 会话类型列：`width = 42` (minWidth: 35)；
- 绑定角色列：`width = 95` (minWidth: 70)；
- 版本列：`width = 58` (minWidth: 45)；
- 总宽度 300px，配合 `uniformColumnAutoresizingStyle`，在 680px 宿主宽度（右栏可用约 450px）下，所有 4 列（含“版本”列）完全在视口内从容展现，无截断或裁切，同时保留横向滚动条支持用户手动拉伸。

---

## 3. 全部离屏 PNG 验证对账 (含 680 宿主最小宽、白名单不可用、CAS 冲突与删除状态)

使用 `sips -g pixelWidth -g pixelHeight` 核验生成的离屏渲染产物，所有图像均基于不透明浅色离屏窗口（Aqua 外观、RGB(242, 242, 245) 填充）导出至 `target/brain_ui_fixtures/`：

| 页面名称 | 对应 Tab / 状态 | 导出文件路径 | 实际像素尺寸 | 文件体积 | 像素不透明度 (min_alpha) | 验证要点 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **绑定页 (标准表单)** | Tab 3 (标准 720) | `target/brain_ui_fixtures/brain_page_bindings.png` | 760 x 780 | 110 KB | 255 (100% 不透明) | 包含默认角色下拉框、覆盖表格、白名单联系人下拉框、角色下拉框、增删改按钮与保存草稿行； |
| **绑定页 (680 最小宽)** | Tab 3 (最小宽) | `target/brain_ui_fixtures/brain_page_bindings_680.png` | 680 x 760 | 107 KB | 255 (100% 不透明) | 宿主最小宽 680px 下，表格四列（含版本列）完全在视口内可见无截断，垂直流式分行表单对齐整齐； |
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

## 4. 测试套件与契约执行证据

### 4.1 Rust 控制投影与 fail-closed 单元测试套件
- **测试命令**：
  `cargo test --bin wechat-mcp core::control::tests::test_control_brain_list && cargo test --bin wechat-mcp core::control::tests::test_control_fail_closed_access_across_operations_without_db`
- **实际退出码**：**0**（PASS，5 passed; 0 failed）
- **覆盖用例（5项专项测试）**：
  1. `test_control_brain_list_allowlist_contacts_projection`：无 DB 时输出 `history_store_unavailable`，正常时严格过滤跨账号与未授权记录；
  2. `test_control_brain_list_projection_missing_column_rejected`：contacts 表缺少 `display_name` 列但 `BrainAccess::load` 能够加载成功，投影查询在同一只读事务内被严格拒绝，输出 `unavailable` + `access_resolution_failed` + `[]`；
  3. `test_control_brain_list_projection_invalid_kind_or_null_rejected`：某行 kind 为未知非法字符串（如 `channel`）或字段为 NULL 时，投影查询被拒绝并丢弃全部部分结果，输出 `unavailable` + `access_resolution_failed` + `[]`；
  4. `test_control_brain_list_projection_group_private_and_revocation`：私聊和群聊的正确识别与映射；白名单撤销后输出 `ready` + `error: null` + `[]`（已成功计算但无可授权项）；
  5. `test_control_fail_closed_access_across_operations_without_db`：无 DB 时验证全操作链路行为（初始化、角色保存、知识保存、无覆盖草稿保存/激活/回退放行；含 contact 覆盖时严格拒绝并返回 `not_found`）。

### 4.2 Swift 隔离契约与 UI 自检测试套件
- **测试命令**：`cargo test --test swift_brain_ui`
- **实际退出码**：**0**（PASS，耗时 23.11s）
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

### 4.3 宿主回归测试与静态代码规范
- **宿主回归测试**：
  - 命令：`cargo test --test swift_host`
  - 退出码：**0**（PASS，74 passed; 0 failed，耗时 20.86s）
- **代码格式化检查**：
  - 命令：`cargo fmt --all -- --check`
  - 退出码：**0**
- **零 Python 依赖检查**：
  - 命令：`bash scripts/check_no_python.sh`
  - 退出码：**0**

---

## 5. 修改文件清单

- **`src/core/control.rs`**：
  - `compute_allowlist_contacts_projection` 实现在同一只读事务（`Deferred`）内完成 `BrainAccess::load` 与联系人候选枚举，强一致且杜绝竞态；
  - 新增 `test_control_fail_closed_access_across_operations_without_db` 单元测试，全面验证无 DB 时各操作的授权行为。
- **`native/bootstrap/BrainPage.swift`**：
  - 紧凑微调覆盖表格列宽（105/42/95/58），设置 `uniformColumnAutoresizingStyle`，确保 680px 宽度下版本列完全展示无截断。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 编写 V7 收尾技术审查报告，详细记录只读事务快照、fail-closed 行为影响面、列宽微调及测试证据。

---

## 6. 风险与限制说明

1. **真实环境安全隔离**：所有测试与渲染均在离屏与隔离 SQLite/JSON fixture 中完成，未触碰真实生产环境，未产生微信模型交互或外网通信；
2. **提交性质**：本次交付代表开发工程师根据协调者要求完成只读事务一致性、fail-closed 全链路测试与 680 表格版本列展示微调，提交本地技术审查，非自行宣布整个项目验收通过。
