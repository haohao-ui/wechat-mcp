# P1 统一眼和手 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 所有操作微信的按键和剪贴板读写都由 Swift 动作组件完成。Rust 里不再有 System Events 按键（`keystroke` / `key code`），也不再用 `pbcopy` 发送正文。

**Architecture:**
- `wechat_click`（Swift，链接 `native/action`）新增三个命令：
  - `input-text`：点输入框 → ⌘A → 写剪贴板 → ⌘V → 读回核对；核对不一致就清空；
  - `clear-input`：⌘A 加 ⌫；
  - 更多按键组合。
- 剪贴板用 `NSPasteboard` 直接读写，并恢复原内容。
- Rust 负责标题核对、发件箱记账和回车时机，通过子进程调用这些命令。
- 到 P2，同一套 Swift 代码改成 worker 的操作。

**Tech Stack:** Swift 5（macOS 12, x86_64）、Rust（tokio）、Python unittest（编译 Swift 并跑 plan-only 测试）。

**Spec:** `docs/superpowers/specs/2026-11-07-rust-core-pluggable-brain-design.md`（第 3 节、第 6 节 P1）

**真机依据:** `docs/observations/2026-11-06-wechat-4.1.15-actions.md` 第 6 节：
- ⌘A 只选中输入框；
- ⌘V 整体替换；
- ⌘A 加 ⌫ 清空；
- 输入框为空时显示占位提示。

## 对设计的一处调整（Ruling）

设计里写的是"P1 中 Rust 的 send_message 改为调用 worker"。但 worker 同一时间只服务一个连接，P1 期间这个连接被 Python 机器人占着，所以 Rust 改为调用 `wechat_click` 的新命令。

- 手的代码仍然只有 Swift 一份，即 `native/action` 加 `native/click`；
- P2 由 Rust 服务接管 worker 后，再把这几个命令改成 worker 的操作；
- 代价：P1 期间仍然有两个进程（worker 和 wechat_click）先后操作同一个窗口。这和今天的状态一样，由 Python 按顺序调用来保证不会同时操作。

## Global Constraints

- 目标平台 macOS 12 x86_64：`swiftc -target x86_64-apple-macos12`；Rust 交叉编译用 `MACOSX_DEPLOYMENT_TARGET=12.0`。
- 只有微信 `com.tencent.xinWeChat` 在最前面时才允许按键，否则退出码为 3，stderr 输出 `wechat_click: frontmost_not_wechat`。
- 不改 MCP 工具名、参数和返回结构，也不改数据库结构。
- 错误文本里绝不出现正文内容，只能写字数和"包含正文 / 不含正文"。
- 已有的 `wechat_click <x> <y> [<w> <h>]`、`--plan-only`、`key <combo>` 行为保持不变。

## Review Focus

1. 剪贴板里原本是图片或文件时，`input-text` 结束后原内容必须**完整恢复**（所有类型都要恢复，不只是文字）。
2. 读回核对失败时，输入框必须被清空；同时**剪贴板也要先恢复**，否则用户的剪贴板会被换成机器人的回复。
3. 正文里有换行、表情、`\r\n` 时，核对规则要和现在 Rust 的 `same_input_text` 一样：只统一换行符，其余逐字比较。
4. 粘贴之后，读回之前，如果前台程序变了，要直接中止，不能继续按 ⌘A ⌘C。
5. Rust 在 `input-text` 成功之后、按回车之前，如果标题核对失败，要调用 `clear-input`，不能把正文留在输入框里。

---

### Task 1: 动作组件补充按键，并新增 `input-text` 和 `clear-input` 命令（Swift）

**Files:**
- Modify: `native/action/Actions.swift`：`keyCombos` 增加 `"delete": (51,false)`、`"escape": (53,false)`、`"cmd+1": (18,true)`
- Create: `native/click/Input.swift`：剪贴板抽象加 `input-text` / `clear-input` 的流程
- Modify: `native/click/main.swift`：分发 `input-text`、`clear-input`
- Modify: `scripts/deploy_host.sh`：编译 wechat_click 时带上 `native/click/*.swift`；同时检查 `tests/test_action_module.py` 的 swiftc 源文件列表
- Test: `tests/test_action_module.py`（`WechatClickCliTests`）

**Interfaces:**
- Produces（命令行约定，Task 2 依赖）：
  - `wechat_click input-text <x> <y>`：从 stdin 读取 UTF-8 正文，长度 1 到 16384 字节。
    - 退出码 0：输入框里的内容和正文完全一致；剪贴板已经恢复；没有按回车。
    - 退出码 4：读回不一致。输入框已清空，剪贴板已恢复。stderr 输出 `wechat_click: input_readback_mismatch: <detail>`，`<detail>` 的格式和 Rust 的 `readback_mismatch_detail` 一样：`复制未取到输入框内容，应为 N 字`，或者 `读回 M 字，应为 N 字，包含正文/不含正文`。
    - 退出码 3：前台不是微信，什么都没按。
    - 退出码 1：参数错误、没有辅助功能权限、事件发送失败。
  - `wechat_click clear-input`：按 ⌘A 再按 ⌫。退出码 0、3 或 1。
  - `wechat_click key <combo>`：`combo` 取以下之一：`cmd+v` `cmd+a` `cmd+c` `return` `delete` `escape` `cmd+1`。
- `--plan-only` 对以上命令同样有效：只输出 JSON，包括 `events` 和 `pasteboard`（剪贴板的写入和恢复顺序），不真正发送事件，也不碰真实剪贴板。
  - 环境变量 `WECHAT_PLAN_READBACK=match|mismatch|sentinel` 用来模拟读回结果，默认 `match`。

- [ ] **Step 1: 写失败测试**（追加到 `WechatClickCliTests`；需要 `import os`）

```python
def test_input_text_plan_replaces_then_reads_back(self):
    run = subprocess.run([str(self.cli), '--plan-only', 'input-text', '500', '600'], input='你好\n第二行',
                         capture_output=True, text=True, timeout=30)
    self.assertEqual(run.returncode, 0, run.stderr)
    out = json.loads(run.stdout)
    downs = [e['code'] for e in out['events'] if e['type'] == 'key_down' and e['code'] != 55]
    self.assertEqual(downs, [0, 9, 0, 8])          # ⌘A ⌘V ⌘A ⌘C, no Return, no Delete
    self.assertEqual(out['events'][0]['type'], 'moved')  # click first
    self.assertEqual([p['op'] for p in out['pasteboard']], ['save', 'write_text', 'write_sentinel', 'read', 'restore'])

def test_input_text_mismatch_clears_and_exits_4_without_text(self):
    env = dict(os.environ, WECHAT_PLAN_READBACK='mismatch')
    run = subprocess.run([str(self.cli), '--plan-only', 'input-text', '500', '600'], input='秘密正文',
                         capture_output=True, text=True, timeout=30, env=env)
    self.assertEqual(run.returncode, 4)
    self.assertIn('input_readback_mismatch', run.stderr)
    self.assertNotIn('秘密正文', run.stderr)
    out = json.loads(run.stdout)
    downs = [e['code'] for e in out['events'] if e['type'] == 'key_down' and e['code'] != 55]
    self.assertEqual(downs[-2:], [0, 51])          # ⌘A ⌫ at the end
    self.assertEqual([p['op'] for p in out['pasteboard']].count('restore'), 1)

def test_input_text_sentinel_reports_copy_missed(self):
    env = dict(os.environ, WECHAT_PLAN_READBACK='sentinel')
    run = subprocess.run([str(self.cli), '--plan-only', 'input-text', '500', '600'], input='你好',
                         capture_output=True, text=True, timeout=30, env=env)
    self.assertEqual(run.returncode, 4)
    self.assertIn('复制未取到输入框内容，应为 2 字', run.stderr)

def test_input_text_rejects_empty_or_huge_body(self):
    for body in ('', 'x' * 16385):
        run = subprocess.run([str(self.cli), '--plan-only', 'input-text', '500', '600'], input=body,
                             capture_output=True, text=True, timeout=30)
        self.assertEqual(run.returncode, 1, len(body))

def test_clear_input_plan(self):
    run, out = self.plan('clear-input')
    self.assertEqual(run.returncode, 0, run.stderr)
    self.assertEqual([e['code'] for e in out['events'] if e['type'] == 'key_down' and e['code'] != 55], [0, 51])

def test_new_key_combos(self):
    self.assertEqual(self.keys('delete'), [('key_down', 51, False), ('key_up', 51, False)])
    self.assertEqual(self.keys('escape'), [('key_down', 53, False), ('key_up', 53, False)])
    self.assertEqual(self.keys('cmd+1')[1], ('key_down', 18, True))
```

（`self.plan`、`self.keys` 都是已有的辅助函数。如果编译需要新文件，就在 `setUpClass` 的 swiftc 参数里加上 `native/click/*.swift`。）

- [ ] **Step 2: 运行，确认失败**

运行：`python3 -m unittest tests.test_action_module.WechatClickCliTests -v`
预期：上面 6 个新测试 FAIL 或 ERROR（命令还不存在）；已有测试 PASS。

- [ ] **Step 3: 实现**
  - `Input.swift` 里定义：
    - `protocol Pasteboard { func save() -> Saved; func writeText(_:); func readText() -> String?; func restore(_ :Saved) }`；
    - 真实实现基于 `NSPasteboard.general`。`save` 要保存每个 item 的每种类型的 data，保证图片和文件都能原样恢复；
    - plan 实现只记录操作顺序，读回结果由 `WECHAT_PLAN_READBACK` 决定。
  - 流程：
    1. `click(12×12 at x,y)`；
    2. 确认前台是微信；
    3. `save`；
    4. ⌘A；
    5. `writeText(正文)`，再 ⌘V，等 0.3 秒；
    6. `writeText(哨兵)`，再 ⌘A ⌘C，等 0.15 秒；
    7. `read`；
    8. `restore`；
    9. 比较。比较规则：两边都先把 `\r\n` 和 `\r` 统一成 `\n`，然后严格相等。不一致时按 ⌘A ⌫，再退出 4。
  - 在 ⌘A ⌘C 之前，以及清空之前，都要再确认一次前台是微信；不是就退出 3，但退出前同样要恢复剪贴板。
  - 无论走哪条路径，`restore` 都只执行一次。

- [ ] **Step 4: 运行，确认通过**

运行：`python3 -m unittest tests.test_action_module -v`
预期：全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add native/action native/click tests/test_action_module.py scripts/deploy_host.sh
git commit -m "feat: wechat_click 新增 input-text / clear-input——NSPasteboard 原生读写并完整恢复，全选替换后读回核对，不一致即清空（依据真机观察第 6 节）"
```

---

### Task 2: Rust 改用新命令，去掉全部 System Events 按键

**Files:**
- Modify: `src/wechat.rs`：`send_message`、`send_file`、`key_press` 周边，⌘1 聚焦（约第 184 行），Esc（约第 484 行）
- Modify: `src/wechat/navigation.rs`：搜索框的 ⌘A 和 ⌘V（约第 109 行、第 237 行）
- Test: `src/wechat.rs` 中的 `send_safety_tests`

**Interfaces:**
- Consumes：Task 1 的命令行约定，即退出码 0、3、4、1 和 stderr 格式。
- Produces：
  - `async fn input_text_with(bin: &Path, x: f64, y: f64, text: &str) -> Result<(), InputError>`，其中 `InputError::{Mismatch(String), NotFront, Failed(anyhow::Error)}`；
  - `async fn clear_input() -> Result<()>`。
- `send_message` 的流程：
  1. 打开会话，取窗口位置；
  2. `input_text`，失败就返回 `NotSent`，`Mismatch` 时错误文本为 `发送前输入正文读回不一致（<detail>），未触发发送`；
  3. 核对标题，失败就先 `clear_input` 再返回 `NotSent`；
  4. `key_press("return")`，失败返回 `Unknown`。
- 不再使用 `ClipboardGuard`，也不再调用 `run_applescript` 去按键。`set frontmost` 仍然可以用 AppleScript，它不是按键。

- [ ] **Step 1: 写失败测试**
  - `input_text_maps_exit_codes`：用假的 helper 脚本分别以 0、3、4（附带 stderr detail）、1 退出，断言 `input_text_with` 返回的变体和 detail 都正确，而且 detail 里不包含正文。
  - `input_text_passes_body_on_stdin_and_coordinates_as_args`：假脚本把参数写进一个文件，把 stdin 写进另一个文件，断言参数是 `input-text 545 609`，stdin 和正文的字节完全相同（包括中文和换行）。
  - `no_system_events_keystrokes_left`：`include_str!("wechat.rs")` 加 `include_str!("wechat/navigation.rs")`，去掉 `#[cfg(test)]` 之后的部分，断言里面不包含 `keystroke`，也不包含 `key code`。

- [ ] **Step 2: 运行，确认失败**

运行：`cargo test input_text_ no_system_events`
预期：编译失败（函数不存在），或者 `no_system_events_keystrokes_left` 失败（还有 9 处）。

- [ ] **Step 3: 实现**
  - 加 `input_text_with`、`input_text`、`clear_input`。按 Interfaces 改 `send_message`。
  - `send_file`：
    - 原来的 `clear_input` AppleScript 改成 `clear_input()`；
    - ⌘V 改成 `key_press("cmd+v")`，回车改成 `key_press("return")`；
    - `set_clipboard_file` 保持不变（AppleScript 设置剪贴板为文件，不涉及按键）。
  - 导航：
    - ⌘1 改成 `key_press("cmd+1")`，Esc 改成 `key_press("escape")`；
    - 搜索框的 ⌘A ⌘V 改成 `key_press`。搜索词写剪贴板仍然用 `clipboard::set_clipboard`，它已经固定 UTF-8。
  - `readback_mismatch_detail` 继续保留：它定义了 detail 的格式，Swift 那边要输出一样的格式。

- [ ] **Step 4: 运行，确认通过**

运行：`cargo test`
预期：全部 PASS（原有 119 个加上新增的）。

- [ ] **Step 5: 提交**

```bash
git add src/wechat.rs src/wechat/navigation.rs
git commit -m "refactor: Rust 发送与导航全部改走 wechat_click（input-text / clear-input / key），移除 System Events 按键"
```

---

### Task 3: 真机验收（由协调者执行，不派给 worker）

- [ ] `scripts/deploy_host.sh work@192.0.2.10 --worker --mcp --yes`，然后重启机器人。
- [ ] 示例群：
  - [ ] 发文字，验证带换行、带表情两种回复都能正常发出；
  - [ ] 录屏确认：输入框里先放一段草稿，机器人回复时草稿被**替换**掉，没有残留；
  - [ ] 发一个附件。
- [ ] 让一次读回失败（例如在机器人粘贴前抢走焦点），确认输入框被清空、你的剪贴板被恢复成原来的内容（图片也要能恢复）。
- [ ] 搜索兜底：打开一个不在会话列表可见区域、但在允许名单里的会话（示例群），确认搜索框输入正常。
- [ ] 在 `docs/observations` 里记录结果，然后删掉录屏帧。
