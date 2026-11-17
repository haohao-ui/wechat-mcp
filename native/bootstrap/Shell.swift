// Shell.swift: Main container shell with sidebar navigation and dynamic page registration.
import Foundation
import AppKit

public struct ShellPageEntry {
    public let id: String
    public let title: String
    public let controller: NSViewController

    public init(id: String, title: String, controller: NSViewController) {
        self.id = id
        self.title = title
        self.controller = controller
    }
}

public final class ShellViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let coreCard = NSBox()
    private let coreLabel = NSTextField(wrappingLabelWithString: "")
    private let coreAction = NSButton()
    private weak var context: StatusPanelController?
    private var statusObserver: NSObjectProtocol?
    private var languageObserver: NSObjectProtocol?
    public var onLanguageChange: (() -> Void)?

    public func bindStatus(_ context: StatusPanelController) {
        self.context = context
        statusObserver = NotificationCenter.default.addObserver(forName: StatusPanelController.statusDidChange, object: context, queue: .main) { [weak self] _ in self?.renderStatus() }
        languageObserver = NotificationCenter.default.addObserver(forName: L10n.didChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.onLanguageChange?() }
        renderStatus()
    }

    deinit {
        for observer in [statusObserver, languageObserver].compactMap({ $0 }) { NotificationCenter.default.removeObserver(observer) }
    }

    private func renderStatus() {
        coreCard.title = L10n.t(.coreServiceTitle)
        guard let summary = context?.statusSummary else { return }
        coreLabel.stringValue = summary.tagText + "\n" + summary.cardText
        coreLabel.textColor = summary.level == .red ? .systemRed : summary.level == .yellow ? .systemOrange : .labelColor
        coreAction.title = summary.primaryActionTitle ?? ""
        coreAction.isHidden = summary.primaryAction == nil
    }

    @objc private func performCoreAction() {
        if let action = context?.statusSummary?.primaryAction { performStatusAction(action) }
    }

    public func performStatusAction(_ action: StatusIssueAction) {
        switch action {
        case .restartCore:
            let alert = NSAlert()
            alert.messageText = L10n.t(.confirmRestartBotTitle)
            alert.informativeText = L10n.t(.confirmRestartBotBody)
            alert.addButton(withTitle: L10n.t(.confirmRestart))
            alert.addButton(withTitle: L10n.t(.cancel))
            if alert.runModal() == .alertFirstButtonReturn { context?.restartBot() }
        case .grantScreenRecording: requestPermission(.screen)
        case .grantAccessibility: requestPermission(.accessibility)
        case .openFolderSettings: openSystemSettings(for: .desktop)
        case .gotoModel: selectPage(id: "model")
        }
    }

    private var pages: [ShellPageEntry] = []
    private let splitView = NSSplitView()
    private let sidebarTable = NSTableView()
    private let contentContainer = NSView()
    private var activePageController: NSViewController?

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 860, height: 760))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupLayout()
    }

    private func setupLayout() {
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(splitView)

        // 左侧侧边栏
        let sidebarScroll = NSScrollView()
        sidebarScroll.hasVerticalScroller = true
        sidebarScroll.autohidesScrollers = true

        let col = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("MainCol"))
        col.title = L10n.t(.appName)
        sidebarTable.addTableColumn(col)
        sidebarTable.headerView = nil
        sidebarTable.dataSource = self
        sidebarTable.delegate = self
        sidebarTable.rowHeight = 46
        sidebarTable.intercellSpacing = NSSize(width: 0, height: 4)
        sidebarScroll.documentView = sidebarTable

        // 右侧内容区
        contentContainer.translatesAutoresizingMaskIntoConstraints = false

        let sidebar = NSView()
        sidebarScroll.translatesAutoresizingMaskIntoConstraints = false
        coreCard.translatesAutoresizingMaskIntoConstraints = false
        coreCard.title = L10n.t(.coreServiceTitle)
        coreAction.target = self
        coreAction.action = #selector(performCoreAction)
        let cardStack = NSStackView(views: [coreLabel, coreAction])
        cardStack.orientation = .vertical
        cardStack.alignment = .leading
        coreCard.setContent(cardStack, inset: 4)
        sidebar.addSubview(sidebarScroll)
        sidebar.addSubview(coreCard)
        NSLayoutConstraint.activate([
            sidebarScroll.leadingAnchor.constraint(equalTo: sidebar.leadingAnchor),
            sidebarScroll.trailingAnchor.constraint(equalTo: sidebar.trailingAnchor),
            sidebarScroll.topAnchor.constraint(equalTo: sidebar.topAnchor),
            sidebarScroll.bottomAnchor.constraint(equalTo: coreCard.topAnchor, constant: -8),
            coreCard.leadingAnchor.constraint(equalTo: sidebar.leadingAnchor, constant: 8),
            coreCard.trailingAnchor.constraint(equalTo: sidebar.trailingAnchor, constant: -8),
            coreCard.bottomAnchor.constraint(equalTo: sidebar.bottomAnchor, constant: -12)
        ])
        splitView.addArrangedSubview(sidebar)
        splitView.addArrangedSubview(contentContainer)

        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: view.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            sidebar.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
            sidebar.widthAnchor.constraint(lessThanOrEqualToConstant: 250)
        ])
    }

    /// 供 Task 6 等外部模块注册新增页面（如 ContactsPage、McpPage）
    public func register(page: NSViewController, id: String, title: String) {
        // 如果已存在同 id，替换
        if let idx = pages.firstIndex(where: { $0.id == id }) {
            let replacingActive = activePageController === pages[idx].controller
            pages[idx] = ShellPageEntry(id: id, title: title, controller: page)
            if replacingActive { selectPage(at: idx) }
        } else {
            pages.append(ShellPageEntry(id: id, title: title, controller: page))
        }
        sidebarTable.reloadData()
        if activePageController == nil && !pages.isEmpty {
            selectPage(at: 0)
        }
    }

    public func selectPage(at index: Int) {
        guard index >= 0 && index < pages.count else { return }
        sidebarTable.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)

        let target = pages[index].controller
        if activePageController === target { return }

        activePageController?.view.removeFromSuperview()
        activePageController?.removeFromParent()

        activePageController = target
        addChild(target)
        target.view.translatesAutoresizingMaskIntoConstraints = false
        contentContainer.addSubview(target.view)

        NSLayoutConstraint.activate([
            target.view.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            target.view.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            target.view.topAnchor.constraint(equalTo: contentContainer.topAnchor),
            target.view.bottomAnchor.constraint(equalTo: contentContainer.bottomAnchor)
        ])
    }

    public var selectedPageID: String? { pages.first(where: { $0.controller === activePageController })?.id }

    public func selectPage(id: String) {
        if let idx = pages.firstIndex(where: { $0.id == id }) {
            selectPage(at: idx)
        }
    }

    // MARK: - NSTableViewDataSource & Delegate

    public func numberOfRows(in tableView: NSTableView) -> Int {
        return pages.count
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let entry = pages[row]
        let cell = NSTextField(wrappingLabelWithString: entry.title)
        cell.maximumNumberOfLines = 2
        cell.toolTip = entry.title
        cell.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        cell.alignment = .left
        return cell
    }

    public func tableViewSelectionDidChange(_ notification: Notification) {
        let row = sidebarTable.selectedRow
        if row >= 0 {
            selectPage(at: row)
        }
    }
}

extension NSBox {
    /// 把内容放进分组框自带的 contentView 并四边约束，分组框才会随内容撑高。
    /// 不要直接 `box.contentView = stack`：那样框的高度不随内容变化，内容溢出、互相重叠（2026-11-08 设备上所有页面"挤在一起"）。
    public func setContent(_ content: NSView, inset: CGFloat = 8) {
        guard let host = self.contentView else {
            self.contentView = content
            return
        }
        host.subviews.forEach { $0.removeFromSuperview() }
        content.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.leadingAnchor, constant: inset),
            content.trailingAnchor.constraint(equalTo: host.trailingAnchor, constant: -inset),
            content.topAnchor.constraint(equalTo: host.topAnchor, constant: inset),
            content.bottomAnchor.constraint(equalTo: host.bottomAnchor, constant: -inset),
        ])
    }
}

/// Keeps the action identity separate from its translated button title.
public final class StatusActionButton: NSButton {
    private let actionValue: StatusIssueAction
    private let handler: (StatusIssueAction) -> Void
    public init(title: String, actionValue: StatusIssueAction, handler: @escaping (StatusIssueAction) -> Void) {
        self.actionValue = actionValue
        self.handler = handler
        super.init(frame: .zero)
        self.title = title
        target = self
        action = #selector(performAction)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func performAction() { handler(actionValue) }
}
