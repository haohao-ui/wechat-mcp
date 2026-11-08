// SessionsPage.swift: Active chat sessions monitoring and handoff management.
import Foundation
import AppKit

public final class SessionsPageController: NSViewController {
    private let context: StatusPanelController
    private let sessionsStack = NSStackView()
    private let emptyLabel = NSTextField(labelWithString: "暂无活跃会话")

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

        box.contentView = sessionsStack
        mainStack.addArrangedSubview(box)

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

    @objc private func handleResumeHandoff(_ sender: NSButton) {
        let sessionName = sender.identifier?.rawValue
        context.resumeHandoff(sessionName: sessionName)
        refreshSessions()
    }
}
