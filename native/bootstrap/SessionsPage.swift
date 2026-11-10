// SessionsPage.swift: Active chat sessions monitoring and handoff management.
import Foundation
import AppKit

public final class SessionsPageController: NSViewController {
    private let context: StatusPanelController
    private let brainClient = BrainClient()
    private let sessionsStack = NSStackView()
    private let emptyLabel = NSTextField(labelWithString: "暂无活跃会话")

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
        refreshSessions()
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
        detailTextView.string = "暂无选中轮次。输入 turn_id 或点击会话行查看详细快照、角色修订与发送状态。"
        textScroll.documentView = detailTextView
        textScroll.heightAnchor.constraint(equalToConstant: 180).isActive = true
        textScroll.widthAnchor.constraint(equalToConstant: 640).isActive = true
        provStack.addArrangedSubview(textScroll)

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

    public func refreshSessions() {
        let (_, input) = context.collectLatestStatus()
        sessionsStack.arrangedSubviews.forEach { $0.removeFromSuperview() }

        if input.sessions.isEmpty {
            sessionsStack.addArrangedSubview(emptyLabel)
            return
        }

        for (name, session) in input.sessions.sorted(by: { $0.key < $1.key }) {
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
        nameLbl.widthAnchor.constraint(equalToConstant: 180).isActive = true

        let humanState = sessionStateHumanReadable(
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
            let resumeBtn = NSButton(title: "恢复自动回复", target: self, action: #selector(handleResumeHandoff(_:)))
            resumeBtn.identifier = NSUserInterfaceItemIdentifier(name)
            rowStack.addArrangedSubview(nameLbl)
            rowStack.addArrangedSubview(stateLbl)
            rowStack.addArrangedSubview(resumeBtn)
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

    private func displayTurnDetail(_ detail: BrainTurnDetail) {
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
}
