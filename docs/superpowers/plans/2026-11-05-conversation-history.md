# 对话历史存储与查询 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为白名单客服 Bot 按联系人永久保存原始对话（含 Bot 回复标示、附件），以最近 N 条作为模型上下文，并提供只读的 MCP 与 HTTP 历史查询接口。

**Architecture:** 新增 `scripts/wechat_bot/history/` 包：`names`（名称标准化）、`store`（唯一写入方使用的 SQLite 存储，WAL）、`query`（只读查询层）、`mcp_server`、`http_server`。`BotService.cycle` 在进入模型/发送前先落库；协调器按 `contact_id` 管理会话目录，并在空闲时排队下载文件原件。远端 worker 改为按候选名称列表精确比对。

**Tech Stack:** Python 3.12 标准库（`sqlite3`、`http.server`、`unicodedata`、`json`）；SQLite 3.54（FTS5 trigram）；Swift（远端 worker）；测试用 `unittest`。

**Spec:** `docs/superpowers/specs/2026-11-05-conversation-history-design.md`（下称“设计”，以 § 引用）

## Global Constraints

- 仅 Python 标准库，不新增依赖；测试用 `python3 -m unittest`，在仓库根目录运行。
- 数据库 `<state_dir>/history/history.sqlite3`，附件 `<state_dir>/history/media/<sha前2位>/<sha256>.<ext>`；目录 0700、文件 0600（复用 `scripts.agent_bridge.codex.private_directory` / `write_private`）。
- 时间：UTC Unix **毫秒**整数。
- 打开连接：`PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000`。查询层用 `file:...?mode=ro` URI。
- ID：联系人 `c_` + 16 位十六进制，消息 `m_` + 16 位十六进制（`secrets.token_hex(8)`）。
- 配置新字段：`context_messages`（默认 5，范围 0..100）、白名单条目可选 `aliases`（字符串数组，≤10）、`history_http`（`{"bind":"127.0.0.1","port":8765}`）。
- `sender` 取值仅 `customer` / `bot` / `self_other`；`capture` 取值仅 `live` / `baseline` / `rebaseline_gap` / `ambiguous` / `imported`。
- 查询接口全部只读；无删除功能；合并只能通过 `control`。
- HTTP 默认 `127.0.0.1:8765`，`Authorization: Bearer <token>`，令牌文件 `<state_dir>/history/http-token`（0600）。
- 先落库再进入模型/发送；写库失败本轮停止该会话，健康状态 `history_write_failed`。
- 不 `git commit`，除非用户明确要求（仓库有大量未提交的既有改动）；每个任务结束以测试通过为准。
- 远端 worker 部署：沿用现有方式（scp 到 `~/wechat-rpa-test/bootstrap-20261104/bot/`，远端 `swiftc -O -parse-as-library -D BOT_WORKER`，原子替换 `~/.wechat-rpa/bin/wechat_bot_worker`，先备份），部署前停止服务。

## Review Focus

1. **目录迁移后 worker 拒绝启动**：`BotService.ownership = sha256(str(self.root))`，会话目录改名后令牌变化，远端 `bot-worker.enabled` 残留旧令牌会让 `NativeCapture.start` 失败。期望：令牌改为由协调器根目录派生（与会话目录无关），迁移前后不变 → Task 6 测试。
2. **OCR 丢失表情导致不匹配**：备注名 `🍓小美丨鞋` 被识别为 `小美丨鞋`。期望：配置 `aliases` 后 worker 认可、Bot 正常处理，发送用实际识别到的标题 → Task 1、Task 5、Task 7 测试。
3. **Bot 写入时 HTTP/MCP 并发读取**：期望读取不报 `database is locked`，看到已提交数据 → Task 8 测试。
4. **翻页期间有新消息写入**：按 seq 升序翻页的游标不跳过、不重复 → Task 8 测试。
5. **崩溃发生在数据库提交与 `cursor.json` 写入之间**：重启后重放同一帧不得重复记录 → Task 3 测试。

---

## File Structure

| 文件 | 职责 |
|---|---|
| Create `scripts/wechat_bot/history/__init__.py` | 包标记 |
| Create `scripts/wechat_bot/history/names.py` | `name_key`、`clean_title`、候选名称 |
| Create `scripts/wechat_bot/history/schema.py` | 建表与迁移 SQL（版本 1） |
| Create `scripts/wechat_bot/history/store.py` | `HistoryStore`：唯一写入方 API |
| Create `scripts/wechat_bot/history/query.py` | `HistoryQuery`：只读查询，供 MCP/HTTP 共用 |
| Create `scripts/wechat_bot/history/mcp_server.py` | stdio MCP 服务 |
| Create `scripts/wechat_bot/history/http_server.py` | HTTP 服务 |
| Create `scripts/wechat_bot/history/migrate.py` | `migrate-history` 逻辑 |
| Modify `scripts/wechat_bot/routing.py` | 条目允许 `aliases`；按 `name_key` 决策 |
| Modify `scripts/wechat_bot/policy.py` | 标题按候选名称校验 |
| Modify `scripts/wechat_bot/service.py` | 配置字段；cycle 落库；上下文；发送收件人 |
| Modify `scripts/wechat_bot/coordinator.py` | contact_id 目录、令牌、文件下载调度、启动回补 |
| Modify `scripts/wechat_bot/native_capture.py` | 传 `accepted_names` |
| Modify `scripts/wechat_bot/control.py` | 新子命令 |
| Modify `native/bot/Worker.swift`、`native/bot/Sessions.swift` | 候选名称列表匹配 |
| Modify `docs/allowlist-bot.md` | 使用说明 |
| Tests：新建 `tests/test_history_names.py`、`tests/test_history_store.py`、`tests/test_history_query.py`、`tests/test_history_servers.py`、`tests/test_history_migrate.py`；修改 `tests/test_bot_routing.py`、`tests/test_wechat_bot_service.py` | |

---

### Task 1: 名称标准化与白名单别名

**Files:**
- Create: `scripts/wechat_bot/history/__init__.py`、`scripts/wechat_bot/history/names.py`
- Modify: `scripts/wechat_bot/routing.py`
- Test: `tests/test_history_names.py`，修改 `tests/test_bot_routing.py`

**Interfaces:**
- Produces: `name_key(raw: str) -> str`；`clean_title(raw: str) -> str`（仅去首尾空白、零宽字符 U+200B–U+200F、U+2060、U+FEFF；worker 用同一规则）；`candidate_names(entry: dict) -> list[str]`（`[recipient, *aliases]`）；`routing.match_entry(config, raw_name, kind='private') -> dict | None`（按 `name_key` 找白名单条目）；`routing.decision(config, recipient, kind='private')` 签名不变，语义改为按 `name_key` 比较（ignorelist 同样按 `name_key`）。

- [ ] **Step 1: 写失败测试**

```python
class NameKeyTests(unittest.TestCase):
    def test_emoji_and_symbols_removed(self):
        self.assertEqual(name_key('🍓小美丨鞋'), name_key('小美丨鞋'))
        self.assertEqual(name_key('✨Ａｂｃ ★'), 'Abc')
    def test_invisible_characters_removed(self):
        self.assertEqual(name_key('小​美️'), '小美')
    def test_all_symbol_name_falls_back(self):
        self.assertEqual(name_key('🍓🍓'), '🍓🍓')
    def test_clean_title_keeps_symbols(self):
        self.assertEqual(clean_title(' ​🍓小美 '), '🍓小美')
```

在 `tests/test_bot_routing.py` 的 `RoutingTests` 增加：

```python
def test_aliases_match_and_conflicts_rejected(self):
    config={'allowlist':[{'recipient':'🍓小美','kind':'private','aliases':['@小美']}]}
    validate_rules(config)
    self.assertEqual(decision(config,'小美'),'allowed')      # OCR 丢了表情
    self.assertEqual(decision(config,'@小美'),'allowed')
    with self.assertRaises(ValueError):
        validate_rules({'allowlist':[target('小美'),target('🍓小美')]})  # name_key 相同
    with self.assertRaises(ValueError):
        validate_rules({'allowlist':[{'recipient':'a','kind':'private','aliases':'a'}]})
def test_ignorelist_uses_name_key(self):
    config={'allowlist':[target('🍓小美')],'ignorelist':[target('小美')]}
    self.assertEqual(decision(config,'🍓小美'),'ignored')
```

- [ ] **Step 2: 运行确认失败**

Run: `python3 -m unittest tests.test_history_names tests.test_bot_routing -v`
Expected: FAIL（`ModuleNotFoundError: scripts.wechat_bot.history` 及新断言失败）

- [ ] **Step 3: 实现**

`name_key` 按设计 §8 四步（`unicodedata.normalize('NFKC')`；删除空白、零宽、变体选择符；删除类别 `So`/`Sk`/`Cf`；结果为空则退回第 2 步结果）。`validate_rules`：条目键集合为 `{'recipient','kind'}` 或再加 `aliases`；`aliases` 为 ≤10 个非空、无首尾空白的字符串；白名单**全部**候选名称的 `name_key` 两两不同，否则 `ValueError('allowlist names collide after normalization')`。

- [ ] **Step 4: 运行确认通过**

Run: `python3 -m unittest discover -s tests -p 'test_*bot*.py' && python3 -m unittest tests.test_history_names`
Expected: 全部 OK（现有 89 项不回退）

---

### Task 2: 数据库结构与联系人

**Files:**
- Create: `scripts/wechat_bot/history/schema.py`、`scripts/wechat_bot/history/store.py`
- Test: `tests/test_history_store.py`

**Interfaces:**
- Consumes: `name_key`、`candidate_names`（Task 1）
- Produces:
  - `HistoryStore.open(state_dir: Path) -> HistoryStore`（建目录 0700、库文件 0600、执行迁移；属性 `conn`）
  - `ensure_contact(account_id: str, entry: dict) -> str`：按 `candidate_names(entry)` 的 `name_key` 找未合并联系人（跟随 `merged_into`）；无则创建；为每个候选名写 `contact_names`（`source` = `config` / `alias`）；更新 `display_name` 为 `recipient`；返回 contact_id。同一 `name_key` 已属于另一未合并联系人时抛 `NameConflict(ValueError)`。
  - `resolve(account_id: str, raw_name: str) -> str | None`（跟随合并）
  - `observe_name(contact_id: str, raw_name: str) -> bool`（写/更新 `observed` 记录；返回该名称是否已是 config/alias）
  - `merge(from_id: str, into_id: str, note: str = '') -> int`、`unmerge(merge_id: int) -> None`
  - `canonical(contact_id: str) -> str`（跟随 `merged_into` 链）

- [ ] **Step 1: 写失败测试**

```python
def test_schema_created_private_and_wal(self):
    store=HistoryStore.open(self.state)
    db=self.state/'history'/'history.sqlite3'
    self.assertEqual(db.stat().st_mode & 0o777, 0o600)
    self.assertEqual(store.conn.execute('PRAGMA journal_mode').fetchone()[0],'wal')
    tables={r[0] for r in store.conn.execute("select name from sqlite_master")}
    self.assertTrue({'contacts','contact_names','messages','sends','media','media_jobs',
                     'contact_cursor','merges','schema_migrations','messages_fts'} <= tables)
def test_ensure_contact_is_stable_and_alias_aware(self):
    a=self.store.ensure_contact('acct',{'recipient':'🍓小美','kind':'private','aliases':['@小美']})
    self.assertTrue(a.startswith('c_') and len(a)==18)
    self.assertEqual(self.store.ensure_contact('acct',{'recipient':'🍓小美','kind':'private'}),a)
    self.assertEqual(self.store.resolve('acct','小美'),a)
    self.assertEqual(self.store.resolve('acct','@小美'),a)
    self.assertIsNone(self.store.resolve('other-acct','小美'))
def test_rename_keeps_contact_when_old_name_is_alias(self):
    a=self.store.ensure_contact('acct',{'recipient':'旧名','kind':'private'})
    self.assertEqual(self.store.ensure_contact('acct',{'recipient':'新名','kind':'private','aliases':['旧名']}),a)
def test_merge_follows_and_unmerge_restores(self):
    a=self.store.ensure_contact('acct',{'recipient':'甲','kind':'private'})
    b=self.store.ensure_contact('acct',{'recipient':'乙','kind':'private'})
    mid=self.store.merge(b,a,'改名')
    self.assertEqual(self.store.resolve('acct','乙'),a)
    self.store.unmerge(mid)
    self.assertEqual(self.store.resolve('acct','乙'),b)
def test_conflicting_name_key_rejected(self):
    self.store.ensure_contact('acct',{'recipient':'小美','kind':'private'})
    self.store.ensure_contact('acct',{'recipient':'别人','kind':'private'})
    with self.assertRaises(NameConflict):
        self.store.ensure_contact('acct',{'recipient':'别人','kind':'private','aliases':['🍓小美']})
```

- [ ] **Step 2: 运行确认失败**

Run: `python3 -m unittest tests.test_history_store -v`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 `schema.py`（设计 §7 全部表 + §7.5.1 `media_jobs` + §9.1 `contact_cursor`）与 `store.py` 上述方法**

`messages_fts` = `fts5(text, content='messages', content_rowid='rowid', tokenize='trigram')`，由 `messages` 的 AFTER INSERT / AFTER UPDATE OF text 触发器同步。索引：`messages(contact_id, seq)` 唯一、`messages(account_id, observed_at)`、`messages(send_key)`。跨未合并联系人的 `name_key` 冲突检查在代码中做。`schema_migrations` 记录版本 1。

- [ ] **Step 4: 运行确认通过**

Run: `python3 -m unittest tests.test_history_store -v`
Expected: OK

---

### Task 3: 消息写入、水位去重与回显配对

**Files:**
- Modify: `scripts/wechat_bot/history/store.py`
- Test: `tests/test_history_store.py`

**Interfaces:**
- Consumes: `scripts.wechat_bot.policy` 的 `new_bubbles`、`AmbiguousAnchor`、`ContinuityGap`、`echo_matches`、`fingerprint`
- Produces:
  - `record_frame(contact_id: str, frame: dict, *, batch_key: str | None = None) -> list[dict]`：单事务。读 `contact_cursor`：
    - 无水位 → 全部可见气泡 `capture=baseline`；
    - 有水位 → `new_bubbles(水位, 当前)` 结果 `capture=live`；
    - `AmbiguousAnchor` → 当前帧中 `error.certain` 之前、水位之后无法定位的部分写 `capture=ambiguous`（`meta.note='repeat count uncertain'`），`certain` 部分写 `live`；
    - `ContinuityGap` → 全部可见气泡 `capture=rebaseline_gap`（`meta.may_duplicate=true`）；
    - 入站 `sender=customer`；出站先回显配对，失败则 `sender=self_other`；`kind` 取 bubble 的 `kind`（缺省 `text`）；
    - 最后把水位更新为当前帧指纹序列。返回新写入的消息行 dict（含 `message_id`、`seq`、`direction`、`sender`、`capture`、`text`、`bubble`）。
  - `record_bot_reply(contact_id: str, send_key: str, reply_text: str, status: str, model: dict, input_message_ids: list[str]) -> str`（写 `sender=bot` 消息与 `sends`；同 `send_key` 重复调用返回原 message_id）
  - `update_send(send_key: str, status: str, echo_observed: bool | None = None) -> None`
  - `send(send_key: str) -> dict | None`
  - `recent_context(contact_id: str, limit: int, exclude_ids: frozenset[str] = frozenset()) -> list[dict]`：`[{'role':'user'|'assistant','text':...}]`，旧→新；合并进该联系人的其他联系人的消息一并计入（按 `observed_at`、`seq` 排序）；排除 `sends.status` ∈ {`superseded`,`stopped_before_send`,`skipped_by_rule`} 的 Bot 消息。

回显配对：在该联系人 `sender='bot'`、`bubble_digest IS NULL`、`sends.status IN ('triggered','unknown')` 的消息中，按 seq 从新到旧取第一条 `echo_matches(bubble['text'], reply_text)` 为真者；配对成功只更新该消息 `bubble_digest` 与 `sends.echo_observed=1`，不新增消息。

- [ ] **Step 1: 写失败测试**

```python
def bub(text,d='inbound_visual',kind='text'): return {'text':text,'digest':text,'direction':d,'kind':kind}
def fr(*b): return {'bubbles':list(b)}

def test_first_frame_is_baseline_then_live_increment(self):
    self.store.record_frame(self.c,fr(bub('a'),bub('b','outbound_visual')))
    rows=self.store.record_frame(self.c,fr(bub('a'),bub('b','outbound_visual'),bub('c')))
    self.assertEqual([(r['text'],r['capture'],r['sender']) for r in rows],[('c','live','customer')])
    self.assertEqual(self.captures(),['baseline','baseline','live'])
def test_identical_repeats_are_two_messages(self):
    self.store.record_frame(self.c,fr(bub('x')))
    self.store.record_frame(self.c,fr(bub('x'),bub('好'),bub('好')))
    self.assertEqual(self.texts(),['x','好','好'])
def test_replaying_same_frame_after_crash_writes_nothing(self):
    self.store.record_frame(self.c,fr(bub('x')))
    f=fr(bub('x'),bub('y'))
    self.store.record_frame(self.c,f)
    self.assertEqual(self.store.record_frame(self.c,f),[])
    self.assertEqual(self.texts(),['x','y'])
def test_ambiguous_and_gap_are_kept_and_marked(self):
    self.store.record_frame(self.c,fr(bub('A')))
    self.store.record_frame(self.c,fr(bub('A'),bub('A')))
    self.assertEqual(self.captures()[-1],'ambiguous')
    self.store.record_frame(self.c,fr(bub('完全不同')))
    self.assertEqual(self.captures()[-1],'rebaseline_gap')
def test_bot_reply_echo_is_paired_not_duplicated(self):
    self.store.record_frame(self.c,fr(bub('问')))
    self.store.record_bot_reply(self.c,'bot-service-k','你好～','attempting',{},[])
    self.store.update_send('bot-service-k','triggered')
    self.store.record_frame(self.c,fr(bub('问'),bub('你好~','outbound_visual')))
    self.assertEqual(self.senders(),['customer','bot'])
    self.assertEqual(self.store.send('bot-service-k')['echo_observed'],1)
def test_unpaired_outbound_is_self_other(self):
    self.store.record_frame(self.c,fr(bub('问')))
    self.store.record_frame(self.c,fr(bub('问'),bub('人工回复','outbound_visual')))
    self.assertEqual(self.senders()[-1],'self_other')
def test_recent_context_limit_order_and_exclusions(self):
    self.store.record_frame(self.c,fr(*[bub(str(i)) for i in range(8)]))
    self.store.record_bot_reply(self.c,'k1','作废','generated',{},[]);self.store.update_send('k1','superseded')
    ctx=self.store.recent_context(self.c,5)
    self.assertEqual([x['text'] for x in ctx],['3','4','5','6','7'])
    self.assertTrue(all(x['role']=='user' for x in ctx))
def test_fts_trigger_indexes_text(self):
    self.store.record_frame(self.c,fr(bub('还是上次那个款')))
    hit=self.store.conn.execute("select count(*) from messages_fts where messages_fts match '上次那'").fetchone()[0]
    self.assertEqual(hit,1)
```

（`texts()`/`captures()`/`senders()` 为测试内按 seq 查询的辅助函数。）

- [ ] **Step 2: 运行确认失败**

Run: `python3 -m unittest tests.test_history_store -v`
Expected: 新测试 FAIL（`AttributeError: record_frame`）

- [ ] **Step 3: 实现上述方法**（`seq` = 该联系人 `max(seq)+1`，在同一事务内取）

- [ ] **Step 4: 运行确认通过**

Run: `python3 -m unittest tests.test_history_store -v`
Expected: OK

---

### Task 4: 附件存档与下载任务队列

**Files:**
- Modify: `scripts/wechat_bot/history/store.py`
- Test: `tests/test_history_store.py`

**Interfaces:**
- Produces:
  - `attach_media(message_id: str, source_path: Path, *, kind: str, filename: str | None, source: str) -> str`：复制到内容寻址路径（0600），写 `media`，设置 `messages.media_id`；返回 SHA-256；>10MB 抛 `ValueError`。
  - `record_frame` 补充：新写入的 `kind='file'` 消息自动建 `media_jobs(status='pending', attempts=0)`；图片不入队（缩略图由 Task 5 同一轮附上）。
  - `next_media_job(contact_id: str) -> dict | None`（`pending`，且距上次尝试 ≥ `30 * 2**attempts` 秒）
  - `finish_media_job(message_id: str, status: str, error: str | None = None) -> None`：`done`/`gone` 为终态；`failed` 时 `attempts+1`，未满 3 次回到 `pending`，满 3 次置 `failed` 终态。
  - `has_pending_media() -> bool`

- [ ] **Step 1: 写失败测试**

```python
def test_attach_media_content_addressed_and_private(self):
    [row]=self.store.record_frame(self.c,fr(bub('[图片]',kind='image')))
    src=self.tmp/'x.png';src.write_bytes(b'png')
    mid=self.store.attach_media(row['message_id'],src,kind='image',filename=None,source='chat_thumbnail')
    path=self.state/'history'/'media'/mid[:2]/(mid+'.png')
    self.assertEqual(path.read_bytes(),b'png');self.assertEqual(path.stat().st_mode&0o777,0o600)
def test_file_messages_queue_download_with_backoff(self):
    [row]=self.store.record_frame(self.c,fr(bub('报价.pdf',kind='file')))
    job=self.store.next_media_job(self.c);self.assertEqual(job['message_id'],row['message_id'])
    self.store.finish_media_job(row['message_id'],'failed','timeout')
    self.assertIsNone(self.store.next_media_job(self.c))   # 退避 60 秒
def test_failed_three_times_is_terminal_and_gone_is_terminal(self):
    [row]=self.store.record_frame(self.c,fr(bub('a.pdf',kind='file')))
    for _ in range(3): self.store.finish_media_job(row['message_id'],'failed','x')
    self.assertFalse(self.store.has_pending_media())
```

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_history_store -v`；Expected: 新测试 FAIL
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — Expected: OK

---

### Task 5: Bot 服务接入（落库、上下文、发送收件人）

**Files:**
- Modify: `scripts/wechat_bot/service.py`、`scripts/wechat_bot/policy.py`
- Test: `tests/test_wechat_bot_service.py`、`tests/test_wechat_bot.py`

**Interfaces:**
- Consumes: Task 2–4 的 `HistoryStore`；Task 1 的 `clean_title`、`candidate_names`
- Produces:
  - `BotService.__init__(config, *, routing_check=None, history=None, contact_id=None, ownership=None)`：`history=None` 时行为与现在一致（旧测试不改）；`ownership` 给定时替代 `sha256(str(self.root))`（Task 6 使用）。
  - `load_config` 接受 `context_messages`（int 0..100，默认 5）与 `history_http`（dict，键仅 `bind`(str)/`port`(int 1..65535)，默认 `{'bind':'127.0.0.1','port':8765}`）。
  - `Policy(account, recipient, trigger, names: list[str] | None = None)`：`check_frame` 标题校验改为 `[clean_title(t) for t in title_lines]` 恰好一项且 ∈ `{clean_title(n) for n in names or [recipient]}`。

cycle 改动（设计 §9）：
1. `check_frame` 之后、任何回复判断之前：`rows = history.record_frame(contact_id, frame)`（每轮调用；无变化返回 `[]`）。新写入的图片消息若 bubble 有 `media_id`，经 `capture.download(media_id)` 取缩略图后 `attach_media(..., source='chat_thumbnail')`（缩略图失败只记日志）。
2. `history.observe_name(contact_id, frame['title_lines'][0])`。
3. 入站批次：写流水后、调用模型前，`context = history.recent_context(contact_id, config['context_messages'], exclude_ids=本批入站 message_id)` + 本批入站（替换 `confirmed['bubbles'][-12:]`）。
4. 状态改为 `attempting` 之前调用 `record_bot_reply(...)`；此后每次写流水状态都 `update_send(key, status)`。
5. `send_message` 的 `recipient` 用 `confirmed['title_lines'][0]`（实际识别到的标题）。
6. 任何 `history` 调用抛异常 → `self.health('history_write_failed', error=str(e)[:200])` 并 `return`（游标不推进，不调用模型）。

- [ ] **Step 1: 写失败测试**（在 `ServiceTests` 中新增子类 `HistoryServiceTests`，`setUp` 用 `HistoryStore.open(self.root/'h')` 与 `ensure_contact` 构造 `BotService(config, history=store, contact_id=cid)`）

```python
async def test_messages_recorded_before_model_call(self):
    self.service.advance(self.old)
    order=[]
    real=self.history.record_frame
    self.history.record_frame=lambda *a,**k:(order.append('db'),real(*a,**k))[1]
    async def model(*a,**k): order.append('model');return '你好，我是AI客服助手。'
    self.model.side_effect=model
    after=frame(*self.new['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
    await self.cycle([self.new,self.new,self.new,after])
    self.assertEqual(order[:2],['db','model'])
    self.assertEqual(self.senders(),['customer','customer','bot'])  # 基线 old、新消息、Bot 回复（回显已配对）
async def test_context_uses_stored_history_limit(self):
    self.config['context_messages']=2
    self.history.record_frame(self.cid,frame(*[bubble(str(i)) for i in range(5)]))
    self.service.advance(frame(*[bubble(str(i)) for i in range(5)]))
    new=frame(*[bubble(str(i)) for i in range(5)],bubble('新问题'))
    await self.cycle([new,new,new,new])
    ctx=self.model.await_args.kwargs['context']
    self.assertEqual([c['text'] for c in ctx],['3','4','新问题'])
async def test_history_failure_blocks_model_and_send(self):
    self.service.advance(self.old)
    self.history.record_frame=Mock(side_effect=sqlite3.OperationalError('disk full'))
    await self.cycle([self.new])
    self.model.assert_not_awaited();self.assertEqual(self.calls,[])
    self.assertEqual(self.service.cursor,self.old)
async def test_send_uses_observed_title_for_alias(self):
    # allowlist [{'recipient':'🍓午夜咖啡','kind':'private','aliases':['午夜咖啡']}]，帧标题 '午夜咖啡'
    ...
    self.assertEqual(sent[0]['recipient'],'午夜咖啡')
def test_config_context_messages_and_http_bounds(self):
    for patch_ in [{'context_messages':-1},{'context_messages':101},{'context_messages':'5'},
                   {'history_http':{'bind':'127.0.0.1','port':0}},{'history_http':{'host':'x'}}]:
        ...assertRaises(ValueError)
```

在 `tests/test_wechat_bot.py` 增加：`Policy('a','🍓午夜咖啡','service',names=['🍓午夜咖啡','午夜咖啡']).check_frame(帧标题'午夜咖啡')` 通过；标题 `'别人'` 抛 `allowlisted title`。

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_wechat_bot_service tests.test_wechat_bot -v`；Expected: 新测试 FAIL
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行全部 Bot 测试** — Run: `python3 -m unittest discover -s tests -p 'test_*bot*.py'`；Expected: OK

---

### Task 6: 协调器（contact_id 目录、令牌、别名、下载调度、启动回补）

**Files:**
- Modify: `scripts/wechat_bot/coordinator.py`、`scripts/wechat_bot/native_capture.py`
- Test: `tests/test_bot_routing.py`

**Interfaces:**
- Consumes: `HistoryStore`（Task 2–4）、`match_entry`、`candidate_names`（Task 1）、`BotService(..., history, contact_id, ownership)`（Task 5）
- Produces:
  - `BotCoordinator.__init__`：`self.history = HistoryStore.open(state_dir)`；若 `state_dir/sessions/` 下存在 64 位十六进制命名的旧目录 → `ValueError('run control migrate-history first')`。
  - `BotCoordinator.session(target)`：`contact_id = history.ensure_contact(account_id, target)`；会话目录 `state_dir/sessions/<contact_id>/`；`ownership = sha256(str(self.root)).hexdigest()`；`self.sessions` 键仍为 `recipient`；`BotService` 的 `Policy` 收到 `candidate_names(target)` 加该联系人已确认的 `observed` 名称。
  - 合并：`history.canonical(contact_id) != contact_id` 的白名单条目在 sweep 中 `disable_session(target, 'merged')`，不轮询。
  - `NativeCapture.__init__(host, client, recipient, directory, owner_token='', names=None)`；`call()` 的 params 固定带 `accepted_names=self.names or [self.recipient]`；`retarget(recipient, directory, names)` 同时更新 `self.names`，`open_session` 传 `target_names=names`；`due_sessions` 调 `capture.sessions(全部白名单候选名)`，返回行用 `match_entry` 映射回条目的 `recipient`。
  - `reconcile_journals()`（`run()` 开头调用）：遍历每个会话目录 `send-*.json`；`history.send(key)` 缺失且流水有 `reply` → `record_bot_reply`；状态不同 → `update_send`。
  - 文件下载调度：sweep 末尾，若本轮没有任何会话进入 `incoming_candidate` 且 `history.next_media_job(当前聊天 contact_id)` 非空，则执行一次：用当前帧找到该消息对应气泡（按 `bubble_digest` 匹配当前帧的 `kind='file'` 气泡）；找不到 → `finish_media_job('gone')`；找到 → `capture.call('prepare_file', media_id=气泡media_id, snapshot_id=帧snapshot_id)` + `capture.download(..., kind='file')` → `attach_media(..., source='original', filename=info['filename'])` + `finish('done')`；worker 报 `file_snapshot_changed` → `gone`；其他错误 → `failed`。每轮最多 1 个。

- [ ] **Step 1: 写失败测试**

```python
async def test_session_directory_is_contact_id_and_token_stable(self):
    child=self.coordinator.session(target('a'))
    cid=self.coordinator.history.resolve('fixture','a')
    self.assertEqual(child.root.name,cid)
    other=self.coordinator.session(target('b'))
    self.assertEqual(child.ownership,other.ownership)
    self.assertEqual(child.ownership,hashlib.sha256(str(self.coordinator.root).encode()).hexdigest())
def test_unmigrated_legacy_directory_refuses(self):
    (self.root/'state'/'sessions'/('0'*64)).mkdir(parents=True)
    with self.assertRaisesRegex(ValueError,'migrate-history'): BotCoordinator(self.path)
async def test_merged_entry_is_not_polled(self):
    self.coordinator.session(target('a'));self.coordinator.session(target('b'))
    h=self.coordinator.history;h.merge(h.resolve('fixture','b'),h.resolve('fixture','a'))
    self.probe_capture(lambda:{})
    await self.coordinator.sweep()
    self.assertNotIn('b',self.visited())
async def test_worker_receives_alias_candidates(self):
    # 复用 test_worker_refusal_keeps_transport_open 的 Pipe/Out 夹具，返回 ok:true
    capture=NativeCapture('fixture',None,'🍓a',self.root/'a',names=['🍓a','a'])
    ...
    await capture.call('capture')
    self.assertEqual(pipe.lines[-1]['params']['accepted_names'],['🍓a','a'])
async def test_session_probe_maps_alias_row_to_entry(self):
    # allowlist [{'recipient':'🍓b','kind':'private','aliases':['b']}]，探测返回行名 'b'
    ...
    self.assertIn('🍓b',self.visited())
async def test_file_download_runs_only_without_pending_replies(self): ...
async def test_file_scrolled_away_is_marked_gone(self): ...
async def test_reconcile_backfills_missing_and_stale_sends(self): ...
```

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_bot_routing -v`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行全部 Bot 测试** — Run: `python3 -m unittest discover -s tests -p 'test_*bot*.py'`；Expected: OK

---

### Task 7: 远端 worker 候选名称匹配

**Files:**
- Modify: `native/bot/Worker.swift`、`native/bot/Sessions.swift`

**Interfaces:**
- Consumes: 请求 params 新字段 `accepted_names: [String]`（1..20 项，每项 ≤64 字符；缺失时等于 `[recipient]`，兼容旧调用）；`open_session` 新字段 `target_names: [String]`（缺失时 `[target]`）
- Produces:
  - `func cleanTitle(_ s: String) -> String`：去首尾空白与 U+200B–U+200F、U+2060、U+FEFF（与 Task 1 `clean_title` 同规则）。
  - `capture`、`read_media`、`prepare_file`：标题校验改为“`title_lines` 恰好一项，且 `cleanTitle(title) ∈ accepted_names.map(cleanTitle)`”，否则仍报 `allowlisted_title_mismatch` / `media_contact_mismatch`；`capture` 返回值增加 `observed_title`（原样）。`analyzeFrame` 的 `recipient` 参数改为传 `nil`，在 worker 内校验。
  - `sessions`：`names` 每项先 `cleanTitle` 再与行名比较，返回的 `name` 为实际行名。
  - `open_session`：在会话列表中找 `cleanTitle(行名) ∈ target_names` 的行，必须恰好 1 行；点击后标题核验同 `capture`。

- [ ] **Step 1: 本机编译**

Run: `swiftc -O -target "$(uname -m)-apple-macos12" -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/bot/*.swift -o <scratchpad>/build/wechat_bot_worker`
Expected: 无 error

- [ ] **Step 2: 停服务、备份并部署远端 worker**（Global Constraints 部署方式）

- [ ] **Step 3: 实机只读验证**（临时探测脚本，方式同 `scratchpad/probe4.py`）：
  - `capture` 以 `accepted_names=['🍓午夜咖啡','午夜咖啡']` → 成功，`observed_title == '午夜咖啡'`；
  - `capture` 以 `accepted_names=['别人']` → `allowlisted_title_mismatch`；
  - `open_session` 以 `target_names=['🍓文件传输助手','文件传输助手']` → `opened: true`。

---

### Task 8: 只读查询层

**Files:**
- Create: `scripts/wechat_bot/history/query.py`
- Test: `tests/test_history_query.py`

**Interfaces:**
- Consumes: Task 2–4 的数据库（测试用 `HistoryStore` 写入夹具数据）
- Produces: `HistoryQuery(db_path: Path, *, default_recent: int = 5)`（属性 `conn`，只读），返回设计 §10.1 结构：
  - `list_contacts(query: str | None = None, cursor: str | None = None, limit: int = 50) -> dict`（limit ≤200；`query` 按 `name_key` 子串匹配任一名称；只返回未合并联系人）
  - `get_contact(contact_id: str | None = None, name: str | None = None) -> dict`（含 `names`、`merged_from`、`message_count`、`last_observed_at`）
  - `get_recent_messages(contact_id: str, limit: int | None = None) -> dict`（limit ≤200，默认 `default_recent`，旧→新）
  - `get_history(contact_id: str, since: int | None = None, until: int | None = None, cursor: str | None = None, limit: int = 100, order: str = 'asc') -> dict`（limit ≤500）
  - `search_messages(q: str, contact_id: str | None = None, since: int | None = None, until: int | None = None, cursor: str | None = None, limit: int = 20) -> dict`（limit ≤100；每项附 `context_ids`：同联系人前后各 2 条 message_id；`len(q) < 3` 用 `LIKE`）
  - `get_message(message_id: str) -> dict`（含 `meta`）
  - `get_media(media_id: str) -> tuple[dict, Path]`
  - `QueryError(code: str, message: str)`，code ∈ `not_found` / `invalid_argument`
  - 联系人消息集合 = 该联系人 + 所有 `canonical()` 指向它的联系人；排序与游标键 `(observed_at, contact_id, seq)`；游标 = `base64url(json([observed_at, contact_id, seq, order]))`，解码失败 → `invalid_argument`。

- [ ] **Step 1: 写失败测试**

```python
def test_read_only_connection_cannot_write(self):
    with self.assertRaises(sqlite3.OperationalError):
        self.q.conn.execute("insert into merges(from_contact,into_contact,created_at) values('x','y',1)")
def test_pagination_stable_while_writing(self):
    page=self.q.get_history(self.c,limit=3)
    seen=[m['text'] for m in page['items']]
    self.writer.record_frame(self.c,fr(*self.visible,bub('新')))
    while page['next_cursor']:
        page=self.q.get_history(self.c,cursor=page['next_cursor'],limit=3)
        seen+=[m['text'] for m in page['items']]
    self.assertEqual(seen,self.expected_texts+['新'])
def test_concurrent_read_during_open_write_transaction(self):
    self.writer.conn.execute('BEGIN IMMEDIATE')
    self.writer.conn.execute("insert into merges(from_contact,into_contact,created_at) values('a','b',1)")
    self.assertIsInstance(self.q.get_recent_messages(self.c)['items'],list)
    self.writer.conn.rollback()
def test_search_trigram_and_short_term_fallback(self):
    self.assertEqual(self.texts(self.q.search_messages('上次那个')),['还是上次那个款'])
    self.assertEqual(self.texts(self.q.search_messages('顺丰')),['已发顺丰'])
def test_merged_contact_history_is_included(self):
    self.writer.merge(self.c2,self.c)
    self.assertIn('乙的消息',self.texts(self.q.get_history(self.c)))
def test_bot_message_carries_send_status(self):
    m=self.q.get_recent_messages(self.c)['items'][-1]
    self.assertEqual((m['sender'],m['send']['status'],m['send']['echo_observed']),('bot','triggered',True))
def test_limits_cursor_and_unknown_ids(self):
    with self.assertRaises(QueryError): self.q.get_history(self.c,limit=501)
    with self.assertRaises(QueryError): self.q.get_history(self.c,cursor='garbage')
    with self.assertRaises(QueryError): self.q.get_message('m_missing')
def test_name_lookup_uses_name_key(self):
    self.assertEqual(self.q.get_contact(name='🍓小美')['contact_id'],self.q.get_contact(name='小美')['contact_id'])
```

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_history_query -v`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 运行确认通过** — Expected: OK

---

### Task 9: MCP 与 HTTP 服务

**Files:**
- Create: `scripts/wechat_bot/history/mcp_server.py`、`scripts/wechat_bot/history/http_server.py`
- Modify: `scripts/wechat_bot/control.py`
- Test: `tests/test_history_servers.py`

**Interfaces:**
- Consumes: `HistoryQuery`（Task 8）、`load_config`（取 `state_dir`、`context_messages`、`history_http`）
- Produces:
  - MCP：`python3 -m scripts.wechat_bot.history.mcp_server [--config PATH]`（默认配置路径同 `control`），stdio 逐行 JSON-RPC 2.0。`initialize` 返回 `protocolVersion: "2024-11-05"`、`serverInfo.name: "wechat-history"`、`capabilities.tools: {}`；`notifications/initialized` 无响应；`tools/list` 返回 7 个工具：`history_list_contacts`、`history_get_contact`、`history_get_recent_messages`、`history_get_history`、`history_search_messages`、`history_get_message`、`history_get_media`，`inputSchema` 与 Task 8 参数一致（`additionalProperties: false`）；结果 `content: [{"type":"text","text": <JSON>}]`；`QueryError` 或参数错误 → `isError: true`；未知方法 → JSON-RPC 错误 `-32601`。`history_get_media` 返回 `{"media":…, "base64":…}`（>10MB 拒绝）。
  - HTTP：`make_server(query: HistoryQuery, token: str, bind: str, port: int) -> ThreadingHTTPServer`；`main()` 读配置，令牌文件不存在则生成（`secrets.token_urlsafe(32)`，0600）。路由（设计 §10.4）：`/v1/contacts`、`/v1/contacts/{id}`、`/v1/contacts/{id}/recent`、`/v1/contacts/{id}/messages`、`/v1/search`、`/v1/messages/{id}`、`/v1/media/{id}`；查询参数名与 Task 8 参数名相同。仅 GET（其他 405）；`Authorization: Bearer` 用 `hmac.compare_digest`，失败 401；`not_found` 404，`invalid_argument` 400，未知路由 404；JSON 用 `ensure_ascii=False`、`Content-Type: application/json; charset=utf-8`；媒体按扩展名给 `Content-Type`。
  - `control history-http install|start|stop|status`：launchd 标签 `local.wechat-rpa.history`，日志 `~/.WeChatRPABot/logs/history-http.*.log`，与 Bot 标签独立。

- [ ] **Step 1: 写失败测试**

```python
def test_mcp_lists_tools_and_returns_history(self):
    out=self.rpc([{'jsonrpc':'2.0','id':1,'method':'initialize','params':{}},
                  {'jsonrpc':'2.0','id':2,'method':'tools/list'},
                  {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'history_get_recent_messages','arguments':{'contact_id':self.c}}}])
    self.assertEqual(len(out[1]['result']['tools']),7)
    self.assertEqual(json.loads(out[2]['result']['content'][0]['text'])['items'][-1]['text'],'最后一条')
def test_mcp_errors(self):
    out=self.rpc([{'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'history_get_message','arguments':{'message_id':'m_missing'}}},
                  {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'history_get_history','arguments':{'contact_id':self.c,'bogus':1}}},
                  {'jsonrpc':'2.0','id':3,'method':'nope'}])
    self.assertTrue(out[0]['result']['isError']);self.assertTrue(out[1]['result']['isError'])
    self.assertEqual(out[2]['error']['code'],-32601)
def test_http_requires_token_and_only_get(self):
    self.assertEqual(self.get('/v1/contacts',token=None).status,401)
    self.assertEqual(self.get('/v1/contacts',token='wrong').status,401)
    self.assertEqual(self.request('POST','/v1/contacts').status,405)
def test_http_history_and_search(self):
    r=self.get(f'/v1/contacts/{self.c}/messages?limit=2');self.assertEqual(r.status,200)
    self.assertIn('next_cursor',json.loads(r.read()))
    r=self.get('/v1/search?q='+urllib.parse.quote('上次那个'));self.assertEqual(r.status,200)
def test_http_emoji_query_and_traversal_are_safe(self):
    self.assertEqual(self.get('/v1/contacts?query='+urllib.parse.quote('🍓小美/../')).status,200)
    self.assertEqual(self.get('/v1/media/..%2F..%2Fhistory.sqlite3').status,404)
def test_http_default_bind_is_loopback(self):
    self.assertEqual(load_config(self.path)['history_http']['bind'],'127.0.0.1')
def test_token_file_private(self):
    self.assertEqual((self.state/'history'/'http-token').stat().st_mode&0o777,0o600)
```

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_history_servers -v`
- [ ] **Step 3: 实现**（HTTP 测试用 `make_server(..., port=0)` 在线程中运行；MCP 测试用子进程 stdin/stdout）
- [ ] **Step 4: 运行确认通过** — Expected: OK

---

### Task 10: 迁移与管理命令

**Files:**
- Create: `scripts/wechat_bot/history/migrate.py`
- Modify: `scripts/wechat_bot/control.py`
- Test: `tests/test_history_migrate.py`

**Interfaces:**
- Consumes: `HistoryStore`、`load_config`
- Produces:
  - `migrate(config: dict) -> dict`，返回 `{'contacts':n,'moved_dirs':n,'imported_sends':n,'baseline_messages':n}`，按设计 §9.3：
    1. 服务运行中（`control.loaded()` 为真，或 `state_dir/service.lock` 能被 `flock` 检测到占用）→ `RuntimeError('stop the bot first')`。
    2. 每个旧目录（64 位十六进制名，含 `binding.json`）→ `ensure_contact(account, {'recipient': binding['recipient'], 'kind':'private'})`，目录改名为 `sessions/<contact_id>`，在 `sessions/` 写 `MOVED-<旧名>-TO-<contact_id>.txt`。
    3. 按流水 `created_at` 升序导入：`input` 每项 → `capture=imported`、`sender=customer` 消息；`reply`（若有）→ `record_bot_reply` + `update_send(status, echo_observed=client_echo_observed)`；按 `send_key` 幂等。
    4. 若无水位：`cursor.json` 的可见气泡以 `capture=baseline` 写入（`meta.may_duplicate_imported=true`）并设水位。
    重复运行不重复导入、不重复迁移。
  - `control` 子命令：`migrate-history`；`history-status`（库文件大小、消息总数、每联系人消息数与最后时间、未匹配别名的 `observed` 名称）；`merge-contact --from X --into Y [--note N]`、`unmerge-contact --merge-id N`（X/Y 为名称（按 `name_key`）或 `c_` 开头的 ID）；`history-backup --to PATH`（`sqlite3.Connection.backup`，目标 0600，目标已存在则拒绝）。

- [ ] **Step 1: 写失败测试**

```python
def test_migrate_moves_dirs_imports_and_baselines(self):
    self.make_legacy_session('午夜咖啡',sends=[('k1','问1','答1',1.0),('k2','问2','答2',2.0)],cursor=['问2','答2','问3'])
    stats=migrate(self.config)
    store=HistoryStore.open(self.state);cid=store.resolve('acct','午夜咖啡')
    self.assertTrue((self.state/'sessions'/cid/'cursor.json').exists())
    self.assertEqual(self.texts(cid),['问1','答1','问2','答2','问2','答2','问3'])
    self.assertEqual(self.captures(cid),['imported']*4+['baseline']*3)
    self.assertEqual(stats['imported_sends'],2)
def test_migrate_is_idempotent(self):
    self.make_legacy_session('a',sends=[('k1','q','r',1.0)],cursor=['q'])
    migrate(self.config);n=self.count();migrate(self.config)
    self.assertEqual(self.count(),n)
def test_migrate_refuses_while_service_running(self):
    with open(self.state/'service.lock','a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        with self.assertRaisesRegex(RuntimeError,'stop the bot'): migrate(self.config)
def test_merge_unmerge_and_backup_commands(self):
    ...  # 调用 control.main(['merge-contact','--from','乙','--into','甲']) 等，断言 resolve 结果与备份文件 0600 且可打开
```

- [ ] **Step 2: 运行确认失败** — Run: `python3 -m unittest tests.test_history_migrate -v`
- [ ] **Step 3: 实现**（`control.main` 若当前不接受 argv 参数，改为 `main(argv=None)`）
- [ ] **Step 4: 运行全部测试** — Run: `python3 -m unittest discover -s tests -p 'test_*bot*.py' && python3 -m unittest discover -s tests -p 'test_history_*.py' && python3 -m unittest tests.test_agent_bridge`；Expected: OK

---

### Task 11: 实机上线与文档

**Files:**
- Modify: `docs/allowlist-bot.md`（新增“对话历史”一节：存储位置、配置字段 `context_messages` / `aliases` / `history_http`、推荐改名流程、管理命令、MCP 配置示例与 HTTP `curl` 示例、设计 §13 限制）

- [ ] **Step 1: 停服务并备份状态目录**：`control stop`；`cp -Rp "<state_dir>" "<state_dir>.before-history-<时间戳>"`
- [ ] **Step 2: 运行迁移**：`python3 -m scripts.wechat_bot.control migrate-history`
Expected: 输出统计；两个会话目录改为 `c_…`；`history-status` 显示导入的 Bot 回复数等于现有流水中有 `reply` 的条数。
- [ ] **Step 3: 部署 Task 7 的 worker（若尚未部署）并启动 Bot**：`control start`，观察 2 分钟日志。
Expected: 无 `history_write_failed`；两个会话 `watching`；worker 正常启动（令牌未变）。
- [ ] **Step 4: 请用户从另一个号向“午夜咖啡”发两条消息**。
Expected: Bot 回复；`history-status` 消息数增加；`get_recent_messages` 末尾为两条 `customer` 与 `bot` 回复（`send.status=triggered`）。
- [ ] **Step 5: 安装并验证 HTTP**：`control history-http install && control history-http start`；
`curl -s -H "Authorization: Bearer $(cat '<state_dir>/history/http-token')" http://127.0.0.1:8765/v1/contacts` 返回两个联系人；不带令牌返回 401。
- [ ] **Step 6: 验证 MCP**：`printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | python3 -m scripts.wechat_bot.history.mcp_server` 列出 7 个工具。
- [ ] **Step 7: 更新文档**，汇报实际测试结果、风险与限制。
