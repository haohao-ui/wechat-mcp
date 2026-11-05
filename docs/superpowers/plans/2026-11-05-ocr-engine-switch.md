# 文字识别引擎切换 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 微信电脑上的全部文字识别（bot worker 与 `wechat_ocr`）可用一个开关在 Vision 与 RapidOCR 之间切换，失败回退 Vision 且可见。

**Architecture:** 新增 Python 辅助程序 `native/rapidocr_server.py`（按行 JSON，常驻或单次）。Swift 侧在 `native/wechat_bubbles.swift` 加引擎选择与常驻客户端，`recognizeNow` 按开关分派，结果统一成现有 `Text`；每次识别记录实际引擎与回退原因。worker 把记录随响应返回，Python bot 写入 health，`control status` 可见。`wechat_ocr.swift` 复用同一开关文件，以单次模式调用辅助程序。

**Tech Stack:** Swift 5.7（远端 macOS 12 Intel）、Python 3.9（远端 `/usr/bin/python3`）、`rapidocr_onnxruntime` + `onnxruntime==1.19.2`、unittest。

**Spec:** `docs/superpowers/specs/2026-11-05-ocr-engine-switch-design.md`

## Global Constraints

- 默认引擎 vision；开关文件 `~/.wechat-rpa/ocr.json`，`{"engine":"vision"|"rapidocr"}`，不存在 = vision。
- 环境变量 `WECHAT_OCR_ENGINE` 优先于文件；worker 接受 `vision`/`rapidocr`，`wechat_ocr` 另保留 `auto`/`tesseract`。无效值 → `invalid_ocr_engine`。
- 文件按修改时间缓存，改了即生效，不需重启。
- rapidocr 不可用/单次失败 → 本次用 Vision，记录 `engine_used:"vision"` 与 `fallback:"<原因>"`；不静默。
- 单次识别超时 12 秒；辅助程序首次加载上限 30 秒（低于 Python 侧 35 秒请求超时）。
- 临时图片目录 `~/.wechat-rpa/ocr-tmp/`，权限 700，识别后删除。
- 测试/路径覆盖用环境变量：`WECHAT_OCR_CONFIG`（开关文件路径）、`WECHAT_RAPIDOCR_PYTHON`（默认 `/usr/bin/python3`）、`WECHAT_RAPIDOCR_SCRIPT`（默认 `~/.wechat-rpa/bin/rapidocr_server.py`）、`WECHAT_OCR_TMP`。
- 不改气泡判定、配色标定、Tesseract 逻辑；不碰 `native/macos`。
- Swift 代码须能被远端 Swift 5.7 编译（不用 5.9+ 语法，如 `if` 表达式、`consume`）。

## Review Focus

1. 辅助程序启动后崩溃或卡住：下一次识别应重启它并回退 Vision，不能把 worker 卡死超过 12 秒。
2. 辅助程序输出夹杂非 JSON 行（例如库打印的警告）：应跳过非 JSON 行，不当成识别结果或报错退出。
3. 开关文件被改成无效内容：worker 请求返回 `invalid_ocr_engine`，不回退、不猜。
4. 空白图片（RapidOCR 返回 None）：结果为空数组，不是失败、不触发回退。
5. 识别文字含换行或制表符：JSON 往返不丢字，`Text.body` 与 Vision 语义一致（单行）。

---

### Task 1: RapidOCR 辅助程序

**Files:**
- Create: `native/rapidocr_server.py`
- Test: `tests/test_rapidocr_server.py`

**Interfaces:**
- Produces: 命令 `python3 rapidocr_server.py`（常驻：每行读 `{"image": "<path>"}`，每行写一条 JSON）与 `python3 rapidocr_server.py --once <path>`（写一条后退出）。启动加载完成后先输出 `{"ready": true}`（常驻与单次都输出）。
- 输出：`{"lines":[{"text":str,"box":[[x,y],[x,y],[x,y],[x,y]],"score":float}]}` 或 `{"error":str}`。
- Python 内部：`handle(request: dict, engine) -> dict`，`engine(path)` 返回 RapidOCR 的 `(result, elapse)`，result 为 `[[box, text, score], ...]` 或 None。

- [ ] **Step 1: 写失败测试**

```python
class HandleTests(unittest.TestCase):
    def test_lines_are_converted(self):
        engine = lambda path: ([[[[1,2],[11,2],[11,8],[1,8]], '你好', 0.98]], 0.1)
        self.assertEqual(server.handle({'image':'/x.png'}, engine),
                         {'lines':[{'text':'你好','box':[[1,2],[11,2],[11,8],[1,8]],'score':0.98}]})
    def test_blank_image_is_empty_not_error(self):
        self.assertEqual(server.handle({'image':'/x.png'}, lambda p:(None,0.0)), {'lines':[]})
    def test_bad_request_and_engine_error(self):
        self.assertIn('error', server.handle({}, lambda p:(None,0)))
        def boom(p): raise RuntimeError('model')
        self.assertEqual(server.handle({'image':'/x.png'}, boom), {'error':'RuntimeError: model'})
    def test_serve_loop_writes_ready_then_one_line_per_request(self):
        # serve(stdin, stdout, engine): ready first, skips blank lines, one JSON line per request
        ...
```

`server` 通过 `importlib` 按路径加载 `native/rapidocr_server.py`；模块导入时不得导入 rapidocr（只在 `main()` 里导入）。

- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_rapidocr_server -v` → 失败（文件不存在）
- [ ] **Step 3: 实现** `handle`、`serve(stdin, stdout, engine)`、`main()`；`main` 中 `from rapidocr_onnxruntime import RapidOCR`，失败则输出 `{"error":"rapidocr_unavailable: ..."}` 并以 2 退出。分数转 float、坐标转 float，`ensure_ascii=False`，每行 flush。
- [ ] **Step 4: 运行确认通过** — 同上 → PASS
- [ ] **Step 5: 提交** `feat: RapidOCR 辅助程序（按行 JSON，常驻/单次）`

### Task 2: worker 引擎选择、常驻客户端与回退

**Files:**
- Modify: `native/wechat_bubbles.swift`（`recognizeNow` 分派；新增开关读取、`RapidOCRClient`、`ocrReport`；CLI 输出带 `ocr` 字段）
- Test: `tests/test_native_frames.py`

**Interfaces:**
- Consumes: Task 1 的辅助程序协议。
- Produces:
  - `func currentOCREngine() throws -> String`（"vision"/"rapidocr"；无效抛 `Failure.invalid("invalid_ocr_engine")`）
  - `func rapidTexts(_ lines: [[String:Any]]) -> [Text]`（四点框取中心：x 为四点 x 均值，y 为四点 y 均值；去掉首尾空白后为空的丢弃；按 y 排序）
  - 全局 `var ocrReport: [String:Any]`，形如 `["engine":"rapidocr","engine_used":"vision","fallback":"rapidocr_timeout"]`；`resetOCRReport()` 每个请求/每帧开头调用；只在有回退时带 `fallback`。
  - `analyzeFrame` 返回值不变；CLI（`BubbleCLI`）在输出 JSON 中加 `"ocr": ocrReport`。
  - `warmUpRecognizer()`：引擎为 rapidocr 时启动辅助程序并等待 `ready`（≤30 秒），否则保持现有 Vision 预热。

- [ ] **Step 1: 写失败测试**（在 `NativeFrameTests` 中，均通过环境变量控制子进程）

```python
def test_default_engine_reports_vision(self):
    frame = self.parse('r1-dark-top')
    self.assertEqual(frame['ocr'], {'engine':'vision','engine_used':'vision'})

def test_rapidocr_without_helper_falls_back_visibly(self):
    env = {'WECHAT_OCR_ENGINE':'rapidocr','WECHAT_RAPIDOCR_SCRIPT':'/nonexistent/rapidocr_server.py'}
    frame = self.parse('r1-dark-top', env=env)
    self.assertEqual(frame['ocr']['engine_used'], 'vision')
    self.assertIn('fallback', frame['ocr'])
    self.assertEqual([(b['direction'],b['kind']) for b in frame['bubbles']], [t[:2] for t in TOP])

def test_invalid_engine_is_refused(self):
    with tempfile.NamedTemporaryFile('w', suffix='.json') as f:
        f.write('{"engine":"paddle"}'); f.flush()
        self.assertIn('invalid_ocr_engine', self.parse('r1-dark-top', env={'WECHAT_OCR_CONFIG':f.name})['error'])

def test_frames_parse_the_same_with_rapidocr(self):
    # Skips unless WECHAT_RAPIDOCR_PYTHON points at a python with rapidocr_onnxruntime installed.
    # Runs every CASES entry with WECHAT_OCR_ENGINE=rapidocr + WECHAT_RAPIDOCR_SCRIPT=native/rapidocr_server.py,
    # asserts engine_used == 'rapidocr' and the same (direction, kind) sequence and title as Vision;
    # text containment mismatches are collected and printed (spec 验收 2: listed for the user), not asserted.
```

`parse(name, window=None, env=None)` 合并 `os.environ`；测试默认设 `WECHAT_OCR_CONFIG` 指向不存在的临时路径，避免读到本机真实 `~/.wechat-rpa/ocr.json`。

- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_native_frames -v` → 前三个失败（无 `ocr` 字段/未拒绝），第四个跳过
- [ ] **Step 3: 实现**
  - 开关读取：env 优先；否则读 `WECHAT_OCR_CONFIG` 或 `~/.wechat-rpa/ocr.json`，按 mtime 缓存。
  - `RapidOCRClient`：`Process` + 管道常驻；写 PNG（`CGImageDestination`）到临时目录（`0o700` 创建），发一行请求，在后台队列读一行（跳过非 JSON 行），`DispatchSemaphore` 等待 12 秒；超时/EOF/`error` → 终止进程、置空、抛出带原因的错误（`rapidocr_timeout`/`rapidocr_exited`/`rapidocr_error`/`rapidocr_unavailable`）；`defer` 删临时图。
  - `recognizeNow`：engine == rapidocr 时先试客户端，失败则记录 fallback 后走现有 Vision 代码；Vision 代码不变。
- [ ] **Step 4: 本地准备 RapidOCR 并运行** — 在 scratchpad 建 venv（python3.12，`pip install rapidocr_onnxruntime`），`WECHAT_RAPIDOCR_PYTHON=<venv>/bin/python python3 -m unittest tests.test_native_frames -v` → 全部 PASS，第四个打印不一致清单（可为空）
- [ ] **Step 5: 提交** `feat: 气泡识别可切换 RapidOCR，失败回退 Vision 并标记`

### Task 3: worker 响应、bot 记录与状态展示

**Files:**
- Modify: `native/bot/Worker.swift`（每个请求前 `resetOCRReport()`；响应加 `"ocr": ocrReport`；启动预热沿用 `warmUpRecognizer()`）
- Modify: `scripts/wechat_bot/native_capture.py`（`call` 读取 `response.get('ocr')`，存为 `self.last_ocr`；有 `fallback` 时 `logger.warning`）
- Modify: `scripts/wechat_bot/service.py`（health 写入 `ocr: {"engine_used":..., "fallbacks": <累计次数>, "last_fallback": <原因或 null>}`）
- Test: `tests/test_wechat_bot_service.py`、`tests/test_wechat_bot.py`

**Interfaces:**
- Consumes: Task 2 的 `ocrReport`、`resetOCRReport()`。
- Produces: `NativeCapture.last_ocr: dict | None`；health.json 的 `ocr` 字段（`control status` 原样输出 health，无需改 control）。

- [ ] **Step 1: 写失败测试**
  - `test_call_records_ocr_report_and_warns_on_fallback`：用现有 worker 测试替身返回 `{"ok":true,"result":{},"ocr":{"engine":"rapidocr","engine_used":"vision","fallback":"rapidocr_timeout"}}`，断言 `last_ocr` 相等且产生 warning 日志。
  - `test_health_reports_ocr_fallbacks`：service 跑两个 cycle，capture 替身 `last_ocr` 分别有/无 fallback，断言 health `ocr == {"engine_used":"vision","fallbacks":1,"last_fallback":"rapidocr_timeout"}`（`engine_used` 取最后一次报告）。
- [ ] **Step 2: 运行确认失败** — `python3 -m unittest tests.test_wechat_bot tests.test_wechat_bot_service -v` → 两个新测试失败
- [ ] **Step 3: 实现** 上述三处。
- [ ] **Step 4: 运行通过并编译 worker** — 上述测试 PASS；`swiftc -O -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/bot/*.swift -o <scratch>/wechat_bot_worker` 成功；全量 `python3 -m unittest discover -s tests` 通过
- [ ] **Step 5: 提交** `feat: worker 返回实际识别引擎，bot 记录回退并在状态中显示`

### Task 4: wechat_ocr 命令支持 rapidocr 与开关文件

**Files:**
- Modify: `native/wechat_ocr.swift`
- Test: `tests/test_native_frames.py`（新增 `WechatOcrCliTests`）

**Interfaces:**
- Consumes: Task 1 `--once` 模式；与 Task 2 相同的开关文件/环境变量规则（`wechat_ocr.swift` 是独立脚本，规则在此重写一份简版）。
- Produces: `WECHAT_OCR_ENGINE` 接受 `rapidocr`；未设环境变量时读开关文件（vision 或无文件 → 现有 `auto` 行为；rapidocr → RapidOCR 优先，失败回退现有 Vision/Tesseract 链并向 stderr 写 `ocr_fallback: <原因>`）。输出 `[TextLine]` 格式不变（`x`,`y` 为像素中心）。

- [ ] **Step 1: 写失败测试**
  - `test_cli_rapidocr_reads_title`：（需 `WECHAT_RAPIDOCR_PYTHON`，否则跳过）对 `r1-dark-top.png` 运行，某行 `text` 含 `示例联系人甲`，stderr 不含 `ocr_fallback`。
  - `test_cli_rapidocr_missing_helper_falls_back`：`WECHAT_OCR_ENGINE=rapidocr WECHAT_RAPIDOCR_SCRIPT=/nonexistent` → 退出码 0、stderr 含 `ocr_fallback`、输出含 `示例联系人甲`。
  - `test_cli_reads_engine_from_switch_file`：`WECHAT_OCR_CONFIG` 指向 `{"engine":"rapidocr"}` 且 helper 不存在 → stderr 含 `ocr_fallback`（证明文件被读取）。
- [ ] **Step 2: 运行确认失败** — 后两个失败（rapidocr 现被当作无效值）
- [ ] **Step 3: 实现** `runRapidOCR() -> [TextLine]?`（启动 `--once`，等待最多 30 秒，取 `ready` 之后第一条 JSON），接入引擎选择。
- [ ] **Step 4: 运行通过** — `WECHAT_RAPIDOCR_PYTHON=<venv>/bin/python python3 -m unittest tests.test_native_frames -v` → PASS
- [ ] **Step 5: 提交** `feat: wechat_ocr 支持 RapidOCR 与开关文件`

### Task 5: 安装、文档、远端部署与验收

**Files:**
- Modify: `scripts/install.sh`（可选步骤：`RPA_INSTALL_RAPIDOCR=1` 时用 `/usr/bin/python3 -m pip install --user rapidocr_onnxruntime==<版本> onnxruntime==1.19.2`，并拷贝 `rapidocr_server.py` 到 `~/.wechat-rpa/bin/`；默认不装）
- Modify: `README.md`（OCR 段落）、`docs/allowlist-bot.md`、`skills/wechat-operator/references/bot.md`（开关、回退、状态字段）、`docs/rpa-runtime.md`（一句覆盖范围）

- [ ] **Step 1:** `bash -n scripts/install.sh` 通过；文档更新。
- [ ] **Step 2: 远端安装**（微信电脑联网约 50MB）：pip 安装；`/usr/bin/python3 ~/.wechat-rpa/bin/rapidocr_server.py --once <一张截图>` 输出 `ready` 与 `lines`。
- [ ] **Step 3: 部署** worker 与 `wechat_ocr`（按此前部署方式在远端替换），开关保持 vision，确认 bot 正常一轮（验收 4 前半）。
- [ ] **Step 4: 对比**（验收 2）：远端两种引擎各跑一次固定截图集合（夹具 + 当次测试会话截图），记录气泡差异与每帧耗时，写入 ledger 并汇报。
- [ ] **Step 5: 真机切换**（验收 3）：写 `{"engine":"rapidocr"}`，请用户在测试会话发文字、图片、PDF 各一轮，确认回复；`status` 显示 `engine_used: rapidocr`、`fallbacks: 0`；`send_file` 核对一次（需用户指定对象）。之后按用户决定保留或切回 vision（验收 4）。
- [ ] **Step 6: 提交** `docs: RapidOCR 切换的安装与使用说明`
