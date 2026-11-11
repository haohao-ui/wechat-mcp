# I3 大脑配置与对话追溯界面实现 技术审查报告 (V8 核心回归修复版)

- **日期**：2026-11-11
- **任务编号**：I3（当前派发：`task_2c2e54da855a` / `ctx_ad89d9c4e6b4`，前序：`task_802e58faf577`、`task_c2bbf88549ee`、`task_c9618a210184`、`task_02daa1ecaee1`、`task_65419b44ff9f`、`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`70d02e5`（前序集成分支：`56d34d0` / `e585431`）
- **交付分支**：`work/prod-brain-ui`
- **状态**：依据协调者最新审查指令（msg_427475814813），修复 Q1 集成测试中暴露的 2 项核心测试失败：
  1. **生产代码零修改**：`Store::resolve`、`ControlContactAccess None => false`、运行时白名单授权算法一律未变，绝不为了让测试通过而恢复 `None => true` 或因 `defaultRef` 绕过 contact 授权；
  2. **`test_brain_initialize_control` 最小测试 Fixture 修复**：使用 `HistoryStore::open` 运行完整迁移生成真实 SQLite 数据库（严格限制临时目录权限 0700/0600，绝不触碰真实 HOME）；配置 `recipient` 姓名（`"客户张三"`）与 `contact_id`（`"c_legacy_cust"`）严格不同；通过真实 `BrainAccess` 验证默认角色解析，并追加未知联系人（`c_unknown`）、未授权联系人（`c_unauth`）、跨账号联系人（`c_other_acc`）及跨账号 Context 的严格拒绝断言；
  3. **`test_turn_model_retry_pins_initial_snapshot_version` 彻底移除 `DummyAccess`**：补齐隔离完整迁移 history 数据库，通过真实 `ControlContactAccess` 进行所有 `store.write` 与 `store.resolve` 调用；验证第一次真实 cycle 锁定初始快照 v1，管理员写入并激活 v2 后，第二次真实 cycle 重试依然锁定 v1 且两次快照的 `snapshot_id` 完全一致；
  4. **全量单线程核心回归验证**：除既有预期失败 `core::open_api::compat_tests::ops_tools_list_unchanged` 外，全部 960 项核心单元测试、Swift UI 契约测试（102 项断言）及 74 项宿主回归测试均以退出码 0 完全通过；
  5. 提交独立 commit 供 Q1 团队单独 cherry-pick，非自行宣布整个项目验收通过。

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

### 4.1 Q1 核心回归测试修复（本次修复重点）
- **`core::control::tests::test_brain_initialize_control`**：
  - 测试命令：`cargo test --bin wechat-mcp core::control::tests::test_brain_initialize_control`
  - 退出码：**0**（PASS，1 passed; 0 failed）
  - 验证逻辑：使用真实 `HistoryStore::open` 迁移 DB 填入白名单联系人（recipient 姓名 `"客户张三"` 与 ID `"c_legacy_cust"` 严格不同）；通过真实 `BrainAccess` 成功解析默认角色（`"原内置客服"`）；追加未知（`c_unknown`）、未授权（`c_unauth`）、跨账号（`c_other_acc`）及跨账号 Context 的 4 项强制拒绝断言。
- **`core::turns::tests::test_turn_model_retry_pins_initial_snapshot_version`**：
  - 测试命令：`cargo test --bin wechat-mcp core::turns::tests::test_turn_model_retry_pins_initial_snapshot_version`
  - 退出码：**0**（PASS，1 passed; 0 failed）
  - 验证逻辑：彻底移除 `DummyAccess`；为 `fixture_config` 初始化隔离 `history.sqlite3` 数据库，使用真实 `ControlContactAccess` 证明权限并驱动生产 `svc.cycle`；验证首次 cycle 失败锁定初始快照 v1，管理员写入并激活 v2 后，第二次 cycle 重试依然锁定 v1 且两次快照的 `snapshot_id` 完全一致。

### 4.2 控制投影与 fail-closed 专项测试套件
- **测试命令**：
  `cargo test --bin wechat-mcp core::control::tests::test_control_brain_list && cargo test --bin wechat-mcp core::control::tests::test_control_fail_closed_access_across_operations_without_db`
- **实际退出码**：**0**（PASS，5 passed; 0 failed）
- **覆盖用例（5项专项测试）**：
  1. `test_control_brain_list_allowlist_contacts_projection`：无 DB 时输出 `history_store_unavailable`，正常时严格过滤跨账号与未授权记录；
  2. `test_control_brain_list_projection_missing_column_rejected`：contacts 表缺少 `display_name` 列但 `BrainAccess::load` 能够加载成功，投影查询在同一只读事务内被严格拒绝，输出 `unavailable` + `access_resolution_failed` + `[]`；
  3. `test_control_brain_list_projection_invalid_kind_or_null_rejected`：某行 kind 为未知非法字符串（如 `channel`）或字段为 NULL 时，投影查询被拒绝并丢弃全部部分结果，输出 `unavailable` + `access_resolution_failed` + `[]`；
  4. `test_control_brain_list_projection_group_private_and_revocation`：私聊和群聊的正确识别与映射；白名单撤销后输出 `ready` + `error: null` + `[]`（已成功计算但无可授权项）；
  5. `test_control_fail_closed_access_across_operations_without_db`：无 DB 时验证全操作链路行为（初始化、角色保存、知识保存、无覆盖草稿保存/激活/回退放行；含 contact 覆盖时严格拒绝并返回 `not_found`）。

### 4.3 Swift 隔离契约与 UI 自检测试套件
- **测试命令**：`cargo test --test swift_brain_ui`
- **实际退出码**：**0**（PASS，耗时 23.58s）
- **测试断言统计**：**全部 102 项断言 100% 通过**（Total passed: 102, failed: 0）。

### 4.4 宿主回归测试与静态代码规范
- **宿主回归测试**：
  - 命令：`cargo test --test swift_host`
  - 退出码：**0**（PASS，74 passed; 0 failed，耗时 18.35s）
- **单线程全核心回归测试**：
  - 命令：`cargo test --bin wechat-mcp -- --test-threads=1`
  - 结果：**960 passed; 1 failed**（耗时 67.21s，唯一失败为协调者确认的既有预期失败 `core::open_api::compat_tests::ops_tools_list_unchanged`）
- **代码格式化检查**：
  - 命令：`cargo fmt --all -- --check`
  - 退出码：**0**
- **零 Python 依赖检查**：
  - 命令：`bash scripts/check_no_python.sh`
  - 退出码：**0**

---

## 5. 修改文件清单

- **`src/core/control.rs`**：
  - `test_brain_initialize_control`：使用完整迁移的隔离 history 数据库与真实 `BrainAccess` 验证默认角色解析，追加未知、未授权、跨账号联系人及跨账号上下文的严格拒绝断言。
- **`src/core/turns.rs`**：
  - `test_turn_model_retry_pins_initial_snapshot_version`：彻底移除 `DummyAccess`，通过真实迁移的隔离 history 数据库与真实 `ControlContactAccess` 验证重试快照锁定 v1 与原子激活隔离。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 编写 V8 技术审查报告，记录精确测试退出码、断言数与回归验证证据。

---

## 6. 风险与限制说明

1. **真实环境安全隔离**：所有测试与渲染均在离屏与隔离 SQLite/JSON fixture 中完成，临时目录权限严格 0700/0600，未触碰真实生产环境，未产生微信模型交互或外网通信；
2. **提交性质**：本次交付代表开发工程师根据协调者最新指令完成 Q1 集成测试中两项核心测试失败的最小 fixture 修复，全核心单线程回归仅保留既有 ops_tools 失败，提交独立 commit 供 Q1 cherry-pick，非自行宣布整个项目验收通过。
