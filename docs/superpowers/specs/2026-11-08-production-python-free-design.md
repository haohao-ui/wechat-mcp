# 微信客服助手：生产上线与完全撤除 Python —— 设计

日期：2026-11-08　分支：feat/wechat-bot-history　状态：已定方向（用户 2026-11-08 决定），待计划

## 1. 为什么做

用户原话（2026-11-08）：

> 我们现在要把这个当做一下正式生产准备上线的产品。所有的都统一。要能完全撤下 python

前提：P1（统一手）、P2（Rust 核心）、P4（外部大脑）已经开发完成，并部署到专用 Mac。但目前仍由 Python 机器人负责回复，Rust 核心还没有在线跑过。

## 2. 用户决定（2026-11-08）

| 问题 | 决定 |
|---|---|
| 文字识别的备用引擎 RapidOCR（Python） | **只用 Vision**：删除 RapidOCR 及其所有调用分支 |
| 每次更新都要重新授权 | **本机自签名证书**：专用 Mac 的钥匙串里放一个固定的代码签名证书，之后的每次构建都用它签名 |
| agent_bridge | **直接删除**，由 P4 的外部大脑（`next_turn` / `reply`）取代 |
| 测试中的 Python | **全部改成 Rust 或 Swift**，仓库里不留任何 Python |

## 3. 目标 / 非目标 / 反目标

**目标**
- G1 **运行时零 Python**：专用 Mac 上不需要 Python，也不需要任何 pip 包。
- G2 **仓库零 Python**：没有 `.py` 文件，CI 不调用 python。
- G3 **一套程序，一个入口**：
  - 微信客服助手（Swift）负责托管常驻的 `wechat-mcp serve-core`（Rust）；
  - Swift worker 和 `wechat_click` 是唯一操作微信的程序；
  - launchd 只保留助手这一个任务；
  - 部署只用一条命令。
- G4 **升级不用重新授权**：签名证书固定以后，系统授权记录不会因为升级而失效。
- G5 **生产可运维**：
  - 面板显示版本号；
  - 日志会轮换；
  - 状态和数据库可以备份、可以恢复；
  - 切换和回滚各有一条命令。
- G6 **Rust 补齐 Python 剩下的功能**：
  - 历史查询的 MCP 工具；
  - SQLite 备份；
  - 真机校准。

**非目标**
- 分发到其他人的 Mac：不做公证，不用 Developer ID；
- Windows；
- 多账号。

**反目标**
- 撤掉 Python 的过程中，客服停止回复超过一次切换窗口（几分钟）；
- 删除代码之前，没有先在真机上验证 Rust 已经能完整替代；
- 为了"统一"改变已经在真机上验证过的行为（发送路径、核对、幂等）。

## 4. 范围

### 4.1 Rust 补齐
- **历史 MCP 工具**：把 `scripts/wechat_bot/history/mcp_server.py` 的工具搬进 wechat-mcp 的工具列表，名称、参数和返回值保持一致，底层读 T2 的历史库。
- **`wechat-mcp backup`**：替代 `scripts/backup_db.py`，在有 WAL 的情况下也能拿到一致的 SQLite 备份。用 SQLite 的 backup API，覆盖历史库和网关库。
- **校准**：Rust 的 `control calibrate` 在真机上完成一次，结果和 Python 版本的 `calibrate-result.json` 对照。
- **诊断用的画面录制**：在 Rust 核心里加同样的 `frame-dump.enabled` 开关，Python 删除以后仍然能用回放来排查问题。回放工具改为"录制的画面配上 Rust 自己的日志"。

### 4.2 删除
- 删除以下代码：
  - `scripts/wechat_bot/`
  - `scripts/agent_bridge/`
  - `scripts/backup_db.py`
  - `native/rapidocr_server.py`
- `native/wechat_ocr.swift`、`native/wechat_bubbles.swift`：删除 RapidOCR 分支和 `WECHAT_OCR_ENGINE=rapidocr`，只保留 Vision。
- `scripts/install.sh`、`scripts/build.sh`、`.github/workflows/*.yml`：去掉 rapidocr、pip 和 python 相关内容。
- `scripts/deploy_host.sh`：删除 `--bot`，加入签名证书的设置。
- 专用 Mac 上：
  - 移除 launchd 任务 `local.wechat-rpa.bot` 和 `~/.wechat-rpa/bot`；
  - 卸载用户目录下的 rapidocr 和 onnxruntime。卸载前先列出清单，经用户确认后再删。

### 4.3 测试迁移（Python → Rust / Swift）
- 机器人的测试：P2 已经翻译成 Rust 测试，直接删掉 Python 原件。
- Swift 组件的测试（动作组件、宿主、气泡、OCR、屏幕、锚点、布局、表情词库），现在都由 Python 编译 Swift 并检查输出。改为 **Rust 集成测试**（`tests/*.rs`）：
  - 用 `std::process::Command` 调用 `swiftc` 编译自测程序；
  - 运行后用 `serde_json` 检查输出；
  - 断言内容逐条照搬。

  这样 `cargo test` 就是唯一的测试入口。
- 协议冒烟测试（`protocol_smoke.py`、`desktop_runtime_smoke.py`）改成 Rust 集成测试，启动构建好的 `wechat-mcp` 跑 JSON-RPC 往返。

### 4.4 签名与部署
- 专用 Mac 上：
  - 一次性生成自签名的代码签名证书 `WeChat CS Local Signing`，放进登录钥匙串，并设置为允许 `codesign` 使用；
  - 之后所有构建产物都用它签名：助手 App、`wechat_bot_worker`、`wechat_click`、`wechat-mcp`。

  证书不变，系统授权所依据的签名要求就不变，升级后不用重新授权。
- 部署：
  - `deploy_host.sh <host> --yes` 一条命令完成构建、签名、安装、重启；
  - 每次部署都记录版本号，由 git 提交号和时间组成；
  - 面板显示当前版本号。

### 4.5 运维
- 日志轮换：`core.log` 和 `host.log` 单个文件最大 10MB，保留 5 份。
- 每天自动备份历史库和网关库，保留 7 天。由核心在空闲时执行 `backup`。
- 切换和回滚：
  - `wechat-mcp control switch-core on|off`。Python 删除以后，`off` 就不再有意义，届时一并去掉。

## 5. 上线顺序（每一步都可以回退，直到第 5 步）

1. **补齐与签名**（开发和部署，Python 继续运行）：Rust 补齐的功能，新的签名方式（最后一次需要重新授权）。
2. **切换**：
   - 停掉 Python 的 launchd 任务，创建 `core.enabled`，由 Rust 接管；
   - 在示例群做真机验收：文字、图片、文件、群聊、转人工、外部大脑、发送失败重试；
   - 发现问题就立即切回 Python。
3. **观察期**：Rust 负责回复 3 天，每天检查日志和健康状态。Python 的文件原样保留，用于回退。
4. **删除代码**：仓库里删除 Python 并迁移测试（这一步可以和第 2、3 步并行开发，但要等第 3 步通过后才合并）。
5. **清理专用 Mac**：删除 Python 的任务、目录和 pip 包，部署只有 Rust 和 Swift 的版本。之后要回退，就只能回到上一个 git 版本。

## 6. 风险
- **自签名证书**：从 ssh 调用 `codesign` 需要钥匙串处于解锁状态。第一次签名时，系统可能会弹出"允许 codesign 访问钥匙串"的对话框，需要用户在专用 Mac 上点一次"始终允许"。
- **只用 Vision**：如果以后专用 Mac 升级到 Vision 识别有问题的系统版本，就没有备用引擎了。要求：**专用 Mac 的系统版本固定在 macOS 12**，升级前必须先验证 Vision 识别。
- **测试迁移量**：Python 驱动的 Swift 测试大约有 200 项，要逐条照搬断言，不能借机删减。
- **推送前**：git 历史里有一份没有脱敏的样本文件（提交 a073987），需要用户决定是否清理历史后再推送。这件事在上线之前必须定下来。
