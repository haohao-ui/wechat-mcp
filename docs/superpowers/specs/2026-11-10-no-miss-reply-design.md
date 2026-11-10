# 不漏回（P0）：入站待办队列与对账设计

日期：2026-11-10
分支：`feat/wechat-bot-history`
状态：P0.1（入站落库 + 重启恢复）与 P0.2（重基线对账）已实现并验证；P0.3 暂停自愈与 P1 检测补扫见后续

## 1. 背景与边界

可见界面 + OCR 的机器人**无法保证“绝对零漏”**：截图失败、主题/布局变化、窗口被遮挡都可能看不见。本设计的目标是**不静默丢失**：

> 每条被识别到的入站消息，最终必须是以下之一：已回复（回显确认）、已发兜底话术、已转人工、已进待办队列并告警。
> 不允许“看见了却按历史吞掉、且无人知道”。

2026-11-10 真机已验证的静默丢失路径（本次要堵住的）：

| # | 路径 | 现状 |
|---|---|---|
| 1 | 建轮次后、发送前 core 重启：cursor 已推进、journal 变 `not_sent/closed_on_startup`，消息永久不再处理 | 本设计 P0.1 |
| 2 | 连续性缺口/主题切换重基线：可见消息被当历史，不回复 | 本设计 P0.2（+已有内容比对） |
| 3 | `paused`（标题/布局/模型失败）后不自动恢复 | P0.3（后续） |
| 4 | 几百联系人时被挤出会话列表可见区的未读会话探测不到 | P1 |
| 5 | 完全相同消息的歧义锚点按设计跳过 | 已知边界，不承诺 |
| 6 | 发送结果 unknown 不重发 | P0.1 的终态覆盖（记为 unresolved，需人工/告警） |

## 2. 不变式

1. **入站必落库**：检测到新入站时，先写 `pending_replies`，再推进 cursor/建轮次。
2. **回复必闭环**：每条待办必须走到终态 `replied` / `failed_explicit` / `handoff` / `unresolved`。
3. **基线必对账**：任何重基线之后，用历史库核对“最新可见入站是否已被回复”，未回复的重新入队。
4. **重启必恢复**：启动时读取未结项，能证明已回复的关闭，否则重建轮次继续回复。
5. **不可静默**：所有恢复/对账动作都写日志与 health 指标，可被运维看到。

## 3. 数据模型

`pending_replies`（migration version 5，内联于 `src/core/history/store.rs` 的 `MIGRATIONS`；v3/v4 已被媒体与 contact_profiles 迁移占用）：

| 列 | 说明 |
|---|---|
| `pending_id` | 主键 |
| `contact_id` | 外键 contacts；按联系人隔离 |
| `account_id` | 账户维度 |
| `bubble_digest` / `bubble_text` | 去重键（同联系人同气泡只保留一条 open） |
| `bubbles_json` | 单条 Bubble 的 JSON（保留 kind/media_id/phash 等，恢复时可直接重建轮次） |
| `status` | `pending` / `replied` / `failed_explicit` / `handoff` / `unresolved` |
| `attempts` / `last_error` / `first_seen_at` / `updated_at` | 重试与观测 |

唯一索引 `pending_open_once(contact_id, bubble_digest, bubble_text) WHERE status='pending'`：并发/重复检测不会产生重复待办。

## 4. 生命周期

```
检测到入站（cycle 建/扩轮次）
  └─ enqueue_pending(status=pending)          # 先落库
        ├─ 模型/发送成功 ─────────────────→ resolve_pending(replied)
        ├─ 兜底话术已发（ensure_unreadable_named）→ replied
        ├─ brain skip / handoff ───────────→ handoff
        └─ 发送 unknown / 超出重试 ────────→ unresolved（告警，人工核对）

重基线（主题切换 / 缺口恢复失败）
  └─ keep_unanswered_visible(newest inbound)
        ├─ 历史已证明有回复 → 不动
        └─ 否则 enqueue_pending(pending)      # 下一次 cycle 由轮次处理

core 重启（BotService::new）
  └─ recover_pending()
        ├─ match_recent_inbound + replied_after → replied（不重复发）
        └─ 否则 turn = 未结项气泡（frame=cursor）→ 下一轮正常生成/发送
```

## 5. 与现有机制的关系

- **cursor** 仍是“看到过什么”的视觉基线，不再承担“是否已回复”的事实；事实只认 `messages`/`sends`/`pending_replies`。
- **journal** 仍是单次发送的事实来源；`pending_replies` 是跨重启的待办事实。
- **幂等**：发送键 `batch_key` 与 `sends` 表不变；恢复路径先查历史再决定是否重新生成，降低重复回复概率（at-least-once，极少数重复可接受；漏回不可接受）。
- **health** 增加 `pending_replies: {open, oldest_ms, by_status}`，便于宿主页面/告警接入。

## 6. 验收（自动化）

1. 新入站建立轮次后，`pending_replies` 有一条 open；发送成功后变 `replied`，`open=0`。
2. 构造“已落库但未发送”的 pending + 同目录 cursor，重启 `BotService`：自动建 turn 并发出回复；若历史已有回复则只关闭待办、不重发。
3. 主题切换重基线且最新可见入站未被历史回复：重基线后待办里出现该消息，后续 cycle 回复；已回复的不会误入队。
4. `pending_summary` 的 open/oldest/by_status 与数据库一致（存储层单测）。

## 7. 非目标与后续

- P0 不解决“几百联系人时的检测覆盖”（P1：会话列表滚动补扫 + 持久全局队列）。
- 不解决“单窗口吞吐上限 ~5 条/分钟”（P2：多账号/多设备 worker 或官方客服 API）。
- 不改变“歧义重复消息按设计跳过”的既有策略。
- `paused` 自愈与告警（P0.3）单独实现：退避重试 N 次后再暂停并告警。
