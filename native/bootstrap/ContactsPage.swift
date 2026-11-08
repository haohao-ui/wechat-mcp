// ContactsPage.swift: Address book sync, category tabs, and contact handling management.
import Foundation
import AppKit

extension String: @retroactive Error {}

public enum Handling: String, Codable {
    case autoReply = "auto_reply"
    case recordOnly = "record_only"
    case ignored = "ignored"
    case unignore = "unignore"
}

public struct ContactProfileItem: Codable {
    public let contactId: String
    public let category: String
    public let displayName: String
    public let nickname: String?
    public let remark: String?
    public let wechatId: String?
    public let identity: String?
    public let region: String?
    public let source: String?
    public let addedOn: String?
    public let tags: [String]?
    public let enterprise: String?
    public let realName: String?
    public let handling: String
    public let removed: Bool?
    public let lastMessageAtMs: Int64?
    public let messageCount: Int?

    enum CodingKeys: String, CodingKey {
        case contactId = "contact_id"
        case category
        case displayName = "display_name"
        case nickname
        case remark
        case wechatId = "wechat_id"
        case identity
        case region
        case source
        case addedOn = "added_on"
        case tags
        case enterprise
        case realName = "real_name"
        case handling
        case removed
        case lastMessageAtMs = "last_message_at_ms"
        case messageCount = "message_count"
    }

    public init(
        contactId: String,
        category: String,
        displayName: String,
        nickname: String? = nil,
        remark: String? = nil,
        wechatId: String? = nil,
        identity: String? = nil,
        region: String? = nil,
        source: String? = nil,
        addedOn: String? = nil,
        tags: [String]? = nil,
        enterprise: String? = nil,
        realName: String? = nil,
        handling: String,
        removed: Bool? = nil,
        lastMessageAtMs: Int64? = nil,
        messageCount: Int? = nil
    ) {
        self.contactId = contactId
        self.category = category
        self.displayName = displayName
        self.nickname = nickname
        self.remark = remark
        self.wechatId = wechatId
        self.identity = identity
        self.region = region
        self.source = source
        self.addedOn = addedOn
        self.tags = tags
        self.enterprise = enterprise
        self.realName = realName
        self.handling = handling
        self.removed = removed
        self.lastMessageAtMs = lastMessageAtMs
        self.messageCount = messageCount
    }
}

public struct ContactsSyncInfo: Codable {
    public let finishedAtMs: Int64?
    public let complete: Bool
    public let perCategory: [SyncPerCategory]?
    public let unreadableRows: Int?
    public let needsReview: Int?

    enum CodingKeys: String, CodingKey {
        case finishedAtMs = "finished_at_ms"
        case complete
        case perCategory = "per_category"
        case unreadableRows = "unreadable_rows"
        case needsReview = "needs_review"
    }
}

public struct SyncPerCategory: Codable {
    public let category: String
    public let expected: Int
    public let synced: Int
}

public struct ContactReviewItem: Codable {
    public let id: Int
    public let reason: String
    public let candidates: [String]
    public let name: String
}

public struct ContactsListPayload: Codable {
    public let accountId: String?
    public let sync: ContactsSyncInfo?
    public let contacts: [ContactProfileItem]
    public let reviews: [ContactReviewItem]?

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case sync
        case contacts
        case reviews
    }
}

public struct ContactsCategoryCounts {
    public let personal: Int
    public let work: Int
    public let group: Int
    public let official: Int
    public let ignored: Int

    public init(personal: Int, work: Int, group: Int, official: Int, ignored: Int) {
        self.personal = personal
        self.work = work
        self.group = group
        self.official = official
        self.ignored = ignored
    }
}

public struct ContactsPageLogic {
    public static func parseContactsPayload(json: String) throws -> (payload: ContactsListPayload, counts: ContactsCategoryCounts) {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "ContactsPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 数据"])
        }
        let decoder = JSONDecoder()
        let payload = try decoder.decode(ContactsListPayload.self, from: data)
        let counts = calculateCounts(contacts: payload.contacts)
        return (payload, counts)
    }

    public static func calculateCounts(contacts: [ContactProfileItem]) -> ContactsCategoryCounts {
        var personal = 0
        var work = 0
        var group = 0
        var official = 0
        var ignored = 0

        for c in contacts {
            if c.handling == "ignored" {
                ignored += 1
            }
            switch c.category {
            case "personal": personal += 1
            case "work": work += 1
            case "group": group += 1
            case "official": official += 1
            default: break
            }
        }
        return ContactsCategoryCounts(personal: personal, work: work, group: group, official: official, ignored: ignored)
    }

    public static func applyHandling(
        config: [String: Any],
        name: String,
        kind: String,
        category: String,
        handling: Handling
    ) -> Result<[String: Any], String> {
        if handling == .autoReply && category == "official" {
            return .failure("公众号、服务号不可加入自动回复")
        }

        var formInputs: [String: Any] = [:]
        var allowRows = (config["allowlist"] as? [[String: Any]]) ?? []
        var ignoreRows = (config["ignorelist"] as? [[String: Any]]) ?? []

        let effectiveKind = (kind == "group") ? "group" : "private"

        switch handling {
        case .autoReply:
            ignoreRows.removeAll { ($0["recipient"] as? String) == name }
            if !allowRows.contains(where: { ($0["recipient"] as? String) == name }) {
                allowRows.append(["recipient": name, "kind": effectiveKind])
            }
        case .ignored:
            if !ignoreRows.contains(where: { ($0["recipient"] as? String) == name }) {
                ignoreRows.append(["recipient": name, "kind": effectiveKind])
            }
        case .unignore:
            ignoreRows.removeAll { ($0["recipient"] as? String) == name }
        case .recordOnly:
            if ignoreRows.contains(where: { ($0["recipient"] as? String) == name }) {
                ignoreRows.removeAll { ($0["recipient"] as? String) == name }
            } else {
                allowRows.removeAll { ($0["recipient"] as? String) == name }
            }
        }

        formInputs["allowlist_rows"] = allowRows.map { [
            "recipient": $0["recipient"] as? String ?? "",
            "kind": $0["kind"] as? String ?? "private"
        ] }
        formInputs["ignorelist_rows"] = ignoreRows.map { [
            "recipient": $0["recipient"] as? String ?? "",
            "kind": $0["kind"] as? String ?? "private"
        ] }

        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: config, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(error.localizedDescription)
        }
    }
}


public struct ControlRun {
    public let status: Int32
    public let output: Data
    public let timedOut: Bool
}

/// 运行 wechat-mcp control 子命令并取标准输出。
/// 必须边运行边读完输出再等结束：先 waitUntilExit 再读管道，输出超过管道缓冲（约 64KB）就会互相等死。
/// 标准错误丢弃（不接从不读取的管道）；超过 timeout 秒仍未结束则终止并返回 timedOut。
public func runControlCommand(executable: String, arguments: [String], timeout: Double) -> ControlRun {
    let proc = Process()
    proc.executableURL = URL(fileURLWithPath: executable)
    proc.arguments = arguments
    let pipe = Pipe()
    proc.standardOutput = pipe
    proc.standardError = FileHandle.nullDevice
    do { try proc.run() } catch { return ControlRun(status: -1, output: Data(), timedOut: false) }

    let lock = NSLock()
    var collected = Data()
    let readDone = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .utility).async {
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        lock.lock(); collected = data; lock.unlock()
        readDone.signal()
    }
    var timedOut = false
    if readDone.wait(timeout: .now() + timeout) == .timedOut {
        timedOut = true
        proc.terminate()
        _ = readDone.wait(timeout: .now() + 2)
    }
    proc.waitUntilExit()
    lock.lock(); let out = collected; lock.unlock()
    return ControlRun(status: proc.terminationStatus, output: out, timedOut: timedOut)
}

/// 宿主调用的 wechat-mcp 路径
public func controlBinaryPath() -> String {
    let rpaBin = (NSHomeDirectory() as NSString).appendingPathComponent(".wechat-rpa/bin/wechat-mcp")
    return FileManager.default.isExecutableFile(atPath: rpaBin) ? rpaBin : "/usr/local/bin/wechat-mcp"
}

public final class ContactsPageController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let context: StatusPanelController

    // Data state
    private var allContacts: [ContactProfileItem] = []
    private var displayedContacts: [ContactProfileItem] = []
    private var reviews: [ContactReviewItem] = []
    private var selectedCategory: String = "personal" // personal, work, group, official, ignored

    // UI elements
    private let syncInfoLabel = NSTextField(labelWithString: "正在加载通讯录...")
    private let reviewNoticeBox = NSBox()
    private let reviewNoticeLabel = NSTextField(wrappingLabelWithString: "")
    private let categorySegment = NSSegmentedControl()
    private let contactsTable = NSTableView()

    // Detail Panel
    private let detailBox = NSBox()
    private let detailNameLabel = NSTextField(labelWithString: "未选择联系人")
    private let detailWechatLabel = NSTextField(labelWithString: "-")
    private let detailRemarkLabel = NSTextField(labelWithString: "-")
    private let detailRegionLabel = NSTextField(labelWithString: "-")
    private let detailSourceLabel = NSTextField(labelWithString: "-")
    private let detailAddedOnLabel = NSTextField(labelWithString: "-")
    private let detailTagsLabel = NSTextField(labelWithString: "-")
    private let detailEnterpriseLabel = NSTextField(labelWithString: "-")
    private let detailHandlingLabel = NSTextField(labelWithString: "-")
    private let detailActionBtn = NSButton(title: "切换处理方式", target: nil, action: nil)

    public init(context: StatusPanelController) {
        self.context = context
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 720, height: 720))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        loadContactsData()
    }

    private func setupUI() {
        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 12
        mainStack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(mainStack)

        // 1. 顶部操作栏与同步状态
        let topBar = NSStackView()
        topBar.orientation = .horizontal
        topBar.spacing = 12
        topBar.alignment = .centerY

        let initSyncBtn = NSButton(title: "初始化通讯录", target: self, action: #selector(handleInitSync))
        initSyncBtn.bezelStyle = .rounded
        let quickSyncBtn = NSButton(title: "立即同步", target: self, action: #selector(handleQuickSync))
        let refreshBtn = NSButton(title: "刷新", target: self, action: #selector(handleRefresh))

        syncInfoLabel.font = NSFont.systemFont(ofSize: 12)
        syncInfoLabel.textColor = .secondaryLabelColor

        topBar.addArrangedSubview(initSyncBtn)
        topBar.addArrangedSubview(quickSyncBtn)
        topBar.addArrangedSubview(refreshBtn)
        topBar.addArrangedSubview(syncInfoLabel)
        mainStack.addArrangedSubview(topBar)

        // 2. 待确认项提示栏（默认隐藏）
        reviewNoticeBox.title = "待确认项"
        reviewNoticeBox.boxType = .custom
        reviewNoticeBox.fillColor = NSColor.systemYellow.withAlphaComponent(0.15)
        reviewNoticeBox.borderColor = NSColor.systemOrange
        reviewNoticeBox.borderWidth = 1
        reviewNoticeBox.cornerRadius = 4
        reviewNoticeBox.translatesAutoresizingMaskIntoConstraints = false
        reviewNoticeLabel.font = NSFont.systemFont(ofSize: 12)
        reviewNoticeBox.contentView = reviewNoticeLabel
        reviewNoticeBox.isHidden = true
        mainStack.addArrangedSubview(reviewNoticeBox)

        // 3. 分类标签栏
        categorySegment.segmentCount = 5
        categorySegment.setLabel("联系人", forSegment: 0)
        categorySegment.setLabel("企业微信", forSegment: 1)
        categorySegment.setLabel("群聊", forSegment: 2)
        categorySegment.setLabel("公众号", forSegment: 3)
        categorySegment.setLabel("已忽略", forSegment: 4)
        categorySegment.selectedSegment = 0
        categorySegment.target = self
        categorySegment.action = #selector(handleCategoryChanged)
        mainStack.addArrangedSubview(categorySegment)

        // 4. 下方分栏视图：左侧表格，右侧详情
        let splitView = NSSplitView()
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false

        // 左侧表格
        let tableScroll = NSScrollView()
        tableScroll.hasVerticalScroller = true
        tableScroll.translatesAutoresizingMaskIntoConstraints = false

        let colName = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("NameCol"))
        colName.title = "名字 / 备注"
        colName.width = 160
        contactsTable.addTableColumn(colName)

        let colWx = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("WxCol"))
        colWx.title = "微信号"
        colWx.width = 110
        contactsTable.addTableColumn(colWx)

        let colMsgs = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("MsgsCol"))
        colMsgs.title = "消息数"
        colMsgs.width = 60
        contactsTable.addTableColumn(colMsgs)

        let colHandling = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("HandlingCol"))
        colHandling.title = "处理方式"
        colHandling.width = 130
        contactsTable.addTableColumn(colHandling)

        contactsTable.dataSource = self
        contactsTable.delegate = self
        contactsTable.rowHeight = 32
        tableScroll.documentView = contactsTable

        // 右侧详情
        setupDetailBox()

        splitView.addArrangedSubview(tableScroll)
        splitView.addArrangedSubview(detailBox)
        mainStack.addArrangedSubview(splitView)

        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            mainStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            mainStack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
            reviewNoticeBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            splitView.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            tableScroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 380),
            detailBox.widthAnchor.constraint(greaterThanOrEqualToConstant: 240)
        ])
    }

    private func setupDetailBox() {
        detailBox.title = "联系人详情"
        detailBox.boxType = .primary
        detailBox.translatesAutoresizingMaskIntoConstraints = false

        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false

        detailNameLabel.font = NSFont.boldSystemFont(ofSize: 14)
        stack.addArrangedSubview(detailNameLabel)

        let grid = NSGridView(views: [
            [NSTextField(labelWithString: "微信号:"), detailWechatLabel],
            [NSTextField(labelWithString: "备注名:"), detailRemarkLabel],
            [NSTextField(labelWithString: "地区:"), detailRegionLabel],
            [NSTextField(labelWithString: "添加时间:"), detailAddedOnLabel],
            [NSTextField(labelWithString: "来源:"), detailSourceLabel],
            [NSTextField(labelWithString: "企业/实名:"), detailEnterpriseLabel],
            [NSTextField(labelWithString: "标签:"), detailTagsLabel],
            [NSTextField(labelWithString: "处理方式:"), detailHandlingLabel]
        ])
        grid.rowSpacing = 6
        grid.columnSpacing = 10
        stack.addArrangedSubview(grid)

        detailActionBtn.target = self
        detailActionBtn.action = #selector(handleDetailAction)
        detailActionBtn.isEnabled = false
        stack.addArrangedSubview(detailActionBtn)

        detailBox.contentView = stack
    }

    private func updateTabs(counts: ContactsCategoryCounts) {
        categorySegment.setLabel("联系人 (\(counts.personal))", forSegment: 0)
        categorySegment.setLabel("企业微信 (\(counts.work))", forSegment: 1)
        categorySegment.setLabel("群聊 (\(counts.group))", forSegment: 2)
        categorySegment.setLabel("公众号 (\(counts.official))", forSegment: 3)
        categorySegment.setLabel("已忽略 (\(counts.ignored))", forSegment: 4)
    }

    public func loadContactsData() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "contacts-list", "--json"], timeout: 30)
            do {
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8) {
                    let (payload, counts) = try ContactsPageLogic.parseContactsPayload(json: jsonStr)
                    DispatchQueue.main.async {
                        self?.applyPayload(payload, counts: counts)
                    }
                    return
                }
            } catch {
                // Command failed or not found
            }

            DispatchQueue.main.async {
                self?.handleLoadFallback()
            }
        }
    }

    private func applyPayload(_ payload: ContactsListPayload, counts: ContactsCategoryCounts) {
        self.allContacts = payload.contacts
        self.reviews = payload.reviews ?? []
        updateTabs(counts: counts)

        if let sync = payload.sync {
            var syncDesc = sync.complete ? "上次同步已完成" : "同步未完整"
            if let per = sync.perCategory, !per.isEmpty {
                let parts = per.map { "\($0.category) \($0.synced)/\($0.expected)" }
                syncDesc += " (" + parts.joined(separator: ", ") + ")"
            }
            syncInfoLabel.stringValue = syncDesc
            syncInfoLabel.textColor = sync.complete ? .systemGreen : .systemOrange
        } else {
            syncInfoLabel.stringValue = "尚未同步通讯录"
            syncInfoLabel.textColor = .secondaryLabelColor
        }

        if !reviews.isEmpty {
            reviewNoticeBox.isHidden = false
            let names = reviews.map { $0.name }.joined(separator: "、")
            reviewNoticeLabel.stringValue = "存在 \(reviews.count) 条重名待确认联系人（如：\(names)），请核对身份！"
        } else {
            reviewNoticeBox.isHidden = true
        }

        filterDisplayedContacts()
    }

    private func handleLoadFallback() {
        syncInfoLabel.stringValue = "控制命令不可用 (离线状态)"
        syncInfoLabel.textColor = .secondaryLabelColor
        // 占位数据展示
        let counts = ContactsCategoryCounts(personal: 0, work: 0, group: 0, official: 0, ignored: 0)
        updateTabs(counts: counts)
        self.allContacts = []
        filterDisplayedContacts()
    }

    private func filterDisplayedContacts() {
        switch selectedCategory {
        case "ignored":
            displayedContacts = allContacts.filter { $0.handling == "ignored" }
        case "work":
            displayedContacts = allContacts.filter { $0.category == "work" && $0.handling != "ignored" }
        case "group":
            displayedContacts = allContacts.filter { $0.category == "group" && $0.handling != "ignored" }
        case "official":
            displayedContacts = allContacts.filter { $0.category == "official" && $0.handling != "ignored" }
        default:
            displayedContacts = allContacts.filter { $0.category == "personal" && $0.handling != "ignored" }
        }
        contactsTable.reloadData()
        clearDetail()
    }

    private func clearDetail() {
        detailNameLabel.stringValue = "未选择联系人"
        detailWechatLabel.stringValue = "-"
        detailRemarkLabel.stringValue = "-"
        detailRegionLabel.stringValue = "-"
        detailSourceLabel.stringValue = "-"
        detailAddedOnLabel.stringValue = "-"
        detailTagsLabel.stringValue = "-"
        detailEnterpriseLabel.stringValue = "-"
        detailHandlingLabel.stringValue = "-"
        detailActionBtn.isEnabled = false
    }

    private func showDetail(for item: ContactProfileItem) {
        detailNameLabel.stringValue = item.displayName
        detailWechatLabel.stringValue = item.wechatId ?? (item.identity == "weak" ? "（弱身份，无微信号）" : "-")
        detailRemarkLabel.stringValue = item.remark ?? "-"
        detailRegionLabel.stringValue = item.region ?? "-"
        detailSourceLabel.stringValue = item.source ?? "-"
        detailAddedOnLabel.stringValue = item.addedOn ?? "-"
        detailTagsLabel.stringValue = (item.tags?.isEmpty ?? true) ? "-" : item.tags!.joined(separator: ", ")
        let ent = [item.enterprise, item.realName].compactMap { $0 }.joined(separator: " / ")
        detailEnterpriseLabel.stringValue = ent.isEmpty ? "-" : ent

        switch item.handling {
        case "auto_reply":
            detailHandlingLabel.stringValue = "自动回复"
            detailActionBtn.title = "设为忽略"
            detailActionBtn.isEnabled = true
        case "ignored":
            detailHandlingLabel.stringValue = "已忽略"
            detailActionBtn.title = "取消忽略"
            detailActionBtn.isEnabled = true
        default:
            if item.category == "official" {
                detailHandlingLabel.stringValue = "永不回复"
                detailActionBtn.title = "不可修改"
                detailActionBtn.isEnabled = false
            } else {
                detailHandlingLabel.stringValue = "仅记录"
                detailActionBtn.title = "加入自动回复"
                detailActionBtn.isEnabled = true
            }
        }
    }

    // MARK: - Actions

    @objc private func handleCategoryChanged(_ sender: NSSegmentedControl) {
        switch sender.selectedSegment {
        case 1: selectedCategory = "work"
        case 2: selectedCategory = "group"
        case 3: selectedCategory = "official"
        case 4: selectedCategory = "ignored"
        default: selectedCategory = "personal"
        }
        filterDisplayedContacts()
    }

    @objc private func handleInitSync() {
        let alert = NSAlert()
        alert.messageText = "初始化通讯录"
        alert.informativeText = "将暂停自动回复并全量同步微信通讯录，预计耗时 2-3 分钟。是否继续？"
        alert.addButton(withTitle: "开始初始化")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn {
            triggerSync(full: true)
        }
    }

    @objc private func handleQuickSync() {
        triggerSync(full: false)
    }

    @objc private func handleRefresh() {
        loadContactsData()
    }

    private func triggerSync(full: Bool) {
        syncInfoLabel.stringValue = "正在启动通讯录同步..."
        syncInfoLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            // 只提交同步请求（由核心在后台执行），不等待同步完成；同步结果由 contacts-list 的 sync 字段反映
            _ = runControlCommand(executable: controlBinaryPath(), arguments: full ? ["control", "contacts-sync", "--full"] : ["control", "contacts-sync"], timeout: 30)

            DispatchQueue.main.async {
                self?.loadContactsData()
            }
        }
    }

    @objc private func handleDetailAction() {
        let row = contactsTable.selectedRow
        guard row >= 0 && row < displayedContacts.count else { return }
        let item = displayedContacts[row]

        let nextHandling: Handling
        switch item.handling {
        case "auto_reply": nextHandling = .ignored
        case "ignored": nextHandling = .unignore
        default: nextHandling = .autoReply
        }

        applyAndPersistHandling(item: item, handling: nextHandling)
    }

    private func applyAndPersistHandling(item: ContactProfileItem, handling: Handling) {
        let res = ContactsPageLogic.applyHandling(
            config: context.currentRawConfig,
            name: item.displayName,
            kind: item.category == "group" ? "group" : "private",
            category: item.category,
            handling: handling
        )

        switch res {
        case .success(let updated):
            do {
                try context.persistConfig(updated)
                loadContactsData()
            } catch {
                let alert = NSAlert(error: error)
                alert.runModal()
            }
        case .failure(let err):
            let alert = NSAlert()
            alert.messageText = "操作失败"
            alert.informativeText = err
            alert.runModal()
        }
    }

    // MARK: - NSTableViewDataSource & Delegate

    public func numberOfRows(in tableView: NSTableView) -> Int {
        return displayedContacts.count
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < displayedContacts.count, let colId = tableColumn?.identifier.rawValue else { return nil }
        let item = displayedContacts[row]

        switch colId {
        case "NameCol":
            let cell = NSTextField(labelWithString: item.displayName)
            cell.font = NSFont.systemFont(ofSize: 12)
            return cell
        case "WxCol":
            let wx = item.wechatId ?? (item.identity == "weak" ? "（企微无微信号）" : "-")
            let cell = NSTextField(labelWithString: wx)
            cell.font = NSFont.systemFont(ofSize: 11)
            cell.textColor = .secondaryLabelColor
            return cell
        case "MsgsCol":
            let count = item.messageCount ?? 0
            let cell = NSTextField(labelWithString: "\(count)")
            cell.font = NSFont.systemFont(ofSize: 11)
            return cell
        case "HandlingCol":
            let container = NSStackView()
            container.orientation = .horizontal
            container.spacing = 8
            container.alignment = .centerY

            if item.category == "official" {
                let lbl = NSTextField(labelWithString: "永不回复")
                lbl.font = NSFont.systemFont(ofSize: 11)
                lbl.textColor = .secondaryLabelColor
                container.addArrangedSubview(lbl)
            } else if item.handling == "ignored" {
                let lbl = NSTextField(labelWithString: "已忽略")
                lbl.font = NSFont.systemFont(ofSize: 11)
                lbl.textColor = .systemRed

                let unignoreBtn = NSButton(title: "取消忽略", target: self, action: #selector(handleTableAction(_:)))
                unignoreBtn.bezelStyle = .inline
                unignoreBtn.tag = row
                unignoreBtn.identifier = NSUserInterfaceItemIdentifier("unignore")

                container.addArrangedSubview(lbl)
                container.addArrangedSubview(unignoreBtn)
            } else {
                let chk = NSButton(checkboxWithTitle: "自动回复", target: self, action: #selector(handleTableAction(_:)))
                chk.state = (item.handling == "auto_reply") ? .on : .off
                chk.tag = row
                chk.identifier = NSUserInterfaceItemIdentifier("toggle_auto_reply")

                let ignoreBtn = NSButton(title: "忽略", target: self, action: #selector(handleTableAction(_:)))
                ignoreBtn.bezelStyle = .inline
                ignoreBtn.tag = row
                ignoreBtn.identifier = NSUserInterfaceItemIdentifier("ignore")

                container.addArrangedSubview(chk)
                container.addArrangedSubview(ignoreBtn)
            }
            return container
        default:
            return nil
        }
    }

    @objc private func handleTableAction(_ sender: NSButton) {
        let row = sender.tag
        guard row >= 0 && row < displayedContacts.count else { return }
        let item = displayedContacts[row]
        let actionId = sender.identifier?.rawValue ?? ""

        let nextHandling: Handling
        if actionId == "toggle_auto_reply" {
            nextHandling = (sender.state == .on) ? .autoReply : .recordOnly
        } else if actionId == "ignore" {
            nextHandling = .ignored
        } else if actionId == "unignore" {
            nextHandling = .unignore
        } else {
            return
        }

        applyAndPersistHandling(item: item, handling: nextHandling)
    }

    public func tableViewSelectionDidChange(_ notification: Notification) {
        let row = contactsTable.selectedRow
        if row >= 0 && row < displayedContacts.count {
            showDetail(for: displayedContacts[row])
        } else {
            clearDetail()
        }
    }
}
