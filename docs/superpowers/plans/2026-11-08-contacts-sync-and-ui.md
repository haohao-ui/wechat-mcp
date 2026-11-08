# 通讯录同步与助手界面改版 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按微信分组把通讯录同步进本地联系人库（以微信号为身份），并把微信客服助手改成"总览/会话/联系人/MCP/设置/日志"六页界面。

**Architecture:** Swift worker 新增只读的通讯录操作（截图 + 本机识别 + 按坐标点击，坐标进布局图）；Rust 侧把识别结果解析成结构化资料、写进历史库新表 `contact_profiles`，由协调器在空闲时编排全量/增量同步；宿主应用通过 `wechat-mcp control` 子命令读取联系人与 MCP 信息，界面拆成每页一个 Swift 文件。

**Tech Stack:** Rust（rusqlite、tokio）、Swift/AppKit、Vision、现有 worker socket 协议（JSON 行）。

**Spec:** `docs/superpowers/specs/2026-11-08-contacts-sync-design.md`（§8.1/8.2 为界面）；实测：`docs/observations/2026-11-06-wechat-4.1.15-actions.md` §8、§8.1。

## Global Constraints

- 不读微信数据库/加密数据；只看屏幕。不修改微信里的任何东西。
- 通讯录流程**禁止点击**：资料卡底部「发消息」「语音聊天」「视频聊天」（资料卡区域 y≥440 的按钮带）、公众号资料卡上的任何按钮、「通讯录管理」里除左侧筛选与表格滚动以外的任何控件。「通讯录管理」只用 AX 关闭按钮关闭。
- 通讯录资料只存在专用 Mac 本地；不发模型、不走云端识图；截图用完即删；日志只写数量与结果，不写名字/微信号。
- 给外部大脑的 MCP 历史工具仍只返回允许名单内联系人（现有边界不变）。宿主界面经本机 `control` 子命令可看全部联系人。
- 分类：`official`（公众号、服务号）、`work`（企业微信联系人）、`personal`（联系人）、`group`（群聊）。`official` 永不进允许名单。
- 身份：微信号为强身份；企业微信联系人无微信号 → `identity=weak`，以 分类+名字+企业 识别。
- 同步期间暂停自动回复；每处理完一个联系人检查有没有新消息，有就让出并记断点。
- 忽略名单优先于允许名单（现有 `decision()` 语义不变）。
- 宿主仍为原生 Swift/AppKit，同一签名证书；配置读写沿用 `ConfigForm` 校验。
- 所有命令前台运行读完输出；提交前在干净工作区跑 `cargo test` 全量、`WECHAT_STRICT_VISION=1 cargo test --test swift_native_frames`、`MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --locked --target x86_64-apple-darwin`；改了 Swift 还要编译对应目标。不连接 WeChat Mac，不 push。
- 测试数据只用合成数据（不得出现真实联系人名字/微信号）。

## Review Focus

1. 识别漏行：一屏 10 行只认出 8 行时，同步不得少算人数或把下一行的资料记到上一行名下（按几何行处理，认不出名字的行标 `unreadable`）。
2. 重名：两个联系人显示名相同 → 不自动归属聊天记录，进待确认列表。
3. 同一人两次同步读到的微信号不同（识别错一个字）→ 不覆盖，标待确认。
4. 同步中途客户发消息 → 在一个联系人粒度内让出，断点续传后人数仍然核对一致。
5. 「通讯录管理」窗口中途自己关闭 → 重新打开并从断点继续，不重复写、不漏写。

---

### Task 1: 历史库联系人资料表与身份规则（Rust）

**Files:**
- Modify: `src/core/history/store.rs`（MIGRATIONS 追加一个版本；新函数）
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces（`src/core/history/store.rs`）:
  - `pub struct ProfileInput { pub category: String, pub wechat_id: Option<String>, pub nickname: String, pub remark: Option<String>, pub region: Option<String>, pub source: Option<String>, pub added_on: Option<String>, pub common_groups: Option<i64>, pub enterprise: Option<String>, pub real_name: Option<String>, pub tags: Vec<String>, pub friend_permission: Option<String> }`
  - `pub enum UpsertOutcome { Created(String), Updated(String), NeedsReview { reason: String, candidates: Vec<String> } }`（String = contact_id）
  - `impl HistoryStore { pub fn upsert_profile(&self, account_id: &str, p: &ProfileInput, now_ms: i64) -> Result<UpsertOutcome>; pub fn mark_missing(&self, account_id: &str, category: &str, seen_contact_ids: &[String], now_ms: i64) -> Result<usize>; pub fn list_profiles(&self, account_id: &str, category: Option<&str>) -> Result<Vec<Value>>; pub fn pending_reviews(&self, account_id: &str) -> Result<Vec<Value>> }`

- [ ] **Step 1: 写失败测试**（每个一个 `#[test]`，合成数据）
  - `test_profile_migration_creates_table_and_addressbook_source`：迁移后 `contact_profiles` 存在；`contact_names` 可插入 `source='addressbook'`（SQLite CHECK 需重建表，旧数据保留，行数不变）。
  - `test_upsert_new_personal_creates_contact_and_names`：微信号 `wxid_t1`、昵称"测试甲"、备注"甲备注" → `Created`；`resolve(account,"甲备注")` 与 `resolve(account,"测试甲")` 都返回该 contact。
  - `test_upsert_same_wechat_id_updates_and_keeps_contact`：同微信号改备注 → `Updated(同一 id)`，新备注可 resolve。
  - `test_upsert_matches_existing_history_contact_by_unique_name`：已有聊过天的联系人"测试乙"（无资料）→ 同步到同名、带微信号的条目 → 补到原 contact（不新建），消息数不变。
  - `test_upsert_ambiguous_name_needs_review`：两个现有联系人都叫"重名丙" → `NeedsReview`，`pending_reviews` 列出。
  - `test_upsert_wechat_id_conflict_needs_review`：同一名字 + 已有微信号 `wxid_a`，再来 `wxid_b` → `NeedsReview`，不覆盖。
  - `test_weak_identity_for_work_contact`：`category=work`、无微信号、有 enterprise → `identity='weak'`，再次同步同名同企业 → `Updated` 同一 id。
  - `test_official_category_recorded`：`category=official` 写入后 `list_profiles(Some("official"))` 返回。
  - `test_mark_missing_sets_removed_at_not_deleting`：未在本次 seen 列表里的同分类联系人 `removed_at` 被设置，聊天记录仍在；再次出现时清空 `removed_at`。
- [ ] **Step 2:** `cargo test --bin wechat-mcp history::store` → 新测试 FAIL（函数/表不存在）。
- [ ] **Step 3:** 实现。表结构按 spec §5（`contact_id` PK → contacts，`account_id`，`wechat_id`，`category` CHECK 四值，`nickname`，`remark`，`region`，`source`，`added_on`，`common_groups`，`enterprise`，`real_name`，`tags` JSON 文本，`friend_permission`，`identity` CHECK strong/weak，`first_synced_at`，`last_synced_at`，`removed_at`）；`UNIQUE(account_id, wechat_id)`（wechat_id 非空时）。`contacts.kind`：group → `group`，其余 → `private`。名字登记 source=`addressbook`（备注、昵称各一条）。
- [ ] **Step 4:** 同命令 → PASS；`cargo test` 全量 PASS。
- [ ] **Step 5:** 提交 `feat(history): 联系人资料表与按微信号的身份规则`。

### Task 2: 资料卡与管理表格识别结果解析（Rust，纯函数）

**Files:**
- Create: `src/core/contacts/mod.rs`、`src/core/contacts/parse.rs`；Modify: `src/core/mod.rs`（`pub mod contacts;`）
- Test: `parse.rs` 内 `mod tests`

**Interfaces:**
- Consumes: `ProfileInput`（Task 1 定义；Task 1 未合并前，本任务在 `parse.rs` 里先定义同名同字段结构 `ProfileFields`，集成时由协调者改为使用 `ProfileInput`）
- Produces（`src/core/contacts/parse.rs`）:
  - `pub struct OcrLine { pub text: String, pub x: f64, pub y: f64, pub w: f64, pub h: f64 }`（窗口内坐标，`Deserialize`）
  - `pub fn parse_profile(lines: &[OcrLine], category: &str) -> ProfileParse`；`pub struct ProfileParse { pub fields: ProfileFields, pub missing: Vec<&'static str> }`
  - `pub struct ManagerRow { pub row_y: f64, pub name: Option<String>, pub remark: Option<String>, pub tags: Vec<String>, pub permission: Option<String> }`
  - `pub fn parse_manager_page(lines: &[OcrLine], first_row_y: f64, row_h: f64, rows: usize, columns: &ManagerColumns) -> Vec<ManagerRow>`；`pub struct ManagerColumns { pub name_x: f64, pub remark_x: f64, pub tags_x: f64, pub permission_x: f64 }`
  - `pub fn parse_group_headers(lines: &[OcrLine]) -> Vec<(String, u32, f64)>`（分类、数量、标题 y）

- [ ] **Step 1: 写失败测试**（合成 OCR 行）
  - `test_profile_label_value_split`：行 `"微信号：wxid_t1"`、`"地区中国 测试省"`、`"添加时间2023-06-08"`、`"共同群聊1个"`、`"来源通过搜索手机号添加"` → 各字段正确；标签在单独一行、值在同 y 右侧的另一行时也能配对（`"备注"` x=431 + `"甲备注"` x=504 同 y±6）。
  - `test_profile_remark_placeholder_is_none`：`"添加备注名"` → `remark=None`。
  - `test_official_profile_wechat_id_gh`：category=official，`"微信号gh_abc123"` → wechat_id `gh_abc123`，其余缺失不报错。
  - `test_work_profile_has_no_wechat_id`：category=work，有 `"企业"`/`"实名"` 配对 → enterprise/real_name 有值，wechat_id None，`missing` 含 `wechat_id`。
  - `test_profile_nickname_is_top_large_line`：昵称取资料卡区域最上方一行（y 最小、x≥500）。
  - `test_manager_page_assigns_by_geometry_with_missing_row`：10 行几何、只给 8 行文字（第 3、8 行缺）→ 返回 10 个 `ManagerRow`，第 3、8 行 `name=None`，其余名字与行 y 对应，不错位。
  - `test_manager_columns_split`：同一行的备注/标签/权限按列 x 归属。
  - `test_group_headers_counts`：`"公众号、服务号"`+`"33"` 同行等 → `[("official",33,y),("work",14,y),("personal",22,y)]`；带展开箭头前缀字符也能识别。
- [ ] **Step 2:** `cargo test --bin wechat-mcp contacts::parse` → FAIL。
- [ ] **Step 3:** 实现。字段前缀表：微信号/地区/备注/来源/添加时间/共同群聊/企业/实名/朋友权限；中文冒号与无冒号都接受；`添加时间` 只接受 `YYYY-MM-DD`；同行配对容差 y±6。
- [ ] **Step 4:** PASS；全量 PASS。
- [ ] **Step 5:** 提交 `feat(contacts): 资料卡与通讯录管理表格识别解析`。

### Task 3: Worker 通讯录只读操作与布局（Swift）

**Files:**
- Modify: `native/layout/wechat-4.1.15.json`（新增 `contacts` 段）、`native/common/WechatLayout.swift`（解析 `contacts` 段，可选字段，老布局缺失时报 `layout_missing_contacts`）
- Create: `native/bot/Contacts.swift`
- Modify: `native/bot/Worker.swift`（操作分发加入下列 operation）
- Test: `tests/swift_contacts.rs`（编译 `native/common/WechatLayout.swift` + `native/bot/Contacts.swift` 的纯函数部分 + 自测 main，同 `tests/swift_bounded.rs` 做法）

**Interfaces:**
- 布局 `contacts` 段（窗口内坐标，来自实测 §8/§8.1）：`{"nav_contacts":{"x":30,"y":169},"nav_chats":{"x":30,"y":121},"list":{"x":60,"y":40,"w":240,"h":600},"manager_button":{"x":180,"y":59},"profile":{"x":420,"y":50,"w":400,"h":390},"profile_forbidden":{"x":420,"y":440,"w":420,"h":120},"row_height":50,"manager":{"first_row_y":96,"row_height":48,"rows":10,"columns":{"name_x":300,"remark_x":385,"tags_x":500,"permission_x":620},"filter":{"x":40,"tag_y":167,"all_y":52}}}`
- Worker operations（请求/返回 JSON；`lines` 为 `[{text,x,y,w,h}]` 窗口内坐标，OCR 在本机完成）：
  - `contacts_enter` → `{ok}`：点导航「通讯录」，列表滚到顶。
  - `contacts_headers` → `{lines}`：列表区 OCR。
  - `contacts_click_list(x,y)` → `{lines}`：只允许点在 `list` 区域内（否则 `invalid("contacts_click_outside_list")`），点后返回列表区 OCR。
  - `contacts_scroll_list(direction, ticks)` → `{lines}`。
  - `contacts_open_row(y)` → `{lines}`：点 `list` 内该行（x=180），返回 `profile` 区 OCR（**不含** `profile_forbidden` 区）。
  - `contacts_manager_open` → `{window:{x,y,w,h}}`；`contacts_manager_page` → `{lines, window}`；`contacts_manager_scroll(direction,ticks)` → `{lines, window}`；`contacts_manager_filter(kind:"all"|"tag", y)` → `{lines, window}`（tag 必须传从管理窗口左栏识别到的该标签 y；Ruling 2026-11-08：标签间距≈40 与表格行高 48 不同，不能按序号推算）；`contacts_manager_close` → `{closed: bool}`（AX 关闭按钮）。管理窗口不存在时上述操作返回 `invalid("contacts_manager_missing")`。
  - `contacts_leave` → 点导航「聊天」。
  - 所有操作都在 `withDesktop` 内、`screenGate` 前置；任何点击坐标落在 `profile_forbidden` 区 → `invalid("contacts_forbidden_click")`（硬保护，单测覆盖）。
- [ ] **Step 1: 写失败测试**（`tests/swift_contacts.rs`）
  - `layout_contacts_section_parses`：解析布局得到上述值；缺 `contacts` 段报 `layout_missing_contacts`。
  - `forbidden_click_guard_rejects_profile_buttons`：`contactsClickAllowed(point:(480,496), layout)` == false；`(180,230)` == true；`list` 区外 == false。
  - `manager_window_pick_prefers_titled_manager`：给定窗口列表（主窗口 880×640、标题"通讯录管理" 720×560）选中管理窗口；只有主窗口时返回 nil。
- [ ] **Step 2:** `cargo test --test swift_contacts` → FAIL。
- [ ] **Step 3:** 实现 `Contacts.swift`（纯函数：`contactsClickAllowed`、`pickManagerWindow`、区域裁剪）与 Worker 分发；OCR 复用现有本机识别函数；截图只在内存或临时文件且用后删除。
- [ ] **Step 4:** PASS；`swiftc` 编译 worker 目标（`scripts/build.sh` 第 14 行同参数）通过；全量 cargo test PASS。
- [ ] **Step 5:** 提交 `feat(worker): 通讯录只读操作与禁点区保护`。

### Task 4: 同步编排、控制命令与查询（Rust）

**Files:**
- Create: `src/core/contacts/sync.rs`
- Modify: `src/core/scheduler.rs`（空闲时与请求文件触发同步，类似 `serve_calibration`）、`src/core/control.rs`（子命令）、`src/core/history/mcp_tools.rs`、`src/core/history/query.rs`
- Test: `sync.rs` 内 `mod tests`（Mock worker 返回合成 OCR）；control/query 现有测试模块追加

**Interfaces:**
- Consumes: Task 1 `upsert_profile/mark_missing/list_profiles/pending_reviews`；Task 2 `parse_*`；Task 3 operation 名与返回格式。
- Produces:
  - `pub async fn run_sync(capture: &mut dyn CoordinatorCapture, store: &HistoryStore, account_id: &str, mode: SyncMode, checkpoint: &Path, should_yield: &(dyn Fn() -> bool + Send + Sync), now_ms: i64) -> Result<SyncReport>`；`pub enum SyncMode { Full, Incremental }`；`pub struct SyncReport { pub per_category: Vec<(String, u32, u32)> /* 分类, 期望, 已同步 */, pub unreadable_rows: u32, pub needs_review: u32, pub yielded: bool, pub complete: bool }`
  - 结果写 `<state_dir>/contacts-sync-result.json`（`per_category`、`complete`、`finished_at`），断点 `<state_dir>/contacts-sync-checkpoint.json`。
  - 请求文件 `<state_dir>/contacts-sync-request.json`（`{"id","mode":"full"|"incremental","created_at"}`），协调器处理后写结果。
  - 控制命令：`wechat-mcp control contacts-sync [--full] [--wait SECONDS]`、`contacts-list [--category C] --json`（全部联系人：profile、处理方式 allow/ignore/none、最近对话时间、消息数、待确认项）、`mcp-info --json`（三个入口路径与连通性、`tools/list` 工具名按 外部大脑/微信操作/客户/聊天历史 分组）。配置错误退出 2。
  - MCP 历史工具：`history_list_contacts` 增加可选 `category` 参数并带 profile 字段；`history_get_contact` 带 profile；允许名单边界不变。
- [ ] **Step 1: 写失败测试**
  - `test_full_sync_counts_match_headers`：Mock 三个分组（personal 3、work 1、official 2）→ 每类 期望==已同步，`complete=true`，库里 6 条 profile。
  - `test_sync_yields_between_contacts_and_resumes_from_checkpoint`：`should_yield` 在第 2 个联系人后为 true → `yielded=true`、断点存在；再跑 → 从第 3 个开始，最终完整，无重复。
  - `test_sync_reports_incomplete_when_rows_unreadable`：一行名字认不出 → `unreadable_rows=1`、`complete=false`，结果文件写明差几个。
  - `test_manager_window_closed_midway_is_reopened`：Mock 第二页返回 `contacts_manager_missing` → 重新 `contacts_manager_open` 后继续，无重复。
  - `test_incremental_only_opens_changed_rows`：库里已有 3 人，管理表格 4 人 → 只调用 1 次 `contacts_open_row`。
  - `test_sync_never_requests_forbidden_click`：Mock 记录所有请求；资料卡只经 `contacts_open_row`，无其它点击类操作带资料卡坐标。
  - `test_sync_pauses_and_restores_auto_reply`：同步前写暂停标记、结束删除；用户原本已手动暂停则结束后保持暂停。
  - `test_control_contacts_list_includes_ignore_and_allow_handling`、`test_control_mcp_info_lists_tool_groups`、`test_history_list_contacts_category_filter_respects_allowlist`。
  - `test_sync_logs_counts_only`：同步产生的日志里不含合成联系人名字/微信号。
- [ ] **Step 2:** 跑对应测试 → FAIL。
- [ ] **Step 3:** 实现：全量 = 进通讯录 → 读分组数量 → 管理窗口逐页收集名字/备注/权限，逐个标签筛选收集标签成员 → 逐分组展开、逐行打开资料卡 → `upsert_profile` → `mark_missing` → 核对 → 离开通讯录；增量 = 只读管理表格比较，只打开新增/变化行。每日增量：本地时间 3:00 后、无待回复 turn、距上次成功 > 20 小时。
- [ ] **Step 4:** PASS；全量 PASS；交叉编译 PASS。
- [ ] **Step 5:** 提交 `feat(contacts): 通讯录全量/增量同步、控制命令与查询`。

### Task 5: 宿主界面改版——框架、总览、会话、设置、日志（Swift）

**Files:**
- Create: `native/bootstrap/Shell.swift`（左侧导航 + 页面容器）、`OverviewPage.swift`、`SessionsPage.swift`、`SettingsPage.swift`、`LogsPage.swift`、`LogTail.swift`（文件末尾增量读取，纯逻辑可测）、`Permissions.swift`（权限状态、请求、打开系统设置）
- Modify: `native/bootstrap/main.swift`（窗口内容改为 Shell）、`StatusPanel.swift`（保留数据采集与动作的复用部分；可改名 `PanelModel.swift`）、`StatusModel.swift`（会话状态人话文案）
- Test: `tests/swift_host.rs` 追加

**Interfaces:**
- Consumes: 现有 `evaluate(StatusInput)`、`ConfigFormLogic`、`HostActions`、`CoreSupervisor`。
- Produces: `final class LogTail { init(url: URL, maxLines: Int = 500); func poll() -> [String] }`（从上次位置读新行；文件轮换或截断后从头读）；`enum PermissionKind { case screen, accessibility, desktop, documents, downloads }`、`func permissionStatus(_ k: PermissionKind) -> Bool?`（nil=未知）、`func requestPermission(_:)`、`func openSystemSettings(for:)`；`Shell` 暴露 `func register(page: NSViewController, id: String, title: String)` 供 Task 6 注册。
- [ ] **Step 1: 写失败测试**（`tests/swift_host.rs`）
  - `test_log_tail_reads_only_new_lines_and_caps`：写 3 行 → poll 得 3；追加 2 → 得 2；超过 500 只留最近 500；截断后从头读。
  - `test_session_state_copy_for_missing_row`：`waiting_for_wechat_window` + 原因含 `session_row_not_unique_or_missing` → 文案含"会话不在列表可见范围"。
  - `test_every_legacy_panel_control_has_new_home`：源码检查——spec §8.2 每一项（保存配置、重启桌面服务、重启机器人、放回标准位置、恢复自动回复、打开附件文件夹、打开日志文件夹、复制诊断信息、请求屏幕录制权限、请求辅助功能权限、自动管理微信主窗口、各配置字段键名）在新页面文件里都有对应调用。
  - `test_settings_fields_keep_validation_ranges`：设置页保存走 `ConfigFormLogic`，越界值（冷却 301）返回错误提示。
- [ ] **Step 2:** FAIL。
- [ ] **Step 3:** 实现；布局与文案对照样稿 https://claude.ai/artifact/KiAh3WyFdE7784pjM75MRP（不照搬网页代码）。
- [ ] **Step 4:** PASS；`swiftc -O -target x86_64-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift` 编译通过；全量 cargo test PASS。
- [ ] **Step 5:** 提交 `feat(host): 六页界面框架与总览/会话/设置/日志`。

### Task 6: 宿主界面——联系人页与 MCP 页（Swift）

**Files:**
- Create: `native/bootstrap/ContactsPage.swift`、`McpPage.swift`
- Modify: `native/bootstrap/Shell.swift`（注册两页）
- Test: `tests/swift_host.rs` 追加

**Interfaces:**
- Consumes: Task 4 `control contacts-list --json`、`control contacts-sync`、`control mcp-info --json`、`contacts-sync-result.json`；Task 5 `Shell.register`；`ConfigFormLogic`（写 allowlist/ignorelist、brain 段）。
- Produces: `enum Handling { case autoReply, recordOnly, ignored }`；`func applyHandling(config: [String:Any], name: String, kind: String, category: String, handling: Handling) -> Result<[String:Any], String>`（纯函数）。
- [ ] **Step 1: 写失败测试**
  - `test_apply_handling_ignore_moves_to_ignorelist`、`test_apply_handling_auto_reply_refuses_official`、`test_apply_handling_unignore_restores_previous`（取消忽略后，原来在允许名单的恢复为自动回复）。
  - `test_contacts_page_parses_contacts_list_json`：合成 JSON → 五个分类标签计数正确（含"已忽略"）。
  - `test_mcp_page_brain_settings_round_trip`：外部大脑超时 120、超时后 handoff、排队 30 → 写回 config `brain` 段并通过现有校验。
- [ ] **Step 2:** FAIL。 **Step 3:** 实现。 **Step 4:** PASS + Swift 编译 + 全量。
- [ ] **Step 5:** 提交 `feat(host): 联系人页（分类/忽略/自动回复）与 MCP 页`。

### Task 7: 集成、部署与实机验收（协调者）

- [ ] 依次合并 Task 1–6，每次在干净工作区跑全量测试、严格 Vision、交叉编译、Swift 编译。
- [ ] `scripts/deploy_host.sh work@192.168.3.81 --app --worker --mcp --yes`，确认单一进程树、权限未丢。
- [ ] 实机：「初始化通讯录」→ 结果"联系人 22/22、企业微信 14/14、公众号 33/33"（或明确列出差几个）；同步期间暂停、结束恢复；中途给测试群发消息验证让出与续传。
- [ ] 实机：联系人页勾选/忽略写入配置即时生效；MCP 页「测试连接」；实时日志滚动；设置保存与校验；重启按钮。
- [ ] 请用户在测试群与面板上验收。

## 执行分组

- 第一批（互不依赖，可并行）：Task 1、Task 2、Task 3、Task 5。
- 第二批：Task 4（依赖 1、2、3），Task 6（依赖 4、5）。
- Task 7：协调者。
