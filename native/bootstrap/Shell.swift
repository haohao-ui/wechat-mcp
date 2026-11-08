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
        col.title = "导航"
        sidebarTable.addTableColumn(col)
        sidebarTable.headerView = nil
        sidebarTable.dataSource = self
        sidebarTable.delegate = self
        sidebarTable.rowHeight = 36
        sidebarTable.intercellSpacing = NSSize(width: 0, height: 4)
        sidebarScroll.documentView = sidebarTable

        // 右侧内容区
        contentContainer.translatesAutoresizingMaskIntoConstraints = false

        splitView.addArrangedSubview(sidebarScroll)
        splitView.addArrangedSubview(contentContainer)

        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: view.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            sidebarScroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 140),
            sidebarScroll.widthAnchor.constraint(lessThanOrEqualToConstant: 180)
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

    // MARK: - NSTableViewDataSource & Delegate

    public func numberOfRows(in tableView: NSTableView) -> Int {
        return pages.count
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let entry = pages[row]
        let cell = NSTextField(labelWithString: "  \(entry.title)")
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
