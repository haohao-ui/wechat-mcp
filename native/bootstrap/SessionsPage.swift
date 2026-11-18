// SessionsPage.swift: Active chat sessions monitoring and handoff management.
// 视觉对齐 docs/design/current-pages-v1/02-sessions.png：单张全宽卡片「会话监听列表」，
// 三列（会话 / 状态 / 操作），只有转人工行显示「恢复自动回复」主按钮；
// 轮次追溯（Provenance）作为独立卡片放在下方，功能与之前一致。
import Foundation
import AppKit

public final class SessionsPageController: NSViewController {
    private let context: StatusPanelController
    private let brainClient = BrainClient()
    /// 会话级转人工（issue #2 AC5 / T8）：走只读/受控的 control session-handoff。
    private let handoffClient = SessionDataClient()

    // 会话监听列表卡片
    private let tableCard = NSBox()
    private let tableContent = NSStackView()
    private let tableHeaderBar = NSView()
    private let rowsStack = NSStackView()
    private let emptyContainer = NSView()
    private let emptyLabel = NSTextField(labelWithString: L10n.t(.sessionsEmpty))
    private let handoffFeedback = NSTextField(labelWithString: "")

    // 轮次追溯（Provenance）卡片
    private let provenanceBox = NSBox()
    private let turnSearchField = NSTextField()
    private let queryTurnButton = NSButton(title: L10n.t(.actionQueryTurn), target: nil, action: nil)
    private let detailTextView = NSTextView()

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
        refreshSessions()
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    public override func viewWillAppear() {
        super.viewWillAppear()
        refreshSessions()
    }

    @objc private func hostStatusRefreshed(_ notification: Notification) {
        guard view.window != nil,
              let input = notification.userInfo?["input"] as? StatusInput else { return }
        renderSessions(input.sessions)
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

        // 1. 会话监听列表卡片（白底 / 12 圆角 / 1px 描边由 NSBox.setContent 统一处理）
        tableCard.title = L10n.t(.uiSessions009)
        tableContent.orientation = .vertical
        tableContent.alignment = .leading
        tableContent.spacing = 0
        tableContent.translatesAutoresizingMaskIntoConstraints = false

        tableHeaderBar.translatesAutoresizingMaskIntoConstraints = false
        rowsStack.orientation = .vertical
        rowsStack.alignment = .leading
        rowsStack.spacing = 0
        rowsStack.translatesAutoresizingMaskIntoConstraints = false
        configureTableHeader()
        configureEmptyState()

        tableContent.addArrangedSubview(tableHeaderBar)
        tableContent.addArrangedSubview(rowsStack)
        tableContent.addArrangedSubview(emptyContainer)
        tableCard.setContent(tableContent, inset: 20)
        mainStack.addArrangedSubview(tableCard)

        // 转人工/恢复的即时反馈（失败原因直接写在界面上，不弹系统错误框）
        handoffFeedback.font = NSFont.systemFont(ofSize: 12)
        handoffFeedback.textColor = Theme.Colors.textSecondary
        handoffFeedback.isHidden = true
        mainStack.addArrangedSubview(handoffFeedback)

        // 2. 轮次追溯卡片（保持在列表下方，功能不变）
        provenanceBox.title = L10n.t(.uiSessions010)
        let provStack = buildProvenanceContent()
        provenanceBox.setContent(provStack, inset: 20)
        mainStack.addArrangedSubview(provenanceBox)

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
            tableCard.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            provenanceBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            handoffFeedback.widthAnchor.constraint(lessThanOrEqualTo: mainStack.widthAnchor),
            tableHeaderBar.widthAnchor.constraint(equalTo: tableContent.widthAnchor),
            rowsStack.widthAnchor.constraint(equalTo: tableContent.widthAnchor),
            emptyContainer.widthAnchor.constraint(equalTo: tableContent.widthAnchor)
        ])
    }

    /// 表头：会话 / 状态 / 操作，12pt 次要色，底色为页面浅灰，下方 1px 分隔线（效果图）。
    private func configureTableHeader() {
        tableHeaderBar.wantsLayer = true
        tableHeaderBar.layer?.backgroundColor = Theme.Colors.pageBackground.cgColor
        tableHeaderBar.heightAnchor.constraint(equalToConstant: 40).isActive = true

        let nameLabel = headerLabel(L10n.t(.sessionsTableSessionColumn))
        let statusLabel = headerLabel(L10n.t(.sessionsTableStatusColumn))
        let actionLabel = headerLabel(L10n.t(.sessionsTableOperationColumn))
        let spacer = NSView()
        spacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        spacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        let stack = NSStackView(views: [nameLabel, statusLabel, spacer, actionLabel])
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        tableHeaderBar.addSubview(stack)

        let separator = NSView()
        separator.wantsLayer = true
        separator.layer?.backgroundColor = Theme.Colors.cardBorder.cgColor
        separator.translatesAutoresizingMaskIntoConstraints = false
        tableHeaderBar.addSubview(separator)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: tableHeaderBar.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: tableHeaderBar.trailingAnchor, constant: -12),
            stack.centerYAnchor.constraint(equalTo: tableHeaderBar.centerYAnchor),
            nameLabel.widthAnchor.constraint(equalTo: tableHeaderBar.widthAnchor, multiplier: 0.30),
            separator.leadingAnchor.constraint(equalTo: tableHeaderBar.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: tableHeaderBar.trailingAnchor),
            separator.bottomAnchor.constraint(equalTo: tableHeaderBar.bottomAnchor),
            separator.heightAnchor.constraint(equalToConstant: 1)
        ])
    }

    private func headerLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = Theme.Colors.textSecondary
        label.lineBreakMode = .byTruncatingTail
        return label
    }

    /// 空状态区：浅灰底、8 圆角、居中 13pt 次要色「暂无活跃会话」，无数据时显示（效果图）。
    private func configureEmptyState() {
        emptyContainer.wantsLayer = true
        emptyContainer.layer?.cornerRadius = 8
        emptyContainer.layer?.masksToBounds = true
        emptyContainer.layer?.backgroundColor = Theme.Colors.inputDisabledBg.cgColor
        emptyContainer.translatesAutoresizingMaskIntoConstraints = false
        emptyContainer.heightAnchor.constraint(equalToConstant: 86).isActive = true

        emptyLabel.font = NSFont.systemFont(ofSize: 13)
        emptyLabel.textColor = Theme.Colors.textSecondary
        emptyLabel.alignment = .center
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false
        emptyContainer.addSubview(emptyLabel)
        NSLayoutConstraint.activate([
            emptyLabel.centerXAnchor.constraint(equalTo: emptyContainer.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: emptyContainer.centerYAnchor)
        ])
    }

    private func buildProvenanceContent() -> NSStackView {
        let provStack = NSStackView()
        provStack.orientation = .vertical
        provStack.alignment = .leading
        provStack.spacing = 12
        provStack.translatesAutoresizingMaskIntoConstraints = false

        turnSearchField.placeholderString = L10n.t(.turnPlaceholder)
        turnSearchField.font = NSFont.systemFont(ofSize: 13)
        turnSearchField.lineBreakMode = .byTruncatingTail
        turnSearchField.widthAnchor.constraint(equalToConstant: 300).isActive = true
        queryTurnButton.target = self
        queryTurnButton.action = #selector(handleQueryTurn)

        let searchRow = NSStackView(views: [turnSearchField, queryTurnButton])
        searchRow.orientation = .horizontal
        searchRow.alignment = .centerY
        searchRow.spacing = 8
        provStack.addArrangedSubview(searchRow)

        let textScroll = NSScrollView()
        textScroll.hasVerticalScroller = true
        textScroll.borderType = .bezelBorder
        textScroll.drawsBackground = true
        textScroll.backgroundColor = Theme.Colors.cardBackground
        detailTextView.isEditable = false
        detailTextView.drawsBackground = false
        detailTextView.font = NSFont.userFixedPitchFont(ofSize: 12)
        detailTextView.textColor = Theme.Colors.textPrimary
        detailTextView.string = L10n.t(.provenanceHint)
        textScroll.documentView = detailTextView
        textScroll.heightAnchor.constraint(equalToConstant: 180).isActive = true
        provStack.addArrangedSubview(textScroll)
        textScroll.widthAnchor.constraint(equalTo: provStack.widthAnchor).isActive = true

        return provStack
    }

    public func refreshSessions() {
        let (_, input) = context.collectLatestStatus()
        renderSessions(input.sessions)
    }

    private func renderSessions(_ sessions: [String: SessionHealthInput]) {
        rowsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        let sorted = sessions.sorted(by: { $0.key < $1.key })
        emptyContainer.isHidden = !sorted.isEmpty
        tableContent.setCustomSpacing(sorted.isEmpty ? 20 : 0, after: tableHeaderBar)
        guard !sorted.isEmpty else { return }

        for (index, entry) in sorted.enumerated() {
            let row = createSessionRow(name: entry.key, session: entry.value)
            rowsStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: rowsStack.widthAnchor).isActive = true
            if index < sorted.count - 1 {
                let separator = makeRowSeparator()
                rowsStack.addArrangedSubview(separator)
                separator.widthAnchor.constraint(equalTo: rowsStack.widthAnchor).isActive = true
            }
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

    /// 胶囊文案：基础人话状态 + 数据里已有的人话原因（如「已转人工：等待人工处理」）。
    /// reason 若是机器码（reply_timeout 等）就不拼，避免把内部码当文案展示。
    private func sessionTagText(session: SessionHealthInput) -> String {
        let human = sessionStateHumanReadable(
            state: session.state,
            reason: session.reason,
            code: session.code,
            handoff: session.handoff
        )
        let isHandoff = session.handoff == true || session.state == "handoff"
        guard isHandoff || session.state == "needs_recalibration" else { return human }
        guard let reason = session.reason?.trimmingCharacters(in: .whitespacesAndNewlines),
              !reason.isEmpty,
              reason.range(of: "^[A-Za-z0-9_.:-]+$", options: .regularExpression) == nil else {
            return human
        }
        return L10n.t(.sessionsTagReasonFormat, human, reason)
    }

    /// 会话行：显示名 13pt + 状态胶囊（监听中=绿、需处理=橙）+ 操作按钮；行高 44。
    private func createSessionRow(name: String, session: SessionHealthInput) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.heightAnchor.constraint(equalToConstant: 44).isActive = true

        let isHandoff = session.handoff == true || session.state == "handoff"
        let isListening = session.state == "watching" || session.state == "listening"

        let nameLabel = NSTextField(labelWithString: name)
        nameLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        nameLabel.textColor = Theme.Colors.textPrimary
        nameLabel.lineBreakMode = .byTruncatingTail
        nameLabel.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)

        // 状态文案只用数据里的 state/reason/code 组合，不编造原因。
        let humanState = sessionTagText(session: session)
        let tag = ThemeStatusTag(status: isListening ? .normal : .warning,
                                  customText: humanState,
                                  showsDot: false)
        tag.translatesAutoresizingMaskIntoConstraints = false
        tag.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        tag.toolTip = humanState

        let spacer = NSView()
        spacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        spacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        let rowStack = NSStackView()
        rowStack.orientation = .horizontal
        rowStack.alignment = .centerY
        rowStack.spacing = 16
        rowStack.translatesAutoresizingMaskIntoConstraints = false
        rowStack.addArrangedSubview(nameLabel)
        rowStack.addArrangedSubview(tag)
        rowStack.addArrangedSubview(spacer)

        if isHandoff {
            let resumeButton = ThemePrimaryButton(
                title: L10n.t(.actionResumeAutoReply),
                target: self,
                action: #selector(handleResumeHandoff(_:))
            )
            resumeButton.identifier = NSUserInterfaceItemIdentifier(name)
            rowStack.addArrangedSubview(resumeButton)
        } else if isListening {
            let handoffButton = SessionOutlineButton(
                title: L10n.t(.actionHandoff),
                target: self,
                action: #selector(handleHandoffToHuman(_:))
            )
            handoffButton.identifier = NSUserInterfaceItemIdentifier(name)
            handoffButton.toolTip = L10n.t(.uiSessions011)
            rowStack.addArrangedSubview(handoffButton)
        }

        row.addSubview(rowStack)
        NSLayoutConstraint.activate([
            rowStack.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 12),
            rowStack.trailingAnchor.constraint(equalTo: row.trailingAnchor, constant: -12),
            rowStack.centerYAnchor.constraint(equalTo: row.centerYAnchor),
            nameLabel.widthAnchor.constraint(equalTo: row.widthAnchor, multiplier: 0.30)
        ])
        return row
    }

    @objc private func handleQueryTurn() {
        let query = turnSearchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        detailTextView.string = L10n.t(.turnQuerying, String(describing: query))
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let detail = try self.brainClient.turnDetail(turnId: query)
                DispatchQueue.main.async {
                    self.displayTurnDetail(detail)
                }
            } catch {
                DispatchQueue.main.async {
                    self.detailTextView.string = L10n.t(.uiSessions012, String(describing: error.localizedDescription))
                }
            }
        }
    }

    public func applyFixtureState(sessions: [String: SessionHealthInput], selectedTurnDetail: BrainTurnDetail?) {
        _ = self.view // 确保 viewDidLoad 已执行完毕
        renderSessions(sessions)
        if let detail = selectedTurnDetail {
            self.turnSearchField.stringValue = detail.turnId
            self.displayTurnDetail(detail)
        }
        self.view.layoutSubtreeIfNeeded()
    }

    public func displayTurnDetail(_ detail: BrainTurnDetail) {
        var text = L10n.t(.uiSessions013)
        text += "Turn ID: \(detail.turnId)\n"
        text += "\(L10n.t(.turnContact)): \(detail.contactId)\n"
        text += "\(L10n.t(.turnProvenanceStatus)): \(detail.provenanceDisplayText)\n"
        text += "\(L10n.t(.turnRoleRevision)): \(detail.roleId ?? L10n.t(.turnNotRecorded))\n"
        if let model = detail.model {
            let p = model["provider"]?.value as? String ?? "-"
            let m = model["model"]?.value as? String ?? "-"
            text += "\(L10n.t(.turnModelRef)): \(p) / \(m)\n"
        }
        text += "\(L10n.t(.turnSendKey)): \(detail.sendKey ?? "-")\n"
        text += "\(L10n.t(.turnPhases)): \(detail.phaseLabels.joined(separator: " · "))\n"
        if let dec = detail.decision {
            let act = dec["action"]?.value as? String ?? "-"
            let reply = dec["text"]?.value as? String ?? ""
            text += L10n.t(.turnDecision, act, reply) + "\n"
        }
        detailTextView.string = text
    }

    @objc private func handleResumeHandoff(_ sender: NSButton) {
        let sessionName = sender.identifier?.rawValue
        context.resumeHandoff(sessionName: sessionName)
        refreshSessions()
    }

    /// 转人工：会话列表按显示名分组，而 control session-handoff 要的是历史库里的稳定 session_id，
    /// 所以先按显示名在只读列表里唯一定位，找不到或不唯一就明确报错，绝不猜。
    @objc private func handleHandoffToHuman(_ sender: NSButton) {
        guard let name = sender.identifier?.rawValue, !name.isEmpty else { return }
        // 转人工会停止该会话的自动回复，必须先二次确认。
        let alert = NSAlert()
        alert.messageText = L10n.t(.uiSessions014)
        alert.informativeText = L10n.t(.uiSessions015, String(describing: name))
        alert.addButton(withTitle: L10n.t(.actionHandoff))
        alert.addButton(withTitle: L10n.t(.cancel))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        sender.isEnabled = false
        handoffFeedback.textColor = Theme.Colors.textSecondary
        handoffFeedback.stringValue = L10n.t(.uiSessions016, String(describing: name))
        handoffFeedback.isHidden = false
        let client = handoffClient
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let outcome: Result<Void, Error> = {
                do {
                    let page = try client.list()
                    let matches = page.sessions.filter { $0.displayName == name }
                    guard matches.count == 1, let target = matches.first else {
                        let message = matches.isEmpty
                            ? L10n.t(.uiSessions017, String(describing: name))
                            : L10n.t(.uiSessions018, String(describing: matches.count), String(describing: name))
                        return .failure(NSError(domain: "session-handoff", code: 1,
                                                userInfo: [NSLocalizedDescriptionKey: message]))
                    }
                    _ = try client.handoff(sessionID: target.contactID, toHuman: true)
                    return .success(())
                } catch {
                    return .failure(error)
                }
            }()
            DispatchQueue.main.async {
                guard let self = self else { return }
                switch outcome {
                case .success:
                    self.handoffFeedback.textColor = Theme.Colors.statusNormalText
                    self.handoffFeedback.stringValue = L10n.t(.uiSessions019, String(describing: name))
                case .failure(let error):
                    sender.isEnabled = true
                    self.handoffFeedback.textColor = Theme.Colors.statusErrorText
                    self.handoffFeedback.stringValue = L10n.t(.uiSessions020, String(describing: error.localizedDescription))
                }
                self.handoffFeedback.isHidden = false
                self.refreshSessions()
            }
        }
    }
}

/// 会话行内次要操作按钮：白底 + 1px 描边 + 13pt 深色字（效果图里只有需人工行显示主按钮）。
fileprivate final class SessionOutlineButton: NSButton {
    init(title: String, target: AnyObject? = nil, action: Selector? = nil) {
        super.init(frame: .zero)
        self.target = target
        self.action = action
        self.title = title
        isBordered = false
        bezelStyle = .regularSquare
        setButtonType(.momentaryPushIn)
        focusRingType = .none
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = Theme.Colors.cardBorder.cgColor
        layer?.backgroundColor = Theme.Colors.cardBackground.cgColor
        updateAttributedTitle()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var title: String {
        didSet { updateAttributedTitle() }
    }

    private func updateAttributedTitle() {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: Theme.Colors.textPrimary,
            .font: NSFont.systemFont(ofSize: 13, weight: .medium),
            .paragraphStyle: paragraph
        ])
    }

    override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: base.width + 24, height: max(base.height + 12, 32))
    }
}
