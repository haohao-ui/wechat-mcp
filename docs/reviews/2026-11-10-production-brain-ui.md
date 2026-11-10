# I3 大脑配置与对话追溯界面实现 技术审查报告 (V5 修订版)

- **日期**：2026-11-11
- **任务编号**：I3（当前派发：`task_c9618a210184` / `ctx_af9c2f2cfad1`，前序：`task_02daa1ecaee1`、`task_65419b44ff9f`、`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：采纳协调者批准的“方案一”，在 Rust control 与 Swift 端彻底建立白名单权威只读投影与 680px 流式分行排布：
  1. 在 `src/core/control.rs` 的 `brain-list` 输出中增加权威只读投影 `allowlist_contacts`、`allowlist_contacts_status`（ready/unavailable）、`allowlist_contacts_error`，严格以核心 `BrainAccess` 真实授权为唯一来源过滤联系人，绝不凭姓名构造 ID，绝不扩容 allowlist，杜绝在 Swift 伪造 ID 或拿已存在 override 反向扩权；
  2. 白名单 `unavailable` 时，Swift 绑定页新增/编辑控件自动禁用并给出明确错误原因，已有覆盖仅可展示和安全删除；
  3. 绑定页 680px 宿主最小宽改受限布局流式分行（联系人一行、角色一行、按钮一行），表格横向滚动/紧凑列宽，确保 680px 无截断裁切；
  4. CAS 保存期间增加自增令牌与在途编辑防护，旧异步回调绝不覆写新草稿；
  5. 契约测试覆盖姓名与 ID 不等、群/私聊、跨账号隔离、白名单撤销、坏 DB、旧 shape 缺字段兼容等场景；导出 680/720 离屏 PNG；
  6. 真实控制契约与宿主测试全通；提交本地审查，非整个项目验收通过。

---

## 1. 协调者核复与“方案一”设计原则

针对白名单授权真相与 680px 最小宽布局，协调者复核（邮件 `msg_856a0e72c114`）明确要求采纳“方案一”：
1. **授权真相源在 Rust 核心，不在 UI**：
   - 微信体系中 `display_name`（或配置中的 `recipient`）仅是用户可见文本，并非稳定 `contact_id`；
   - 客户端决不能凭 `recipient` 直接构造或冒充 `contact_id`，也决不能拿已有 override 反向扩充白名单列表；
   - 核心授权通过 `BrainAccess::load` 结合 `contacts` 与 `contact_names` 完成解析，在 `brain-list` 提供权威只读投影。
2. **白名单不可用时的安全退避**：
   - 若 `history.sqlite3` 不可用或解析失败，`brain-list` 明确返回 `status: "unavailable"` 及错误原因，UI 禁用新增/编辑，绝不静默造假 ID。
3. **680px 宿主最小宽流式分行**：
   - 放弃单行挤占的水平排布，表单字段改为联系人一行、角色一行、按钮一行的垂直流式结构，彻底消除 680px 界面右侧溢出或裁切。
4. **并发在途草稿防护**：
   - CAS 保存期间采用自增令牌（`currentSaveToken`），在异步请求返回前用户若有新草稿修改，旧请求回调绝不覆盖新草稿。

---

## 2. 核心整改落地与技术实现

### 2.1 Rust 控制命令新增权威只读投影 (`src/core/control.rs`)
在 `execute_brain_list` 中增加投影计算逻辑：
- 严格限定当前配置的 `account_id`；
- 打开 `state_dir/history/history.sqlite3`，调用 `BrainAccess::load` 解析出当前账号全部合法授权；
- 查询 `contacts` 表，过滤未合并（`merged_into IS NULL`）、匹配当前账号且经 `access.permits(contact_id, kind)` 校验通过的记录；
- 投影输出：
  - `allowlist_contacts`: `[{"contact_id": "...", "display_name": "...", "chat_kind": "private"|"group"}]`
  - `allowlist_contacts_status`: `"ready"` 或 `"unavailable"`
  - `allowlist_contacts_error`: `null`、`"history_store_unavailable"` 或 `"access_resolution_failed"`

### 2.2 Swift 绑定页与草稿防覆写 (`native/bootstrap/BrainPage.swift`)
- **权威投影解析与旧 shape 兼容**：
  - `BrainListPayload` 新增字段解码，若控制命令为旧版本缺省上述字段，优雅回退为 `allowlistContacts: []`、`allowlistContactsStatus: "unavailable"`，杜绝崩溃；
- **彻底移除 UI 自行造 ID 逻辑**：
  - 删除 `reloadAllowlistContacts` 中凭 `recipient` 造 ID 及拿已有 override 扩权限的逻辑，完全以核心 payload 授权结果为准；
- **白名单不可用状态保护**：
  - 当 `allowlistContactsStatus == "unavailable"` 时，禁用添加/更新按钮及联系人下拉框，以标红文本提示错误详情（如历史库不可用），已有覆盖仍支持安全删除；
- **680px 受限宽度流式分行排布**：
  - 表单操作区改为三行垂直排布：
    - 行 1：联系人/群标签（90px） + 联系人下拉框（填满行宽）；
    - 行 2：绑定角色标签（90px） + 角色下拉框（填满行宽）；
    - 行 3：添加覆盖按钮 + 更新选定按钮 + 删除选定按钮；
  - 表格设置紧凑列宽（130/50/120/60），并允许横向滚动，彻底杜绝 680px 视口裁切；
- **自增令牌在途草稿防护**：
  - `handleSaveBindingsDraft` 分配自增 `token`，并在异步回调返回时比对 `token == self.currentSaveToken`，若用户已输入新草稿则拒绝覆盖。

---

## 3. 全部离屏 PNG 验证对账 (含 680 宿主最小宽、白名单不可用、CAS 冲突与删除状态)

所有图像均基于不透明浅色离屏窗口（Aqua 外观、RGB(242, 242, 245) 填充）导出至 `target/brain_ui_fixtures/`：

| 页面名称 | 对应 Tab / 状态 | 导出文件路径 | 实际像素尺寸 | 文件体积 | 像素不透明度 (min_alpha) | 验证要点 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **绑定页 (标准表单)** | Tab 3 (标准 720) | `target/brain_ui_fixtures/brain_page_bindings.png` | 760 x 780 | 110 KB | 255 (100% 不透明) | 包含默认角色下拉框、覆盖表格、白名单联系人下拉框、角色下拉框、增删改按钮与保存草稿行； |
| **绑定页 (680 最小宽)** | Tab 3 (最小宽) | `target/brain_ui_fixtures/brain_page_bindings_680.png` | 680 x 760 | 108 KB | 255 (100% 不透明) | 验证宿主最小宽 680px 下，垂直流式分行表单各行对齐良好，右侧无截断与裁切； |
| **绑定页 (不可用状态)** | Tab 3 (不可用) | `target/brain_ui_fixtures/brain_page_bindings_unavailable_680.png` | 680 x 760 | 105 KB | 255 (100% 不透明) | 验证白名单 unavailable 时，680px 下新增/编辑按钮置灰，展示显式错误提示； |
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

### 4.1 Rust 控制投影单元测试
- **测试用例**：`core::control::tests::test_control_brain_list_allowlist_contacts_projection`
- **验证场景**：
  1. 无 `history.sqlite3` 时输出 `status: "unavailable", error: "history_store_unavailable", contacts: []`；
  2. 正常数据库初始化后，向 `contacts` 插入当前账号私聊联系人 `c_real_1`（姓名 `张三`）、未授权联系人 `c_unauth`（姓名 `李四`）、跨账号联系人 `c_other`（姓名 `王五`）；
  3. `brain-list` 权威投影仅包含当前账号且在白名单中的 `c_real_1`，跨账号与未授权记录被严格排除；
  4. 验证 `contact_id != display_name`，绝不凭姓名构造 ID。
- **实际执行结果**：
  - 命令：`cargo test --bin wechat-mcp core::control::tests::test_control_brain_list_allowlist_contacts_projection`
  - 退出码：**0**（PASS，耗时 0.09s）

### 4.2 Swift 隔离契约与 UI 自检测试
- **测试文件**：`tests/swift_brain_ui.rs` + `tests/host_brain_selftest/main.swift`
- **实际执行结果**：
  - 命令：`cargo test --test swift_brain_ui`
  - 退出码：**0**（PASS，耗时 20.43s）
  - 测试断言覆盖：
    - 旧 shape 缺字段兼容解码断言通过；
    - 新 shape 权威投影与错误状态处理断言通过；
    - 绑定表单纯逻辑增删改查断言通过；
    - 680px 宿主最小宽离屏渲染断言通过；
    - 真实控制通道白名单投影 `c_contract` 且 `contact_id != displayName` 断言通过；
    - 非法 contact_id 保存严格拦截断言通过；
    - CAS 冲突及未提交草稿保留断言通过。

### 4.3 宿主回归测试与静态代码规范
- **宿主回归测试**：
  - 命令：`cargo test --test swift_host`
  - 退出码：**0**（PASS，74 passed; 0 failed，耗时 18.36s）
- **代码格式化检查**：
  - 命令：`cargo fmt --all -- --check`
  - 退出码：**0**
- **零 Python 依赖检查**：
  - 命令：`bash scripts/check_no_python.sh`
  - 退出码：**0**

---

## 5. 修改文件清单

- **`src/core/control.rs`**：
  - `execute_brain_list` 中实现 `allowlist_contacts`、`allowlist_contacts_status` 与 `allowlist_contacts_error` 权威只读投影；
  - 补充 `test_control_brain_list_allowlist_contacts_projection` 单元测试。
- **`native/bootstrap/BrainPage.swift`**：
  - `BrainListPayload` 补充只读投影解码与旧格式缺字段兜底；
  - 彻底移除凭名字伪造 ID 或拿已存在 override 扩充白名单的危险行为；
  - 实现白名单不可用状态下禁用表单与错误提示；
  - 实现 680px 受限宽度垂直流式分行表单；
  - 实现自增令牌异步在途草稿防护。
- **`tests/host_brain_selftest/main.swift`**：
  - 补充 Section 2 旧 shape 缺字段兼容断言；
  - 补充 Section 12.4 最小宽不可用状态离屏 PNG 导出；
  - 补充 Section 13.2 真实控制命令白名单只读投影解析与姓名 ID 不等校验断言。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 更新 V5 修订审查报告，记录方案一实现、测试退出码与产物。

---

## 6. 风险与限制说明

1. **真实环境安全隔离**：所有测试与渲染均在离屏与隔离 SQLite/JSON fixture 中完成，未触碰真实生产环境，未产生微信模型交互或外网通信；
2. **提交性质**：本次交付代表开发工程师根据协调者对“方案一”的核准指示，完成核心只读投影与 680px 绑定表单流式排布整改，提交本地技术审查，非自行宣布整个项目验收通过。
