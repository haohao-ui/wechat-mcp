# 文字识别引擎切换（Vision / RapidOCR）设计

日期：2026-11-05　分支：feat/wechat-bot-history

## 目的

微信电脑上所有文字识别都能在 Apple Vision 与 RapidOCR 之间切换。默认仍是 Vision（已验证可靠）；RapidOCR 作为可选引擎，用于对比中文识别效果，或在 Vision 不可用（如升级系统后超时）时顶上。

## 目标 / 非目标 / 反目标

- 目标：一个开关切换以下全部识别——
  - bot worker：聊天区气泡、标题、会话列表、右键菜单"另存为"（`native/wechat_bubbles.swift` 的 `recognizeNow`，`native/bot/*` 均经此处）；
  - `wechat_ocr` 命令：宿主 `send_file` 核对输入框文件名、旧读取路径（`native/wechat_ocr.swift`，已有 `WECHAT_OCR_ENGINE`）。
- 非目标：`native/macos` 流式运行时（需 macOS 14+，微信电脑不用）；客户图片内容的额外文字提取（另议）；Tesseract 不删也不改。
- 反目标：切换后机器人"看起来在跑"但悄悄换回了另一个引擎而没人知道；为切换改动气泡判定规则或配色标定。

## 开关（设计者决定）

- 开关放在微信电脑上：`~/.wechat-rpa/ocr.json`，内容 `{"engine":"vision"}` 或 `{"engine":"rapidocr"}`。文件不存在 = vision。理由：识别、worker、`wechat_ocr` 都在那台电脑上，一个文件同时管到；本机 bot 配置不必知道。
- 环境变量 `WECHAT_OCR_ENGINE` 仍可覆盖（现有行为，`auto`/`vision`/`tesseract` 保留，新增 `rapidocr`）。
- worker 每次请求检查文件修改时间，改了即生效，不用重启。无效值 → 报错 `invalid_ocr_engine`，不猜。

## RapidOCR 接入

- 安装：微信电脑系统自带 `/usr/bin/python3`（3.9.6），`pip install --user` 固定版本的 `rapidocr_onnxruntime` 与 `onnxruntime==1.19.2`（已确认有 macOS x86_64 可用包；brew 的 Python 3.14 没有 onnxruntime 包，不用）。模型随包自带，识别时不联网。`install.sh` 增加可选步骤，不装也不影响 Vision。
- 辅助程序：`native/rapidocr_server.py`，装到 `~/.wechat-rpa/bin/`。按行读请求 `{"image": "<png 路径>"}`，按行返回 `{"lines":[{"text","box":[[x,y]×4],"score"}]}` 或 `{"error": "..."}`。
- worker 内常驻一个辅助进程（首次使用时启动，避免每帧冷启动）；崩溃或超时则结束它，下次重启。图片写到 `~/.wechat-rpa/ocr-tmp/`（700 权限），识别后删除。单次超时与 Vision 相同（12 秒；首次加载放宽到 60 秒）。
- `wechat_ocr` 命令是一次性进程，直接以单次模式调用辅助程序（冷启动约几秒，`send_file` 可接受）。
- 坐标换算：RapidOCR 返回四点框，换成与 Vision 相同的 `Text(body, 中心 x, 中心 y)`（像素坐标），下游不改。

## 失败处理（设计者决定）

- 选了 rapidocr 但辅助程序不可用或单次失败：本次改用 Vision，并在 worker 响应中带 `ocr_engine_used:"vision"` 与 `ocr_fallback:"<原因>"`。bot 把回退写进日志，`control status` 显示最近一次实际使用的引擎和回退次数——不静默。
- 两个引擎都失败：沿用现有 `vision_timeout` / 窗口不可用的重试处理，不新增暂停条件。

## 验收

1. 单元：辅助程序输出解析、四点框→中心坐标、开关文件读取（缺失/有效/无效）、回退标记。
2. 对比：用现有测试会话的帧（已脱敏夹具）分别以两种引擎跑 `analyzeFrame`，气泡方向、数量、文件卡片识别一致；不一致处逐条列出，由用户决定是否接受。
3. 真机（engine=rapidocr）：文字、图片、PDF 各一轮能回复；`send_file` 核对文件名通过；记录每帧识别耗时与 Vision 对比。
4. 切回 vision 后行为与现在一致。

## 留给实现者

辅助程序的具体批处理/线程设置；`control status` 字段的具体展示格式；`rapidocr_onnxruntime` 的具体固定版本（选支持 Python 3.9 的最新版）。

## 风险

- RapidOCR 的分行方式与 Vision 不同（可能把相邻文字合并或拆开），气泡归属、"第N张图片"标签、文件卡片名+大小判断可能受影响——这正是验收第 2 步要量化的。
- Intel CPU 上识别比 Vision 慢（预计每帧 1–3 秒），轮询节奏会变慢，但不影响正确性。
- 8GB 内存机器上常驻进程约占 200–400MB。
