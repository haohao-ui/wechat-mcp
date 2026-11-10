#!/usr/bin/env python3
"""合规方案财务模型：微信客服官方 API + 本地化 AI 客服中台。

输出 docs/business/合规方案财务模型.xlsx。
运行：python3 docs/business/build_compliance_model.py
"""

from pathlib import Path

import xlsxwriter

OUT = Path(__file__).resolve().parent / "合规方案财务模型.xlsx"

ASSUMPTIONS = [
    ("定价", "标准版年费", 29800, "元/年", "1 企业，10 客服账号，微信客服渠道"),
    ("定价", "专业版年费", 49800, "元/年", "30 客服账号，多业务线，公众号 + 存档副驾驶"),
    ("定价", "旗舰版年费", 98000, "元/年", "不限账号，双机热备，现场实施"),
    ("定价", "实施服务", 15000, "元/客户", "部署、接入、知识库导入、培训"),
    ("定价", "渠道白标", 98000, "元/年", "OEM + 渠道后台 + 联合支持"),
    ("定价", "标准/专业配比", 0.70, "%", "70% 标准 + 30% 专业"),
    ("定价", "混合 ARPU", 35800, "元/年", "29800×70% + 49800×30%"),
    ("成本", "软件毛利率", 0.85, "%", "扣支付、分发、License 服务"),
    ("成本", "实施毛利率", 0.45, "%", "扣交付人力、差旅"),
    ("成本", "客户年支持成本", 8000, "元/年", "工单、远程、版本适配"),
    ("成本", "单人全成本", 200000, "元/年", "精简团队；按市场薪资 30 万则 Y1 亏损扩大"),
    ("成本", "基础设施年费", 80000, "元/年", "relay、license、CI、监控、Apple 开发者"),
    ("成本", "营销费率", 0.15, "% 收入", "内容、投放、渠道分成"),
    ("留存", "年流失率", 0.20, "%", "SMB；企业版可更低"),
    ("获客", "CAC（混合）", 30000, "元/客户", "直销 + 渠道分成加权"),
]

SCENARIOS = {
    "保守": {"new": [6, 15, 30], "team": [2, 2, 3]},
    "中性": {"new": [12, 35, 80], "team": [2, 3, 4]},
    "乐观": {"new": [20, 70, 160], "team": [3, 4, 6]},
}


def a(name):
    for _, k, v, _, _ in ASSUMPTIONS:
        if k == name:
            return v
    raise KeyError(name)


def scenario_rows(name):
    cfg = SCENARIOS[name]
    arpu = a("混合 ARPU")
    impl = a("实施服务")
    gm_l = a("软件毛利率")
    gm_i = a("实施毛利率")
    sup = a("客户年支持成本")
    churn = a("年流失率")
    pcost = a("单人全成本")
    infra = a("基础设施年费")
    mkt = a("营销费率")

    stock = 0.0
    rows = []
    for i in range(3):
        renew = max(stock * (1 - churn), 0)
        stock = renew + cfg["new"][i]
        rev = cfg["new"][i] * (arpu + impl) + renew * arpu
        gross = cfg["new"][i] * (arpu * gm_l + impl * gm_i) + renew * arpu * gm_l
        support = stock * sup
        marketing = rev * mkt
        people = cfg["team"][i] * pcost
        opex = people + infra + support + marketing
        profit = gross - opex
        rows.append(
            {
                "new": cfg["new"][i],
                "team": cfg["team"][i],
                "renew": round(renew),
                "stock": round(stock),
                "rev_license": cfg["new"][i] * arpu + renew * arpu,
                "rev_impl": cfg["new"][i] * impl,
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

    # 摘要
    ws = wb.add_worksheet("摘要")
    ws.hide_gridlines(2)
    ws.set_column("A:A", 26)
    ws.set_column("B:F", 18)
    ws.write("A1", "合规方案财务模型（微信客服官方 API + 本地化 AI 客服中台）", fmt["title"])
    ws.write("A2", "基准日 2026-11-10 ｜ 金额单位：人民币元 ｜ 客户数为期末付费存量", fmt["note"])
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
        "第一年中性亏损约 22 万，需要 40–60 万 runway；保守情景第一年亏损约 35 万。",
        "中性第 3 年收入约 534 万、净利率 27%；乐观约 1,059 万。",
        "收入驱动是新客数 × ARPU（35,800 元）；渠道贡献需达到 30–40% 才能支撑中性情景。",
        "合规方案不需要为封号/法律风险计提，但需要为渠道分成、实施人力、API 适配持续投入。",
    ]
    for i, t in enumerate(conclusions):
        ws.write(row + 2 + i, 0, t, fmt["note"])

    # 假设
    ws = wb.add_worksheet("假设")
    ws.set_column("A:A", 16)
    ws.set_column("B:B", 26)
    ws.set_column("C:C", 14)
    ws.set_column("D:D", 12)
    ws.set_column("E:E", 60)
    ws.write("A1", "模型假设（可直接修改）", fmt["title"])
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

    # 单位经济
    ws = wb.add_worksheet("单位经济")
    ws.set_column("A:A", 26)
    ws.set_column("B:D", 18)
    ws.write("A1", "单位经济", fmt["title"])
    arpu = a("混合 ARPU")
    impl = a("实施服务")
    gm_l, gm_i = a("软件毛利率"), a("实施毛利率")
    churn = a("年流失率")
    cac = a("CAC（混合）")
    sup = a("客户年支持成本")
    ltv = arpu * gm_l / churn + impl * gm_i
    payback = 12 * cac / (arpu * gm_l)
    rows = [
        ("混合 ARPU（元/年）", arpu, "money"),
        ("首年实施收入（元）", impl, "money"),
        ("软件毛利率", gm_l, "pct"),
        ("实施毛利率", gm_i, "pct"),
        ("年流失率", churn, "pct"),
        ("平均生命周期（年）", 1 / churn, "num1"),
        ("LTV（毛利口径，元）", ltv, "money"),
        ("CAC（元）", cac, "money"),
        ("LTV/CAC", ltv / cac, "num1"),
        ("回本周期（月）", payback, "num1"),
        ("客户年支持成本（元）", sup, "money"),
    ]
    for i, (k, v, kind) in enumerate(rows, start=3):
        ws.write(i, 0, k, fmt["txt"])
        if kind == "pct":
            ws.write_number(i, 1, v, fmt["pct"])
        elif kind == "money":
            ws.write_number(i, 1, v, fmt["money"])
        elif kind == "num1":
            ws.write_number(i, 1, v, wb.add_format({"border": 1, "num_format": "0.0"}))
        else:
            ws.write_number(i, 1, v, fmt["num"])
    ws.write(15, 0, "说明", fmt["sub"])
    ws.write(16, 0, "LTV 只算毛利，不含支持成本；实际净值还需扣 8,000 元/年支持成本。CAC 为直销+渠道加权，纯直销约 2–4 万。", fmt["note"])

    # 三情景
    ws = wb.add_worksheet("三情景")
    ws.set_column("A:A", 20)
    ws.set_column("B:D", 16)
    ws.write("A1", "三情景三年预测", fmt["title"])
    ws.write("A2", "单位：元；客户数为期末存量；利润 = 毛利 - 人力 - 固定 - 支持 - 营销", fmt["note"])
    row = 3
    for s in ["保守", "中性", "乐观"]:
        data = scenario_rows(s)
        ws.merge_range(row, 0, row, 3, f"情景：{s}（新增客户 {SCENARIOS[s]['new']}；团队 {SCENARIOS[s]['team']} 人）", fmt["sub"])
        row += 1
        for j, h in enumerate(["指标", "第1年", "第2年", "第3年"]):
            ws.write(row, j, h, fmt["h"])
        row += 1
        metrics = [
            ("新增客户", "new", "num"),
            ("续费客户", "renew", "num"),
            ("期末存量", "stock", "num"),
            ("团队人数", "team", "num"),
            ("软件收入", "rev_license", "money"),
            ("实施收入", "rev_impl", "money"),
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
                v = data[j][key]
                if kind == "pct":
                    ws.write_number(row, 1 + j, v, fmt["pct"])
                elif kind == "money":
                    f = fmt["ok"] if (key == "profit" and v >= 0) else (fmt["bad"] if key == "profit" else fmt["money"])
                    ws.write_number(row, 1 + j, v, f)
                else:
                    ws.write_number(row, 1 + j, v, fmt["num"])
            row += 1
        row += 1

    # 渠道情景
    ws = wb.add_worksheet("渠道情景")
    ws.set_column("A:A", 22)
    ws.set_column("B:F", 16)
    ws.write("A1", "渠道/白标收入情景（未计入三情景）", fmt["title"])
    ws.write("A2", "单位：元/年；白标年费 98,000；渠道分成按 30% 估算", fmt["note"])
    ws.write(3, 0, "渠道伙伴数", fmt["h"])
    for j, share in enumerate(["0% 分成", "20% 分成", "30% 分成"], start=1):
        ws.write(3, j, share, fmt["h"])
    for i, n in enumerate([1, 2, 3, 5, 8, 10], start=4):
        ws.write(i, 0, n, fmt["sub"])
        ws.write_number(i, 1, n * 98000, fmt["money"])
        ws.write_number(i, 2, n * 98000 * 0.8, fmt["money"])
        ws.write_number(i, 3, n * 98000 * 0.7, fmt["money"])
    ws.write(11, 0, "读法", fmt["sub"])
    ws.write(12, 0, "3 家渠道伙伴、30% 分成 ≈ 20.6 万/年净收入；渠道还带来下游客户，是三情景之外最确定的上行空间。", fmt["note"])

    # 敏感性
    ws = wb.add_worksheet("敏感性")
    ws.write("A1", "第 3 年营业收入敏感性（万元）：期末客户数 × 混合 ARPU", fmt["title"])
    stocks = [30, 50, 80, 120, 170, 250, 350]
    arpus = [29800, 35800, 42000, 49800, 60000]
    ws.write(2, 0, "存量客户 \\ ARPU", fmt["h"])
    for j, arpu_v in enumerate(arpus, start=1):
        ws.write(2, j, arpu_v, fmt["h"])
    for i, stock in enumerate(stocks, start=3):
        ws.write(i, 0, stock, fmt["sub"])
        for j, arpu_v in enumerate(arpus, start=1):
            ws.write_number(i, j, stock * arpu_v / 10000, wb.add_format({"border": 1, "num_format": "#,##0"}))

    # 定价
    ws = wb.add_worksheet("定价方案")
    ws.set_column("A:A", 18)
    ws.set_column("B:B", 20)
    ws.set_column("C:C", 55)
    ws.set_column("D:D", 40)
    ws.write("A1", "建议定价（合规版）", fmt["title"])
    for j, h in enumerate(["版本", "价格", "包含", "目标客户"]):
        ws.write(2, j, h, fmt["h"])
    plans = [
        ("标准版", "¥29,800/年", "1 企业；10 客服账号；微信客服渠道；知识库 500 文档；人工接管；审计；邮件支持", "10–30 人企业"),
        ("专业版", "¥49,800/年", "30 客服账号；多业务线；公众号渠道；会话存档副驾驶；SLA 4h；专属群", "30–100 人企业"),
        ("旗舰版", "¥98,000/年", "不限客服账号；双机热备；定制集成；现场实施；年度巡检", "100+ 人/连锁"),
        ("实施服务", "¥15,000 起", "部署、渠道接入、知识库导入、培训", "一次性"),
        ("渠道白标", "¥98,000/年 + 分成", "OEM 品牌、API、渠道后台、联合支持", "企微/客服服务商"),
        ("个人副驾驶", "免费 / ¥299 年", "复制粘贴式 AI 草稿；不含任何自动化", "引流/过渡"),
    ]
    for i, (n, p, inc, tgt) in enumerate(plans, start=3):
        ws.write(i, 0, n, fmt["sub"])
        ws.write(i, 1, p, fmt["txt"])
        ws.write(i, 2, inc, fmt["txt"])
        ws.write(i, 3, tgt, fmt["txt"])

    # 图表
    ws = wb.add_worksheet("图表")
    ws.set_column("A:A", 14)
    ws.set_column("B:D", 14)
    ws.write("A1", "三情景收入（元）", fmt["title"])
    data = {s: scenario_rows(s) for s in ["保守", "中性", "乐观"]}
    ws.write(3, 0, "年份", fmt["h"])
    for j, s in enumerate(data, start=1):
        ws.write(3, j, s, fmt["h"])
    for i in range(3):
        ws.write(4 + i, 0, f"第{i+1}年", fmt["sub"])
        for j, s in enumerate(data, start=1):
            ws.write_number(4 + i, j, data[s][i]["revenue"])
    chart = wb.add_chart({"type": "column"})
    chart.add_series({"name": "保守", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 1, 6, 1], "fill": {"color": "#9DB4CE"}})
    chart.add_series({"name": "中性", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 2, 6, 2], "fill": {"color": "#2F5D9E"}})
    chart.add_series({"name": "乐观", "categories": ["图表", 4, 0, 6, 0], "values": ["图表", 4, 3, 6, 3], "fill": {"color": "#14263F"}})
    chart.set_title({"name": "营业收入（元）"})
    chart.set_y_axis({"num_format": "¥#,##0"})
    chart.set_legend({"position": "bottom"})
    ws.insert_chart("F3", chart, {"x_scale": 1.4, "y_scale": 1.1})

    wb.close()
    print(f"written: {OUT}")


if __name__ == "__main__":
    main()
