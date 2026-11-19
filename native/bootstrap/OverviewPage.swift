// OverviewPage.swift: System status overview with actionable diagnose items.
// 视觉对齐 docs/design/current-pages-v1/01-overview.png：浅色页面 + 三张指标卡 + 系统检查卡片。
// 页标题由 Shell 统一渲染（28pt semibold），本页内容从标题下方开始。
import Foundation
import AppKit

public final class OverviewPageController: NSViewController, ShellPageActionProviding {
    private let context: StatusPanelController
    /// 只读「今日已发送」统计（issue #2 AC4 / T2）。底层拿不到时保持「—」，不编造数字。
    private let sessionData = SessionDataClient()

    // 三张指标卡：标题 13pt 次要色在上，数值 34pt bold 在下（效果图）。
    private let todayValueLabel = NSTextField(labelWithString: "—")
    private let activeSessionsValueLabel = NSTextField(labelWithString: "—")
    private let pendingHandoffValueLabel = NSTextField(labelWithString: "—")
    private let footnoteLabel = NSTextField(wrappingLabelWithString: L10n.t(.valueUnavailable))

    // 顶部操作按钮（与 Shell 页标题同一视觉层级，右对齐排在本页第一行）。
    private let recheckButton = OverviewOutlineButton(
        title: L10n.t(.actionRecheck),
        symbolName: "arrow.clockwise",
        borderColor: Theme.Colors.cardBorder,
        labelColor: Theme.Colors.textPrimary
    )
    private let pauseResumeButton = OverviewOutlineButton(
        title: L10n.t(.actionPauseAutoReply),
        symbolName: "pause.fill",
        borderColor: Theme.Colors.cardBorder,
        labelColor: Theme.Colors.textPrimary
    )

    // 诊断行容器（行间用 1px 分隔线，行内边距 14）。
    private let itemsStack = NSStackView()
    /// 顶部操作行容器：无 Shell 的单页测试仍保留；接入 Shell 时按钮交给标题行，容器塌陷为 0 高。
    private let headerRow = NSStackView()
    /// 窄窗口下隐藏「建议」列，避免被压成一字一行（900pt 最小窗口仍可读）。
    private var suggestionLabels: [NSTextField] = []

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

    public override func viewDidLayout() {
        super.viewDidLayout()
        // 最小窗口（900pt 宽）里四列放不下：隐藏建议列而不是把它压成一字一行。
        let hideSuggestions = view.bounds.width < 860
        for label in suggestionLabels where label.isHidden != hideSuggestions {
            label.isHidden = hideSuggestions
        }
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
        mainStack.spacing = 20
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. 顶部操作行：无 Shell 时右对齐显示；接入 Shell 时按钮被移到页标题行（见 shellPageActionViews）。
        headerRow.orientation = .horizontal
        headerRow.alignment = .centerY
        headerRow.spacing = 12
        let headerSpacer = NSView()
        headerSpacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        headerSpacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        recheckButton.target = self
        recheckButton.action = #selector(handleRecheck)
        pauseResumeButton.target = self
        pauseResumeButton.action = #selector(handlePauseResume)
        headerRow.addArrangedSubview(headerSpacer)
        headerRow.addArrangedSubview(recheckButton)
        headerRow.addArrangedSubview(pauseResumeButton)
        mainStack.addArrangedSubview(headerRow)

        // 2. 三张并排指标卡（等宽、间距 20）。
        let metricsRow = NSStackView()
        metricsRow.orientation = .horizontal
        metricsRow.alignment = .top
        metricsRow.distribution = .fillEqually
        metricsRow.spacing = 20
        let todayCard = makeMetricCard(
            title: L10n.t(.overviewMetricRepliesToday),
            valueLabel: todayValueLabel,
            valueColor: Theme.Colors.textPrimary
        )
        let activeCard = makeMetricCard(
            title: L10n.t(.metricListening),
            valueLabel: activeSessionsValueLabel,
            valueColor: Theme.Colors.statusNormalText
        )
        let handoffCard = makeMetricCard(
            title: L10n.t(.metricHandoff),
            valueLabel: pendingHandoffValueLabel,
            valueColor: Theme.Colors.statusWarningDot
        )
        todayValueLabel.identifier = NSUserInterfaceItemIdentifier("overview.metric.sentToday")
        activeSessionsValueLabel.identifier = NSUserInterfaceItemIdentifier("overview.metric.listening")
        pendingHandoffValueLabel.identifier = NSUserInterfaceItemIdentifier("overview.metric.handoff")
        for card in [todayCard, activeCard, handoffCard] {
            metricsRow.addArrangedSubview(card)
        }
        mainStack.addArrangedSubview(metricsRow)

        // 3. 系统检查与诊断卡片：白底 / 12 圆角 / 1px 描边（NSBox.setContent 统一处理），标题 17pt semibold。
        let checksCard = NSBox()
        checksCard.title = L10n.t(.checksBox)
        itemsStack.orientation = .vertical
        itemsStack.alignment = .leading
        itemsStack.spacing = 0
        itemsStack.translatesAutoresizingMaskIntoConstraints = false
        checksCard.setContent(itemsStack, inset: 20)
        mainStack.addArrangedSubview(checksCard)

        // 4. 底部 12pt 次要色说明（今日发送统计缺失时如实说明）。
        footnoteLabel.font = NSFont.systemFont(ofSize: 12)
        footnoteLabel.textColor = Theme.Colors.textSecondary
        mainStack.addArrangedSubview(footnoteLabel)

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
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 20),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24),
            headerRow.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            metricsRow.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            checksCard.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    /// 指标卡：白底卡片 + 13pt 次要色标题 + 34pt bold 等宽数字（效果图）。
    private func makeMetricCard(title: String, valueLabel: NSTextField, valueColor: NSColor) -> NSBox {
        let card = NSBox()
        card.titlePosition = .noTitle

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = NSFont.systemFont(ofSize: 13)
        titleLabel.textColor = Theme.Colors.textSecondary

        valueLabel.font = NSFont.monospacedDigitSystemFont(ofSize: 34, weight: .bold)
        valueLabel.textColor = valueColor
        valueLabel.lineBreakMode = .byTruncatingTail

        let stack = NSStackView(views: [titleLabel, valueLabel])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        card.setContent(stack, inset: 20)
        return card
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
                self.todayValueLabel.stringValue = count.map(String.init) ?? "—"
                self.footnoteLabel.stringValue = count == nil
                    ? L10n.t(.valueUnavailable)
                    : L10n.t(.sentNotDelivered)
            }
        }
    }

    private func applyStatus(items: [StatusItem], input: StatusInput) {
        // 统计指标
        let activeCount = input.sessions.filter { $0.value.state == "watching" || $0.value.state == "listening" }.count
        let handoffCount = input.sessions.filter { $0.value.handoff == true || $0.value.state == "handoff" }.count
        activeSessionsValueLabel.stringValue = String(describing: activeCount)
        pendingHandoffValueLabel.stringValue = String(describing: handoffCount)

        let isPaused = (input.bot.healthState == "paused")
        pauseResumeButton.title = isPaused ? L10n.t(.actionResumeAutoReply) : L10n.t(.actionPauseAutoReply)

        // 渲染各项检查（行间 1px 分隔线）
        suggestionLabels.removeAll()
        itemsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (index, item) in items.enumerated() {
            if index > 0 {
                itemsStack.addArrangedSubview(makeRowSeparator())
            }
            let row = createItemRow(item: item)
            itemsStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: itemsStack.widthAnchor).isActive = true
        }
    }

    private func makeRowSeparator() -> NSView {
        let line = NSView()
        line.wantsLayer = true
        line.layer?.backgroundColor = Theme.Colors.cardBorder.cgColor
        line.translatesAutoresizingMaskIntoConstraints = false
        line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return line
    }

    private func createItemRow(item: StatusItem) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false

        // 状态圆点
        let dot = NSView()
        dot.wantsLayer = true
        dot.layer?.cornerRadius = 4
        dot.layer?.masksToBounds = true
        dot.layer?.backgroundColor = statusDotColor(for: item.level).cgColor
        dot.translatesAutoresizingMaskIntoConstraints = false
        dot.widthAnchor.constraint(equalToConstant: 8).isActive = true
        dot.heightAnchor.constraint(equalToConstant: 8).isActive = true

        // 名称（13pt 中黑）
        let nameLabel = NSTextField(labelWithString: itemTitle(key: item.key))
        nameLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        nameLabel.textColor = Theme.Colors.textPrimary
        nameLabel.lineBreakMode = .byTruncatingTail
        nameLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)

        // 状态文字（13pt）
        let stateLabel = NSTextField(labelWithString: item.reason)
        stateLabel.font = NSFont.systemFont(ofSize: 13)
        stateLabel.textColor = Theme.Colors.textPrimary
        stateLabel.lineBreakMode = .byTruncatingTail
        stateLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        // 建议（12pt 次要色，可换行）
        let suggestionLabel = NSTextField(wrappingLabelWithString: suggestionText(for: item))
        suggestionLabel.font = NSFont.systemFont(ofSize: 12)
        suggestionLabel.textColor = Theme.Colors.textSecondary
        suggestionLabel.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        suggestionLabel.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        let rowStack = NSStackView()
        rowStack.orientation = .horizontal
        rowStack.alignment = .centerY
        rowStack.spacing = 10
        rowStack.translatesAutoresizingMaskIntoConstraints = false
        rowStack.addArrangedSubview(dot)
        rowStack.addArrangedSubview(nameLabel)
        rowStack.addArrangedSubview(stateLabel)
        if !suggestionLabel.stringValue.isEmpty {
            suggestionLabel.isHidden = view.bounds.width < 860
            suggestionLabels.append(suggestionLabel)
            rowStack.addArrangedSubview(suggestionLabel)
        }
        let buttons = actionButtons(for: item)
        if !buttons.isEmpty {
            let buttonStack = NSStackView(views: buttons)
            buttonStack.orientation = .horizontal
            buttonStack.alignment = .centerY
            buttonStack.spacing = 8
            buttonStack.setContentCompressionResistancePriority(.required, for: .horizontal)
            rowStack.addArrangedSubview(buttonStack)
        }

        row.addSubview(rowStack)
        let nameWidth = nameLabel.widthAnchor.constraint(equalToConstant: 76)
        let stateWidth = stateLabel.widthAnchor.constraint(equalToConstant: 176)
        stateWidth.priority = .defaultHigh
        NSLayoutConstraint.activate([
            rowStack.leadingAnchor.constraint(equalTo: row.leadingAnchor),
            rowStack.trailingAnchor.constraint(equalTo: row.trailingAnchor),
            rowStack.topAnchor.constraint(equalTo: row.topAnchor, constant: 14),
            rowStack.bottomAnchor.constraint(equalTo: row.bottomAnchor, constant: -14),
            row.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            nameWidth,
            stateWidth,
            stateLabel.widthAnchor.constraint(greaterThanOrEqualToConstant: 92)
        ])
        return row
    }

    private func statusDotColor(for level: StatusLevel) -> NSColor {
        switch level {
        case .green: return Theme.Colors.statusNormalDot
        case .yellow: return Theme.Colors.statusWarningDot
        case .red: return Theme.Colors.statusErrorDot
        }
    }

    /// 建议行：有 fix 时「建议：<fix>」；正常态没有 fix 时用既有文案说明无需操作，其余保持空白不编造。
    private func suggestionText(for item: StatusItem) -> String {
        if let fix = item.fix, !fix.isEmpty {
            return L10n.t(.overviewSuggestionPrefix) + fix
        }
        return item.level == .green ? L10n.t(.overviewSuggestionNoAction) : ""
    }

    private func actionButtons(for item: StatusItem) -> [NSButton] {
        switch item.key {
        case "desktop_service":
            return [rowActionButton(title: L10n.t(.actionRestartDesktopService),
                                    action: #selector(confirmRestartDesktopService))]
        case "bot":
            return [rowActionButton(title: L10n.t(.actionRestartBot),
                                    action: #selector(confirmRestartBot))]
        case "wechat":
            return [rowActionButton(title: L10n.t(.actionRepositionWechat),
                                    action: #selector(handleRepositionWechat))]
        case "permissions" where item.level != .green:
            return [
                rowActionButton(title: L10n.t(.overviewRequestScreenPermission),
                                action: #selector(handleRequestScreen)),
                rowActionButton(title: L10n.t(.overviewRequestAccessibilityPermission),
                                action: #selector(handleRequestAccessibility))
            ]
        default:
            return []
        }
    }

    /// 诊断行右侧次要按钮：白底 + 品牌绿描边/文字（对齐效果图）。
    private func rowActionButton(title: String, action: Selector) -> NSButton {
        OverviewOutlineButton(
            title: title,
            borderColor: Theme.Colors.brand,
            labelColor: Theme.Colors.brand,
            minimumHeight: 30,
            horizontalPadding: 12,
            target: self,
            action: action
        )
    }

    private func itemTitle(key: String) -> String {
        switch key {
        case "permissions": return L10n.t(.itemPermissions)
        case "desktop_service": return L10n.t(.itemDesktopService)
        case "bot": return L10n.t(.itemBot)
        case "sessions": return L10n.t(.navSessions)
        case "wechat": return L10n.t(.itemWechat)
        case "layout": return L10n.t(.itemLayout)
        case "disk": return L10n.t(.itemDisk)
        case "brain": return L10n.t(.uiOverview008)
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

    // MARK: - ShellPageActionProviding

    /// 把「重新检查 / 暂停自动回复」交给 Shell 标题行右侧；页面内的动作行保留空容器
    /// （无 Shell 的单页测试仍能遍历到这两个按钮，标题行接入后容器高度为 0，不再占内容区）。
    public func shellPageActionViews() -> [NSView] {
        let actionViews: [NSView] = [recheckButton, pauseResumeButton]
        for actionView in actionViews {
            if headerRow.arrangedSubviews.contains(actionView) {
                headerRow.removeArrangedSubview(actionView)
            }
            actionView.removeFromSuperview()
        }
        if let mainStack = headerRow.superview as? NSStackView {
            mainStack.setCustomSpacing(0, after: headerRow)
        }
        return actionViews
    }
}

/// 页面内次要按钮：白底、1px 描边、13pt 文字，可带 SF Symbols 图标。
/// 顶部操作按钮用描边灰 + 深色字；诊断行操作按钮用品牌绿描边 + 绿字（对齐效果图）。
fileprivate final class OverviewOutlineButton: NSButton {
    private let borderColor: NSColor
    private let labelColor: NSColor
    private let minimumHeight: CGFloat
    private let horizontalPadding: CGFloat

    init(
        title: String,
        symbolName: String? = nil,
        borderColor: NSColor = Theme.Colors.cardBorder,
        labelColor: NSColor = Theme.Colors.textPrimary,
        minimumHeight: CGFloat = 36,
        horizontalPadding: CGFloat = 24,
        target: AnyObject? = nil,
        action: Selector? = nil
    ) {
        self.borderColor = borderColor
        self.labelColor = labelColor
        self.minimumHeight = minimumHeight
        self.horizontalPadding = horizontalPadding
        super.init(frame: .zero)
        self.target = target
        self.action = action
        self.title = title
        if let symbolName,
           let symbol = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil) {
            self.image = symbol
            self.imagePosition = .imageLeading
            self.imageScaling = .scaleProportionallyDown
            self.contentTintColor = labelColor
        }
        setupAppearance()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var title: String {
        didSet { updateAttributedTitle() }
    }

    private func setupAppearance() {
        isBordered = false
        bezelStyle = .regularSquare
        setButtonType(.momentaryPushIn)
        focusRingType = .none
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = borderColor.cgColor
        layer?.backgroundColor = Theme.Colors.cardBackground.cgColor
        updateAttributedTitle()
    }

    private func updateAttributedTitle() {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: labelColor,
            .font: NSFont.systemFont(ofSize: 13, weight: .medium),
            .paragraphStyle: paragraph
        ])
    }

    override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: base.width + horizontalPadding, height: max(base.height + 12, minimumHeight))
    }
}
