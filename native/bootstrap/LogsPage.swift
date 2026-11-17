// LogsPage.swift: Realtime log tailing, filtering, and important events view.
import Foundation
import AppKit

// MARK: - Shared Theme tokens
private enum LogsPageTokens {
    static let brand = Theme.Colors.brand
    static let brandDeep = Theme.Colors.primaryButtonDefault
    static let statusOkText = Theme.Colors.statusNormalText
    static let statusOkBg = Theme.Colors.statusNormalBg
    static let statusWarnText = Theme.Colors.statusWarningText
    static let statusWarnBg = Theme.Colors.statusWarningBg
    static let statusErrText = Theme.Colors.statusErrorText
    static let statusErrBg = Theme.Colors.statusErrorBg
    static let statusOffText = Theme.Colors.statusInactiveText
    static let statusOffBg = Theme.Colors.statusInactiveBg
    static let fieldBorder = Theme.Colors.inputBorder
}

// MARK: - Localized strings
private enum LogsPageStrings {
    static var openLogsTitle: String { L10n.t(.openLogs) }
    static var openAttachmentsTitle: String { L10n.t(.openAttachments) }
    static var copyDiagnosticsTitle: String { L10n.t(.copyDiagnostics) }
    static var exportLogsTitle: String { L10n.t(.logsPageExportLogsTitle) }
    static var diagnosticsCopied: String { L10n.t(.copiedDiagnostics) }
    static var emptyLogsTitle: String { L10n.t(.logsPageEmptyLogsTitle) }
    static var emptyFilterTitle: String { L10n.t(.logsPageEmptyFilterTitle) }
    static var exportSuccess: String { L10n.t(.logsPageExportSuccess) }
    static var exportFailed: String { L10n.t(.logsPageExportFailed) }
}

public final class LogsPageController: NSViewController {
    private let context: StatusPanelController

    // Top action bar
    private let openLogsButton = NSButton(title: LogsPageStrings.openLogsTitle, target: nil, action: nil)
    private let openAttachmentsButton = NSButton(title: LogsPageStrings.openAttachmentsTitle, target: nil, action: nil)
    private let copyDiagnosticsButton = NSButton(title: LogsPageStrings.copyDiagnosticsTitle, target: nil, action: nil)
    private let exportLogsButton = NSButton(title: LogsPageStrings.exportLogsTitle, target: nil, action: nil)
    private let copyMessageLabel = NSTextField(labelWithString: "")

    // Controls
    private let logSourcePopup = NSPopUpButton()
    private let logLevelPopup = NSPopUpButton()
    private let searchField = NSSearchField()
    private let pauseTailButton = NSButton(title: "暂停", target: nil, action: nil)
    private let clearButton = NSButton(title: "清屏", target: nil, action: nil)

    // Log display
    private let textView = NSTextView()
    private let emptyStateLabel = NSTextField(wrappingLabelWithString: "")
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
        exportLogsButton.target = self
        exportLogsButton.action = #selector(handleExportLogs)

        topBar.addArrangedSubview(openLogsButton)
        topBar.addArrangedSubview(openAttachmentsButton)
        topBar.addArrangedSubview(copyDiagnosticsButton)
        topBar.addArrangedSubview(exportLogsButton)
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

        logLevelPopup.addItems(withTitles: ["全部", "警告及以上", "只看错误"])
        logLevelPopup.target = self
        logLevelPopup.action = #selector(handleFilterChanged)

        searchField.placeholderString = "搜索日志..."
        searchField.widthAnchor.constraint(equalToConstant: 180).isActive = true
        searchField.target = self
        searchField.action = #selector(handleFilterChanged)

        pauseTailButton.target = self
        pauseTailButton.action = #selector(handlePauseResumeTail)
        clearButton.target = self
        clearButton.action = #selector(handleClear)

        controlBar.addArrangedSubview(NSTextField(labelWithString: "来源:"))
        controlBar.addArrangedSubview(logSourcePopup)
        controlBar.addArrangedSubview(NSTextField(labelWithString: "级别:"))
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
        textView.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        textView.autoresizingMask = [.width]
        scrollView.documentView = textView
        mainStack.addArrangedSubview(scrollView)

        emptyStateLabel.translatesAutoresizingMaskIntoConstraints = false
        emptyStateLabel.font = NSFont.systemFont(ofSize: 13)
        emptyStateLabel.textColor = .secondaryLabelColor
        emptyStateLabel.alignment = .center
        emptyStateLabel.isHidden = true
        scrollView.addSubview(emptyStateLabel)

        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            mainStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            mainStack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
            scrollView.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            scrollView.heightAnchor.constraint(greaterThanOrEqualToConstant: 500),
            emptyStateLabel.centerXAnchor.constraint(equalTo: scrollView.centerXAnchor),
            emptyStateLabel.centerYAnchor.constraint(equalTo: scrollView.centerYAnchor),
            emptyStateLabel.widthAnchor.constraint(lessThanOrEqualTo: scrollView.widthAnchor, constant: -32)
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

        textView.string = filtered.joined(separator: "\n")
        textView.scrollToEndOfDocument(nil)
        if filtered.isEmpty {
            emptyStateLabel.stringValue = allLines.isEmpty ? LogsPageStrings.emptyLogsTitle : LogsPageStrings.emptyFilterTitle
            emptyStateLabel.isHidden = false
        } else {
            emptyStateLabel.isHidden = true
        }
    }

    @objc private func handleSourceChanged() {
        switchLogSource()
    }

    @objc private func handleFilterChanged() {
        applyFilterAndRender()
    }

    @objc private func handlePauseResumeTail() {
        isPaused.toggle()
        pauseTailButton.title = isPaused ? "继续" : "暂停"
    }

    @objc private func handleClear() {
        allLines.removeAll()
        textView.string = ""
        emptyStateLabel.stringValue = LogsPageStrings.emptyLogsTitle
        emptyStateLabel.isHidden = false
    }

    @objc private func handleOpenLogs() {
        context.openLogs()
    }

    @objc private func handleOpenAttachments() {
        context.openAttachments()
    }

    @objc private func handleCopyDiagnostics() {
        context.copyDiagnostics()
        copyMessageLabel.stringValue = LogsPageStrings.diagnosticsCopied
        copyMessageLabel.textColor = .systemGreen
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { [weak self] in
            self?.copyMessageLabel.stringValue = ""
        }
    }

    @objc private func handleExportLogs() {
        let savePanel = NSSavePanel()
        savePanel.canCreateDirectories = true
        savePanel.nameFieldStringValue = "wechat-rpa-logs-\(Int(Date().timeIntervalSince1970)).txt"
        savePanel.begin { [weak self] response in
            guard response == .OK, let url = savePanel.url, let self = self else { return }
            let content = self.allLines.joined(separator: "\n")
            do {
                try content.write(to: url, atomically: true, encoding: .utf8)
                self.copyMessageLabel.stringValue = LogsPageStrings.exportSuccess
                self.copyMessageLabel.textColor = .systemGreen
                DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { [weak self] in
                    self?.copyMessageLabel.stringValue = ""
                }
            } catch {
                self.copyMessageLabel.stringValue = "\(LogsPageStrings.exportFailed): \(error.localizedDescription)"
                self.copyMessageLabel.textColor = .systemRed
            }
        }
    }
}
