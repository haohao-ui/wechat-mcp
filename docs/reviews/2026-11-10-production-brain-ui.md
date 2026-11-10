# I3 大脑配置与对话追溯界面实现 技术审查报告 (V3 整改版)

- **日期**：2026-11-10
- **任务编号**：I3（当前派发：`task_65419b44ff9f` / `ctx_61cd54745669`，前序：`task_23a5854df5b2`、`task_86a24d0cb23c`、`task_48f9bde2b511`）
- **基线 Commit**：`d9dbea0`
- **交付分支**：`work/prod-brain-ui`
- **状态**：撤回前次关于“无裁切/无截断”及“已完整呈现数据”的错误陈述；全面重构底部操作栏为弹性双行布局，彻底解决右侧按钮与长说明截断；真实装配绑定页下拉框与联系人覆盖；修复 SessionsPage fixture 异步时序并真实呈现会话与结构化轮次追溯（含独立送达未知标签）；覆盖并验证全部 6 个大脑 Tab 与会话页；真实 Rust 控制契约与宿主测试全通；提交本地审查，非整个项目验收通过。

---

## 1. 错误陈述正式撤回与缺陷核查

在上一轮交付中声称的“无裁切”、“无截断”以及“会话页已完整呈现数据”**在此正式撤回**。经实测核实，前次产物存在以下严重缺陷：

1. **右下角操作栏截断与溢出**：
   - 原 `bottomStack` 采用单行水平排列，在 760px 宽度窗口（右侧容器实际宽度约 540px）下，长状态文本（如“角色: VIP 售后专属顾问 · 草稿: rev_v2 · 运行: rev_v1”）加上 3 个操作按钮总宽超过 540px，导致最右侧的“回退版本”按钮被挤出可视区域右边缘裁切；
2. **长说明单行溢出裁切**：
   - 知识页与版本页的说明文本虽然使用了 `wrappingLabelWithString`，但在 NSStackView 中缺乏受限宽度约束与 `preferredMaxLayoutWidth`，导致天然计算为单行展示，右端超出右边缘截断；
3. **绑定页数据未渲染**：
   - 绑定页默认角色下拉框（NSPopUpButton）未实现与 `payload.roles` 的接线填充，仅显示为空白箭头；特定联系人覆盖区域未格式化呈现；
4. **SessionsPage 会话与追溯为空**：
   - fixture 在初始化控制器时，先执行了 `applyFixtureState`，但随后首次访问 `controller.view` 时触发了 AppKit 的 `viewDidLoad()`，其内部的 `setupUI()` 与 `refreshSessions()` 将刚刚注入的会话数据与详情文本重新冲刷成“暂无活跃会话”与“暂无选中轮次”初始空提示。

---

## 2. 核心整改落地与技术设计

### 2.1 底部操作栏弹性双行布局（彻底消除截断）
- 将 `bottomBar` 重构为上下两行垂直 Stack 布局：
  - **上行**：`revisionStatusLabel`，采用 `wrappingLabelWithString`，`maximumNumberOfLines = 0`，`lineBreakMode = .byWordWrapping`，`preferredMaxLayoutWidth = 500`，字号 11，靠左自适应换行；
  - **下行**：`actionRow` 水平排列 3 个核心按钮 `[saveDraftButton, activateButton, rollbackButton]`，按钮总宽度约 260px，在 540px 的容器中留有近 280px 的充足余量，即使在 720px~760px 的最小宿主窗口下，“保存草稿”、“启用配置”、“回退版本”三个按钮均 100% 完整可见，绝不截断。

### 2.2 全局说明文本受限换行与弹性约束
- 知识页、话术页、绑定页、版本页、合成测试页与定义页中的说明标签，统一配置：
  - `maximumNumberOfLines = 0`
  - `lineBreakMode = .byWordWrapping`
  - `preferredMaxLayoutWidth = 460 ~ 480`
  - 显式约束 `widthAnchor.constraint(equalToConstant: 480)` 或设置最大宽度约束，文本在达到容器边缘前自然优雅折行，彻底消除单行文字超出右边缘的裁切。

### 2.3 绑定页数据绑定与覆盖呈现
- 实现 `updateBindingsUI()`，在数据拉取与 fixture 状态应用时自动触发：
  - 清空并根据 `payload.roles` 添加所有角色选项，例如 `VIP 售后专属顾问 (role_vip_service)`、`售前导购助手 (role_pre_sales)`；
  - 根据 `payload.draft?.defaultRef?.roleId ?? payload.active?.defaultRef?.roleId` 自动匹配并选中当前默认角色；
  - 将 `currentBindings.overrides` 格式化为清晰多行规则展示在 `overridesSummaryLabel` 中（如：`• 联系人: 联系人_王经理 [私聊] ➔ VIP 售后专属顾问 (role_vip_service) [版本: rev_v2]`）。

### 2.4 SessionsPage 渲染时序修复与真实 Provenance 呈现
- **时序接线修复**：在 `applyFixtureState` 开始时显式访问 `_ = self.view`，确保 `loadView` 与 `viewDidLoad` 先行完成，然后再向 `sessionsStack` 挂载会话行、向 `detailTextView` 写入轮次详情，并在结束时调用 `self.view.layoutSubtreeIfNeeded()`，杜绝初始化覆写问题；
- **真实结构化轮次详情呈现**：
  - 呈现真实的合成轮次数据：Turn ID（`turn_contract_real_101`）、联系人（`联系人_王经理`）、角色修订（`role_vip_service`）、模型引用（`openai / gpt-4o`）、幂等键（`send_key_8891_vip`）、大脑决策（`send | 回复: 您好！已为您登记售后换货申请，专员将在2小时内与您联系。`）；
  - **独立阶段标签（恪守真实验收准则）**：包含 `模型已生成`、`已发送`、`己方回显`、`送达状态: 未知 (unknown)`。**绝不将客户端回显升级为已送达**，保持阶段标签的独立与客观性。

---

## 3. 全部 6 个大脑 Tab 与会话页离屏 PNG 验证对账

所有图像均基于不透明浅色离屏窗口（Aqua 外观、RGB(242, 242, 245) 填充）导出至 `target/brain_ui_fixtures/`，通过 Python (PIL + NumPy) 及 Tesseract OCR 严格核验：

| 页面名称 | 对应 Tab | 导出文件路径 | 实际像素尺寸 | 文件体积 | 像素不透明度 (min_alpha) | OCR 核心提取内容与验证要点 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **定义页** | Tab 0 | `target/brain_ui_fixtures/brain_page_definition.png`<br>`target/brain_ui_fixtures/brain_page.png` | 760 x 780 | 102 KB | 255 (100% 不透明) | 角色名称、人设、目标、系统能力约束说明折行可见；底部操作栏双行显示，三按钮完整； |
| **知识页** | Tab 1 | `target/brain_ui_fixtures/brain_page_knowledge.png` | 760 x 780 | 91 KB | 255 (100% 不透明) | 业务知识说明折行无截断；两项已引用知识及 SHA256 完整呈现；导入/新建卡片布局严整； |
| **话术页** | Tab 2 | `target/brain_ui_fixtures/brain_page_templates.png` | 760 x 780 | 96 KB | 255 (100% 不透明) | 通用话术模板说明折行无截断；欢迎语、信息不足、格式不支持、文件超限输入框完整对齐； |
| **绑定页** | Tab 3 | `target/brain_ui_fixtures/brain_page_bindings.png` | 760 x 780 | 78 KB | 255 (100% 不透明) | 默认角色下拉框展示选中的 `VIP 售后专属顾问 (role_vip_service)`；特定联系人覆盖展示 `联系人_王经理 [私聊] -> VIP 售后专属顾问`； |
| **版本管理页** | Tab 4 | `target/brain_ui_fixtures/brain_page_version.png` | 760 x 780 | 92 KB | 255 (100% 不透明) | CAS 与原子激活/回退说明折行无截断；版本与代次规则列表清晰，回退版本按钮完整无切边； |
| **合成测试页** | Tab 5 | `target/brain_ui_fixtures/brain_page_preview.png` | 760 x 780 | 76 KB | 255 (100% 不透明) | 标明【未调用模型】；展示模拟客户消息；组装快照摘要、字节预算、系统约束等组装预览结果完整展示； |
| **会话追溯页** | 会话页 | `target/brain_ui_fixtures/sessions_page.png` | 740 x 760 | 58 KB | 255 (100% 不透明) | 会话列表清晰可见两行（联系人_王经理 监听中、技术支持群 已转人工 [恢复自动回复]）；轮次追溯区展示 turn_id、角色修订、模型引用、幂等键与独立阶段标签（送达状态: 未知）； |

---

## 4. 测试套件与门禁执行证据

### 4.1 契约集成与 UI 离屏渲染测试（`tests/swift_brain_ui.rs`）
- **执行命令**：`cargo test --locked --test swift_brain_ui`
- **结果**：`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.53s`
- **测试涵盖**：
  - 包含 13 大组全部通过（包括 16 项真实 Rust 控制契约断言：角色新建/保存/读取、知识保存引用与 SHA、绑定/激活/回退、assemble_only 预览、真实轮次详情各字段解码与 CAS 冲突拦截）；
  - 全新加入的 6 个大脑 Tab 与会话页 PNG 导出与大小校验全部 PASS。

### 4.2 既有宿主功能回归（`tests/swift_host.rs`）
- **执行命令**：`cargo test --locked --test swift_host`
- **结果**：`test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.99s`
- **验证范围**：宿主状态监控、控制命令封装、Supervisor 生命周期、配置表单与联系人同步等 74 项宿主回归用例无一回归。

### 4.3 静态检查与发布门禁
- **`bash scripts/test-gates.sh`**：退出码 0，PASS；
- **`cargo fmt --check`**：退出码 0，PASS；
- **`cargo clippy --locked --all-targets`**：退出码 0，PASS。

---

## 5. 修改文件清单

严格遵循预定所有权范围，不改动公共接口与数据库：
- **`native/bootstrap/BrainPage.swift`**：
  - 将底部操作栏改为上下双行弹性布局（状态上行、按钮下行），根除按钮截断；
  - 知识、话术、绑定、版本、测试各 Tab 说明标签设置多行受限换行约束；
  - 实现 `updateBindingsUI()`，绑定默认角色下拉框与联系人覆盖列表；
  - 增加 `setPreviewFixture()` 供合成测试页快照输出；
  - 独立拆分 `phaseLabels`，送达状态标明未知，杜绝伪造为已送达。
- **`native/bootstrap/SessionsPage.swift`**：
  - 修复 `applyFixtureState` 时序，确保在 `_ = self.view` 加载后挂载会话行与写入轮次详情，并强制执行 `layoutSubtreeIfNeeded()`。
- **`tests/host_brain_selftest/main.swift`**：
  - 扩展第 12 组渲染逻辑，覆盖导出全部 6 个大脑 Tab（定义、知识、话术、绑定、版本、合成测试）以及会话页；
  - 注入合成覆盖与组装预览快照；会话页注入带有 `echo_observed` 真实阶段的轮次数据。
- **`docs/reviews/2026-11-10-production-brain-ui.md`**：
  - 记录本次撤回、五大整改项具体实现、7 张 PNG 详尽尺寸与 OCR 对账证据。

---

## 6. 风险与限制说明

1. **真实环境安全隔离**：本任务所有验证均在离屏与受限沙箱 fixture 中完成，严格遵守安全准则，绝不连接真实微信进程，绝不产生真实模型 Token 计费；
2. **提交性质**：本次交付代表开发工程师根据协调者要求完成 5 点整改的实现与本地验证，提交本地技术审查，非自行宣布整个项目验收通过。
