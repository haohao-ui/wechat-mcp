// OverviewPage.swift: System status overview with actionable diagnose items.
import Foundation
import AppKit

public final class OverviewPageController: NSViewController {
    private let context: StatusPanelController
    /// 只读「今日已发送」统计（issue #2 AC4 / T2）。底层拿不到时保持「—」，不编造数字。
    private let sessionData = SessionDataClient()

    // Top metrics
    private let totalRepliesLabel = NSTextField(labelWithString: "今日已发送: —")
    private let activeSessionsLabel = NSTextField(labelWithString: "监听会话: -")
    private let pendingHandoffLabel = NSTextField(labelWithString: "待人工: -")

    // Top action buttons
    private let recheckButton = NSButton(title: "重新检查", target: nil, action: nil)
    private let pauseResumeButton = NSButton(title: "暂停自动回复", target: nil, action: nil)

    // Diagnose rows container
    private let itemsStack = NSStackView()

    public init(context: StatusPanelController) {
        self.context = context
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 700, height: 720))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        NotificationCenter.default.addObserver(self, selector: #selector(hostStatusRefreshed(_:)),
                                               name: hostStatusDidRefreshNotification, object: context)
        refreshData()
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    public override func viewWillAppear() {
        super.viewWillAppear()
        refreshData()
    }

    @objc private func hostStatusRefreshed(_ notification: Notification) {
        guard view.window != nil,
              let input = notification.userInfo?["input"] as? StatusInput,
              let items = notification.userInfo?["items"] as? [StatusItem] else { return }
        applyStatus(items: items, input: input)
    }

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 24
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. Top Card: Metrics and Primary Actions
        let topCard = NSBox()
        topCard.title = "运行概览"
        topCard.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        let topStack = NSStackView()
        topStack.orientation = .vertical
        topStack.alignment = .leading
        topStack.spacing = 16
        let metrics = NSStackView()
        metrics.distribution = .fillEqually
        metrics.spacing = 12
        for label in [totalRepliesLabel, activeSessionsLabel, pendingHandoffLabel] {
            label.font = NSFont.systemFont(ofSize: 16, weight: .semibold)
            let card = NSBox()
            card.titlePosition = .noTitle
            card.setContent(label, inset: 16)
            metrics.addArrangedSubview(card)
        }
        recheckButton.target = self
        recheckButton.action = #selector(handleRecheck)
        pauseResumeButton.target = self
        pauseResumeButton.action = #selector(handlePauseResume)
        let actions = NSStackView(views: [recheckButton, pauseResumeButton])
        actions.spacing = 12
        topStack.addArrangedSubview(metrics)
        topStack.addArrangedSubview(actions)
        metrics.widthAnchor.constraint(equalTo: topStack.widthAnchor).isActive = true
        topCard.setContent(topStack)
        mainStack.addArrangedSubview(topCard)

        // 2. Health check items list
        let itemsBox = NSBox()
        itemsBox.title = "系统检查与诊断"
        itemsBox.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        itemsStack.orientation = .vertical
        itemsStack.alignment = .leading
        itemsStack.spacing = 10
        itemsStack.translatesAutoresizingMaskIntoConstraints = false

        itemsBox.setContent(itemsStack)
        mainStack.addArrangedSubview(itemsBox)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(mainStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            mainStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            topCard.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            itemsBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    public func refreshData() {
        let (items, input) = context.collectLatestStatus()
        applyStatus(items: items, input: input)
        refreshSentToday()
    }

    /// 发送统计走只读 control status：读不到就显示「—」，不把「已触发」当成送达。
    private func refreshSentToday() {
        let client = sessionData
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let count = (try? client.sentToday()) ?? nil
            DispatchQueue.main.async {
                guard let self = self else { return }
                self.totalRepliesLabel.stringValue = "今日已发送: " + (count.map(String.init) ?? "—")
            }
        }
    }

    private func applyStatus(items: [StatusItem], input: StatusInput) {
        // 统计指标
        let activeCount = input.sessions.filter { $0.value.state == "watching" || $0.value.state == "listening" }.count
        let handoffCount = input.sessions.filter { $0.value.handoff == true || $0.value.state == "handoff" }.count
        activeSessionsLabel.stringValue = "监听会话: \(activeCount)"
        pendingHandoffLabel.stringValue = "待人工: \(handoffCount)"

        let isPaused = (input.bot.healthState == "paused")
        pauseResumeButton.title = isPaused ? "恢复自动回复" : "暂停自动回复"

        // 渲染各项检查
        itemsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        for item in items {
            let row = createItemRow(item: item)
            itemsStack.addArrangedSubview(row)
        }
    }

    private func createItemRow(item: StatusItem) -> NSView {
        let rowStack = NSStackView()
        rowStack.orientation = .horizontal
        rowStack.alignment = .centerY
        rowStack.spacing = 10

        // 状态圆点
        let dot = NSTextField(labelWithString: "●")
        dot.font = NSFont.systemFont(ofSize: 14)
        switch item.level {
        case .green: dot.textColor = .systemGreen
        case .yellow: dot.textColor = .systemYellow
        case .red: dot.textColor = .systemRed
        }

        // 标题与说明
        let titleLbl = NSTextField(labelWithString: itemTitle(key: item.key))
        titleLbl.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        titleLbl.widthAnchor.constraint(equalToConstant: 80).isActive = true

        let descText = item.fix != nil ? "\(item.reason) （建议: \(item.fix!)）" : item.reason
        let descLbl = NSTextField(wrappingLabelWithString: descText)
        descLbl.font = NSFont.systemFont(ofSize: 12)
        descLbl.textColor = .secondaryLabelColor

        rowStack.addArrangedSubview(dot)
        rowStack.addArrangedSubview(titleLbl)

        // 行内操作按钮
        if item.key == "desktop_service" {
            let btn = NSButton(title: "重启桌面服务", target: self, action: #selector(confirmRestartDesktopService))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "bot" {
            let btn = NSButton(title: "重启机器人", target: self, action: #selector(confirmRestartBot))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "wechat" {
            let btn = NSButton(title: "放回标准位置", target: self, action: #selector(handleRepositionWechat))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "permissions" && item.level != .green {
            let scrBtn = NSButton(title: "请求屏幕录制权限", target: self, action: #selector(handleRequestScreen))
            let accBtn = NSButton(title: "请求辅助功能权限", target: self, action: #selector(handleRequestAccessibility))
            rowStack.addArrangedSubview(scrBtn)
            rowStack.addArrangedSubview(accBtn)
        }

        let group = NSStackView(views: [rowStack, descLbl])
        group.orientation = .vertical
        group.alignment = .leading
        group.spacing = 6
        return group
    }

    private func itemTitle(key: String) -> String {
        switch key {
        case "permissions": return "权限"
        case "desktop_service": return "桌面服务"
        case "bot": return "机器人"
        case "sessions": return "会话"
        case "wechat": return "微信"
        case "layout": return "布局"
        case "disk": return "磁盘"
        case "brain": return "MCP/大脑"
        default: return key
        }
    }

    @objc private func handleRecheck() {
        context.refresh()
        refreshData()
    }

    @objc private func handlePauseResume() {
        context.pauseResumeBot()
        refreshData()
    }

    @objc private func confirmRestartDesktopService() {
        let alert = NSAlert()
        alert.messageText = "确认重启桌面服务？"
        alert.informativeText = "重启期间可能会短暂中断正在进行的屏幕与微信交互。"
        alert.addButton(withTitle: "确认重启")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn {
            context.restartDesktopService()
            refreshData()
        }
    }

    @objc private func confirmRestartBot() {
        let alert = NSAlert()
        alert.messageText = "确认重启机器人？"
        alert.informativeText = "重启机器人将重新初始化核心进程及所有监听任务。"
        alert.addButton(withTitle: "确认重启")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn {
            context.restartBot()
            refreshData()
        }
    }

    @objc private func handleRepositionWechat() {
        context.repositionWechat()
        refreshData()
    }

    @objc private func handleRequestScreen() {
        requestPermission(.screen)
    }

    @objc private func handleRequestAccessibility() {
        requestPermission(.accessibility)
    }
}
