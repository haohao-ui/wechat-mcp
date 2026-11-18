// SessionsPage.swift: Active chat sessions monitoring and handoff management.
import Foundation
import AppKit

public final class SessionsPageController: NSViewController {
    private let context: StatusPanelController
    private let brainClient = BrainClient()
    /// 会话级转人工（issue #2 AC5 / T8）：走只读/受控的 control session-handoff。
    private let handoffClient = SessionDataClient()
    private let sessionsStack = NSStackView()
    private let emptyLabel = NSTextField(labelWithString: "暂无活跃会话")
    private let handoffFeedback = NSTextField(labelWithString: "")

    // Provenance Detail Area
    private let provenanceBox = NSBox()
    private let turnSearchField = NSTextField()
    private let queryTurnButton = NSButton(title: "查询轮次", target: nil, action: nil)
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
        mainStack.spacing = 16
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        let box = NSBox()
        box.title = "会话监听列表"
        box.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        sessionsStack.orientation = .vertical
        sessionsStack.alignment = .leading
        sessionsStack.spacing = 10
        sessionsStack.translatesAutoresizingMaskIntoConstraints = false

        emptyLabel.textColor = .secondaryLabelColor
        sessionsStack.addArrangedSubview(emptyLabel)

        box.setContent(sessionsStack)
        mainStack.addArrangedSubview(box)

        // 转人工/恢复的即时反馈（失败原因直接写在界面上，不弹系统错误框）
        handoffFeedback.textColor = .secondaryLabelColor
        handoffFeedback.font = NSFont.systemFont(ofSize: 11)
        mainStack.addArrangedSubview(handoffFeedback)

        // 轮次追溯详情区域
        provenanceBox.title = "对话轮次追溯 (Provenance)"
        provenanceBox.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)

        let provStack = NSStackView()
        provStack.orientation = .vertical
        provStack.alignment = .leading
        provStack.spacing = 8
        provStack.translatesAutoresizingMaskIntoConstraints = false

        turnSearchField.placeholderString = "输入 turn_id 查询详细追溯"
        turnSearchField.widthAnchor.constraint(equalToConstant: 260).isActive = true
        queryTurnButton.target = self
        queryTurnButton.action = #selector(handleQueryTurn)

        let searchRow = NSStackView(views: [turnSearchField, queryTurnButton])
        searchRow.orientation = .horizontal
        searchRow.spacing = 8
        provStack.addArrangedSubview(searchRow)

        let textScroll = NSScrollView()
        textScroll.hasVerticalScroller = true
        detailTextView.isEditable = false
        detailTextView.font = NSFont.userFixedPitchFont(ofSize: 11)
        detailTextView.string = "暂无选中轮次。请在上方输入框手动输入有效 turn_id 并点击查询，以查看详细快照、角色修订与发送状态。"
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
            box.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            provenanceBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    public func refreshSessions() {
        let (_, input) = context.collectLatestStatus()
        renderSessions(input.sessions)
    }

    private func renderSessions(_ sessions: [String: SessionHealthInput]) {
        sessionsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        if sessions.isEmpty {
            sessionsStack.addArrangedSubview(emptyLabel)
            return
        }

        for (name, session) in sessions.sorted(by: { $0.key < $1.key }) {
            let row = createSessionRow(name: name, session: session)
            sessionsStack.addArrangedSubview(row)
        }
    }

    private func createSessionRow(name: String, session: SessionHealthInput) -> NSView {
        let rowStack = NSStackView()
        rowStack.orientation = .horizontal
        rowStack.alignment = .centerY
        rowStack.spacing = 12

        let nameLbl = NSTextField(labelWithString: name)
        nameLbl.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        nameLbl.widthAnchor.constraint(equalToConstant: 140).isActive = true

        let humanState = sessionStateHumanReadable(
            state: session.state,
            reason: session.reason,
            code: session.code,
            handoff: session.handoff
        )

        let stateLbl = NSTextField(wrappingLabelWithString: humanState)
        stateLbl.font = NSFont.systemFont(ofSize: 12)
        stateLbl.widthAnchor.constraint(lessThanOrEqualToConstant: 320).isActive = true

        if session.handoff == true || session.state == "handoff" {
            stateLbl.textColor = .systemOrange
            let resumeBtn = NSButton(title: "恢复自动回复", target: self, action: #selector(handleResumeHandoff(_:)))
            resumeBtn.identifier = NSUserInterfaceItemIdentifier(name)
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
            rowStack.addArrangedSubview(resumeBtn)
        } else if session.state == "watching" || session.state == "listening" {
            stateLbl.textColor = .systemGreen
            let handoffBtn = NSButton(title: "转人工", target: self, action: #selector(handleHandoffToHuman(_:)))
            handoffBtn.identifier = NSUserInterfaceItemIdentifier(name)
            handoffBtn.toolTip = "停止自动回复，把这个会话交给人工处理"
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
            rowStack.addArrangedSubview(handoffBtn)
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
        detailTextView.string = "正在查询轮次 [\(query)] 的追溯详情..."
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let detail = try self.brainClient.turnDetail(turnId: query)
                DispatchQueue.main.async {
                    self.displayTurnDetail(detail)
                }
            } catch {
                DispatchQueue.main.async {
                    self.detailTextView.string = "未找到轮次追溯 (该轮次 ID 不存在或为旧版无追溯轮次): \(error.localizedDescription)"
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
        var text = "【轮次追溯详情】\n"
        text += "Turn ID: \(detail.turnId)\n"
        text += "联系人: \(detail.contactId)\n"
        text += "追溯记录状态: \(detail.provenanceDisplayText)\n"
        text += "角色修订: \(detail.roleId ?? "未记录")\n"
        if let model = detail.model {
            let p = model["provider"]?.value as? String ?? "-"
            let m = model["model"]?.value as? String ?? "-"
            text += "模型引用: \(p) / \(m)\n"
        }
        text += "幂等键: \(detail.sendKey ?? "-")\n"
        text += "执行状态阶段: \(detail.phaseLabels.joined(separator: " · "))\n"
        if let dec = detail.decision {
            let act = dec["action"]?.value as? String ?? "-"
            let reply = dec["text"]?.value as? String ?? ""
            text += "大脑决策: \(act) | 回复: \(reply)\n"
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
        sender.isEnabled = false
        handoffFeedback.textColor = .secondaryLabelColor
        handoffFeedback.stringValue = "正在把「\(name)」转人工…"
        let client = handoffClient
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let outcome: Result<Void, Error> = {
                do {
                    let page = try client.list()
                    let matches = page.sessions.filter { $0.displayName == name }
                    guard matches.count == 1, let target = matches.first else {
                        let message = matches.isEmpty
                            ? "历史库里找不到会话「\(name)」，无法定位稳定会话 ID"
                            : "存在 \(matches.count) 个同名会话「\(name)」，不能按名字转人工"
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
                    self.handoffFeedback.textColor = .systemGreen
                    self.handoffFeedback.stringValue = "已把「\(name)」转人工，自动回复已停止"
                case .failure(let error):
                    sender.isEnabled = true
                    self.handoffFeedback.textColor = .systemRed
                    self.handoffFeedback.stringValue = "转人工失败：\(error.localizedDescription)"
                }
                self.refreshSessions()
            }
        }
    }
}
