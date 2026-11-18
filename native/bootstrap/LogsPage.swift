// LogsPage.swift: Realtime log tailing, filtering, and important events view.
import Foundation
import AppKit

// Strip terminal control sequences only for display; preserve the original log files.
func plainLogDisplayText(_ line: String) -> String {
    line.replacingOccurrences(of: "\u{001B}\\[[0-?]*[ -/]*[@-~]", with: "", options: .regularExpression)
}

public final class LogsPageController: NSViewController, ShellPageActionProviding {
    private let context: StatusPanelController

    private static let pageMargin: CGFloat = 24
    private static let labelColumnWidth: CGFloat = 70

    // Top action bar（无 Shell 时在本页顶部；接入 Shell 时移到页标题行右侧）
    private var topBar: NSStackView!
    private var topBarCollapsed = false
    private let openLogsButton = NSButton(title: L10n.t(.openLogs), target: nil, action: nil)
    private let openAttachmentsButton = NSButton(title: L10n.t(.openAttachments), target: nil, action: nil)
    private let copyDiagnosticsButton = NSButton(title: L10n.t(.copyDiagnostics), target: nil, action: nil)
    private let copyStatusTag = ThemeStatusTag(status: .normal, customText: L10n.t(.copiedDiagnostics))

    // Controls
    private let logSourcePopup = NSPopUpButton()
    private let levelControl = NSSegmentedControl(
        labels: [L10n.t(.logsLevelAll), L10n.t(.logsLevelWarnAndAbove), L10n.t(.logsLevelErrorOnly)],
        trackingMode: .selectOne,
        target: nil,
        action: nil
    )
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

    // MARK: - Shared visual helpers (令牌全部来自 Theme.Colors)

    private func styleSecondaryButton(_ button: NSButton) {
        button.bezelStyle = .rounded
        button.controlSize = .regular
        button.font = NSFont.systemFont(ofSize: 13)
        button.setContentHuggingPriority(.required, for: .horizontal)
    }

    private func makeFieldLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        label.textColor = Theme.Colors.textPrimary
        label.setContentHuggingPriority(.required, for: .horizontal)
        label.setContentCompressionResistancePriority(.required, for: .horizontal)
        label.widthAnchor.constraint(equalToConstant: Self.labelColumnWidth).isActive = true
        return label
    }

    private func makeSpacer() -> NSView {
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        spacer.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return spacer
    }

    private func makeRow(_ views: [NSView], spacing: CGFloat = 10) -> NSStackView {
        let row = NSStackView(views: views)
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = spacing
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }

    private func makeDivider() -> NSView {
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = Theme.Colors.cardBorder.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        divider.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return divider
    }

    // MARK: - Layout

    private func setupUI() {
        view.wantsLayer = true
        view.layer?.backgroundColor = Theme.Colors.pageBackground.cgColor

        // 1. 顶部操作行：次要按钮 + 复制成功状态
        openLogsButton.target = self
        openLogsButton.action = #selector(handleOpenLogs)
        openAttachmentsButton.target = self
        openAttachmentsButton.action = #selector(handleOpenAttachments)
        copyDiagnosticsButton.target = self
        copyDiagnosticsButton.action = #selector(handleCopyDiagnostics)
        for button in [openLogsButton, openAttachmentsButton, copyDiagnosticsButton] {
            styleSecondaryButton(button)
        }
        copyStatusTag.isHidden = true
        copyStatusTag.setContentHuggingPriority(.required, for: .horizontal)
        topBar = makeRow([openLogsButton, openAttachmentsButton, copyDiagnosticsButton, copyStatusTag, makeSpacer()])
        view.addSubview(topBar)

        let divider = makeDivider()
        view.addSubview(divider)

        // 2. 筛选与搜索
        logSourcePopup.addItems(withTitles: ["core.log", "bot-worker.log", "host.log"])
        logSourcePopup.target = self
        logSourcePopup.action = #selector(handleSourceChanged)

        levelControl.selectedSegment = 0
        levelControl.segmentStyle = .rounded
        levelControl.font = NSFont.systemFont(ofSize: 12)
        levelControl.target = self
        levelControl.action = #selector(handleLevelChanged)
        levelControl.widthAnchor.constraint(equalToConstant: 240).isActive = true

        searchField.placeholderString = L10n.t(.logsSearchPlaceholder)
        searchField.font = NSFont.systemFont(ofSize: 13)
        searchField.target = self
        searchField.action = #selector(handleFilterChanged)
        searchField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        searchField.widthAnchor.constraint(greaterThanOrEqualToConstant: 120).isActive = true

        pauseTailButton.target = self
        pauseTailButton.action = #selector(handlePauseResumeTail)
        styleSecondaryButton(pauseTailButton)
        clearButton.target = self
        clearButton.action = #selector(handleClear)
        styleSecondaryButton(clearButton)

        let sourceRow = makeRow([makeFieldLabel(L10n.t(.logsSourceLabel)), logSourcePopup, makeSpacer()])
        let levelRow = makeRow([
            makeFieldLabel(L10n.t(.logsLevelLabel)),
            levelControl,
            searchField,
            pauseTailButton,
            clearButton
        ])
        let filterStack = NSStackView(views: [sourceRow, levelRow])
        filterStack.orientation = .vertical
        filterStack.alignment = .leading
        filterStack.spacing = 10
        filterStack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(filterStack)

        // 3. 日志白卡片
        let logCard = NSBox()
        logCard.title = ""
        logCard.titlePosition = .noTitle
        logCard.boxType = .primary
        logCard.translatesAutoresizingMaskIntoConstraints = false

        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = Theme.Colors.cardBackground
        scrollView.borderType = .noBorder

        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = true
        textView.drawsBackground = true
        textView.backgroundColor = Theme.Colors.cardBackground
        textView.textColor = Theme.Colors.textPrimary
        textView.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        textView.textContainerInset = NSSize(width: 12, height: 12)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        scrollView.documentView = textView
        logCard.setContent(scrollView, inset: 8)
        view.addSubview(logCard)

        NSLayoutConstraint.activate([
            topBar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.pageMargin),
            topBar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.pageMargin),
            topBar.topAnchor.constraint(equalTo: view.topAnchor, constant: 8),

            divider.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.pageMargin),
            divider.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.pageMargin),
            divider.topAnchor.constraint(equalTo: topBar.bottomAnchor, constant: 14),

            filterStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.pageMargin),
            filterStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.pageMargin),
            filterStack.topAnchor.constraint(equalTo: divider.bottomAnchor, constant: 12),

            sourceRow.widthAnchor.constraint(equalTo: filterStack.widthAnchor),
            levelRow.widthAnchor.constraint(equalTo: filterStack.widthAnchor),

            logCard.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.pageMargin),
            logCard.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.pageMargin),
            logCard.topAnchor.constraint(equalTo: filterStack.bottomAnchor, constant: 16),
            logCard.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
            logCard.heightAnchor.constraint(greaterThanOrEqualToConstant: 200)
        ])
    }

    // MARK: - Tailing

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

    // MARK: - Rendering

    private func levelColor(for token: String) -> NSColor? {
        switch token {
        case "[ERROR]", "[FATAL]", "ERROR", "FATAL":
            return Theme.Colors.statusErrorText
        case "[WARN]", "[WARNING]", "WARN", "WARNING":
            return Theme.Colors.statusWarningText
        case "[INFO]", "INFO":
            return Theme.Colors.statusNormalText
        case "[DEBUG]", "DEBUG", "[TRACE]", "TRACE":
            return Theme.Colors.textSecondary
        default:
            return nil
        }
    }

    private func attributedLogText(_ lines: [String]) -> NSAttributedString {
        let baseFont = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = 22
        paragraph.maximumLineHeight = 22
        paragraph.lineBreakMode = .byClipping

        let result = NSMutableAttributedString()
        let tokens = ["[ERROR]", "[FATAL]", "[WARN]", "[WARNING]", "[INFO]", "[DEBUG]", "[TRACE]",
                      "ERROR", "FATAL", "WARN", "WARNING", "INFO", "DEBUG", "TRACE"]
        for (index, raw) in lines.enumerated() {
            let line = plainLogDisplayText(raw) + (index == lines.count - 1 ? "" : "\n")
            let attributed = NSMutableAttributedString(string: line, attributes: [
                .font: baseFont,
                .foregroundColor: Theme.Colors.textPrimary,
                .paragraphStyle: paragraph
            ])
            let nsLine = line as NSString
            for token in tokens {
                guard let color = levelColor(for: token) else { continue }
                var searchRange = NSRange(location: 0, length: nsLine.length)
                while true {
                    let found = nsLine.range(of: token, options: [], range: searchRange)
                    if found.location == NSNotFound { break }
                    attributed.addAttribute(.foregroundColor, value: color, range: found)
                    let next = found.location + max(found.length, 1)
                    if next >= nsLine.length { break }
                    searchRange = NSRange(location: next, length: nsLine.length - next)
                }
            }
            result.append(attributed)
        }
        return result
    }

    private func applyFilterAndRender() {
        let levelFilter = levelControl.selectedSegment
        let keyword = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        let filtered = allLines.filter { line in
            let lower = line.lowercased()
            if !keyword.isEmpty && !lower.contains(keyword) {
                return false
            }
            if levelFilter == 1 { // 警告及以上
                return lower.contains("warn") || lower.contains("error") || lower.contains("fatal")
            } else if levelFilter == 2 { // 只看错误
                return lower.contains("error") || lower.contains("fatal")
            }
            return true
        }

        textView.textStorage?.setAttributedString(attributedLogText(filtered))
        textView.scrollToEndOfDocument(nil)
    }

    // MARK: - Actions

    @objc private func handleSourceChanged() {
        switchLogSource()
    }

    @objc private func handleLevelChanged() {
        applyFilterAndRender()
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
        textView.textStorage?.setAttributedString(NSAttributedString())
    }

    @objc private func handleOpenLogs() {
        context.openLogs()
    }

    @objc private func handleOpenAttachments() {
        context.openAttachments()
    }

    @objc private func handleCopyDiagnostics() {
        context.copyDiagnostics()
        copyStatusTag.isHidden = false
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { [weak self] in
            self?.copyStatusTag.isHidden = true
        }
    }

    // MARK: - ShellPageActionProviding

    /// 三个入口按钮 + 复制成功状态交给 Shell 标题行右侧；页面内 topBar 保留空容器，
    /// 无 Shell 的单页测试仍能遍历到这些按钮（容器高度塌陷为 0，下面的分隔线随之上移）。
    public func shellPageActionViews() -> [NSView] {
        guard topBar != nil else { return [] }
        let actionViews: [NSView] = [openLogsButton, openAttachmentsButton, copyDiagnosticsButton, copyStatusTag]
        for actionView in actionViews {
            if topBar.arrangedSubviews.contains(actionView) {
                topBar.removeArrangedSubview(actionView)
            }
            actionView.removeFromSuperview()
        }
        // 空 stack 的高度在手动约束下会变成不确定值，显式钉成 0，分隔线与筛选行随之上移。
        if !topBarCollapsed {
            topBarCollapsed = true
            topBar.heightAnchor.constraint(equalToConstant: 0).isActive = true
        }
        return actionViews
    }
}
