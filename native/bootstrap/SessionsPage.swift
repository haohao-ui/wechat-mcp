// SessionsPage.swift: Active chat sessions monitoring and handoff management.
import Foundation
import AppKit

public final class SessionsPageController: NSViewController {
    private let context: StatusPanelController
    private let brainClient = BrainClient()
    private let sessionsStack = NSStackView()
    private let banner = NSTextField(wrappingLabelWithString: "")
    private let bannerAction = NSButton()
    private let messageView = NSTextView()
    private let messageTitle = NSTextField(labelWithString: "")
    private let handoffButton = NSButton()
    private let moreSessions = NSButton()
    private let moreMessages = NSButton()
    private let refreshButton = NSButton()
    private var nextSessionOffset: Int?
    private var nextMessageOffset: Int?
    private var selectedContact: String?
    private var rows: [RecordedSession] = []
    private var messages: [RecordedMessage] = []
    private var handoffChanging = false
    private var loading = false
    private var hasAPIData = false
    private var detailGeneration = 0
    private var statusObserver: NSObjectProtocol?
    public var onStatusAction: ((StatusIssueAction) -> Void)?
    private let emptyLabel = NSTextField(labelWithString: L10n.t(.sessionsEmpty))

    // Provenance Detail Area
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
        refreshSessions()
        statusObserver = NotificationCenter.default.addObserver(forName: StatusPanelController.statusDidChange, object: context, queue: .main) { [weak self] _ in
            self?.renderBanner()
            if self?.hasAPIData == true { self?.renderRecordedRows() }
        }
    }

    deinit { if let observer = statusObserver { NotificationCenter.default.removeObserver(observer) } }

    private func renderBanner() {
        guard let summary = context.statusSummary else { return }
        banner.stringValue = summary.bannerText ?? ""
        banner.isHidden = summary.bannerText == nil
        bannerAction.title = summary.primaryActionTitle ?? ""
        bannerAction.isHidden = summary.primaryAction == nil
    }

    @objc private func performBannerAction() {
        if let action = context.statusSummary?.primaryAction { onStatusAction?(action) }
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

        bannerAction.target = self
        bannerAction.action = #selector(performBannerAction)
        mainStack.addArrangedSubview(banner)
        mainStack.addArrangedSubview(bannerAction)
        refreshButton.title = L10n.t(.actionRecheck)
        refreshButton.target = self
        refreshButton.action = #selector(reloadSessions)
        mainStack.addArrangedSubview(refreshButton)
        let box = NSBox()
        box.title = L10n.t(.sessionsListBox)
        box.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        sessionsStack.orientation = .vertical
        sessionsStack.alignment = .leading
        sessionsStack.spacing = 10
        sessionsStack.translatesAutoresizingMaskIntoConstraints = false

        emptyLabel.textColor = .secondaryLabelColor
        sessionsStack.addArrangedSubview(emptyLabel)

        box.setContent(sessionsStack)
        mainStack.addArrangedSubview(box)
        moreSessions.title = L10n.t(.loadMore)
        moreSessions.target = self
        moreSessions.action = #selector(loadMoreSessions)
        moreSessions.isHidden = true
        mainStack.addArrangedSubview(moreSessions)
        let conversation = NSBox()
        conversation.title = L10n.t(.navSessions)
        let detailStack = NSStackView()
        detailStack.orientation = .vertical
        detailStack.alignment = .leading
        detailStack.spacing = 8
        detailStack.addArrangedSubview(messageTitle)
        detailStack.addArrangedSubview(NSTextField(wrappingLabelWithString: L10n.t(.recordedOnlyNote)))
        let messageScroll = NSScrollView()
        messageScroll.hasVerticalScroller = true
        messageView.frame = NSRect(x: 0, y: 0, width: 600, height: 220)
        messageView.isVerticallyResizable = true
        messageView.isHorizontallyResizable = false
        messageView.autoresizingMask = [.width]
        messageView.textContainer?.widthTracksTextView = true
        messageView.isEditable = false
        messageView.isSelectable = true
        messageView.font = .systemFont(ofSize: 13)
        messageView.string = L10n.t(.detailSelectHint)
        messageScroll.documentView = messageView
        messageScroll.heightAnchor.constraint(equalToConstant: 220).isActive = true
        detailStack.addArrangedSubview(messageScroll)
        messageScroll.widthAnchor.constraint(equalTo: detailStack.widthAnchor).isActive = true
        handoffButton.title = L10n.t(.actionHandoff)
        handoffButton.isEnabled = false
        handoffButton.target = self
        handoffButton.action = #selector(changeSelectedHandoff)
        detailStack.addArrangedSubview(handoffButton)
        moreMessages.title = L10n.t(.loadMore)
        moreMessages.target = self
        moreMessages.action = #selector(loadMoreMessages)
        moreMessages.isHidden = true
        detailStack.addArrangedSubview(moreMessages)
        conversation.setContent(detailStack)
        mainStack.addArrangedSubview(conversation)
        conversation.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        // 轮次追溯详情区域
        provenanceBox.title = L10n.t(.provenanceBox)
        provenanceBox.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        let provStack = NSStackView()
        provStack.orientation = .vertical
        provStack.alignment = .leading
        provStack.spacing = 8
        provStack.translatesAutoresizingMaskIntoConstraints = false

        turnSearchField.placeholderString = L10n.t(.turnPlaceholder)
        turnSearchField.widthAnchor.constraint(equalToConstant: 260).isActive = true
        queryTurnButton.target = self
        queryTurnButton.action = #selector(handleQueryTurn)

        let searchRow = NSStackView(views: [turnSearchField, queryTurnButton])
        searchRow.orientation = .horizontal
        searchRow.spacing = 8
        provStack.addArrangedSubview(searchRow)

        let textScroll = NSScrollView()
        textScroll.hasVerticalScroller = true
        detailTextView.frame = NSRect(x: 0, y: 0, width: 600, height: 180)
        detailTextView.isVerticallyResizable = true
        detailTextView.isHorizontallyResizable = false
        detailTextView.autoresizingMask = [.width]
        detailTextView.textContainer?.widthTracksTextView = true
        detailTextView.isEditable = false
        detailTextView.font = NSFont.userFixedPitchFont(ofSize: 11)
        detailTextView.string = L10n.t(.provenanceHint)
        textScroll.documentView = detailTextView
        textScroll.heightAnchor.constraint(equalToConstant: 180).isActive = true
        provStack.addArrangedSubview(textScroll)
        textScroll.widthAnchor.constraint(equalTo: provStack.widthAnchor).isActive = true

        provenanceBox.setContent(provStack)
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
            mainStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            box.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    @objc private func reloadSessions() { refreshSessions() }
    @objc private func loadMoreSessions() { if let offset = nextSessionOffset { fetchSessions(offset: offset) } }
    @objc private func loadMoreMessages() {
        if let id = selectedContact, let offset = nextMessageOffset { fetchMessages(id: id, offset: offset) }
    }

    public func refreshSessions() {
        _ = context.collectLatestStatus()
        renderBanner()
        fetchSessions(offset: 0)
    }

    private func fetchSessions(offset: Int) {
        guard !loading && !handoffChanging else { return }
        loading = true
        updateHandoffButton()
        moreSessions.isEnabled = false
        refreshButton.isEnabled = false
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let page = try? SessionDataClient().list(offset: offset)
            DispatchQueue.main.async {
                guard let self = self else { return }
                self.loading = false
                self.moreSessions.isEnabled = true
                self.refreshButton.isEnabled = true
                self.sessionsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
                guard let page = page else {
                    self.hasAPIData = false
                    self.handoffButton.isEnabled = false
                    self.moreSessions.isHidden = true
                    self.sessionsStack.addArrangedSubview(NSTextField(wrappingLabelWithString: L10n.t(.sessionsApiUnavailable)))
                    let (_, input) = self.context.collectLatestStatus()
                    for (name, session) in input.sessions.sorted(by: { $0.key < $1.key }) {
                        self.sessionsStack.addArrangedSubview(self.createSessionRow(name: name, session: session))
                    }
                    return
                }
                if offset == 0 { self.rows = page.sessions } else { self.rows += page.sessions }
                self.nextSessionOffset = page.nextOffset
                self.moreSessions.isHidden = page.nextOffset == nil
                self.hasAPIData = true
                self.renderRecordedRows()
                self.updateHandoffButton()
            }
        }
    }

    private func renderRecordedRows() {
        sessionsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if rows.isEmpty { sessionsStack.addArrangedSubview(emptyLabel) }
        for session in rows {
            let button = NSButton(title: session.title(paused: context.statusSummary?.autoReplyPaused ?? true), target: self, action: #selector(selectConversation(_:)))
            button.identifier = NSUserInterfaceItemIdentifier(session.contactID)
            button.bezelStyle = .inline
            sessionsStack.addArrangedSubview(button)
        }
    }

    @objc private func selectConversation(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue else { return }
        selectedContact = id
        detailGeneration += 1
        messages = []
        nextMessageOffset = nil
        moreMessages.isHidden = true
        messageTitle.stringValue = rows.first(where: { $0.contactID == id })?.displayName ?? ""
        messageView.string = L10n.t(.loading)
        updateHandoffButton()
        fetchMessages(id: id, offset: 0)
    }

    private func fetchMessages(id: String, offset: Int) {
        let generation = detailGeneration
        moreMessages.isEnabled = false
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let page = try? SessionDataClient().detail(contactID: id, offset: offset)
            DispatchQueue.main.async {
                guard let self = self, self.selectedContact == id, self.detailGeneration == generation, !self.handoffChanging else { return }
                self.moreMessages.isEnabled = true
                guard let page = page else { self.messageView.string = L10n.t(.detailUnavailable); return }
                if let session = page.session, let index = self.rows.firstIndex(where: { $0.contactID == id }) {
                    self.rows[index] = session
                    self.updateHandoffButton()
                    self.renderRecordedRows()
                }
                if offset == 0 { self.messages = page.messages } else { self.messages = page.messages + self.messages }
                self.nextMessageOffset = page.nextOffset
                self.moreMessages.isHidden = page.nextOffset == nil
                self.messageView.string = self.messages.isEmpty ? L10n.t(.detailEmpty) : self.messages.map { $0.displayText }.joined(separator: "\n\n")
            }
        }
    }

    private func createSessionRow(name: String, session: SessionHealthInput) -> NSView {
        let rowStack = NSStackView()
        rowStack.orientation = .vertical
        rowStack.alignment = .leading
        rowStack.spacing = 12

        let nameLbl = NSTextField(labelWithString: name)
        nameLbl.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        nameLbl.widthAnchor.constraint(equalToConstant: 180).isActive = true

        let humanState = context.statusSummary?.autoReplyPaused == true && ["watching", "listening", "replying"].contains(session.state) ? L10n.t(.modePaused) : sessionStateHumanReadable(
            state: session.state,
            reason: session.reason,
            code: session.code,
            handoff: session.handoff
        )

        let stateLbl = NSTextField(labelWithString: humanState)
        stateLbl.font = NSFont.systemFont(ofSize: 12)
        stateLbl.widthAnchor.constraint(equalToConstant: 240).isActive = true

        if session.handoff == true || session.state == "handoff" {
            stateLbl.textColor = .systemOrange
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
        } else if session.state == "watching" || session.state == "listening" {
            stateLbl.textColor = .systemGreen
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
        } else {
            stateLbl.textColor = .systemYellow
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
        }

        return rowStack
    }

    @objc private func handleQueryTurn() {
        let query = turnSearchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        detailTextView.string = L10n.t(.turnQuerying, query)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let detail = try self.brainClient.turnDetail(turnId: query)
                DispatchQueue.main.async {
                    self.displayTurnDetail(detail)
                }
            } catch {
                DispatchQueue.main.async {
                    self.detailTextView.string = L10n.t(.turnNotFound)
                }
            }
        }
    }

    public func applyFixtureState(sessions: [String: SessionHealthInput], selectedTurnDetail: BrainTurnDetail?) {
        _ = self.view // 确保 viewDidLoad 已执行完毕
        sessionsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if sessions.isEmpty {
            sessionsStack.addArrangedSubview(emptyLabel)
        } else {
            for (name, session) in sessions.sorted(by: { $0.key < $1.key }) {
                let row = createSessionRow(name: name, session: session)
                sessionsStack.addArrangedSubview(row)
            }
        }
        if let detail = selectedTurnDetail {
            self.turnSearchField.stringValue = detail.turnId
            self.displayTurnDetail(detail)
        }
        self.view.layoutSubtreeIfNeeded()
    }

    public func displayTurnDetail(_ detail: BrainTurnDetail) {
        var text = L10n.t(.turnDetailTitle) + "\n"
        text += "Turn ID: \(detail.turnId)\n"
        text += L10n.t(.turnContact) + ": \(detail.contactId)\n"
        text += L10n.t(.turnRoleRevision) + ": " + (detail.roleId ?? L10n.t(.turnNotRecorded)) + "\n"
        // Do not expose raw provider metadata, decisions or errors in the provenance panel.
        text += L10n.t(.turnProvenanceStatus) + ": " + L10n.t(detail.hasProvenance ? .statusOK : .turnNotRecorded) + "\n"
        text += L10n.t(.turnSendKey) + ": " + (detail.sendKey ?? L10n.t(.turnNotRecorded)) + "\n"
        if let phase = detail.execution?["phase"]?.value as? String, ["sent", "triggered"].contains(phase) {
            text += L10n.t(.termSent) + " · " + L10n.t(.deliveryUnknown)
        } else { text += L10n.t(.turnPhases) + ": " + L10n.t(.modelStateUnknown) }
        if let decision = detail.decision, let action = decision["action"]?.value as? String,
           ["send", "skip", "handoff"].contains(action) {
            let label = L10n.t(action == "handoff" ? .actionHandoff : action == "skip" ? .skipTurn : .termAutoReply)
            text += "\n" + L10n.t(.turnDecision, label, decision["text"]?.value as? String ?? "")
        }
        detailTextView.string = text
    }

    private func updateHandoffButton() {
        let session = rows.first { $0.contactID == selectedContact }
        handoffButton.title = L10n.t(session?.handoff == true ? .actionResumeAutoReply : .actionHandoff)
        handoffButton.isEnabled = hasAPIData && session?.handoff != nil && !handoffChanging && !loading
    }

    @objc private func changeSelectedHandoff() {
        guard !handoffChanging, let id = selectedContact,
              let session = rows.first(where: { $0.contactID == id }), let current = session.handoff else { return }
        let human = !current
        let alert = NSAlert()
        alert.messageText = L10n.t(human ? .actionHandoff : .actionResumeAutoReply)
        alert.informativeText = L10n.t(human ? .handoffConfirmBody : .resumeConfirmBody, session.displayName)
        alert.addButton(withTitle: L10n.t(.continueAction))
        alert.addButton(withTitle: L10n.t(.cancel))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        detailGeneration += 1 // Discard detail reads started before this mutation.
        handoffChanging = true
        updateHandoffButton()
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let state = try SessionDataClient().handoff(sessionID: id, toHuman: human)
                DispatchQueue.main.async {
                    guard let self = self else { return }
                    self.handoffChanging = false
                    self.detailGeneration += 1
                    if let index = self.rows.firstIndex(where: { $0.contactID == id }) {
                        let row = self.rows[index]
                        self.rows[index] = RecordedSession(contactID: row.contactID, displayName: row.displayName,
                            kind: row.kind, unread: row.unread, autoReplying: state.handoff ? false : nil, handoff: state.handoff)
                    }
                    self.renderRecordedRows()
                    self.updateHandoffButton()
                    self.refreshSessions()
                    if let selected = self.selectedContact { self.fetchMessages(id: selected, offset: 0) }
                }
            } catch {
                DispatchQueue.main.async {
                    guard let self = self else { return }
                    self.handoffChanging = false
                    self.updateHandoffButton()
                    let failure = NSAlert()
                    failure.messageText = L10n.t(.actionFailed)
                    failure.informativeText = L10n.t(.handoffChangeFailed)
                    failure.runModal()
                }
            }
        }
    }


}

public struct RecordedSession: Decodable {
    public let contactID: String
    public let displayName: String
    public let kind: String?
    public let unread: Int?
    public let autoReplying: Bool?
    public let handoff: Bool?
    enum CodingKeys: String, CodingKey {
        case contactID = "contact_id", displayName = "display_name", kind, unread
        case autoReplying = "auto_replying", handoff
    }
    public var displayTitle: String { title(paused: false) }
    public func title(paused: Bool) -> String {
        var parts = [displayName]
        if kind == "group" { parts.append(L10n.t(.kindGroup)) }
        if kind == "private" { parts.append(L10n.t(.kindPrivate)) }
        if let unread = unread { parts.append(L10n.t(.unreadCount, unread)) }
        if handoff == true { parts.append(L10n.t(.modeManual)) }
        else if let active = autoReplying { parts.append(L10n.t(active && !paused ? .modeAuto : .modePaused)) }
        return parts.joined(separator: " · ")
    }
}

public struct RecordedMessage: Decodable {
    public let text: String?
    public let direction: String?
    public let autoReply: Bool?
    public let sendStatus: String?
    enum CodingKeys: String, CodingKey {
        case text, direction, autoReply = "auto_reply", sendStatus = "send_status"
    }
    public var displayText: String {
        var marks: [String] = []
        if direction == "inbound" { marks.append(L10n.t(.messageFromCustomer)) }
        if direction == "outbound" { marks.append(L10n.t(.messageFromUs)) }
        if autoReply == true { marks.append(L10n.t(.termAutoReply)) }
        // Only the fixed, evidenced T2 status marks a send. Null never becomes "sent".
        if sendStatus == L10n.text(.termSent, in: .zh) {
            marks.append(L10n.t(.termSent))
            marks.append(L10n.t(.deliveryUnknown))
        }
        return marks.joined(separator: " · ") + "\n" + (text ?? "")
    }
}

public struct SessionListPage: Decodable {
    public let sessions: [RecordedSession]
    public let nextOffset: Int?
    enum CodingKeys: String, CodingKey { case sessions, nextOffset = "next_offset" }
}
public struct SessionMessagePage: Decodable {
    public let session: RecordedSession?
    public let messages: [RecordedMessage]
    public let nextOffset: Int?
    enum CodingKeys: String, CodingKey { case session, messages, nextOffset = "next_offset" }
}

/// Uses T2's versioned JSON envelope and never includes command output in errors.
public final class SessionDataClient {
    public typealias Executor = (String, [String], Double) -> ControlRun
    private let executor: Executor
    private let configPath: String
    public init(configPath: String? = nil, executor: @escaping Executor = runControlCommand) {
        self.executor = executor
        self.configPath = configPath ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".WeChatRPABot/config.json").path
    }
    private func run(_ action: String, request: [String: Any]? = nil) throws -> Data {
        var args = ["control", action, "--config", configPath]
        if let request = request {
            let data = try JSONSerialization.data(withJSONObject: request, options: .sortedKeys)
            args += ["--request-json", String(decoding: data, as: UTF8.self)]
        }
        let result = executor(controlBinaryPath(), args, 15)
        guard result.status == 0 && !result.timedOut else { throw SessionDataError.unavailable }
        return result.output
    }
    private struct Envelope<Value: Decodable>: Decodable { let ok: Bool; let data: Value? }
    private func payload<T: Decodable>(_ action: String, request: [String: Any]) throws -> T {
        let envelope = try JSONDecoder().decode(Envelope<T>.self, from: run(action, request: request))
        guard envelope.ok, let value = envelope.data else { throw SessionDataError.unavailable }
        return value
    }
    public func list(offset: Int = 0) throws -> SessionListPage {
        try payload("sessions-list", request: ["limit": 50, "offset": offset])
    }
    public func detail(contactID: String, offset: Int = 0) throws -> SessionMessagePage {
        try payload("session-detail", request: ["contact_id": contactID, "limit": 50, "offset": offset])
    }
    public struct HandoffState: Decodable { public let sessionID: String; public let handoff: Bool
        enum CodingKeys: String, CodingKey { case sessionID = "session_id", handoff }
    }
    public func handoff(sessionID: String, toHuman: Bool) throws -> HandoffState {
        let state: HandoffState = try payload("session-handoff", request: ["session_id": sessionID, "to": toHuman ? "human" : "bot"])
        guard state.sessionID == sessionID, state.handoff == toHuman else { throw SessionDataError.unavailable }
        return state
    }
    public func sentToday() throws -> Int? {
        let object = try JSONSerialization.jsonObject(with: run("status")) as? [String: Any]
        let stats = object?["stats"] as? [String: Any]
        guard let count = stats?["sent_today"] as? Int, count >= 0 else { return nil }
        return count
    }
}
public enum SessionDataError: Error { case unavailable }
