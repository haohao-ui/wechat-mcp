// ContactsPage.swift: Address book sync, category tabs, and contact handling management.
import Foundation
import AppKit

// MARK: - Shared Theme tokens
private enum ContactsPageTokens {
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
private enum ContactsPageStrings {
    static var emptyAll: String { L10n.t(.contactsPageEmptyAll) }
    static var emptyCategory: String { L10n.t(.contactsPageEmptyCategory) }
    static var selectDetailPrompt: String { L10n.t(.contactsPageSelectDetailPrompt) }
    static var syncStarted: String { L10n.t(.contactsPageSyncStarted) }
    static var personaBinding: String { L10n.t(.contactsPagePersonaBinding) }
}

/// 页面逻辑的校验错误（不要让 String 直接实现 Error：设备上的 Swift 5.7 不支持 @retroactive）
public struct PageError: Error, CustomStringConvertible, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var description: String { message }
}

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
            throw NSError(domain: "ContactsPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "Invalid UTF-8 data"])
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

    /// 分组的中文名（同步状态里不显示 work/personal 这类内部代码）
    public static func categoryTitle(_ category: String) -> String {
        switch category {
        case "personal": return L10n.t(.contactsCategoryPersonal)
        case "work": return L10n.t(.contactsCategoryWork)
        case "group": return L10n.t(.contactsCategoryGroup)
        case "official": return L10n.t(.contactsCategoryOfficial)
        default: return category
        }
    }

    /// 列表"微信号"一列：没有微信号就显示 —（原因放在详情里；列表里写说明文字会让联系人分组也显示成企业微信）
    public static func listWechatText(_ item: ContactProfileItem) -> String {
        if let w = item.wechatId, !w.isEmpty { return w }
        return "—"
    }

    /// 详情里的微信号：没有时说明原因
    public static func detailWechatText(_ item: ContactProfileItem) -> String {
        if let w = item.wechatId, !w.isEmpty { return w }
        switch item.category {
        case "work": return L10n.t(.contactsWechatWorkNone)
        case "personal" where item.identity != nil: return L10n.t(.contactsWechatUnread)
        default: return "—"
        }
    }

    public static func applyHandling(
        config: [String: Any],
        name: String,
        kind: String,
        category: String,
        handling: Handling
    ) -> Result<[String: Any], PageError> {
        if handling == .autoReply && category == "official" {
            return .failure(PageError(L10n.t(.contactsErrorOfficialNoAutoReply)))
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
            return .failure(PageError(error.localizedDescription))
        }
    }
}


public final class ContactsPageController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let context: StatusPanelController

    // Data state
    private var allContacts: [ContactProfileItem] = []
    private var displayedContacts: [ContactProfileItem] = []
    private var reviews: [ContactReviewItem] = []
    private var selectedCategory: String = "personal" // personal, work, group, official, ignored

    // UI elements
    private let syncInfoLabel = NSTextField(labelWithString: L10n.t(.contactsSyncLoading))
    private let reviewNoticeBox = NSBox()
    private let reviewNoticeLabel = NSTextField(wrappingLabelWithString: "")
    private let categorySegment = NSSegmentedControl()
    private let contactsTable = NSTableView()
    private let emptyLabel = NSTextField(wrappingLabelWithString: "")

    // Detail Panel
    private let detailBox = NSBox()
    private let detailNameLabel = NSTextField(labelWithString: L10n.t(.contactsNoSelected))
    private let detailWechatLabel = NSTextField(labelWithString: "-")
    private let detailRemarkLabel = NSTextField(labelWithString: "-")
    private let detailRegionLabel = NSTextField(labelWithString: "-")
    private let detailSourceLabel = NSTextField(labelWithString: "-")
    private let detailAddedOnLabel = NSTextField(labelWithString: "-")
    private let detailTagsLabel = NSTextField(labelWithString: "-")
    private let detailEnterpriseLabel = NSTextField(labelWithString: "-")
    private let detailHandlingLabel = NSTextField(labelWithString: "-")
    private let detailActionBtn = NSButton(title: L10n.t(.contactsChangeHandling), target: nil, action: nil)

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
        // 剩余高度全部给下方列表（默认按"重力区"排列不拉伸，列表只剩内容初始高度、后面的行被截掉）
        mainStack.distribution = .fill
        mainStack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(mainStack)

        // 1. 顶部操作栏与同步状态
        let topBar = NSStackView()
        topBar.orientation = .horizontal
        topBar.spacing = 12
        topBar.alignment = .centerY

        let initSyncBtn = NSButton(title: L10n.t(.contactsInitSync), target: self, action: #selector(handleInitSync))
        initSyncBtn.bezelStyle = .rounded
        let quickSyncBtn = NSButton(title: L10n.t(.contactsQuickSync), target: self, action: #selector(handleQuickSync))
        let refreshBtn = NSButton(title: L10n.t(.actionRefresh), target: self, action: #selector(handleRefresh))

        syncInfoLabel.font = NSFont.systemFont(ofSize: 12)
        syncInfoLabel.textColor = .secondaryLabelColor

        topBar.addArrangedSubview(initSyncBtn)
        topBar.addArrangedSubview(quickSyncBtn)
        topBar.addArrangedSubview(refreshBtn)
        topBar.addArrangedSubview(syncInfoLabel)
        mainStack.addArrangedSubview(topBar)

        // 2. 待确认项提示栏（默认隐藏）
        reviewNoticeBox.title = L10n.t(.contactsReviewNoticeTitle)
        reviewNoticeBox.boxType = .custom
        reviewNoticeBox.fillColor = NSColor.systemYellow.withAlphaComponent(0.15)
        reviewNoticeBox.borderColor = NSColor.systemOrange
        reviewNoticeBox.borderWidth = 1
        reviewNoticeBox.cornerRadius = 4
        reviewNoticeBox.translatesAutoresizingMaskIntoConstraints = false
        reviewNoticeLabel.font = NSFont.systemFont(ofSize: 12)
        reviewNoticeBox.setContent(reviewNoticeLabel)
        reviewNoticeBox.isHidden = true
        mainStack.addArrangedSubview(reviewNoticeBox)

        // 3. 分类标签栏
        categorySegment.segmentCount = 5
        categorySegment.setLabel(L10n.t(.contactsCategoryPersonal), forSegment: 0)
        categorySegment.setLabel(L10n.t(.contactsCategoryWork), forSegment: 1)
        categorySegment.setLabel(L10n.t(.contactsCategoryGroup), forSegment: 2)
        categorySegment.setLabel(L10n.t(.contactsCategoryOfficial), forSegment: 3)
        categorySegment.setLabel(L10n.t(.contactsCategoryIgnored), forSegment: 4)
        categorySegment.selectedSegment = 0
        categorySegment.target = self
        categorySegment.action = #selector(handleCategoryChanged)
        mainStack.addArrangedSubview(categorySegment)

        // 4. 下方：左侧表格占剩余宽度，右侧详情面板固定宽度（选中联系人才显示；隐藏时表格占满整行）。
        //    不用 NSSplitView：面板隐藏后再显示，分栏会把它留成 0 宽（实测被挤成一条）。
        let contentRow = NSStackView()
        contentRow.orientation = .horizontal
        contentRow.alignment = .top
        contentRow.distribution = .fill
        contentRow.spacing = 12
        contentRow.translatesAutoresizingMaskIntoConstraints = false

        // 左侧表格
        let tableScroll = NSScrollView()
        tableScroll.hasVerticalScroller = true
        tableScroll.translatesAutoresizingMaskIntoConstraints = false

        let colName = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("NameCol"))
        colName.title = L10n.t(.contactsColName)
        colName.width = 160
        colName.minWidth = 150  // 详情面板打开时名字列最窄也要放下约 10 个字，再长显示"…"，悬停看全名
        contactsTable.addTableColumn(colName)

        let colWx = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("WxCol"))
        colWx.title = L10n.t(.contactsColWechat)
        colWx.width = 110
        colWx.minWidth = 40
        contactsTable.addTableColumn(colWx)

        let colMsgs = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("MsgsCol"))
        colMsgs.title = L10n.t(.contactsColMsgs)
        colMsgs.width = 60
        colMsgs.minWidth = 32
        contactsTable.addTableColumn(colMsgs)

        let colHandling = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("HandlingCol"))
        colHandling.title = L10n.t(.contactsColHandling)
        colHandling.width = 140
        colHandling.minWidth = 130  // 放"自动回复"勾选框和"忽略"按钮，不能被压窄
        contactsTable.addTableColumn(colHandling)

        contactsTable.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        // 详情面板出现时表格变窄：列间距收紧，四列在最小宽度下仍能放下、不用横向滚动
        contactsTable.intercellSpacing = NSSize(width: 6, height: contactsTable.intercellSpacing.height)
        contactsTable.dataSource = self
        contactsTable.delegate = self
        contactsTable.rowHeight = 32
        tableScroll.documentView = contactsTable

        // 空态提示
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false
        emptyLabel.alignment = .center
        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.font = NSFont.systemFont(ofSize: 13)
        emptyLabel.isHidden = true
        tableScroll.addSubview(emptyLabel)

        // 右侧详情
        setupDetailBox()

        contentRow.addArrangedSubview(tableScroll)
        contentRow.addArrangedSubview(detailBox)
        // 详情面板：点了名字才出现，取消选择就收起
        detailBox.isHidden = true
        topBar.setContentHuggingPriority(.required, for: .vertical)
        categorySegment.setContentHuggingPriority(.required, for: .vertical)
        reviewNoticeBox.setContentHuggingPriority(.required, for: .vertical)
        contentRow.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)
        tableScroll.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)
        tableScroll.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        detailBox.setContentHuggingPriority(.required, for: .horizontal)
        mainStack.addArrangedSubview(contentRow)

        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            mainStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            mainStack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
            reviewNoticeBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            contentRow.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            contentRow.heightAnchor.constraint(greaterThanOrEqualToConstant: 240),
            tableScroll.heightAnchor.constraint(equalTo: contentRow.heightAnchor),
            tableScroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 380),
            detailBox.widthAnchor.constraint(equalToConstant: 230),
            emptyLabel.centerXAnchor.constraint(equalTo: tableScroll.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: tableScroll.centerYAnchor),
            emptyLabel.widthAnchor.constraint(lessThanOrEqualTo: tableScroll.widthAnchor, constant: -32)
        ])
    }

    private func setupDetailBox() {
        detailBox.title = L10n.t(.contactsDetailBoxTitle)
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
            [NSTextField(labelWithString: L10n.t(.contactsDetailWechat)), detailWechatLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailRemark)), detailRemarkLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailRegion)), detailRegionLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailAddedOn)), detailAddedOnLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailSource)), detailSourceLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailEnterprise)), detailEnterpriseLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailTags)), detailTagsLabel],
            [NSTextField(labelWithString: L10n.t(.contactsDetailHandling)), detailHandlingLabel]
        ])
        grid.rowSpacing = 6
        grid.columnSpacing = 10
        stack.addArrangedSubview(grid)

        detailActionBtn.target = self
        detailActionBtn.action = #selector(handleDetailAction)
        detailActionBtn.isEnabled = false
        stack.addArrangedSubview(detailActionBtn)

        detailBox.setContent(stack)
    }

    private func updateTabs(counts: ContactsCategoryCounts) {
        categorySegment.setLabel(L10n.t(.contactsTabPersonalCount, counts.personal), forSegment: 0)
        categorySegment.setLabel(L10n.t(.contactsTabWorkCount, counts.work), forSegment: 1)
        categorySegment.setLabel(L10n.t(.contactsTabGroupCount, counts.group), forSegment: 2)
        categorySegment.setLabel(L10n.t(.contactsTabOfficialCount, counts.official), forSegment: 3)
        categorySegment.setLabel(L10n.t(.contactsTabIgnoredCount, counts.ignored), forSegment: 4)
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
            var syncDesc = sync.complete ? L10n.t(.contactsSyncCompleted) : L10n.t(.contactsSyncIncomplete)
            if let per = sync.perCategory, !per.isEmpty {
                let parts = per.map { "\(ContactsPageLogic.categoryTitle($0.category)) \($0.synced)/\($0.expected)" }
                syncDesc += " (" + parts.joined(separator: ", ") + ")"
            }
            syncInfoLabel.stringValue = syncDesc
            syncInfoLabel.textColor = sync.complete ? .systemGreen : .systemOrange
        } else {
            syncInfoLabel.stringValue = L10n.t(.contactsSyncNever)
            syncInfoLabel.textColor = .secondaryLabelColor
        }

        if !reviews.isEmpty {
            reviewNoticeBox.isHidden = false
            let names = reviews.map { $0.name }.joined(separator: "、")
            reviewNoticeLabel.stringValue = L10n.t(.contactsReviewPrompt, reviews.count, names)
        } else {
            reviewNoticeBox.isHidden = true
        }

        filterDisplayedContacts()
    }

    private func handleLoadFallback() {
        syncInfoLabel.stringValue = L10n.t(.contactsOffline)
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
        if displayedContacts.isEmpty {
            emptyLabel.stringValue = allContacts.isEmpty ? ContactsPageStrings.emptyAll : ContactsPageStrings.emptyCategory
            emptyLabel.isHidden = false
        } else {
            emptyLabel.isHidden = true
        }
        clearDetail()
    }

    private func clearDetail() {
        detailBox.isHidden = true
        detailNameLabel.stringValue = L10n.t(.contactsNoSelected)
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
        detailBox.isHidden = false
        detailNameLabel.stringValue = item.displayName
        detailWechatLabel.stringValue = ContactsPageLogic.detailWechatText(item)
        detailRemarkLabel.stringValue = item.remark ?? "-"
        detailRegionLabel.stringValue = item.region ?? "-"
        detailSourceLabel.stringValue = item.source ?? "-"
        detailAddedOnLabel.stringValue = item.addedOn ?? "-"
        detailTagsLabel.stringValue = (item.tags?.isEmpty ?? true) ? "-" : item.tags!.joined(separator: ", ")
        let ent = [item.enterprise, item.realName].compactMap { $0 }.joined(separator: " / ")
        detailEnterpriseLabel.stringValue = ent.isEmpty ? "-" : ent

        switch item.handling {
        case "auto_reply":
            detailHandlingLabel.stringValue = L10n.t(.termAutoReply)
            detailActionBtn.title = L10n.t(.contactsActionSetIgnore)
            detailActionBtn.isEnabled = true
        case "ignored":
            detailHandlingLabel.stringValue = L10n.t(.contactsHandlingIgnored)
            detailActionBtn.title = L10n.t(.contactsActionUnignore)
            detailActionBtn.isEnabled = true
        default:
            if item.category == "official" {
                detailHandlingLabel.stringValue = L10n.t(.contactsHandlingNeverReply)
                detailActionBtn.title = L10n.t(.contactsActionImmutable)
                detailActionBtn.isEnabled = false
            } else {
                detailHandlingLabel.stringValue = L10n.t(.contactsHandlingRecordOnly)
                detailActionBtn.title = L10n.t(.contactsActionAddAutoReply)
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
        alert.messageText = L10n.t(.contactsInitSyncDialogTitle)
        alert.informativeText = L10n.t(.contactsInitSyncDialogMsg)
        alert.addButton(withTitle: L10n.t(.contactsInitSyncDialogConfirm))
        alert.addButton(withTitle: L10n.t(.cancel))
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
        syncInfoLabel.stringValue = L10n.t(.contactsPageSyncStarted)
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
            alert.messageText = L10n.t(.actionFailedTitle)
            alert.informativeText = err.message
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
            // 名字放不下时末尾显示"…"并可悬停看全名（不能直接裁掉：用户会以为名字只有这么长）
            let cell = NSTextField(labelWithString: item.displayName)
            cell.font = NSFont.systemFont(ofSize: 12)
            cell.lineBreakMode = .byTruncatingTail
            cell.toolTip = item.displayName
            return cell
        case "WxCol":
            let wx = ContactsPageLogic.listWechatText(item)
            let cell = NSTextField(labelWithString: wx)
            cell.lineBreakMode = .byTruncatingTail
            cell.toolTip = wx
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
                let lbl = NSTextField(labelWithString: L10n.t(.contactsHandlingNeverReply))
                lbl.font = NSFont.systemFont(ofSize: 11)
                lbl.textColor = .secondaryLabelColor
                container.addArrangedSubview(lbl)
            } else if item.handling == "ignored" {
                let lbl = NSTextField(labelWithString: L10n.t(.contactsHandlingIgnored))
                lbl.font = NSFont.systemFont(ofSize: 11)
                lbl.textColor = .systemRed

                let unignoreBtn = NSButton(title: L10n.t(.contactsActionUnignore), target: self, action: #selector(handleTableAction(_:)))
                unignoreBtn.bezelStyle = .inline
                unignoreBtn.tag = row
                unignoreBtn.identifier = NSUserInterfaceItemIdentifier("unignore")

                container.addArrangedSubview(lbl)
                container.addArrangedSubview(unignoreBtn)
            } else {
                let chk = NSButton(checkboxWithTitle: L10n.t(.termAutoReply), target: self, action: #selector(handleTableAction(_:)))
                chk.state = (item.handling == "auto_reply") ? .on : .off
                chk.tag = row
                chk.identifier = NSUserInterfaceItemIdentifier("toggle_auto_reply")

                let ignoreBtn = NSButton(title: L10n.t(.contactsActionIgnore), target: self, action: #selector(handleTableAction(_:)))
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
