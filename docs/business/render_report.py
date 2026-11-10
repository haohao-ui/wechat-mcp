#!/usr/bin/env python3
"""把 docs/business/商业化分析报告.md 渲染为带排版的 HTML，再用 Chrome 打印为 PDF。

只做本项目 Markdown 子集：标题、段落、表格、列表、代码块、引用、分隔线、粗体/行内代码/链接。
运行：python3 docs/business/render_report.py
"""

import html
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MD = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "docs/business/商业化分析报告.md"
HTML_OUT = MD.with_suffix(".html")
PDF_OUT = MD.with_suffix(".pdf")

COVERS = {
    "商业化分析报告": {
        "kicker": "COMMERCIALIZATION REVIEW",
        "title": "wechat-mcp<br>商业化分析报告",
        "sub": "个人微信 AI 客服 · macOS 桌面 Agent · 合规与路径选择",
        "meta": "版本 v1.0 ｜ 2026-11-09<br>分析对象：/path/to/wechat-mcp（HEAD cb92ce5）<br>配套：市场情报 · 合规评估 · 产品就绪度 · 财务模型<br>性质：商业尽调与决策建议，不构成法律意见",
    },
    "合规商业化方案": {
        "kicker": "COMPLIANT GO-TO-MARKET",
        "title": "wechat-mcp<br>合规商业化方案",
        "sub": "微信客服官方 API · 本地化 AI 客服中台 · 合规路径",
        "meta": "版本 v1.0 ｜ 2026-11-10<br>替代前版第 8 章路径建议；原报告作为底稿保留<br>配套：合规方案财务模型 · 微信客服 API 文档<br>性质：产品与商业方案，不构成法律意见",
    },
}
COVER = COVERS.get(MD.stem, {
    "kicker": "BUSINESS REVIEW",
    "title": MD.stem,
    "sub": "",
    "meta": "wechat-mcp 商业化系列文档",
})

CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


def inline(text: str) -> str:
    # 先保护行内代码
    codes = []

    def stash(m):
        codes.append(m.group(1))
        return f"\x00{len(codes)-1}\x00"

    text = re.sub(r"`([^`]+)`", stash, text)
    text = html.escape(text, quote=False)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"<em>\1</em>", text)
    for i, c in enumerate(codes):
        text = text.replace(f"\x00{i}\x00", f"<code>{html.escape(c)}</code>")
    return text


def parse(md: str):
    lines = md.splitlines()
    out = []
    toc = []
    i = 0
    in_code = False
    code_buf = []
    list_buf = []
    list_type = None

    def flush_list():
        nonlocal list_buf, list_type
        if list_buf:
            tag = "ol" if list_type == "ol" else "ul"
            out.append(f"<{tag}>" + "".join(list_buf) + f"</{tag}>")
            list_buf = []
            list_type = None

    def flush_code():
        nonlocal code_buf
        if code_buf:
            out.append("<pre><code>" + html.escape("\n".join(code_buf)) + "</code></pre>")
            code_buf = []

    while i < len(lines):
        line = lines[i]
        if line.strip().startswith("```"):
            if in_code:
                flush_code()
                in_code = False
            else:
                flush_list()
                in_code = True
            i += 1
            continue
        if in_code:
            code_buf.append(line)
            i += 1
            continue

        # 表格
        if line.strip().startswith("|") and i + 1 < len(lines) and re.match(
            r"^\s*\|?[\s:-]+\|[\s:|-]*$", lines[i + 1]
        ):
            flush_list()
            header = [c.strip() for c in line.strip().strip("|").split("|")]
            i += 2
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            out.append("<table><thead><tr>")
            out.append("".join(f"<th>{inline(h)}</th>" for h in header))
            out.append("</tr></thead><tbody>")
            for r in rows:
                cells = (r + [""] * len(header))[: len(header)]
                out.append("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in cells) + "</tr>")
            out.append("</tbody></table>")
            continue

        # 标题
        m = re.match(r"^(#{1,4})\s+(.*)$", line)
        if m:
            flush_list()
            level = len(m.group(1))
            raw = m.group(2).strip()
            anchor = re.sub(r"[^\w\u4e00-\u9fff-]+", "-", raw).strip("-").lower()
            out.append(f'<h{level} id="{anchor}">{inline(raw)}</h{level}>')
            if level in (2, 3):
                toc.append((level, raw, anchor))
            i += 1
            continue

        # 分隔线
        if re.match(r"^\s*---+\s*$", line):
            flush_list()
            out.append("<hr>")
            i += 1
            continue

        # 引用
        if line.strip().startswith(">"):
            flush_list()
            quote = []
            while i < len(lines) and lines[i].strip().startswith(">"):
                quote.append(lines[i].strip()[1:].strip())
                i += 1
            out.append("<blockquote>" + "<br>".join(inline(q) for q in quote) + "</blockquote>")
            continue

        # 列表
        m = re.match(r"^\s*([-*]|\d+\.)\s+(.*)$", line)
        if m:
            marker = "ol" if m.group(1)[0].isdigit() else "ul"
            if list_type != marker:
                flush_list()
                list_type = marker
            list_buf.append(f"<li>{inline(m.group(2))}</li>")
            i += 1
            continue

        # 空行
        if not line.strip():
            flush_list()
            i += 1
            continue

        # 段落
        flush_list()
        para = [line.strip()]
        i += 1
        while i < len(lines) and lines[i].strip() and not re.match(
            r"^\s*(#{1,4}\s|\||>|```|[-*]\s|\d+\.\s|---+\s*$)", lines[i]
        ):
            para.append(lines[i].strip())
            i += 1
        out.append("<p>" + inline(" ".join(para)) + "</p>")
    flush_list()
    flush_code()
    return out, toc


CSS = """
@page { size: A4; margin: 18mm 16mm 18mm 16mm; }
* { box-sizing: border-box; }
body { font-family: "PingFang SC","Hiragino Sans GB","Microsoft YaHei",-apple-system,sans-serif;
       font-size: 10.5pt; line-height: 1.65; color: #1f2733; margin: 0; }
.cover { height: 250mm; display: flex; flex-direction: column; justify-content: center;
         page-break-after: always; padding-left: 6mm; }
.cover .kicker { color: #2f5d9e; font-size: 11pt; letter-spacing: 2px; }
.cover h1 { font-size: 30pt; margin: 10mm 0 4mm; line-height: 1.2; color: #14263f; }
.cover .sub { font-size: 12pt; color: #52606d; margin-bottom: 20mm; }
.cover .meta { font-size: 9.5pt; color: #7b8794; line-height: 1.9; border-top: 1px solid #d9e2ec; padding-top: 6mm; }
.toc { page-break-after: always; }
.toc h2 { border: none; margin-top: 0; }
.toc ul { list-style: none; padding-left: 0; margin: 0; }
.toc li { margin: 0; padding: 2.6mm 0; border-bottom: 1px dotted #bcccdc; font-size: 11pt; }
.toc a { color: #1f2733; text-decoration: none; }
h1 { font-size: 22pt; color: #14263f; border-bottom: 3px solid #2f5d9e; padding-bottom: 3mm; margin: 0 0 6mm; }
h2 { font-size: 16pt; color: #14263f; margin: 10mm 0 4mm; padding-left: 3mm; border-left: 4px solid #2f5d9e; }
h3 { font-size: 12.5pt; color: #1d3f6e; margin: 7mm 0 3mm; }
h4 { font-size: 11pt; color: #334e68; margin: 5mm 0 2mm; }
p { margin: 2.5mm 0; text-align: justify; }
a { color: #1d5fa8; text-decoration: none; word-break: break-all; }
strong { color: #14263f; }
code { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 9pt;
       background: #f0f4f8; padding: 0.5mm 1.2mm; border-radius: 2px; color: #9c2f2f; }
pre { background: #f5f7fa; border: 1px solid #d9e2ec; border-radius: 4px; padding: 3mm;
      overflow-x: auto; page-break-inside: avoid; }
pre code { background: none; color: #1f2733; font-size: 8.5pt; line-height: 1.45; }
table { width: 100%; border-collapse: collapse; margin: 3mm 0 4mm; font-size: 9pt;
        page-break-inside: auto; }
thead { display: table-header-group; }
tr { page-break-inside: avoid; }
th { background: #1d3f6e; color: #fff; text-align: left; padding: 2mm 1.8mm; font-weight: 600; }
td { border-bottom: 1px solid #d9e2ec; padding: 1.8mm; vertical-align: top; }
tbody tr:nth-child(even) { background: #f7f9fc; }
blockquote { margin: 3mm 0; padding: 2.5mm 4mm; background: #f0f6ff;
             border-left: 4px solid #2f5d9e; color: #334e68; font-size: 9.5pt; }
hr { border: none; border-top: 1px solid #d9e2ec; margin: 6mm 0; }
ul, ol { padding-left: 6mm; margin: 2mm 0; }
li { margin: 1mm 0; }
h2, h3 { page-break-after: avoid; }
"""


def main():
    md = MD.read_text(encoding="utf-8")
    body, toc = parse(md)
    toc_html = "".join(
        f'<li><a href="#{a}">{html.escape(t)}</a></li>'
        for lvl, t, a in toc if lvl == 2
    )
    page = f"""<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8">
<title>{COVER["title"].replace("<br>", " ")}</title><style>{CSS}</style></head><body>
<div class="cover">
  <div class="kicker">{COVER["kicker"]}</div>
  <h1>{COVER["title"]}</h1>
  <div class="sub">{COVER["sub"]}</div>
  <div class="meta">
    {COVER["meta"]}
  </div>
</div>
<div class="toc"><h2>目录</h2><ul>{toc_html}</ul></div>
{''.join(body)}
</body></html>"""
    HTML_OUT.write_text(page, encoding="utf-8")
    print(f"written: {HTML_OUT}")

    profile = ROOT / "target/chrome-profile"
    profile.mkdir(parents=True, exist_ok=True)
    cmd = [
        CHROME,
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        f"--user-data-dir={profile}",
        "--no-pdf-header-footer",
        "--virtual-time-budget=4000",
        f"--print-to-pdf={PDF_OUT}",
        HTML_OUT.as_uri(),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    if r.returncode != 0:
        print("chrome failed:", r.returncode)
        print(r.stderr[-2000:])
        sys.exit(1)
    print(f"written: {PDF_OUT} ({PDF_OUT.stat().st_size/1024:.0f} KB)")


if __name__ == "__main__":
    main()
