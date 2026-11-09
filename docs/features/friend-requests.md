# 好友申请规则处理

配置缺省关闭。启用前建议先在专用账号上使用 dry_run 检查诊断截图，确认微信当前版本的验证表单；未知界面会记录 needs_review 并退出，不会自动重试。

```json
"friend_requests": {
  "enabled": false,
  "mode": "dry_run",
  "accept_sources": ["扫一扫", "搜索"],
  "accept_keywords": [],
  "match": "all",
  "daily_limit": 20,
  "check_interval_seconds": 120
}
```

来源和验证消息分别做包含匹配；每类空数组视为满足，all 要求两类都满足，any 满足任意一类即可。只扫描等待验证的申请，每次最多处理五个新身份；不符合的申请保留原状。已处理的申请按账号、昵称、完整验证消息、来源的摘要去重，不使用红点判断处理状态。没有新红点时，仍有等待验证申请则每三十分钟重新扫描。规则变化后可重新评估 not_matched，needs_review 和 accepted 不重试。配置会重新读取，暂停、停止、账号变化或规则变化会在下一项操作前中止。

`friend-requests.json` 使用现有原子写入函数，权限 0600，只记录摘要、长度、决策、规则命中索引和时间，不保存申请原文。live 和 dry_run 按本地自然日分别限额，账号间分别计数。进入验证界面前先持久化 needs_review，进程中断后不会再次操作不确定的申请。

验证后的截图和 OCR JSON 保存在 `diagnostics/friend-requests/<Unix时间>-<UUID>/`，目录 0700、文件 0600。诊断文件包含界面原文，仅保留在本地状态目录。只点击 OCR 定位的昵称、前往验证和许可的验证/取消按钮；dry_run 永不点击确定、完成、确认。独立窗口和主窗口都截图，窗口文字不符合已知词汇时退出并保留 needs_review，等待人工检查。

live 已确认通过后，先 contacts_leave、释放桌面租约，再提交 mode=incremental 的标准 contacts-sync 请求，由现有同步处理器另取租约执行；dry_run 和零 accepted 不同步。同步失败只记 friend_request_contacts_sync_failed，不改变通过结果。不会自动回复、加入允许名单、拒绝、删除或拉黑。

验证对话框尚无真机验收证据。保守识别可能把备注值、其他新文案或内嵌表单背景文字归为未知；这些情况需要人工审核，不代表通过成功。未启动生产机器人，也未操作真实好友申请。
