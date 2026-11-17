// OverviewPage.swift: System status overview with actionable diagnose items.
import Foundation
import AppKit

public final class OverviewPageController: NSViewController {
    private let context: StatusPanelController

    // Top metrics
    private let totalRepliesLabel = NSTextField(labelWithString: L10n.t(.sentToday) + ": —")
    private let activeSessionsLabel = NSTextField(labelWithString: L10n.t(.metricListening) + ": —")
    private let pendingHandoffLabel = NSTextField(labelWithString: L10n.t(.metricHandoff) + ": —")

    // Top action buttons
    private let recheckButton = NSButton(title: L10n.t(.actionRecheck), target: nil, action: nil)
    private let pauseResumeButton = NSButton(title: L10n.t(.actionPauseAutoReply), target: nil, action: nil)

    // Diagnose rows container
    private let itemsStack = NSStackView()
    private let issuesStack = NSStackView()
    private var statsLoading = false
    private var statusObserver: NSObjectProtocol?
    public var onStatusAction: ((StatusIssueAction) -> Void)?

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
        refreshData()
        statusObserver = NotificationCenter.default.addObserver(forName: StatusPanelController.statusDidChange, object: context, queue: .main) { [weak self] _ in
            if let input = self?.context.statusInput { self?.renderData(input: input) }
        }
    }

    deinit { if let observer = statusObserver { NotificationCenter.default.removeObserver(observer) } }

    private func renderSummary() {
        guard let summary = context.statusSummary else { return }
        issuesStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if summary.issues.isEmpty { issuesStack.addArrangedSubview(NSTextField(wrappingLabelWithString: L10n.t(.needsActionNone))) }
        for issue in summary.issues {
            let button = StatusActionButton(title: issue.actionTitle, actionValue: issue.action) { [weak self] action in self?.onStatusAction?(action) }
            let row = NSStackView(views: [NSTextField(wrappingLabelWithString: issue.text), button])
            issuesStack.addArrangedSubview(row)
        }
    }

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 16
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. Top Card: Metrics and Primary Actions
        let topCard = NSBox()
        topCard.title = L10n.t(.overviewRunBox)
        topCard.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        let topStack = NSStackView()
        topStack.orientation = .vertical
        topStack.alignment = .leading
        topStack.spacing = 16

        for lbl in [totalRepliesLabel, activeSessionsLabel, pendingHandoffLabel] {
            lbl.font = NSFont.systemFont(ofSize: 13, weight: .medium)
            topStack.addArrangedSubview(lbl)
        }

        recheckButton.target = self
        recheckButton.action = #selector(handleRecheck)
        pauseResumeButton.target = self
        pauseResumeButton.action = #selector(handlePauseResume)

        topStack.addArrangedSubview(recheckButton)
        topStack.addArrangedSubview(pauseResumeButton)

        topCard.setContent(topStack)
        mainStack.addArrangedSubview(topCard)
        mainStack.addArrangedSubview(NSTextField(wrappingLabelWithString: L10n.t(.sentNotDelivered)))
        let needsBox = NSBox()
        needsBox.title = L10n.t(.needsActionTitle)
        issuesStack.orientation = .vertical
        issuesStack.alignment = .leading
        issuesStack.spacing = 8
        needsBox.setContent(issuesStack)
        mainStack.addArrangedSubview(needsBox)
        needsBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        // 2. Health check items list
        let itemsBox = NSBox()
        itemsBox.title = L10n.t(.checksBox)
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
            mainStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            topCard.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            itemsBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    public func refreshData() {
        let (_, input) = context.collectLatestStatus()
        renderData(input: input)
    }

    private func renderData(input: StatusInput) {
        let items = evaluate(input)
        // 统计指标
        let activeCount = input.sessions.filter { $0.value.state == "watching" || $0.value.state == "listening" }.count
        let handoffCount = input.sessions.filter { $0.value.handoff == true || $0.value.state == "handoff" }.count
        activeSessionsLabel.stringValue = L10n.t(.metricListening) + ": \(activeCount)"
        pendingHandoffLabel.stringValue = L10n.t(.metricHandoff) + ": \(handoffCount)"

        let isPaused = context.statusSummary?.autoReplyPaused ?? true
        pauseResumeButton.isEnabled = context.statusSummary?.state == .normal || context.statusSummary?.state == .manuallyPaused
        pauseResumeButton.title = isPaused ? L10n.t(.actionResumeAutoReply) : L10n.t(.actionPauseAutoReply)

        renderSummary()
        if !statsLoading {
            statsLoading = true
            DispatchQueue.global(qos: .utility).async { [weak self] in
                let count = try? SessionDataClient().sentToday()
                DispatchQueue.main.async {
                    self?.statsLoading = false
                    self?.totalRepliesLabel.stringValue = L10n.t(.sentToday) + ": " + (count.map(String.init) ?? "—")
                }
            }
        }
        // 渲染各项检查
        itemsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        for item in items {
            let row = createItemRow(item: item)
            itemsStack.addArrangedSubview(row)
        }
    }

    private func createItemRow(item: StatusItem) -> NSView {
        let rowStack = NSStackView()
        rowStack.orientation = .vertical
        rowStack.alignment = .leading
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

        let descText = item.fix != nil ? item.reason + L10n.t(.suggestionSuffix, item.fix!) : item.reason
        let descLbl = NSTextField(wrappingLabelWithString: descText)
        descLbl.font = NSFont.systemFont(ofSize: 12)

        rowStack.addArrangedSubview(dot)
        rowStack.addArrangedSubview(titleLbl)
        rowStack.addArrangedSubview(descLbl)

        // 行内操作按钮
        if item.key == "desktop_service" {
            let btn = NSButton(title: L10n.t(.actionRestartDesktopService), target: self, action: #selector(confirmRestartDesktopService))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "bot" {
            let btn = NSButton(title: L10n.t(.actionRestartBot), target: self, action: #selector(confirmRestartBot))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "wechat" {
            let btn = NSButton(title: L10n.t(.actionRepositionWechat), target: self, action: #selector(handleRepositionWechat))
            rowStack.addArrangedSubview(btn)
        } else if item.key == "permissions" && item.level != .green {
            let scrBtn = NSButton(title: L10n.t(.permScreen), target: self, action: #selector(handleRequestScreen))
            let accBtn = NSButton(title: L10n.t(.permAccessibility), target: self, action: #selector(handleRequestAccessibility))
            rowStack.addArrangedSubview(scrBtn)
            rowStack.addArrangedSubview(accBtn)
        }

        return rowStack
    }

    private func itemTitle(key: String) -> String {
        return statusItemTitle(key: key)
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
        alert.messageText = L10n.t(.confirmRestartDesktopTitle)
        alert.informativeText = L10n.t(.confirmRestartDesktopBody)
        alert.addButton(withTitle: L10n.t(.confirmRestart))
        alert.addButton(withTitle: L10n.t(.cancel))
        if alert.runModal() == .alertFirstButtonReturn {
            context.restartDesktopService()
            refreshData()
        }
    }

    @objc private func confirmRestartBot() {
        let alert = NSAlert()
        alert.messageText = L10n.t(.confirmRestartBotTitle)
        alert.informativeText = L10n.t(.confirmRestartBotBody)
        alert.addButton(withTitle: L10n.t(.confirmRestart))
        alert.addButton(withTitle: L10n.t(.cancel))
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
