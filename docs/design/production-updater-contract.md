# I5：完整应用版本包与可靠更新核心契约设计

日期：2026-11-10  
状态：契约设计提案，提交协调者审核；尚未放行生产部署与真实发布。

---

## 1. 依赖与 Crate 审计

### 1.1 依赖复用与新增原则
- 严格遵循零引入未审核外部依赖原则。
- **RFC 8785 JSON 规范化**：直接复用已有直接依赖 `serde_json_canonicalizer = "=0.3.2"`（MIT），生成确定性签名字节。
- **SHA-256 哈希计算**：直接复用已有直接依赖 `sha2 = "0.11.1"`（MIT/Apache-2.0）。
- **文件与路径安全**：直接复用已有直接依赖 `libc = "=0.2.189"`（MIT/Apache-2.0）与标准库 `std::fs`。
- **Ed25519 签名验证**：
  - 申请显式直接引入已有锁定传递依赖 `ring = { version = "=0.17.14", default-features = false }`（ISC/MIT 许可证，已在 `Cargo.lock` 中通过 `reqwest -> rustls -> ring` 存在）。
  - 用途：仅在验证阶段调用 `ring::signature::UnparsedPublicKey::verify` 检验 Ed25519 发布者签名；不引入新的外部 crate，不开启无关特性。
- **解包处理**：
  - 采用轻量受控解包，或使用系统原生 `/usr/bin/tar` 进行预检解包（限制最大条目数、总展开大小），并在解包后逐文件用 Rust 进行沙箱级别严格校验（绝对路径、跨越、软硬链、权限位、sha256 与 mode 白名单）。

---

## 2. 发布清单（Manifest）与签名封套（Envelope）闭合契约

### 2.1 签名封套结构与域分离（Domain Separation）
签名封套严禁信任发布包自带的公钥；发布包仅携带 `key_id`、`signature_algorithm` 与 `signature`。
客户端内置受信根公钥表 `TrustedKeyStore`（支持版本化、有效期与撤销列表）。

**封套结构（Envelope Schema v1）**：
```json
{
  "envelope_schema": 1,
  "key_id": "wechat-mcp-release-2026-v1",
  "signature_algorithm": "ed25519",
  "signature": "<base64_encoded_64_bytes_signature>",
  "manifest": { ... }
}
```

**签名字节计算与域分离**：
为防止跨协议/跨类型签名重放，签名字节采用带固定域前缀的规范化格式：
```text
sign_bytes = "WECHAT_MCP_UPDATE_MANIFEST_V1\n" + canonicalize(manifest_json)
```
其中 `canonicalize` 严格遵循 RFC 8785 规则（通过 `serde_json_canonicalizer`）。

### 2.2 发布清单闭合字段（Manifest Schema v1）
清单只允许受控闭合字段，拒绝未知关键属性：
```json
{
  "manifest_schema": 1,
  "release_id": "0.2.0-20261110",
  "sequence": 1002,
  "channel": "stable",
  "published_at": 1794297600,
  "expires_at": 1825833600,
  "target": {
    "os": "macos",
    "arch": "arm64",
    "min_os_version": "12.0",
    "max_os_version": null
  },
  "artifact": {
    "filename": "wechat-mcp-0.2.0-macos-arm64.tar.gz",
    "sha256": "3a8b...64hex",
    "size_bytes": 45123980,
    "max_unpacked_bytes": 150000000,
    "max_file_count": 500
  },
  "components": [
    {
      "path": "bin/wechat-mcp",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "4c9d...64hex",
      "size_bytes": 18450112
    },
    {
      "path": "bin/wechat-updater",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "7f1a...64hex",
      "size_bytes": 4200192
    },
    {
      "path": "bin/wechat_bot_worker",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "1a2b...64hex",
      "size_bytes": 5242880
    },
    {
      "path": "bin/wechat_click",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "8d3e...64hex",
      "size_bytes": 2097152
    },
    {
      "path": "bin/desktop_helper",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "5b4c...64hex",
      "size_bytes": 3145728
    },
    {
      "path": "resources/gateway.example.json",
      "kind": "regular_file",
      "file_mode": 420,
      "sha256": "9e2a...64hex",
      "size_bytes": 1024
    }
  ],
  "compatibility": {
    "config_schema_min": 1,
    "config_schema_max": 2,
    "profile_schema_min": 1,
    "profile_schema_max": 2,
    "history_schema_min": 1,
    "history_schema_max": 3,
    "min_updater_version": 1,
    "in_place_rollback_supported": true
  },
  "release_notes": "版本更新说明：安全解包与生命周期协调。"
}
```

### 2.3 兼容性与防降级规则
1. **架构与系统匹配**：
   - `target.os == "macos"`；
   - `target.arch` 必须精确匹配本机体系（`uname -m`：`arm64` 或 `x86_64`），禁止跨架构混用（禁止在 arm64 上静默运行 x86_64 二进制）；
   - 当前系统版本必须 `>= min_os_version`。
2. **防降级（Anti-Rollback）**：
   - 本机在受保护 state 目录持久化记录 `highest_accepted_sequence`。
   - 候选安装包的 `sequence` 必须严格大于 `highest_accepted_sequence`。
   - 若 `sequence <= highest_accepted_sequence`，除非携带经受信任根签名的显式 `rollback_authorization`，否则直接拒绝（报 `downgrade_rejected`）。
   - 相同 sequence 但不同 sha256 摘要：直接拒绝并报错（报 `digest_mismatch_for_sequence`）。
   - 跨 channel 重放（如 beta channel 签发的包提交给 stable）：直接拒绝。

---

## 3. 安全解包与组件检验

解包位于专用隔离暂存目录 `update-transactions/<tx-id>/staging/`（权限 `0700`）。
严格执行安全检验策略：
1. **拒绝路径穿越**：相对路径中任何段包含 `..`、以 `/` 起始、包含 `\` 或 NUL 字符立即中止。
2. **拒绝大小写归一冲突与重复路径**：统一使用 UTF-8 NFC 规范化；同一个包内不得出现冲突路径。
3. **拒绝软链接与硬链接**：严禁创建 symlink、hardlink、fifo、socket、设备节点。
4. **资源上限防护（压缩炸弹防御）**：
   - 解包条目总数超过 `artifact.max_file_count`（默认 500）时中止；
   - 解包总字节数超过 `artifact.max_unpacked_bytes` 时中止；
   - 单文件解包超出预期大小时中止。
5. **权限位限制**：
   - 可执行文件严格限制为 `0755`（rwxr-xr-x）；
   - 普通文件严格限制为 `0644`（rw-r--r--）；
   - 禁止 setuid (`04000`)、setgid (`02000`)、sticky (`01000`) 及 group/other 可写位。
6. **组件白名单与摘要匹配**：
   - 解包后的文件集合必须与 `components` 清单中列出的相对路径完全一致，不能有多余未知文件；
   - 逐文件计算 SHA-256 并核对大小与权限，任一不符全量清理暂存并终止事务。

---

## 4. 本机重签名与 TCC 策略

1. **发布者签名验证先于本机重签名**：
   - 原始发布包解包后，首先确认各文件 sha256 完整通过发布者签名校验；
   - 严禁对未验签或验签失败的文件执行重签名。
2. **本机专用钥匙串重签名**：
   - 使用目标机受保护钥匙串中的 "WeChat CS Local Signing" 本机证书执行 `codesign --force --sign ...`；
   - 重签名后立即执行 `codesign --verify --verbose=2` 核验签名有效性；
   - 若钥匙串处于锁定状态、证书不存在或 codesign 返回非零退出码：
     - 中止事务；
     - 清理暂存区；
     - 保持现有稳定运行版本不变；
     - 记入日志与状态返回 `local_signing_failed`。
3. **TCC 权限声明**：
   - 明确重签名保持相同签名证书身份，但不能保证 macOS 系统 TCC 无需重新提示授权；
   - 若更新后权限缺失，保持维护态暂停，不盲目尝试重启循环。

---

## 5. 更新事务日志（Update Journal）与断电恢复状态机

### 5.1 事务目录与持久化结构
- 事务日志文件路径：`state/update-journal.json`（权限 `0600`，父目录 `0700`）。
- 写入规则：采用原子写入模式（写入同目录临时文件 `update-journal.json.tmp.<uuid>` -> `fsync` -> `rename` -> 父目录 `fsync`）。
- 并发防护：跨进程文件排他锁 `update.lock`，任何时刻只允许单一更新事务执行。

### 5.2 状态机阶段（Stages）
```text
Idle
  │
  ▼
Staged (已解包并完成组件白名单及原始摘要校验)
  │
  ▼
PublisherVerified (发布者签名及清单各约束校验通过)
  │
  ▼
LocalSigned (本机代码重签名及 codesign 校验通过)
  │
  ▼
Draining (持有 I4 core.maintenance 标志，核心已暂停接单并确认停止)
  │
  ▼
BackupVerified (已完成当前完整数据备份 v3 并核验证书/摘要)
  │
  ▼
Switching (原子切换 current 指针至 releases/<new_release_id>)
  │
  ▼
CandidateTesting (候选维护态自检：组件版本一致、配置 schema 兼容、worker 连接)
  │
  ├─ 自检通过 ──► Committed (清理旧事务，提升 highest_accepted_sequence，解除维护)
  │
  └─ 自检失败 ──► RollbackStarted
                     │
                     ▼
                  RolledBack (恢复 current 指针指向 previous，解除维护)
                     │
                     ▼ (若发现切换后已产生新业务数据)
                  RecoveryRequired (严禁静默覆盖新数据，保持维护暂停待人工介入)
```

### 5.3 断电与崩溃恢复规则
系统或更新器启动前，必须首先检查 `update-journal.json`：
1. **`Staged` / `PublisherVerified` / `LocalSigned`**：尚未影响当前运行版本。丢弃候选暂存，移除 journal，恢复正常运行。
2. **`Draining` / `BackupVerified`**：检查当前进程状态与 I4 `core.maintenance` 标志。若尚未切换版本指针，保留旧版本运行，安全解除维护标志。
3. **`Switching`**：检查实际 `current` 软链目标。若指针指向新版本但尚未自检，进入 `CandidateTesting`；若指向旧版本，继续旧版本。
4. **`CandidateTesting`**：重跑自检。若通过则 Commit；若失败则执行 Rollback。
5. **`RollbackStarted`**：确认 `current` 指针已切回 `previous`，验证数据未被破坏后进入 `RolledBack`。
6. **`RecoveryRequired` 或 Journal 损坏/反序列化失败/字段非法**：
   - **Fail Closed**：立即写入/保留 `core.maintenance` 持久标志，阻止 Supervisor 拉起核心；
   - 保持只读状态，向 UI/CLI 输出具体损坏证据与人工恢复指引；
   - 严禁自行猜测版本或强制启动最高目录。

---

## 6. 只读控制接口契约（`wechat-mcp control` 扩展）

### 6.1 `wechat-mcp control update-status`
- **输入**：`--config <config_path>`
- **输出 JSON**：
```json
{
  "current_release": {
    "release_id": "0.1.0-20261108",
    "sequence": 1001,
    "architecture": "arm64",
    "components_consistent": true
  },
  "candidate_release": null,
  "transaction": {
    "status": "idle",
    "transaction_id": null,
    "stage": null,
    "error": null,
    "recovery_required": false
  },
  "guard": {
    "highest_accepted_sequence": 1001,
    "maintenance_active": false
  }
}
```

### 6.2 `wechat-mcp control update-check`
- **输入**：`--config <config_path> [--manifest-path <local_path>]`（首期限定本地合成清单或离线包路径）
- **输出 JSON**：
```json
{
  "status": "available",
  "candidate": {
    "release_id": "0.2.0-20261110",
    "sequence": 1002,
    "published_at": 1794297600,
    "size_bytes": 45123980,
    "release_notes": "...",
    "compatible": true,
    "requires_local_signing": true
  },
  "verification": {
    "publisher_verified": true,
    "key_id": "wechat-mcp-release-2026-v1"
  }
}
```
若版本过低、架构不符或签名错误：
```json
{
  "status": "incompatible",
  "reason": "downgrade_rejected: candidate sequence 1000 <= current 1001",
  "candidate": null
}
```

---

## 7. 代码与文件所有权规划

- `src/updater/mod.rs`：更新器模块入口与公共类型定义；
- `src/updater/manifest.rs`：Manifest 封套反序列化、RFC8785 域分离与签名校验、白名单与大小校验；
- `src/updater/unpack.rs`：受控安全解包与逐文件 SHA-256 校验；
- `src/updater/journal.rs`：事务日志原子写入、阶段流转与断电恢复状态机；
- `src/updater/signing.rs`：本地 codesign 重签名适配器；
- `src/updater/rollback.rs`：备份联动与防止静默覆盖新数据回退检查；
- `src/bin/wechat-updater.rs`：独立更新器 CLI 工具；
- `src/core/control.rs`：仅扩展 `update-status` / `update-check` 只读派发；
- `tests/updater_tests.rs`：更新核心隔离测试（包含损坏包、路径穿越、防降级、事务断电、数据保留回退测试）。
