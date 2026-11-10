#!/usr/bin/env python3
"""wechat-mcp 商业化财务模型生成器。

输出 docs/business/财务模型.xlsx，包含：摘要、假设、单位经济、三情景预测、敏感性、定价方案。
所有金额单位为人民币元，客户数为期末存量付费客户。
运行：python3 docs/business/build_model.py
"""

from pathlib import Path

import xlsxwriter

OUT = Path(__file__).resolve().parent / "财务模型.xlsx"

# ---------------------------------------------------------------------------
# 假设（可在 Excel 内修改；本脚本给出可复现的基准值）
# ---------------------------------------------------------------------------
ASSUMPTIONS = [
    # (分组, 名称, 值, 单位, 说明)
    ("定价", "个人版年费", 1999, "元/年", "单机单账号，自动回复白名单、历史库"),
    ("定价", "专业版年费", 3999, "元/年", "多白名单、外部大脑 API、通讯录同步、优先支持"),
    ("定价", "私有化首年", 19800, "元/套", "含专用 Mac 部署、标定、培训；不含硬件"),
    ("定价", "私有化续费", 3980, "元/年", "升级、标定维护、远程支持"),
    ("定价", "服务商白标", 99800, "元/年", "10 席位起，OEM 品牌与 API 接入"),
    ("定价", "混合 ARPU（软件）", 2500, "元/年", "个人版/专业版加权"),
    ("成本", "软件毛利率", 0.88, "%", "扣模型 API、支付通道、分发"),
    ("成本", "私有化毛利率", 0.62, "%", "扣部署人力、标定、差旅、硬件补贴"),
    ("成本", "个人客户年支持成本", 400, "元/年", "工单、远程排障、版本适配"),
    ("成本", "私有化客户年支持成本", 2000, "元/年", "专属群、远程、现场差旅另计"),
    ("成本", "单人全成本", 200000, "元/年", "精简团队：低薪创始人+兼职；按市场薪资需上调至 30–40 万"),
    ("成本", "基础设施年费", 30000, "元/年", "license 服务、更新分发、监控、Apple 开发者"),
    ("成本", "营销费率", 0.15, "% 收入", "内容、投放、渠道分成"),
    ("留存", "个人版年流失率", 0.35, "%", "封号、版本不适配、需求消失"),
    ("留存", "私有化年流失率", 0.15, "%", "替换成本高，但依赖单一机器"),
    ("获客", "自营内容 CAC", 400, "元/付费客户", "GitHub/短视频/社群内容"),
    ("获客", "渠道 CAC", 1200, "元/付费客户", "代理/服务商分成，含返点"),
    ("获客", "试用转付费率", 0.04, "%", "下载/试用 → 付费"),
]

# 三情景：新增付费客户数（个人/软件、私有化）与团队人数
SCENARIOS = {
    "保守": {
        "new_software": [60, 100, 150],
        "new_private": [4, 8, 12],
        "team": [1, 1, 2],
    },
    "中性": {
        "new_software": [120, 220, 350],
        "new_private": [8, 16, 26],
        "team": [1, 2, 2],
    },
    "乐观": {
        "new_software": [220, 400, 650],
        "new_private": [15, 35, 55],
        "team": [2, 3, 4],
    },
}


def a(name):
    for _, k, v, _, _ in ASSUMPTIONS:
        if k == name:
            return v
    raise KeyError(name)


def scenario_rows(s):
    cfg = SCENARIOS[s]
    price_sw = a("混合 ARPU（软件）")
    price_pv = a("私有化首年")
    renew_pv = a("私有化续费")
    churn_sw = a("个人版年流失率")
    churn_pv = a("私有化年流失率")
    gm_sw = a("软件毛利率")
    gm_pv = a("私有化毛利率")
    sup_sw = a("个人客户年支持成本")
    sup_pv = a("私有化客户年支持成本")
    team_cost = a("单人全成本")
    infra = a("基础设施年费")
    mkt = a("营销费率")

    rows = []
    stock_sw = 0.0
    stock_pv = 0.0
    for i in range(3):
        new_sw = cfg["new_software"][i]
        new_pv = cfg["new_private"][i]
        # 期末存量 = 期初存量 × (1-流失) + 新增
        stock_sw = stock_sw * (1 - churn_sw) + new_sw
        stock_pv = stock_pv * (1 - churn_pv) + new_pv
        renew_pv_cnt = max(stock_pv - new_pv, 0)
        rev_sw = stock_sw * price_sw
        rev_pv_new = new_pv * price_pv
        rev_pv_renew = renew_pv_cnt * renew_pv
        rev = rev_sw + rev_pv_new + rev_pv_renew
        gross = rev_sw * gm_sw + rev_pv_new * gm_pv + rev_pv_renew * gm_pv
        support = stock_sw * sup_sw + stock_pv * sup_pv
        marketing = rev * mkt
        people = cfg["team"][i] * team_cost
        opex = people + infra + support + marketing
        profit = gross - opex
        rows.append(
            {
                "year": f"第{i + 1}年",
                "new_sw": new_sw,
                "new_pv": new_pv,
                "stock_sw": round(stock_sw),
                "stock_pv": round(stock_pv),
                "rev_sw": rev_sw,
                "rev_pv": rev_pv_new + rev_pv_renew,
                "revenue": rev,
                "gross": gross,
                "support": support,
                "marketing": marketing,
                "people": people,
                "infra": infra,
                "opex": opex,
                "profit": profit,
                "margin": profit / rev if rev else 0,
            }
        )
    return rows


def main():
    wb = xlsxwriter.Workbook(str(OUT))
    fmt = {
        "title": wb.add_format({"bold": True, "font_size": 16, "font_color": "#1F3864"}),
        "h": wb.add_format({"bold": True, "bg_color": "#1F3864", "font_color": "white", "border": 1, "align": "center", "valign": "vcenter", "text_wrap": True}),
        "sub": wb.add_format({"bold": True, "bg_color": "#D9E2F3", "border": 1}),
        "txt": wb.add_format({"border": 1, "text_wrap": True, "valign": "top"}),
        "num": wb.add_format({"border": 1, "num_format": "#,##0"}),
        "money": wb.add_format({"border": 1, "num_format": "¥#,##0"}),
        "pct": wb.add_format({"border": 1, "num_format": "0.0%"}),
        "note": wb.add_format({"italic": True, "font_color": "#666666", "text_wrap": True, "valign": "top"}),
        "ok": wb.add_format({"border": 1, "num_format": "¥#,##0", "font_color": "#006100", "bg_color": "#C6EFCE"}),
        "bad": wb.add_format({"border": 1, "num_format": "¥#,##0", "font_color": "#9C0006", "bg_color": "#FFC7CE"}),
    }

    # ---------------- 摘要 ----------------
    ws = wb.add_worksheet("摘要")
    ws.hide_gridlines(2)
    ws.set_column("A:A", 26)
    ws.set_column("B:F", 18)
    ws.write("A1", "wechat-mcp 商业化财务模型", fmt["title"])
    ws.write("A2", "基准日 2026-11-09 ｜ 金额单位：人民币元 ｜ 客户数为期末付费存量", fmt["note"])
    ws.write("A4", "情景", fmt["h"])
    for j, h in enumerate(["第1年收入", "第1年利润", "第2年收入", "第2年利润", "第3年收入", "第3年利润"], start=1):
        ws.write(4, j, h, fmt["h"])
    row = 5
    for s in ["保守", "中性", "乐观"]:
        rows = scenario_rows(s)
        ws.write(row, 0, s, fmt["sub"])
        for j in range(3):
            ws.write_number(row, 1 + j * 2, rows[j]["revenue"], fmt["money"])
            ws.write_number(row, 2 + j * 2, rows[j]["profit"], fmt["ok"] if rows[j]["profit"] >= 0 else fmt["bad"])
        row += 1
    ws.write(row + 1, 0, "关键结论", fmt["sub"])
    conclusions = [
        "个人版/专业版是现金流基本盘，私有化是客单价来源；两者毛利相差约 26 个百分点。",
        "保守情景第 3 年才勉强打平；该业务的可行形态是 1–2 人极致精简团队，或提高私有化占比。",
        "中性情景第 3 年收入约 195 万元、净利率 27%，乐观情景约 376 万元，属于「小而美」而非 VC 规模化标的。",
        "要突破千万级收入，必须解决三件事：Windows 支持、多账号/群聊、以及合规形态（企业微信/白标授权）。",
    ]
    for i, t in enumerate(conclusions):
        ws.write(row + 2 + i, 0, t, fmt["note"])

    # ---------------- 假设 ----------------
    ws = wb.add_worksheet("假设")
    ws.set_column("A:A", 16)
    ws.set_column("B:B", 26)
    ws.set_column("C:C", 14)
    ws.set_column("D:D", 12)
    ws.set_column("E:E", 60)
    ws.write("A1", "模型假设（可直接修改，情景表为静态计算值）", fmt["title"])
    for j, h in enumerate(["分组", "参数", "值", "单位", "说明"]):
        ws.write(2, j, h, fmt["h"])
    for i, (g, k, v, unit, note) in enumerate(ASSUMPTIONS, start=3):
        ws.write(i, 0, g, fmt["txt"])
        ws.write(i, 1, k, fmt["txt"])
        if isinstance(v, float) and v < 1:
            ws.write_number(i, 2, v, fmt["pct"])
        elif isinstance(v, (int, float)):
            ws.write_number(i, 2, v, fmt["num"])
        else:
            ws.write(i, 2, v, fmt["txt"])
        ws.write(i, 3, unit, fmt["txt"])
        ws.write(i, 4, note, fmt["txt"])

    # ---------------- 单位经济 ----------------
    ws = wb.add_worksheet("单位经济")
    ws.set_column("A:A", 22)
    ws.set_column("B:D", 18)
    ws.write("A1", "单位经济（按方案）", fmt["title"])
    heads = ["指标", "个人版", "专业版", "私有化"]
    for j, h in enumerate(heads):
        ws.write(2, j, h, fmt["h"])
    # 简化：个人版 1999、专业版 3999、私有化首年 19800
    sw_price = a("个人版年费")
    pro_price = a("专业版年费")
    pv_price = a("私有化首年")
    renew = a("私有化续费")
    gm_sw, gm_pv = a("软件毛利率"), a("私有化毛利率")
    churn_sw, churn_pv = a("个人版年流失率"), a("私有化年流失率")
    life_sw = 1 / churn_sw
    life_pv = 1 / churn_pv
    # 私有化 LTV = 首年毛利 + 续费毛利 × (生命-1)
    ltv_sw = sw_price * gm_sw * life_sw
    ltv_pro = pro_price * gm_sw * life_sw
    ltv_pv = pv_price * gm_pv + renew * gm_pv * (life_pv - 1)
    cac_sw = a("自营内容 CAC")
    cac_pv = a("渠道 CAC") + 3000  # 私有化交付获客额外成本
    data = [
        ("年费（元）", sw_price, pro_price, pv_price),
        ("毛利率", gm_sw, gm_sw, gm_pv),
        ("年流失率", churn_sw, churn_sw, churn_pv),
        ("平均生命周期（年）", life_sw, life_sw, life_pv),
        ("LTV（毛利口径，元）", ltv_sw, ltv_pro, ltv_pv),
        ("CAC（元）", cac_sw, cac_sw * 1.2, cac_pv),
        ("LTV/CAC", ltv_sw / cac_sw, ltv_pro / (cac_sw * 1.2), ltv_pv / cac_pv),
        ("回本周期（月）", 12 * cac_sw / (sw_price * gm_sw), 12 * (cac_sw * 1.2) / (pro_price * gm_sw), 12 * cac_pv / (pv_price * gm_pv)),
    ]
    for i, (k, v1, v2, v3) in enumerate(data, start=3):
        ws.write(i, 0, k, fmt["txt"])
        for j, v in enumerate([v1, v2, v3], start=1):
            if "率" in k:
                ws.write_number(i, j, v, fmt["pct"])
            elif "年" in k and "费" not in k:
                ws.write_number(i, j, v, wb.add_format({"border": 1, "num_format": "0.0"}))
            elif "周期" in k:
                ws.write_number(i, j, v, wb.add_format({"border": 1, "num_format": "0.0"}))
            elif "LTV/CAC" in k:
                ws.write_number(i, j, v, wb.add_format({"border": 1, "num_format": "0.0"}))
            else:
                ws.write_number(i, j, v, fmt["money"])
    ws.write(12, 0, "说明", fmt["sub"])
    ws.write(13, 0, "LTV 只算毛利，不含支持成本；实际净值再看支持成本与退款。个人版 LTV/CAC 偏高是因为内容获客成本低，但封号与版本问题会显著抬高实际流失。", fmt["note"])

    # ---------------- 三情景预测 ----------------
    ws = wb.add_worksheet("三情景预测")
    ws.set_column("A:A", 18)
    ws.set_column("B:M", 14)
    ws.write("A1", "三情景三年预测", fmt["title"])
    ws.write("A2", "单位：元；客户数为期末存量；利润=毛利-人力-固定-支持-营销", fmt["note"])
    row = 3
    for s in ["保守", "中性", "乐观"]:
        rows = scenario_rows(s)
        ws.merge_range(row, 0, row, 12, f"情景：{s}（新增付费客户 个人版 {SCENARIOS[s]['new_software']} / 私有化 {SCENARIOS[s]['new_private']}；团队 {SCENARIOS[s]['team']} 人）", fmt["sub"])
        row += 1
        headers = ["指标", "第1年", "第2年", "第3年"]
        for j, h in enumerate(headers):
            ws.write(row, j, h, fmt["h"])
        row += 1
        metrics = [
            ("新增个人版客户", "new_sw", "num"),
            ("新增私有化客户", "new_pv", "num"),
            ("期末个人版存量", "stock_sw", "num"),
            ("期末私有化存量", "stock_pv", "num"),
            ("软件收入", "rev_sw", "money"),
            ("私有化收入", "rev_pv", "money"),
            ("营业收入", "revenue", "money"),
            ("毛利", "gross", "money"),
            ("支持成本", "support", "money"),
            ("营销费用", "marketing", "money"),
            ("人力成本", "people", "money"),
            ("固定费用", "infra", "money"),
            ("经营利润", "profit", "money"),
            ("净利率", "margin", "pct"),
        ]
        for name, key, kind in metrics:
            ws.write(row, 0, name, fmt["txt"])
            for j in range(3):
                v = rows[j][key]
                if kind == "pct":
                    ws.write_number(row, 1 + j, v, fmt["pct"])
                elif kind == "money":
                    f = fmt["ok"] if (key == "profit" and v >= 0) else (fmt["bad"] if key == "profit" else fmt["money"])
                    ws.write_number(row, 1 + j, v, f)
                else:
                    ws.write_number(row, 1 + j, v, fmt["num"])
            row += 1
        row += 1

    # ---------------- 敏感性 ----------------
    ws = wb.add_worksheet("敏感性")
    ws.write("A1", "第 3 年营业收入敏感性（万元）：存量客户数 × 混合 ARPU", fmt["title"])
    stock_list = [200, 400, 600, 800, 1000, 1500, 2000]
    arpu_list = [1999, 2500, 3000, 3999, 5000]
    ws.write(2, 0, "存量付费客户 \\ ARPU", fmt["h"])
    for j, arpu in enumerate(arpu_list, start=1):
        ws.write(2, j, arpu, fmt["h"])
    for i, stock in enumerate(stock_list, start=3):
        ws.write(i, 0, stock, fmt["sub"])
        for j, arpu in enumerate(arpu_list, start=1):
            ws.write_number(i, j, stock * arpu / 10000, wb.add_format({"border": 1, "num_format": "#,##0"}))

    # ---------------- 定价方案 ----------------
    ws = wb.add_worksheet("定价方案")
    ws.set_column("A:A", 18)
    ws.set_column("B:B", 16)
    ws.set_column("C:C", 50)
    ws.set_column("D:D", 46)
    ws.write("A1", "建议定价阶梯（首年）", fmt["title"])
    for j, h in enumerate(["方案", "价格", "包含", "目标客户与边界"]):
        ws.write(2, j, h, fmt["h"])
    plans = [
        ("社区版（免费）", "0", "MCP 服务 + 通用桌面工具 + 公众号文章抓取；不含微信自动回复", "开发者/技术型个人，用于建立口碑与分发"),
        ("个人版", "¥1,999/年", "单机单账号；白名单私聊自动回复；历史库；14 天试用", "个体运营者，日咨询 < 100 条"),
        ("专业版", "¥3,999/年", "多白名单；外部大脑 API（接自有模型/知识库）；通讯录同步；优先支持", "2–5 人小团队；有自建 AI 能力"),
        ("私有化（专用机）", "¥19,800 首年 + ¥3,980/年", "专用 Mac 部署、标定、培训、远程运维；不含硬件", "不愿自建环境的中小商家；要求开箱即用"),
        ("服务商白标", "¥99,800/年 起", "OEM 品牌、API、席位管理、联合交付", "私域代运营/软件代理商（需合规审查）"),
    ]
    for i, (n, p, inc, tgt) in enumerate(plans, start=3):
        ws.write(i, 0, n, fmt["sub"])
        ws.write(i, 1, p, fmt["txt"])
        ws.write(i, 2, inc, fmt["txt"])
        ws.write(i, 3, tgt, fmt["txt"])

    # ---------------- 技术授权情景 ----------------
    ws = wb.add_worksheet("技术授权情景")
    ws.set_column("A:A", 20)
    ws.set_column("B:F", 16)
    ws.write("A1", "技术授权 / 白标收入情景（未计入三情景预测）", fmt["title"])
    ws.write("A2", "单位：元/年；毛利按 90% 估算；用于评估主线路径的上行空间", fmt["note"])
    ws.write(3, 0, "授权客户数", fmt["h"])
    fees = [50000, 100000, 200000]
    for j, fee in enumerate(fees, start=1):
        ws.write(3, j, f"{fee/10000:.0f} 万/年", fmt["h"])
    for i, n in enumerate([1, 2, 3, 5, 8, 10, 15], start=4):
        ws.write(i, 0, n, fmt["sub"])
        for j, fee in enumerate(fees, start=1):
            ws.write_number(i, j, n * fee, fmt["money"])
    ws.write(12, 0, "读法", fmt["sub"])
    ws.write(13, 0, "1 家 10 万元授权 ≈ 40 个个人版客户 ≈ 5 个私有化客户；3 家授权即可覆盖 1 人团队全年成本。授权收入毛利约 90%，是主线路径的核心上行空间。", fmt["note"])

    # ---------------- 图表 ----------------
    ws = wb.add_worksheet("图表")
    ws.set_column("A:A", 14)
    ws.set_column("B:D", 14)
    ws.write("A1", "三情景收入与利润（元）", fmt["title"])
    scenarios = {s: scenario_rows(s) for s in ["保守", "中性", "乐观"]}

    # 数据表 1：收入
    ws.write(3, 0, "年份", fmt["h"])
    for j, s in enumerate(scenarios, start=1):
        ws.write(3, j, s, fmt["h"])
    for i in range(3):
        ws.write(4 + i, 0, f"第{i+1}年", fmt["sub"])
        for j, s in enumerate(scenarios, start=1):
            ws.write_number(4 + i, j, scenarios[s][i]["revenue"])

    chart1 = wb.add_chart({"type": "column"})
    chart1.add_series({"name": "保守", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 1, 6, 1], "fill": {"color": "#9DB4CE"}})
    chart1.add_series({"name": "中性", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 2, 6, 2], "fill": {"color": "#2F5D9E"}})
    chart1.add_series({"name": "乐观", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 3, 6, 3], "fill": {"color": "#14263F"}})
    chart1.set_title({"name": "营业收入（元）"})
    chart1.set_y_axis({"num_format": "¥#,##0"})
    chart1.set_legend({"position": "bottom"})
    ws.insert_chart("F3", chart1, {"x_scale": 1.4, "y_scale": 1.1})

    # 数据表 2：利润
    ws.write(10, 0, "年份", fmt["h"])
    for j, s in enumerate(scenarios, start=1):
        ws.write(10, j, s, fmt["h"])
    for i in range(3):
        ws.write(11 + i, 0, f"第{i+1}年", fmt["sub"])
        for j, s in enumerate(scenarios, start=1):
            ws.write_number(11 + i, j, scenarios[s][i]["profit"])

    chart2 = wb.add_chart({"type": "column"})
    chart2.add_series({"name": "保守", "categories": ["图表", 11, 0, 13, 0], "values": ["图表", 11, 1, 13, 1], "fill": {"color": "#9DB4CE"}})
    chart2.add_series({"name": "中性", "categories": ["图表", 11, 0, 13, 0], "values": ["图表", 11, 2, 13, 2], "fill": {"color": "#2F5D9E"}})
    chart2.add_series({"name": "乐观", "categories": ["图表", 11, 0, 13, 0], "values": ["图表", 11, 3, 13, 3], "fill": {"color": "#14263F"}})
    chart2.set_title({"name": "经营利润（元）"})
    chart2.set_y_axis({"num_format": "¥#,##0"})
    chart2.set_legend({"position": "bottom"})
    ws.insert_chart("F19", chart2, {"x_scale": 1.4, "y_scale": 1.1})

    wb.close()
    print(f"written: {OUT}")


if __name__ == "__main__":
    main()
