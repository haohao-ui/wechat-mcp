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

/// 页面把「页标题行右侧」的操作视图（按钮、状态标签等）交给 Shell 统一摆放。
/// Shell 负责标题行布局；页面不再在内容区自绘这一行（无 Shell 的单页测试仍保留页面内动作行）。
public protocol ShellPageActionProviding: AnyObject {
    /// 页标题行右侧要展示的操作视图（按钮等）；Shell 负责布局与悬停区域。
    func shellPageActionViews() -> [NSView]
}

public final class ShellViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private var pages: [ShellPageEntry] = []
    private let splitView = NSSplitView()
    private let sidebarTable = NSTableView()
    private let contentContainer = NSView()
    private let pageHost = NSView()
    private let pageTitle = NSTextField(labelWithString: "")
    private let titleRow = NSStackView()
    private let titleSpacer = NSView()
    private let actionsStack = NSStackView()
    private var activePageController: NSViewController?
    /// 当前选中页面的 id：切换语言重建侧栏时用来保持原页面。
    public private(set) var selectedPageID: String = ""

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 1180, height: 800))
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
        col.title = "导航"
        sidebarTable.addTableColumn(col)
        sidebarTable.headerView = nil
        sidebarTable.dataSource = self
        sidebarTable.delegate = self
        sidebarTable.rowHeight = 40
        sidebarTable.style = .sourceList
        sidebarTable.backgroundColor = Theme.Colors.sidebarBackground
        sidebarTable.setAccessibilityLabel(L10n.t(.uiShell047))
        sidebarTable.intercellSpacing = NSSize(width: 0, height: 2)
        sidebarScroll.documentView = sidebarTable

        // 右侧内容区
        contentContainer.translatesAutoresizingMaskIntoConstraints = false
        contentContainer.wantsLayer = true
        contentContainer.layer?.backgroundColor = Theme.Colors.pageBackground.cgColor
        pageTitle.font = NSFont.systemFont(ofSize: 28, weight: .semibold)
        pageTitle.textColor = Theme.Colors.textPrimary
        pageTitle.setAccessibilityLabel(L10n.t(.uiShell048))
        pageTitle.setContentHuggingPriority(.required, for: .horizontal)
        pageTitle.setContentCompressionResistancePriority(.required, for: .horizontal)

        titleSpacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        titleSpacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        actionsStack.orientation = .horizontal
        actionsStack.alignment = .centerY
        actionsStack.spacing = 8

        titleRow.orientation = .horizontal
        titleRow.alignment = .centerY
        titleRow.spacing = 12
        titleRow.translatesAutoresizingMaskIntoConstraints = false
        titleRow.addArrangedSubview(pageTitle)
        titleRow.addArrangedSubview(titleSpacer)
        titleRow.addArrangedSubview(actionsStack)

        pageHost.translatesAutoresizingMaskIntoConstraints = false
        contentContainer.addSubview(titleRow)
        contentContainer.addSubview(pageHost)
        NSLayoutConstraint.activate([
            titleRow.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor, constant: 24),
            titleRow.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor, constant: -24),
            titleRow.topAnchor.constraint(equalTo: contentContainer.topAnchor, constant: 20),
            titleRow.heightAnchor.constraint(greaterThanOrEqualToConstant: 36),
            pageHost.topAnchor.constraint(equalTo: titleRow.bottomAnchor, constant: 16),
            pageHost.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            pageHost.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            pageHost.bottomAnchor.constraint(equalTo: contentContainer.bottomAnchor)
        ])

        splitView.addArrangedSubview(sidebarScroll)
        splitView.addArrangedSubview(contentContainer)

        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: view.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            sidebarScroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 220),
            sidebarScroll.widthAnchor.constraint(lessThanOrEqualToConstant: 220)
        ])
    }

    /// 供 Task 6 等外部模块注册新增页面（如 ContactsPage、McpPage）
    public func register(page: NSViewController, id: String, title: String) {
        // 如果已存在同 id，替换
        if let idx = pages.firstIndex(where: { $0.id == id }) {
            pages[idx] = ShellPageEntry(id: id, title: title, controller: page)
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
        selectedPageID = pages[index].id
        sidebarTable.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)

        let target = pages[index].controller
        pageTitle.stringValue = pages[index].title
        if activePageController === target { return }

        activePageController?.view.removeFromSuperview()
        activePageController?.removeFromParent()

        activePageController = target
        addChild(target)
        target.view.translatesAutoresizingMaskIntoConstraints = false
        pageHost.addSubview(target.view)

        NSLayoutConstraint.activate([
            target.view.leadingAnchor.constraint(equalTo: pageHost.leadingAnchor),
            target.view.trailingAnchor.constraint(equalTo: pageHost.trailingAnchor),
            target.view.topAnchor.constraint(equalTo: pageHost.topAnchor),
            target.view.bottomAnchor.constraint(equalTo: pageHost.bottomAnchor)
        ])

        // 页面视图加载完成后再取它在标题行右侧要展示的操作视图（按钮由页面持有，Shell 只负责摆放）。
        refreshPageActions(for: target)
    }

    /// 清空上一页的动作视图，换成新页面提供的按钮/状态标签；未实现协议的页面标题行右侧留空。
    private func refreshPageActions(for controller: NSViewController) {
        for actionView in actionsStack.arrangedSubviews {
            actionsStack.removeArrangedSubview(actionView)
            actionView.removeFromSuperview()
        }
        guard let provider = controller as? ShellPageActionProviding else { return }
        for actionView in provider.shellPageActionViews() {
            actionView.translatesAutoresizingMaskIntoConstraints = false
            actionsStack.addArrangedSubview(actionView)
        }
    }

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
        let symbols = ["overview": "square.grid.2x2", "sessions": "bubble.left.and.bubble.right",
                       "contacts": "person.2", "brain": "brain", "mcp": "link",
                       "settings": "gearshape", "logs": "doc.text", "update": "arrow.down.circle"]
        let icon = NSImageView(image: NSImage(systemSymbolName: symbols[entry.id] ?? "square", accessibilityDescription: nil) ?? NSImage())
        icon.contentTintColor = .secondaryLabelColor
        icon.widthAnchor.constraint(equalToConstant: 16).isActive = true
        icon.heightAnchor.constraint(equalToConstant: 16).isActive = true
        let label = NSTextField(labelWithString: entry.title)
        label.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        label.textColor = Theme.Colors.textPrimary
        let row = NSStackView(views: [icon, label])
        row.spacing = 10
        row.alignment = .centerY
        row.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 8)
        return row
    }

    public func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        return NavigationRowView()
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
        let embedded: NSView
        if boxType == .primary {
            let stack = NSStackView()
            if titlePosition != .noTitle && !title.isEmpty {
                let heading = NSTextField(labelWithString: title)
                heading.font = NSFont.systemFont(ofSize: 17, weight: .semibold)
                heading.textColor = Theme.Colors.textPrimary
                stack.addArrangedSubview(heading)
            }
            stack.addArrangedSubview(content)
            stack.orientation = .vertical
            stack.alignment = .leading
            stack.spacing = 14
            content.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
            embedded = stack
            boxType = .custom
            titlePosition = .noTitle
            fillColor = Theme.Colors.cardBackground
            borderColor = Theme.Colors.cardBorder
            borderWidth = 1
            cornerRadius = 12
        } else {
            embedded = content
        }
        guard let host = self.contentView else {
            self.contentView = content
            return
        }
        host.subviews.forEach { $0.removeFromSuperview() }
        embedded.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(embedded)
        NSLayoutConstraint.activate([
            embedded.leadingAnchor.constraint(equalTo: host.leadingAnchor, constant: inset),
            embedded.trailingAnchor.constraint(equalTo: host.trailingAnchor, constant: -inset),
            embedded.topAnchor.constraint(equalTo: host.topAnchor, constant: inset),
            embedded.bottomAnchor.constraint(equalTo: host.bottomAnchor, constant: -inset),
        ])
    }
}

private final class NavigationRowView: NSTableRowView {
    override func drawSelection(in dirtyRect: NSRect) {
        guard selectionHighlightStyle != .none else { return }
        Theme.Colors.selectedRowBackground.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 8, dy: 2), xRadius: 8, yRadius: 8).fill()
    }
}
