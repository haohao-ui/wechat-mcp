// LogsPage.swift: Realtime log tailing, filtering, and important events view.
import Foundation
import AppKit

// Strip terminal control sequences only for display; preserve the original log files.
func plainLogDisplayText(_ line: String) -> String {
    line.replacingOccurrences(of: "\u{001B}\\[[0-?]*[ -/]*[@-~]", with: "", options: .regularExpression)
}

public final class LogsPageController: NSViewController {
    private let context: StatusPanelController

    // Top action bar
    private let openLogsButton = NSButton(title: L10n.t(.openLogs), target: nil, action: nil)
    private let openAttachmentsButton = NSButton(title: L10n.t(.openAttachments), target: nil, action: nil)
    private let copyDiagnosticsButton = NSButton(title: L10n.t(.copyDiagnostics), target: nil, action: nil)
    private let copyMessageLabel = NSTextField(labelWithString: "")

    // Controls
    private let logSourcePopup = NSPopUpButton()
    private let logLevelPopup = NSPopUpButton()
    private let searchField = NSSearchField()
    private let pauseTailButton = NSButton(title: L10n.t(.actionPause), target: nil, action: nil)
    private let clearButton = NSButton(title: L10n.t(.logsClear), target: nil, action: nil)

    // Log display
    private let textView = NSTextView()
    private var allLines: [String] = []
    private var currentTail: LogTail?
    private var tailTimer: Timer?
    private var isPaused = false

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
        switchLogSource()
        startTailTimer()
    }

    deinit {
        tailTimer?.invalidate()
    }

    private func setupUI() {
        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 12
        mainStack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(mainStack)

        // 1. Top action bar
        let topBar = NSStackView()
        topBar.orientation = .horizontal
        topBar.alignment = .centerY
        topBar.spacing = 10

        openLogsButton.target = self
        openLogsButton.action = #selector(handleOpenLogs)
        openAttachmentsButton.target = self
        openAttachmentsButton.action = #selector(handleOpenAttachments)
        copyDiagnosticsButton.target = self
        copyDiagnosticsButton.action = #selector(handleCopyDiagnostics)

        topBar.addArrangedSubview(openLogsButton)
        topBar.addArrangedSubview(openAttachmentsButton)
        topBar.addArrangedSubview(copyDiagnosticsButton)
        topBar.addArrangedSubview(copyMessageLabel)
        mainStack.addArrangedSubview(topBar)

        // 2. Filter & Controls
        let controlBar = NSStackView()
        controlBar.orientation = .horizontal
        controlBar.alignment = .centerY
        controlBar.spacing = 10

        logSourcePopup.addItems(withTitles: ["core.log", "bot-worker.log", "host.log"])
        logSourcePopup.target = self
        logSourcePopup.action = #selector(handleSourceChanged)

        logLevelPopup.addItems(withTitles: [L10n.t(.logsLevelAll), L10n.t(.logsLevelWarnAndAbove), L10n.t(.logsLevelErrorOnly)])
        logLevelPopup.target = self
        logLevelPopup.action = #selector(handleFilterChanged)

        searchField.placeholderString = L10n.t(.logsSearchPlaceholder)
        searchField.widthAnchor.constraint(greaterThanOrEqualToConstant: 120).isActive = true
        searchField.target = self
        searchField.action = #selector(handleFilterChanged)

        pauseTailButton.target = self
        pauseTailButton.action = #selector(handlePauseResumeTail)
        clearButton.target = self
        clearButton.action = #selector(handleClear)

        controlBar.addArrangedSubview(NSTextField(labelWithString: L10n.t(.contactsDetailSource)))
        controlBar.addArrangedSubview(logSourcePopup)
        controlBar.addArrangedSubview(NSTextField(labelWithString: L10n.t(.logsLevelLabel)))
        controlBar.addArrangedSubview(logLevelPopup)
        controlBar.addArrangedSubview(searchField)
        controlBar.addArrangedSubview(pauseTailButton)
        controlBar.addArrangedSubview(clearButton)
        mainStack.addArrangedSubview(controlBar)

        // 3. Log Scroll View
        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true

        textView.isEditable = false
        textView.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        textView.autoresizingMask = [.width]
        scrollView.documentView = textView
        mainStack.addArrangedSubview(scrollView)

        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            mainStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            mainStack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
            scrollView.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            scrollView.heightAnchor.constraint(greaterThanOrEqualToConstant: 200)
        ])
    }

    private func startTailTimer() {
        tailTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.pollLogs()
        }
    }

    private func switchLogSource() {
        let source = logSourcePopup.titleOfSelectedItem ?? "core.log"
        let logDir = context.logDirectoryURL
        let fileURL = logDir.appendingPathComponent(source)
        currentTail = LogTail(url: fileURL, maxLines: 500)
        allLines.removeAll()
        pollLogs()
    }

    private func pollLogs() {
        guard !isPaused, let tail = currentTail else { return }
        let newLines = tail.poll()
        guard !newLines.isEmpty else { return }
        allLines.append(contentsOf: newLines)
        if allLines.count > 500 {
            allLines = Array(allLines.suffix(500))
        }
        applyFilterAndRender()
    }

    private func applyFilterAndRender() {
        let levelFilter = logLevelPopup.indexOfSelectedItem
        let keyword = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        let filtered = allLines.filter { line in
            let lower = line.lowercased()
            if !keyword.isEmpty && !lower.contains(keyword) {
                return false
            }
            if levelFilter == 1 { // 警告及以上
                return lower.contains("warn") || lower.contains("error")
            } else if levelFilter == 2 { // 只看错误
                return lower.contains("error") || lower.contains("fatal")
            }
            return true
        }

        textView.string = filtered.map(plainLogDisplayText).joined(separator: "\n")
        textView.scrollToEndOfDocument(nil)
    }

    @objc private func handleSourceChanged() {
        switchLogSource()
    }

    @objc private func handleFilterChanged() {
        applyFilterAndRender()
    }

    @objc private func handlePauseResumeTail() {
        isPaused.toggle()
        pauseTailButton.title = isPaused ? L10n.t(.actionResume) : L10n.t(.actionPause)
    }

    @objc private func handleClear() {
        allLines.removeAll()
        textView.string = ""
    }

    @objc private func handleOpenLogs() {
        context.openLogs()
    }

    @objc private func handleOpenAttachments() {
        context.openAttachments()
    }

    @objc private func handleCopyDiagnostics() {
        context.copyDiagnostics()
        copyMessageLabel.stringValue = L10n.t(.copiedDiagnostics)
        copyMessageLabel.textColor = .systemGreen
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { [weak self] in
            self?.copyMessageLabel.stringValue = ""
        }
    }
}
