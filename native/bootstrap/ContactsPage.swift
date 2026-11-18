// ContactsPage.swift: Address book sync, category tabs, and contact handling management.
import Foundation
import AppKit

// MARK: - Shared Theme tokens
private enum ContactsPageTokens {
    static let pageBackground = Theme.Colors.pageBackground
    static let cardBackground = Theme.Colors.cardBackground
    static let cardBorder = Theme.Colors.cardBorder
    static let textPrimary = Theme.Colors.textPrimary
    static let textSecondary = Theme.Colors.textSecondary
    static let primary = Theme.Colors.primaryButtonDefault
    static let primaryText = Theme.Colors.primaryButtonText
    static let warningText = Theme.Colors.statusWarningText
    static let warningBg = Theme.Colors.statusWarningBg
    static let warningDot = Theme.Colors.statusWarningDot
    static let errorText = Theme.Colors.statusErrorText
    static let errorDot = Theme.Colors.statusErrorDot
    static let normalText = Theme.Colors.statusNormalText
    static let normalDot = Theme.Colors.statusNormalDot
    static let inactiveText = Theme.Colors.statusInactiveText
    static let inactiveBg = Theme.Colors.statusInactiveBg
}

// MARK: - Localized strings
private enum ContactsPageStrings {
    static var emptyAll: String { L10n.t(.contactsPageEmptyAll) }
    static var emptyCategory: String { L10n.t(.contactsPageEmptyCategory) }
    static var syncStarted: String { L10n.t(.contactsPageSyncStarted) }
    static var presetTag: String { L10n.t(.contactsSourcePresetTag) }
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
    /// 同步失败时后端写入的人话/机器原因（例如 "private visual operation failed: invalid(\"user_input_detected\")"）。
    public let error: String?
    /// 同步被中途暂停（检测到用户操作电脑或服务关闭）。
    public let stopped: Bool?

    enum CodingKeys: String, CodingKey {
        case finishedAtMs = "finished_at_ms"
        case complete
        case perCategory = "per_category"
        case unreadableRows = "unreadable_rows"
        case needsReview = "needs_review"
        case error
        case stopped
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
        if item.category == "work" { return L10n.t(.contactsWechatWorkNone) }
        if isUnsyncedPreset(item) { return "—" }
        if item.category == "personal" && item.identity != nil {
            return L10n.t(.contactsWechatUnread)
        }
        return "—"
    }

    /// 该行是否只是配置（授权名单）预置、从未由微信通讯录同步过。
    ///
    /// contacts-list 用 `contacts LEFT JOIN contact_profiles`：预置行只有 contacts 记录，没有
    /// contact_profiles 资料行，因此 wechat_id 及各资料字段全空（identity 回退为 weak、
    /// category 回退为 personal）。真实同步过的联系人至少有一行 profile（nickname NOT NULL），
    /// 所以用「微信号为空 且 所有资料字段为空」判定，避免把合法没有微信号的企业微信联系人误标。
    public static func isUnsyncedPreset(_ item: ContactProfileItem) -> Bool {
        let wechat = (item.wechatId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !wechat.isEmpty { return false }
        let hasProfile = item.nickname != nil
            || item.remark != nil
            || item.region != nil
            || item.source != nil
            || item.addedOn != nil
            || (item.tags?.isEmpty == false)
            || item.enterprise != nil
            || item.realName != nil
        return !hasProfile
    }

    /// 把后端同步错误翻成人话。未知错误退回通用文案，原始错误串放到悬停提示里。
    public static func humanizedSyncError(_ raw: String) -> String {
        let lower = raw.lowercased()
        if lower.contains("user_input_detected") {
            return L10n.t(.contactsSyncReasonUserInput)
        }
        if lower.contains("desktop_busy") || lower.contains("busy") {
            return L10n.t(.contactsSyncReasonDesktopBusy)
        }
        if lower.contains("layout_mismatch") || lower.contains("layout_missing") || lower.contains("input_divider") {
            return L10n.t(.contactsSyncReasonLayoutMismatch)
        }
        if lower.contains("target_not_reached") || lower.contains("window_not_found") || lower.contains("contacts_entry") {
            return L10n.t(.contactsSyncReasonTargetNotReached)
        }
        if lower.contains("image_snapshot_changed") {
            return L10n.t(.contactsSyncReasonImageChanged)
        }
        return L10n.t(.contactsSyncReasonGeneric)
    }

    /// 同步未完成时列出人能看懂的原因（不再只写"同步未完整"）。
    public static func incompleteSyncReasons(_ sync: ContactsSyncInfo) -> String {
        var parts: [String] = []
        if sync.stopped == true {
            parts.append(L10n.t(.contactsSyncReasonStopped))
        }
        if let unreadable = sync.unreadableRows, unreadable > 0 {
            parts.append(L10n.t(.contactsSyncReasonUnreadableFmt, String(describing: unreadable)))
        }
        if let review = sync.needsReview, review > 0 {
            parts.append(L10n.t(.contactsSyncReasonReviewFmt, String(describing: review)))
        }
        if let per = sync.perCategory {
            let mismatched = per.filter { $0.expected != $0.synced }
            if !mismatched.isEmpty {
                let detail = mismatched
                    .map { "\(categoryTitle($0.category)) \($0.synced)/\($0.expected)" }
                    .joined(separator: ", ")
                parts.append(L10n.t(.contactsSyncReasonCategoriesFmt, detail))
            }
        }
        if parts.isEmpty {
            parts.append(L10n.t(.contactsSyncReasonGeneric))
        }
        return parts.joined(separator: " ")
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

// MARK: - Page-local controls (all colors come from Theme tokens)

/// 次要按钮：白底、1px 描边、深色字（设计稿的「刷新 / 忽略 / 复制」类按钮）。
private final class ContactsSecondaryButton: NSButton {
    public init(title: String, target: AnyObject? = nil, action: Selector? = nil) {
        super.init(frame: .zero)
        self.target = target
        self.action = action
        self.title = title
        setup()
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override var title: String {
        didSet { updateTitle() }
    }

    private func setup() {
        isBordered = false
        bezelStyle = .regularSquare
        setButtonType(.momentaryPushIn)
        focusRingType = .none
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.borderWidth = 1
        layer?.masksToBounds = true
        updateTitle()
    }

    private func updateTitle() {
        layer?.backgroundColor = ContactsPageTokens.cardBackground.cgColor
        layer?.borderColor = Theme.Colors.inputBorder.cgColor
        attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: ContactsPageTokens.textPrimary,
            .font: NSFont.systemFont(ofSize: 13)
        ])
    }

    public override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: max(base.width + 28, 64), height: 32)
    }
}

/// 小圆角来源标签（预置行的「来自授权名单·未同步」、已忽略标签）。
private final class ContactsTagView: NSView {
    private let label = NSTextField(labelWithString: "")

    public init(text: String, textColor: NSColor, background: NSColor) {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 4
        layer?.masksToBounds = true
        label.stringValue = text
        label.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        label.textColor = textColor
        label.alignment = .center
        label.lineBreakMode = .byTruncatingTail
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        layer?.backgroundColor = background.cgColor
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2)
        ])
        setContentHuggingPriority(.required, for: .horizontal)
        label.setContentHuggingPriority(.required, for: .horizontal)
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

/// 分段标签按钮：选中绿底白字，未选中白底深色字。
private final class ContactsSegmentButton: NSButton {
    public var isSelectedTab: Bool = false {
        didSet { updateAppearance() }
    }

    public init(title: String, tag: Int, target: AnyObject?, action: Selector?) {
        super.init(frame: .zero)
        self.title = title
        self.tag = tag
        self.target = target
        self.action = action
        isBordered = false
        bezelStyle = .regularSquare
        setButtonType(.momentaryChange)
        focusRingType = .none
        wantsLayer = true
        layer?.cornerRadius = 0
        layer?.masksToBounds = true
        updateAppearance()
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override var title: String {
        didSet { updateAppearance() }
    }

    private func updateAppearance() {
        layer?.backgroundColor = (isSelectedTab ? ContactsPageTokens.primary : ContactsPageTokens.cardBackground).cgColor
        attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: isSelectedTab ? ContactsPageTokens.primaryText : ContactsPageTokens.textPrimary,
            .font: NSFont.systemFont(ofSize: 13, weight: .medium)
        ])
    }

    public override var intrinsicContentSize: NSSize {
        return NSSize(width: super.intrinsicContentSize.width, height: 32)
    }
}

/// 分段标签容器：外圈 1px 描边 + 8 圆角，内部选中项绿底。
private final class ContactsSegmentBar: NSView {
    private var buttons: [ContactsSegmentButton] = []
    private let stack = NSStackView()

    public var selectedIndex: Int {
        get { buttons.firstIndex(where: { $0.isSelectedTab }) ?? 0 }
        set {
            for (index, button) in buttons.enumerated() {
                button.isSelectedTab = (index == newValue)
            }
        }
    }

    public init(titles: [String], target: AnyObject?, action: Selector?) {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.masksToBounds = true
        layer?.backgroundColor = ContactsPageTokens.cardBorder.cgColor

        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.distribution = .fillEqually
        stack.spacing = 1
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        for (index, title) in titles.enumerated() {
            let button = ContactsSegmentButton(title: title, tag: index, target: target, action: action)
            button.translatesAutoresizingMaskIntoConstraints = false
            buttons.append(button)
            stack.addArrangedSubview(button)
        }
        buttons.first?.isSelectedTab = true

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 1),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -1),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 1),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -1),
            heightAnchor.constraint(equalToConstant: 34)
        ])
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public func setTitles(_ titles: [String]) {
        for (index, title) in titles.enumerated() where index < buttons.count {
            buttons[index].title = title
        }
    }
}

/// 表格行：选中 #E6F4EC，底部 1px 分隔线，无斑马纹。
private final class ContactsTableRowView: NSTableRowView {
    public override func drawSelection(in dirtyRect: NSRect) {
        guard selectionHighlightStyle != .none else { return }
        Theme.Colors.selectedRowBackground.setFill()
        bounds.fill()
    }

    public override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        Theme.Colors.cardBorder.setFill()
        NSRect(x: 0, y: 0, width: bounds.width, height: 1).fill()
    }
}

/// 表头：白底 + 底部 1px 分隔线，文字用 12pt 次要色自绘（原生表头会画灰底和分隔线）。
private final class ContactsTableHeaderView: NSTableHeaderView {
    public override func draw(_ dirtyRect: NSRect) {
        ContactsPageTokens.cardBackground.setFill()
        bounds.fill()
        ContactsPageTokens.cardBorder.setFill()
        NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()

        guard let table = tableView else { return }
        for index in 0..<table.numberOfColumns {
            let column = table.tableColumns[index]
            let attr = column.headerCell.attributedStringValue
            let size = attr.size()
            let cellRect = headerRect(ofColumn: index)
            let textRect = NSRect(
                x: cellRect.minX + 10,
                y: cellRect.midY - size.height / 2,
                width: max(cellRect.width - 20, 0),
                height: size.height
            )
            attr.draw(in: textRect)
        }
    }
}

// MARK: - Page Controller

public final class ContactsPageController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private let context: StatusPanelController

    // Data state
    private var allContacts: [ContactProfileItem] = []
    private var displayedContacts: [ContactProfileItem] = []
    private var reviews: [ContactReviewItem] = []
    private var selectedCategory: String = "personal" // personal, work, group, official, ignored

    // UI elements
    private let syncInfoLabel = NSTextField(labelWithString: L10n.t(.contactsSyncLoading))
    private let syncInfoIcon = NSImageView()
    private var contentRow: NSStackView!
    private var detailWidthConstraint: NSLayoutConstraint!
    private var detailHeightConstraint: NSLayoutConstraint!
    private var stackedDetailWidthConstraint: NSLayoutConstraint!
    private var stackedDetailHeightConstraint: NSLayoutConstraint!
    private var tableHeightConstraint: NSLayoutConstraint!
    private var stackedTableHeightConstraint: NSLayoutConstraint!
    private var contentRowMinHeightConstraint: NSLayoutConstraint!
    private var lastLayoutWidth: CGFloat = 0
    private var reviewBanner: ThemeStatusBanner!
    private var syncBanner: ThemeStatusBanner!
    private var presetBanner: ThemeStatusBanner!
    private var segmentBar: ContactsSegmentBar!
    private let contactsTable = NSTableView()
    private let emptyLabel = NSTextField(wrappingLabelWithString: "")

    // Detail Panel
    private var detailBox: NSBox!
    private let detailNameLabel = NSTextField(labelWithString: L10n.t(.contactsNoSelected))
    private let detailSourceTag = NSView()
    private let detailWechatLabel = NSTextField(labelWithString: "—")
    private let detailRemarkLabel = NSTextField(labelWithString: "—")
    private let detailRegionLabel = NSTextField(labelWithString: "—")
    private let detailSourceLabel = NSTextField(labelWithString: "—")
    private let detailAddedOnLabel = NSTextField(labelWithString: "—")
    private let detailTagsLabel = NSTextField(labelWithString: "—")
    private let detailEnterpriseLabel = NSTextField(labelWithString: "—")
    private let detailHandlingLabel = NSTextField(labelWithString: "—")
    private let detailActionBtn: ThemePrimaryButton

    public init(context: StatusPanelController) {
        self.context = context
        self.detailActionBtn = ThemePrimaryButton(title: L10n.t(.contactsChangeHandling))
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

    public override func viewDidLayout() {
        super.viewDidLayout()
        guard let contentRow = contentRow else { return }
        let width = view.bounds.width
        guard abs(width - lastLayoutWidth) > 1 else { return }
        lastLayoutWidth = width
        let stacked = width < 780
        let isStacked = (contentRow.orientation == .vertical)
        guard stacked != isStacked else { return }

        // 窄窗口：表格与详情上下排列，表格占满宽度；宽窗口：恢复左表右详情。
        contentRow.orientation = stacked ? .vertical : .horizontal
        contentRow.alignment = stacked ? .leading : .top
        if stacked {
            detailWidthConstraint.isActive = false
            detailHeightConstraint.isActive = false
            tableHeightConstraint.isActive = false
            contentRowMinHeightConstraint.isActive = false
            stackedDetailWidthConstraint.isActive = true
            stackedDetailHeightConstraint.isActive = true
            stackedTableHeightConstraint.isActive = true
        } else {
            stackedDetailWidthConstraint.isActive = false
            stackedDetailHeightConstraint.isActive = false
            stackedTableHeightConstraint.isActive = false
            contentRowMinHeightConstraint.isActive = true
            detailWidthConstraint.isActive = true
            detailHeightConstraint.isActive = true
            tableHeightConstraint.isActive = true
        }
    }


    // MARK: - Layout

    private func setupUI() {
        view.wantsLayer = true
        view.layer?.backgroundColor = ContactsPageTokens.pageBackground.cgColor

        // 整页可纵向滚动：窗口很小时（三条横幅 + 表格最小高度）不会撑破视口；
        // 空间足够时 document 高度等于视口，表格仍然占满剩余高度。
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scrollView)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        scrollView.documentView = document

        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 16
        mainStack.distribution = .fill
        mainStack.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(mainStack)

        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: view.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            document.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
            mainStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 8),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24)
        ])
        let fillViewport = document.heightAnchor.constraint(equalTo: scrollView.contentView.heightAnchor)
        fillViewport.priority = .defaultLow
        fillViewport.isActive = true

        // 1. 顶部操作行：右上角主按钮 / 次要按钮 + 同步状态
        let actionRow = makeActionRow()
        mainStack.addArrangedSubview(actionRow)
        actionRow.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        // 2. 横幅区：待确认项 / 同步失败原因 / 授权名单预置联系人
        reviewBanner = ThemeStatusBanner(status: .warning, title: L10n.t(.contactsReviewNoticeTitle), message: "")
        reviewBanner.isHidden = true
        mainStack.addArrangedSubview(reviewBanner)
        reviewBanner.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        syncBanner = ThemeStatusBanner(status: .warning, title: L10n.t(.contactsSyncStatusIncomplete), message: "")
        syncBanner.isHidden = true
        mainStack.addArrangedSubview(syncBanner)
        syncBanner.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        presetBanner = ThemeStatusBanner(status: .inactive, title: ContactsPageStrings.presetTag, message: "")
        presetBanner.isHidden = true
        mainStack.addArrangedSubview(presetBanner)
        presetBanner.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        // 3. 分类分段标签（选中绿底白字）
        let initialTitles = [
            L10n.t(.contactsTabPersonalCount, 0),
            L10n.t(.contactsTabWorkCount, 0),
            L10n.t(.contactsTabGroupCount, 0),
            L10n.t(.contactsTabOfficialCount, 0),
            L10n.t(.contactsTabIgnoredCount, 0)
        ]
        segmentBar = ContactsSegmentBar(titles: initialTitles, target: self, action: #selector(handleCategoryChanged(_:)))
        segmentBar.translatesAutoresizingMaskIntoConstraints = false
        mainStack.addArrangedSubview(segmentBar)
        segmentBar.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        // 4. 下方：左侧表格卡片 + 右侧详情卡片。窗口太窄时改为上下排列（页内可滚动）。
        contentRow = NSStackView()
        contentRow.orientation = .horizontal
        contentRow.alignment = .top
        contentRow.distribution = .fill
        contentRow.spacing = 20
        contentRow.translatesAutoresizingMaskIntoConstraints = false

        let tableCard = makeTableCard()
        setupDetailBox()
        contentRow.addArrangedSubview(tableCard)
        contentRow.addArrangedSubview(detailBox)
        contentRow.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)
        mainStack.addArrangedSubview(contentRow)
        contentRow.widthAnchor.constraint(equalTo: mainStack.widthAnchor).isActive = true

        contentRowMinHeightConstraint = contentRow.heightAnchor.constraint(greaterThanOrEqualToConstant: 320)
        contentRowMinHeightConstraint.isActive = true
        tableHeightConstraint = tableCard.heightAnchor.constraint(equalTo: contentRow.heightAnchor)
        tableHeightConstraint.isActive = true
        stackedTableHeightConstraint = tableCard.heightAnchor.constraint(greaterThanOrEqualToConstant: 320)
        detailHeightConstraint = detailBox.heightAnchor.constraint(equalTo: contentRow.heightAnchor)
        detailHeightConstraint.isActive = true
        detailWidthConstraint = detailBox.widthAnchor.constraint(equalToConstant: 300)
        detailWidthConstraint.isActive = true

        stackedDetailWidthConstraint = detailBox.widthAnchor.constraint(equalTo: contentRow.widthAnchor)
        stackedDetailHeightConstraint = detailBox.heightAnchor.constraint(greaterThanOrEqualToConstant: 380)

        detailBox.isHidden = true

        // 顶部行、横幅、标签栏保持自身高度，剩余高度全部给列表
        for view in [mainStack.arrangedSubviews[0], reviewBanner!, syncBanner!, presetBanner!, segmentBar!] {
            view.setContentHuggingPriority(.required, for: .vertical)
        }
        tableCard.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)
        tableCard.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        detailBox.setContentHuggingPriority(.required, for: .horizontal)
    }

    private func makeActionRow() -> NSView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 8
        row.translatesAutoresizingMaskIntoConstraints = false

        let spacer = NSView()
        spacer.translatesAutoresizingMaskIntoConstraints = false
        spacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        spacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        let initSyncBtn = ThemePrimaryButton(title: L10n.t(.contactsInitSync), target: self, action: #selector(handleInitSync))
        initSyncBtn.heightAnchor.constraint(equalToConstant: 36).isActive = true
        let quickSyncBtn = ThemePrimaryButton(title: L10n.t(.contactsQuickSync), target: self, action: #selector(handleQuickSync))
        quickSyncBtn.heightAnchor.constraint(equalToConstant: 36).isActive = true
        let refreshBtn = ContactsSecondaryButton(title: L10n.t(.actionRefresh), target: self, action: #selector(handleRefresh))

        syncInfoIcon.translatesAutoresizingMaskIntoConstraints = false
        syncInfoIcon.imageScaling = .scaleProportionallyDown
        syncInfoIcon.widthAnchor.constraint(equalToConstant: 16).isActive = true
        syncInfoIcon.heightAnchor.constraint(equalToConstant: 16).isActive = true
        syncInfoIcon.isHidden = true

        syncInfoLabel.font = NSFont.systemFont(ofSize: 12)
        syncInfoLabel.textColor = ContactsPageTokens.textSecondary
        syncInfoLabel.lineBreakMode = .byTruncatingTail
        syncInfoLabel.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        syncInfoLabel.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        row.addArrangedSubview(spacer)
        row.addArrangedSubview(initSyncBtn)
        row.addArrangedSubview(quickSyncBtn)
        row.addArrangedSubview(refreshBtn)
        row.addArrangedSubview(syncInfoIcon)
        row.addArrangedSubview(syncInfoLabel)
        return row
    }

    private func makeTableCard() -> NSBox {
        let tableCard = NSBox()
        tableCard.boxType = .custom
        tableCard.fillColor = ContactsPageTokens.cardBackground
        tableCard.borderColor = ContactsPageTokens.cardBorder
        tableCard.borderWidth = 1
        tableCard.cornerRadius = 12
        tableCard.titlePosition = .noTitle
        tableCard.translatesAutoresizingMaskIntoConstraints = false
        tableCard.wantsLayer = true
        tableCard.layer?.cornerRadius = 12
        tableCard.layer?.masksToBounds = true

        let tableScroll = NSScrollView()
        tableScroll.hasVerticalScroller = true
        tableScroll.hasHorizontalScroller = false
        tableScroll.drawsBackground = false
        tableScroll.translatesAutoresizingMaskIntoConstraints = false

        let colName = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("NameCol"))
        colName.title = L10n.t(.contactsColName)
        colName.width = 200
        colName.minWidth = 160
        contactsTable.addTableColumn(colName)

        let colWx = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("WxCol"))
        colWx.title = L10n.t(.contactsColWechat)
        colWx.width = 110
        colWx.minWidth = 60
        contactsTable.addTableColumn(colWx)

        let colMsgs = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("MsgsCol"))
        colMsgs.title = L10n.t(.contactsColMsgs)
        colMsgs.width = 70
        colMsgs.minWidth = 50
        contactsTable.addTableColumn(colMsgs)

        let colHandling = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("HandlingCol"))
        colHandling.title = L10n.t(.contactsColHandling)
        colHandling.width = 170
        colHandling.minWidth = 160
        contactsTable.addTableColumn(colHandling)

        for column in contactsTable.tableColumns {
            let header = NSTableHeaderCell(textCell: column.title)
            header.attributedStringValue = NSAttributedString(string: column.title, attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: ContactsPageTokens.textSecondary
            ])
            header.drawsBackground = false
            column.headerCell = header
        }

        contactsTable.headerView = ContactsTableHeaderView(frame: NSRect(x: 0, y: 0, width: 0, height: 26))
        contactsTable.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        contactsTable.intercellSpacing = NSSize(width: 10, height: 0)
        contactsTable.dataSource = self
        contactsTable.delegate = self
        contactsTable.rowHeight = 40
        contactsTable.backgroundColor = ContactsPageTokens.cardBackground
        contactsTable.gridStyleMask = []
        contactsTable.selectionHighlightStyle = .regular
        contactsTable.allowsEmptySelection = true
        contactsTable.focusRingType = .none
        tableScroll.documentView = contactsTable

        emptyLabel.alignment = .center
        emptyLabel.textColor = ContactsPageTokens.textSecondary
        emptyLabel.font = NSFont.systemFont(ofSize: 13)
        emptyLabel.isHidden = true
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false
        tableScroll.addSubview(emptyLabel)
        NSLayoutConstraint.activate([
            emptyLabel.centerXAnchor.constraint(equalTo: tableScroll.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: tableScroll.centerYAnchor),
            emptyLabel.widthAnchor.constraint(lessThanOrEqualTo: tableScroll.widthAnchor, constant: -48)
        ])

        tableCard.setContent(tableScroll, inset: 1)
        return tableCard
    }

    private func setupDetailBox() {
        detailBox = NSBox()
        detailBox.boxType = .custom
        detailBox.fillColor = ContactsPageTokens.cardBackground
        detailBox.borderColor = ContactsPageTokens.cardBorder
        detailBox.borderWidth = 1
        detailBox.cornerRadius = 12
        detailBox.titlePosition = .noTitle
        detailBox.translatesAutoresizingMaskIntoConstraints = false
        detailBox.wantsLayer = true
        detailBox.layer?.cornerRadius = 12
        detailBox.layer?.masksToBounds = true

        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: L10n.t(.contactsDetailBoxTitle))
        title.font = NSFont.systemFont(ofSize: 17, weight: .semibold)
        title.textColor = ContactsPageTokens.textPrimary
        stack.addArrangedSubview(title)

        detailNameLabel.font = NSFont.systemFont(ofSize: 20, weight: .semibold)
        detailNameLabel.textColor = ContactsPageTokens.textPrimary
        detailNameLabel.lineBreakMode = .byTruncatingTail
        stack.addArrangedSubview(detailNameLabel)

        // 预置行来源标记（仅预置行显示）
        let tag = ContactsTagView(
            text: ContactsPageStrings.presetTag,
            textColor: ContactsPageTokens.warningText,
            background: ContactsPageTokens.warningBg
        )
        tag.translatesAutoresizingMaskIntoConstraints = false
        detailSourceTag.addSubview(tag)
        detailSourceTag.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            tag.leadingAnchor.constraint(equalTo: detailSourceTag.leadingAnchor),
            tag.trailingAnchor.constraint(equalTo: detailSourceTag.trailingAnchor),
            tag.topAnchor.constraint(equalTo: detailSourceTag.topAnchor),
            tag.bottomAnchor.constraint(equalTo: detailSourceTag.bottomAnchor)
        ])
        detailSourceTag.isHidden = true
        stack.addArrangedSubview(detailSourceTag)

        let fieldRows: [(String, NSTextField)] = [
            (L10n.t(.contactsDetailWechat), detailWechatLabel),
            (L10n.t(.contactsDetailRemark), detailRemarkLabel),
            (L10n.t(.contactsDetailRegion), detailRegionLabel),
            (L10n.t(.contactsDetailAddedOn), detailAddedOnLabel),
            (L10n.t(.contactsDetailSource), detailSourceLabel),
            (L10n.t(.contactsDetailEnterprise), detailEnterpriseLabel),
            (L10n.t(.contactsDetailTags), detailTagsLabel),
            (L10n.t(.contactsDetailHandling), detailHandlingLabel)
        ]
        let fieldsStack = NSStackView()
        fieldsStack.orientation = .vertical
        fieldsStack.alignment = .leading
        fieldsStack.spacing = 10
        fieldsStack.translatesAutoresizingMaskIntoConstraints = false
        for (title, valueLabel) in fieldRows {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .centerY
            row.spacing = 12
            row.translatesAutoresizingMaskIntoConstraints = false

            let titleLabel = makeDetailFieldLabel(title)
            titleLabel.widthAnchor.constraint(equalToConstant: 84).isActive = true

            valueLabel.font = NSFont.systemFont(ofSize: 13)
            valueLabel.textColor = ContactsPageTokens.textPrimary
            valueLabel.alignment = .left
            valueLabel.lineBreakMode = .byTruncatingTail
            valueLabel.toolTip = valueLabel.stringValue
            valueLabel.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

            row.addArrangedSubview(titleLabel)
            row.addArrangedSubview(valueLabel)
            fieldsStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: fieldsStack.widthAnchor).isActive = true
        }
        stack.addArrangedSubview(fieldsStack)
        fieldsStack.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        let spacer = NSView()
        spacer.translatesAutoresizingMaskIntoConstraints = false
        spacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .vertical)
        spacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .vertical)
        stack.addArrangedSubview(spacer)

        detailActionBtn.target = self
        detailActionBtn.action = #selector(handleDetailAction)
        detailActionBtn.isEnabled = false
        detailActionBtn.translatesAutoresizingMaskIntoConstraints = false
        detailActionBtn.heightAnchor.constraint(equalToConstant: 36).isActive = true
        stack.addArrangedSubview(detailActionBtn)
        detailActionBtn.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        detailBox.setContent(stack, inset: 20)
    }

    private func makeDetailFieldLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13)
        label.textColor = ContactsPageTokens.textSecondary
        label.alignment = .left
        return label
    }

    private func updateTabs(counts: ContactsCategoryCounts) {
        segmentBar.setTitles([
            L10n.t(.contactsTabPersonalCount, counts.personal),
            L10n.t(.contactsTabWorkCount, counts.work),
            L10n.t(.contactsTabGroupCount, counts.group),
            L10n.t(.contactsTabOfficialCount, counts.official),
            L10n.t(.contactsTabIgnoredCount, counts.ignored)
        ])
    }

    // MARK: - Data loading

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
        renderSyncStatus(payload.sync)
        renderReviewBanner()
        renderPresetBanner()
        filterDisplayedContacts()
    }

    private func renderSyncStatus(_ sync: ContactsSyncInfo?) {
        guard let sync = sync else {
            syncInfoLabel.stringValue = L10n.t(.contactsSyncNever)
            syncInfoLabel.textColor = ContactsPageTokens.textSecondary
            syncInfoIcon.isHidden = true
            syncBanner.isHidden = true
            return
        }

        if let raw = sync.error, !raw.isEmpty {
            syncInfoLabel.stringValue = L10n.t(.contactsSyncStatusFailed)
            syncInfoLabel.textColor = ContactsPageTokens.errorText
            syncInfoIcon.isHidden = false
            syncInfoIcon.image = symbolImage("xmark.octagon.fill", tint: ContactsPageTokens.errorDot)
            syncInfoLabel.toolTip = raw
            syncBanner.status = .error
            syncBanner.titleText = L10n.t(.contactsSyncStatusFailed)
            syncBanner.messageText = ContactsPageLogic.humanizedSyncError(raw)
            syncBanner.isHidden = false
            return
        }

        syncInfoLabel.toolTip = nil
        if sync.complete {
            syncInfoLabel.stringValue = L10n.t(.contactsSyncCompleted)
            syncInfoLabel.textColor = ContactsPageTokens.normalText
            syncInfoIcon.isHidden = false
            syncInfoIcon.image = symbolImage("checkmark.circle.fill", tint: ContactsPageTokens.normalDot)
            syncBanner.isHidden = true
        } else {
            syncInfoLabel.stringValue = L10n.t(.contactsSyncStatusIncomplete)
            syncInfoLabel.textColor = ContactsPageTokens.warningText
            syncInfoIcon.isHidden = false
            syncInfoIcon.image = symbolImage("exclamationmark.triangle.fill", tint: ContactsPageTokens.warningDot)
            syncBanner.status = .warning
            syncBanner.titleText = L10n.t(.contactsSyncStatusIncomplete)
            syncBanner.messageText = ContactsPageLogic.incompleteSyncReasons(sync)
            syncBanner.isHidden = false
        }
    }

    private func renderReviewBanner() {
        guard !reviews.isEmpty else {
            reviewBanner.isHidden = true
            return
        }
        let names = reviews.map { $0.name }.joined(separator: ", ")
        reviewBanner.titleText = L10n.t(.contactsReviewNoticeTitle)
        reviewBanner.messageText = L10n.t(.contactsReviewPrompt, reviews.count, names)
        reviewBanner.isHidden = false
    }

    private func renderPresetBanner() {
        let presetCount = allContacts.filter { ContactsPageLogic.isUnsyncedPreset($0) }.count
        guard presetCount > 0 else {
            presetBanner.isHidden = true
            return
        }
        presetBanner.titleText = ContactsPageStrings.presetTag
        presetBanner.messageText = L10n.t(.contactsPresetBannerMessageFmt, String(describing: presetCount))
        presetBanner.isHidden = false
    }

    private func symbolImage(_ name: String, tint: NSColor) -> NSImage? {
        let config = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?.withSymbolConfiguration(config)
        image?.isTemplate = true
        syncInfoIcon.contentTintColor = tint
        return image
    }

    private func handleLoadFallback() {
        syncInfoLabel.stringValue = L10n.t(.contactsOffline)
        syncInfoLabel.textColor = ContactsPageTokens.textSecondary
        syncInfoIcon.isHidden = true
        syncBanner.isHidden = true
        reviewBanner.isHidden = true
        presetBanner.isHidden = true
        let counts = ContactsCategoryCounts(personal: 0, work: 0, group: 0, official: 0, ignored: 0)
        updateTabs(counts: counts)
        self.allContacts = []
        self.reviews = []
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
            clearDetail()
        } else {
            emptyLabel.isHidden = true
            // 和效果图一致：进入页面即选中第一行并在右侧显示详情
            contactsTable.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            showDetail(for: displayedContacts[0])
        }
    }

    private func clearDetail() {
        detailBox.isHidden = true
        detailSourceTag.isHidden = true
        detailNameLabel.stringValue = L10n.t(.contactsNoSelected)
        detailWechatLabel.stringValue = "—"
        detailRemarkLabel.stringValue = "—"
        detailRegionLabel.stringValue = "—"
        detailSourceLabel.stringValue = "—"
        detailAddedOnLabel.stringValue = "—"
        detailTagsLabel.stringValue = "—"
        detailEnterpriseLabel.stringValue = "—"
        detailHandlingLabel.stringValue = "—"
        detailActionBtn.isEnabled = false
    }

    private func showDetail(for item: ContactProfileItem) {
        let isPreset = ContactsPageLogic.isUnsyncedPreset(item)
        detailBox.isHidden = false
        detailSourceTag.isHidden = !isPreset
        detailNameLabel.stringValue = item.displayName
        detailNameLabel.toolTip = item.displayName
        detailWechatLabel.stringValue = ContactsPageLogic.detailWechatText(item)
        detailRemarkLabel.stringValue = item.remark ?? "—"
        detailRegionLabel.stringValue = item.region ?? "—"
        if isPreset {
            detailSourceLabel.stringValue = ContactsPageStrings.presetTag
        } else {
            detailSourceLabel.stringValue = item.source ?? "—"
        }
        detailAddedOnLabel.stringValue = item.addedOn ?? "—"
        detailTagsLabel.stringValue = (item.tags?.isEmpty ?? true) ? "—" : item.tags!.joined(separator: ", ")
        let ent = [item.enterprise, item.realName].compactMap { $0 }.joined(separator: " / ")
        detailEnterpriseLabel.stringValue = ent.isEmpty ? "—" : ent

        for label in [detailWechatLabel, detailRemarkLabel, detailRegionLabel, detailSourceLabel,
                      detailAddedOnLabel, detailEnterpriseLabel, detailTagsLabel, detailHandlingLabel] {
            label.toolTip = label.stringValue
        }

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

    @objc private func handleCategoryChanged(_ sender: NSButton) {
        switch sender.tag {
        case 1: selectedCategory = "work"
        case 2: selectedCategory = "group"
        case 3: selectedCategory = "official"
        case 4: selectedCategory = "ignored"
        default: selectedCategory = "personal"
        }
        segmentBar.selectedIndex = sender.tag
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
        syncInfoLabel.stringValue = ContactsPageStrings.syncStarted
        syncInfoLabel.textColor = ContactsPageTokens.textSecondary
        syncInfoIcon.isHidden = true

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
                try context.persistConfig(updated, restartCore: false)
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

    public func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        return ContactsTableRowView()
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < displayedContacts.count, let colId = tableColumn?.identifier.rawValue else { return nil }
        let item = displayedContacts[row]

        switch colId {
        case "NameCol":
            // 名字放不下时末尾显示"…"并可悬停看全名（不能直接裁掉：用户会以为名字只有这么长）
            let nameLabel = NSTextField(labelWithString: item.displayName)
            nameLabel.font = NSFont.systemFont(ofSize: 13)
            nameLabel.textColor = ContactsPageTokens.textPrimary
            nameLabel.lineBreakMode = .byTruncatingTail
            nameLabel.toolTip = item.displayName

            let cell = NSStackView()
            cell.orientation = .vertical
            cell.alignment = .leading
            cell.spacing = 2
            cell.addArrangedSubview(nameLabel)

            // 需求修复：把「授权名单预置」的行和微信通讯录同步来的联系人区分开
            if ContactsPageLogic.isUnsyncedPreset(item) {
                let tag = ContactsTagView(
                    text: ContactsPageStrings.presetTag,
                    textColor: ContactsPageTokens.warningText,
                    background: ContactsPageTokens.warningBg
                )
                cell.addArrangedSubview(tag)
            }
            return cell
        case "WxCol":
            let wx = ContactsPageLogic.listWechatText(item)
            let cell = NSTextField(labelWithString: wx)
            cell.lineBreakMode = .byTruncatingTail
            cell.toolTip = wx
            cell.font = NSFont.systemFont(ofSize: 13)
            cell.textColor = ContactsPageTokens.textSecondary
            return cell
        case "MsgsCol":
            let count = item.messageCount ?? 0
            let cell = NSTextField(labelWithString: "\(count)")
            cell.font = NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .regular)
            cell.textColor = ContactsPageTokens.textPrimary
            return cell
        case "HandlingCol":
            let container = NSStackView()
            container.orientation = .horizontal
            container.spacing = 10
            container.alignment = .centerY

            if item.category == "official" {
                let lbl = NSTextField(labelWithString: L10n.t(.contactsHandlingNeverReply))
                lbl.font = NSFont.systemFont(ofSize: 13)
                lbl.textColor = ContactsPageTokens.textSecondary
                container.addArrangedSubview(lbl)
            } else if item.handling == "ignored" {
                let tag = ContactsTagView(
                    text: L10n.t(.contactsHandlingIgnored),
                    textColor: ContactsPageTokens.textSecondary,
                    background: ContactsPageTokens.inactiveBg
                )
                let unignoreBtn = ContactsSecondaryButton(title: L10n.t(.contactsActionUnignore), target: self, action: #selector(handleTableAction(_:)))
                unignoreBtn.tag = row
                unignoreBtn.identifier = NSUserInterfaceItemIdentifier("unignore")
                container.addArrangedSubview(tag)
                container.addArrangedSubview(unignoreBtn)
            } else {
                let chk = NSButton(checkboxWithTitle: L10n.t(.termAutoReply), target: self, action: #selector(handleTableAction(_:)))
                chk.state = (item.handling == "auto_reply") ? .on : .off
                chk.font = NSFont.systemFont(ofSize: 13)
                chk.contentTintColor = ContactsPageTokens.primary
                chk.tag = row
                chk.identifier = NSUserInterfaceItemIdentifier("toggle_auto_reply")

                let ignoreBtn = ContactsSecondaryButton(title: L10n.t(.contactsActionIgnore), target: self, action: #selector(handleTableAction(_:)))
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
