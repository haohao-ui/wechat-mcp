// McpPage.swift: MCP tools configuration, brain routing parameters, and connectivity testing.
import Foundation
import AppKit

public struct McpExternalBrainInfo: Codable {
    public let timeoutSeconds: Int?
    public let timeoutAction: String?
    public let queueMinutes: Int?

    enum CodingKeys: String, CodingKey {
        case timeoutSeconds = "timeout_seconds"
        case timeoutAction = "timeout_action"
        case queueMinutes = "queue_minutes"
    }

    public init(timeoutSeconds: Int? = 120, timeoutAction: String? = "skip", queueMinutes: Int? = 30) {
        self.timeoutSeconds = timeoutSeconds
        self.timeoutAction = timeoutAction
        self.queueMinutes = queueMinutes
    }
}

public struct McpBrainInfo: Codable {
    public let mode: String
    public let external: McpExternalBrainInfo?

    public init(mode: String, external: McpExternalBrainInfo? = nil) {
        self.mode = mode
        self.external = external
    }
}

public struct McpEndpointInfo: Codable {
    public let id: String
    public let label: String
    public let value: String
    public let ok: Bool
    public let detail: String

    public init(id: String, label: String, value: String, ok: Bool, detail: String) {
        self.id = id
        self.label = label
        self.value = value
        self.ok = ok
        self.detail = detail
    }
}

public struct McpToolGroupInfo: Codable {
    public let title: String
    public let tools: [String]

    public init(title: String, tools: [String]) {
        self.title = title
        self.tools = tools
    }
}

public struct McpInfoPayload: Codable {
    public let brain: McpBrainInfo?
    public let endpoints: [McpEndpointInfo]?
    public let toolGroups: [McpToolGroupInfo]?

    enum CodingKeys: String, CodingKey {
        case brain
        case endpoints
        case toolGroups = "tool_groups"
    }

    public init(brain: McpBrainInfo?, endpoints: [McpEndpointInfo]?, toolGroups: [McpToolGroupInfo]?) {
        self.brain = brain
        self.endpoints = endpoints
        self.toolGroups = toolGroups
    }
}

public struct OpenApiTokenPayload: Codable {
    public let token: String

    public init(token: String) {
        self.token = token
    }
}

public struct OpenApiCheckPayload: Codable {
    public let ok: Bool
    public let busy: Bool
    public let tools: [String]
    public let error: String?

    public init(ok: Bool, busy: Bool, tools: [String], error: String?) {
        self.ok = ok
        self.busy = busy
        self.tools = tools
        self.error = error
    }
}

public struct McpPageLogic {
    public static func updateBrainSettings(
        config: [String: Any],
        timeoutSeconds: Int,
        timeoutAction: String,
        queueMinutes: Int
    ) -> Result<[String: Any], PageError> {
        let formInputs: [String: Any] = [
            "brain_mode": "external",
            "brain_timeout_seconds": timeoutSeconds,
            "brain_timeout_action": timeoutAction,
            "brain_queue_minutes": queueMinutes
        ]

        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: config, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(PageError(error.localizedDescription))
        }
    }

    public static func updateBrainModeBuiltin(config: [String: Any]) -> Result<[String: Any], PageError> {
        let formInputs: [String: Any] = ["brain_mode": "builtin"]
        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: config, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(PageError(error.localizedDescription))
        }
    }

    /// Turns the open API on or off; bind and port keep their values (defaults when absent).
    public static func updateOpenApiEnabled(config: [String: Any], enabled: Bool) -> [String: Any] {
        var updated = config
        var openApi = config["open_api"] as? [String: Any] ?? [:]
        openApi["enabled"] = enabled
        if openApi["bind"] == nil { openApi["bind"] = "127.0.0.1" }
        if openApi["port"] == nil { openApi["port"] = 8770 }
        updated["open_api"] = openApi
        return updated
    }

    public static func parseMcpInfoPayload(json: String) throws -> McpInfoPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.t(.uiMcp113)])
        }
        let decoder = JSONDecoder()
        return try decoder.decode(McpInfoPayload.self, from: data)
    }

    public static func parseTokenPayload(json: String) throws -> OpenApiTokenPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.t(.uiMcp113)])
        }
        return try JSONDecoder().decode(OpenApiTokenPayload.self, from: data)
    }

    public static func parseCheckPayload(json: String) throws -> OpenApiCheckPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.t(.uiMcp113)])
        }
        return try JSONDecoder().decode(OpenApiCheckPayload.self, from: data)
    }

    public static func brainPagePointerText() -> String {
        return L10n.t(.uiMcp114)
    }

    public static func getPrimaryNonLoopbackIPv4() -> String? {
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0, let first = ifaddr else { return nil }
        defer { freeifaddrs(ifaddr) }

        var ptr = Optional(first)
        while let cur = ptr {
            defer { ptr = cur.pointee.ifa_next }
            let name = String(cString: cur.pointee.ifa_name)
            if name == "lo0" {
                continue
            }
            if let sa = cur.pointee.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) {
                var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                if getnameinfo(sa, socklen_t(sa.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                    let ip = String(cString: host)
                    if !ip.isEmpty && ip != "127.0.0.1" {
                        return ip
                    }
                }
            }
        }
        return nil
    }
}

// MARK: - Page-local controls (colors only from Theme tokens)

private enum McpPageTokens {
    static let pageBackground = Theme.Colors.pageBackground
    static let cardBackground = Theme.Colors.cardBackground
    static let cardBorder = Theme.Colors.cardBorder
    static let textPrimary = Theme.Colors.textPrimary
    static let textSecondary = Theme.Colors.textSecondary
    static let primary = Theme.Colors.primaryButtonDefault
    static let primaryText = Theme.Colors.primaryButtonText
    static let normalText = Theme.Colors.statusNormalText
    static let warningText = Theme.Colors.statusWarningText
    static let errorText = Theme.Colors.statusErrorText
    static let inactiveBg = Theme.Colors.statusInactiveBg
    static let inactiveText = Theme.Colors.statusInactiveText
}

/// 次要按钮：白底、1px 描边、深色字。
private final class McpSecondaryButton: NSButton {
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
        layer?.backgroundColor = McpPageTokens.cardBackground.cgColor
        layer?.borderColor = Theme.Colors.inputBorder.cgColor
        attributedTitle = NSAttributedString(string: title, attributes: [
            .foregroundColor: McpPageTokens.textPrimary,
            .font: NSFont.systemFont(ofSize: 13)
        ])
    }

    public override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: max(base.width + 28, 64), height: 32)
    }
}

/// 只读值框（URL / 令牌 / 技术文本的浅色底框）。
private final class McpReadOnlyBox: NSView {
    private let label = NSTextField(labelWithString: "")

    public var stringValue: String {
        get { label.stringValue }
        set {
            label.stringValue = newValue
            label.toolTip = newValue
        }
    }

    public init(monospaced: Bool = false) {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.borderWidth = 1
        layer?.borderColor = McpPageTokens.cardBorder.cgColor
        layer?.backgroundColor = McpPageTokens.cardBackground.cgColor
        label.font = monospaced
            ? NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
            : NSFont.systemFont(ofSize: 13)
        label.textColor = McpPageTokens.textPrimary
        label.lineBreakMode = .byTruncatingTail
        label.isSelectable = true
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            heightAnchor.constraint(equalToConstant: 30)
        ])
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

// MARK: - Page Controller

public final class McpPageController: NSViewController {
    private let context: StatusPanelController

    // UI - Brain route + timeout policy
    private let modeBuiltinRadio = NSButton(radioButtonWithTitle: L10n.t(.uiMcp115), target: nil, action: nil)
    private let modeExternalRadio = NSButton(radioButtonWithTitle: L10n.t(.uiMcp116), target: nil, action: nil)
    private let timeoutField = ThemeTextField(placeholder: nil)
    private let timeoutActionPopup = NSPopUpButton()
    private let queueMinutesField = ThemeTextField(placeholder: nil)
    private let saveBrainButton: ThemePrimaryButton
    private let brainStatusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Open API
    private let openApiEnabledCheckbox = NSButton(checkboxWithTitle: L10n.t(.uiMcp118), target: nil, action: nil)
    private let openApiStateTag = ThemeStatusTag(status: .inactive, customText: L10n.t(.mcpStatusDisabled))
    private let openApiUrlBox = McpReadOnlyBox(monospaced: true)
    private let openApiTokenBox = McpReadOnlyBox(monospaced: true)
    private let copyTokenButton: McpSecondaryButton
    private let rotateTokenButton: McpSecondaryButton
    private let testOpenApiButton: McpSecondaryButton
    private let openApiStatusLabel = NSTextField(wrappingLabelWithString: "")
    private var currentToken: String?

    // UI - Connection endpoints
    private let endpointsStack = NSStackView()
    private let testConnButton: McpSecondaryButton
    private let testConnStatusLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Developer tools
    private let configExampleLabel = NSTextField(wrappingLabelWithString: L10n.t(.uiResidualMcpConfigExample))
    private let copyOpsSshButton: McpSecondaryButton
    private let copyOpenApiButton: McpSecondaryButton
    private let copyFeedbackLabel = NSTextField(wrappingLabelWithString: "")
    private let techDetailsLabel = NSTextField(wrappingLabelWithString: "")
    private let copyTechDetailsButton: McpSecondaryButton
    private let techDetailsFeedbackLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Tools
    private let toolsStack = NSStackView()

    // State
    private var lastMcpPayload: McpInfoPayload?
    private var currentMcpRawJSON: String?
    private var lastOpenApiError: String?

    public init(context: StatusPanelController) {
        self.context = context
        self.saveBrainButton = ThemePrimaryButton(title: L10n.t(.uiMcp117))
        self.copyTokenButton = McpSecondaryButton(title: L10n.t(.actionCopy))
        self.rotateTokenButton = McpSecondaryButton(title: L10n.t(.mcpRegenerate))
        self.testOpenApiButton = McpSecondaryButton(title: L10n.t(.modelPageButtonTest))
        self.testConnButton = McpSecondaryButton(title: L10n.t(.modelPageButtonTest))
        self.copyOpsSshButton = McpSecondaryButton(title: L10n.t(.mcpCopyOpsSshBtn))
        self.copyOpenApiButton = McpSecondaryButton(title: L10n.t(.mcpCopyOpenApiBtn))
        self.copyTechDetailsButton = McpSecondaryButton(title: L10n.t(.mcpCopyTechDetails))
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 720, height: 860))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        populateCurrentValues()
        loadMcpInfo()
        loadOpenApiToken()
    }

    // MARK: - Layout

    private func setupUI() {
        view.wantsLayer = true
        view.layer?.backgroundColor = McpPageTokens.pageBackground.cgColor

        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let contentStack = NSStackView()
        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = 20
        contentStack.translatesAutoresizingMaskIntoConstraints = false

        let brainCard = makeBrainCard()
        contentStack.addArrangedSubview(brainCard)

        let openApiCard = makeOpenApiCard()
        contentStack.addArrangedSubview(openApiCard)

        let endpointsCard = makeEndpointsCard()
        contentStack.addArrangedSubview(endpointsCard)

        let developerCard = makeDeveloperCard()
        contentStack.addArrangedSubview(developerCard)

        let toolsCard = makeToolsCard()
        contentStack.addArrangedSubview(toolsCard)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 8),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24),
            brainCard.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            openApiCard.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            endpointsCard.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            developerCard.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            toolsCard.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
        ])
    }

    private func makeCard(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 17, weight: .semibold)
        box.boxType = .primary
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    private func makeSectionLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        label.textColor = McpPageTokens.textPrimary
        return label
    }

    private func makeFieldLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13)
        label.textColor = McpPageTokens.textPrimary
        return label
    }

    private func makeHelperLabel(_ text: String) -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: text)
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = McpPageTokens.textSecondary
        return label
    }

    private func makeStatusLabel() -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: "")
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = McpPageTokens.textSecondary
        return label
    }

    private func makeBrainCard() -> NSBox {
        let card = makeCard(title: L10n.t(.uiMcp119))
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        modeBuiltinRadio.target = self
        modeBuiltinRadio.action = #selector(handleBrainModeChanged)
        modeBuiltinRadio.font = NSFont.systemFont(ofSize: 13)
        modeExternalRadio.target = self
        modeExternalRadio.action = #selector(handleBrainModeChanged)
        modeExternalRadio.font = NSFont.systemFont(ofSize: 13)
        stack.addArrangedSubview(modeBuiltinRadio)
        stack.addArrangedSubview(modeExternalRadio)

        // 超时策略（字段旁标单位与范围）
        stack.addArrangedSubview(makeSectionLabel(L10n.t(.mcpSectionTimeoutPolicy)))

        timeoutField.translatesAutoresizingMaskIntoConstraints = false
        timeoutField.widthAnchor.constraint(equalToConstant: 120).isActive = true
        timeoutActionPopup.translatesAutoresizingMaskIntoConstraints = false
        timeoutActionPopup.widthAnchor.constraint(equalToConstant: 140).isActive = true
        timeoutActionPopup.controlSize = .regular
        timeoutActionPopup.font = NSFont.systemFont(ofSize: 13)
        timeoutActionPopup.addItems(withTitles: ["skip", "handoff", "builtin"])
        queueMinutesField.translatesAutoresizingMaskIntoConstraints = false
        queueMinutesField.widthAnchor.constraint(equalToConstant: 120).isActive = true

        let paramsStack = NSStackView()
        paramsStack.orientation = .vertical
        paramsStack.alignment = .leading
        paramsStack.spacing = 12
        paramsStack.translatesAutoresizingMaskIntoConstraints = false
        paramsStack.addArrangedSubview(makeParamRow(label: L10n.t(.uiMcp120), control: timeoutField, helper: L10n.t(.mcpFieldTimeoutHelper), labelWidth: 110))
        paramsStack.addArrangedSubview(makeParamRow(label: L10n.t(.uiMcp121), control: timeoutActionPopup, helper: L10n.t(.mcpFieldTimeoutActionHelper), labelWidth: 110))
        paramsStack.addArrangedSubview(makeParamRow(label: L10n.t(.uiMcp122), control: queueMinutesField, helper: L10n.t(.mcpFieldQueueMinutesHelper), labelWidth: 110))
        stack.addArrangedSubview(paramsStack)
        paramsStack.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        // 保存
        let saveRow = NSStackView()
        saveRow.orientation = .horizontal
        saveRow.alignment = .centerY
        saveRow.spacing = 12
        saveBrainButton.target = self
        saveBrainButton.action = #selector(handleSaveBrain)
        saveBrainButton.heightAnchor.constraint(equalToConstant: 36).isActive = true
        brainStatusMessageLabel.font = NSFont.systemFont(ofSize: 12)
        brainStatusMessageLabel.textColor = McpPageTokens.textSecondary
        brainStatusMessageLabel.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        saveRow.addArrangedSubview(saveBrainButton)
        saveRow.addArrangedSubview(brainStatusMessageLabel)
        stack.addArrangedSubview(saveRow)

        // 客服角色页面入口（不改原功能，改成次要按钮样式）
        let pointerLabel = NSTextField(wrappingLabelWithString: McpPageLogic.brainPagePointerText())
        pointerLabel.font = NSFont.systemFont(ofSize: 12)
        pointerLabel.textColor = McpPageTokens.textSecondary
        let goToBrainBtn = McpSecondaryButton(title: L10n.t(.uiMcp123), target: self, action: #selector(handleGoToBrain))
        let pointerRow = NSStackView(views: [pointerLabel, goToBrainBtn])
        pointerRow.orientation = .horizontal
        pointerRow.alignment = .centerY
        pointerRow.spacing = 12
        pointerLabel.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        stack.addArrangedSubview(pointerRow)

        card.setContent(stack, inset: 20)
        return card
    }

    /// 一行参数：标签（固定宽）+ 控件 + 单位/范围说明，说明紧跟控件，右侧留白。
    private func makeParamRow(label: String, control: NSView, helper: String, labelWidth: CGFloat) -> NSView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 12
        row.translatesAutoresizingMaskIntoConstraints = false

        let title = makeFieldLabel(label)
        title.widthAnchor.constraint(equalToConstant: labelWidth).isActive = true
        let hint = makeHelperLabel(helper)
        hint.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(200), for: .horizontal)
        let spacer = NSView()
        spacer.translatesAutoresizingMaskIntoConstraints = false
        spacer.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        spacer.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

        row.addArrangedSubview(title)
        row.addArrangedSubview(control)
        row.addArrangedSubview(hint)
        row.addArrangedSubview(spacer)
        return row
    }

    private func makeOpenApiCard() -> NSBox {
        let card = makeCard(title: L10n.t(.mcpEndpointOpenApi))
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        openApiEnabledCheckbox.target = self
        openApiEnabledCheckbox.action = #selector(handleOpenApiToggle)
        openApiEnabledCheckbox.font = NSFont.systemFont(ofSize: 13)
        stack.addArrangedSubview(openApiEnabledCheckbox)

        // 状态行：状态标签
        let stateRow = NSStackView()
        stateRow.orientation = .horizontal
        stateRow.alignment = .centerY
        stateRow.spacing = 10
        stateRow.addArrangedSubview(makeFieldLabel(L10n.t(.mcpLabelStatus)))
        openApiStateTag.translatesAutoresizingMaskIntoConstraints = false
        stateRow.addArrangedSubview(openApiStateTag)
        stack.addArrangedSubview(stateRow)

        openApiUrlBox.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(openApiUrlBox)
        openApiUrlBox.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        let tokenRow = NSStackView()
        tokenRow.orientation = .horizontal
        tokenRow.alignment = .centerY
        tokenRow.spacing = 10
        openApiTokenBox.translatesAutoresizingMaskIntoConstraints = false
        openApiTokenBox.widthAnchor.constraint(greaterThanOrEqualToConstant: 200).isActive = true
        copyTokenButton.target = self
        copyTokenButton.action = #selector(handleCopyToken)
        rotateTokenButton.target = self
        rotateTokenButton.action = #selector(handleRotateToken)
        tokenRow.addArrangedSubview(openApiTokenBox)
        tokenRow.addArrangedSubview(copyTokenButton)
        tokenRow.addArrangedSubview(rotateTokenButton)
        stack.addArrangedSubview(tokenRow)

        let testRow = NSStackView()
        testRow.orientation = .horizontal
        testRow.alignment = .centerY
        testRow.spacing = 12
        testOpenApiButton.target = self
        testOpenApiButton.action = #selector(handleTestOpenApi)
        openApiStatusLabel.font = NSFont.systemFont(ofSize: 12)
        openApiStatusLabel.textColor = McpPageTokens.textSecondary
        testRow.addArrangedSubview(testOpenApiButton)
        testRow.addArrangedSubview(openApiStatusLabel)
        stack.addArrangedSubview(testRow)

        card.setContent(stack, inset: 20)
        return card
    }

    private func makeEndpointsCard() -> NSBox {
        let card = makeCard(title: L10n.t(.mcpSectionEndpoints))
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        endpointsStack.orientation = .vertical
        endpointsStack.alignment = .leading
        endpointsStack.spacing = 0
        endpointsStack.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(endpointsStack)
        endpointsStack.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        let testRow = NSStackView()
        testRow.orientation = .horizontal
        testRow.alignment = .centerY
        testRow.spacing = 12
        testConnButton.target = self
        testConnButton.action = #selector(handleTestConnection)
        testConnStatusLabel.font = NSFont.systemFont(ofSize: 12)
        testConnStatusLabel.textColor = McpPageTokens.textSecondary
        testRow.addArrangedSubview(testConnButton)
        testRow.addArrangedSubview(testConnStatusLabel)
        stack.addArrangedSubview(testRow)

        card.setContent(stack, inset: 20)
        return card
    }

    private func makeDeveloperCard() -> NSBox {
        let card = makeCard(title: L10n.t(.mcpSectionDeveloperTools))
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false

        // 客户端接入配置示例（可选中复制）
        stack.addArrangedSubview(makeSectionLabel(L10n.t(.mcpBoxConfigExample)))
        configExampleLabel.stringValue = L10n.t(.uiResidualMcpConfigExample)
            .replacingOccurrences(of: "\\n", with: "\n")
        configExampleLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        configExampleLabel.textColor = McpPageTokens.textPrimary
        configExampleLabel.isSelectable = true
        // 多行配置示例：可换行的等宽文本，可选中复制
        let exampleContainer = NSView()
        exampleContainer.wantsLayer = true
        exampleContainer.layer?.cornerRadius = 8
        exampleContainer.layer?.backgroundColor = McpPageTokens.inactiveBg.cgColor
        exampleContainer.translatesAutoresizingMaskIntoConstraints = false
        configExampleLabel.translatesAutoresizingMaskIntoConstraints = false
        exampleContainer.addSubview(configExampleLabel)
        NSLayoutConstraint.activate([
            configExampleLabel.leadingAnchor.constraint(equalTo: exampleContainer.leadingAnchor, constant: 12),
            configExampleLabel.trailingAnchor.constraint(equalTo: exampleContainer.trailingAnchor, constant: -12),
            configExampleLabel.topAnchor.constraint(equalTo: exampleContainer.topAnchor, constant: 10),
            configExampleLabel.bottomAnchor.constraint(equalTo: exampleContainer.bottomAnchor, constant: -10)
        ])
        stack.addArrangedSubview(exampleContainer)
        exampleContainer.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        let copyRow = NSStackView()
        copyRow.orientation = .horizontal
        copyRow.alignment = .centerY
        copyRow.spacing = 10
        copyOpsSshButton.target = self
        copyOpsSshButton.action = #selector(handleCopyOpsSshConfig)
        copyOpenApiButton.target = self
        copyOpenApiButton.action = #selector(handleCopyOpenApiConfig)
        copyFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        copyFeedbackLabel.textColor = McpPageTokens.textSecondary
        copyRow.addArrangedSubview(copyOpsSshButton)
        copyRow.addArrangedSubview(copyOpenApiButton)
        copyRow.addArrangedSubview(copyFeedbackLabel)
        stack.addArrangedSubview(copyRow)

        // 技术详情（原始参数名 / socket 路径 / 错误码，可复制）
        stack.addArrangedSubview(makeSectionLabel(L10n.t(.mcpTechDetailsTitle)))
        let techHint = makeHelperLabel(L10n.t(.mcpTechDetailsHint))
        stack.addArrangedSubview(techHint)

        let techContainer = NSView()
        techContainer.wantsLayer = true
        techContainer.layer?.cornerRadius = 8
        techContainer.layer?.backgroundColor = McpPageTokens.inactiveBg.cgColor
        techContainer.translatesAutoresizingMaskIntoConstraints = false
        techDetailsLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        techDetailsLabel.textColor = McpPageTokens.textSecondary
        techDetailsLabel.isSelectable = true
        techDetailsLabel.translatesAutoresizingMaskIntoConstraints = false
        techContainer.addSubview(techDetailsLabel)
        NSLayoutConstraint.activate([
            techDetailsLabel.leadingAnchor.constraint(equalTo: techContainer.leadingAnchor, constant: 12),
            techDetailsLabel.trailingAnchor.constraint(equalTo: techContainer.trailingAnchor, constant: -12),
            techDetailsLabel.topAnchor.constraint(equalTo: techContainer.topAnchor, constant: 10),
            techDetailsLabel.bottomAnchor.constraint(equalTo: techContainer.bottomAnchor, constant: -10)
        ])
        stack.addArrangedSubview(techContainer)
        techContainer.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true

        let techCopyRow = NSStackView()
        techCopyRow.orientation = .horizontal
        techCopyRow.alignment = .centerY
        techCopyRow.spacing = 10
        copyTechDetailsButton.target = self
        copyTechDetailsButton.action = #selector(handleCopyTechDetails)
        techDetailsFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        techDetailsFeedbackLabel.textColor = McpPageTokens.textSecondary
        techCopyRow.addArrangedSubview(copyTechDetailsButton)
        techCopyRow.addArrangedSubview(techDetailsFeedbackLabel)
        stack.addArrangedSubview(techCopyRow)

        card.setContent(stack, inset: 20)
        return card
    }

    private func makeToolsCard() -> NSBox {
        let card = makeCard(title: L10n.t(.mcpSectionTools))
        toolsStack.orientation = .vertical
        toolsStack.alignment = .leading
        toolsStack.spacing = 12
        toolsStack.translatesAutoresizingMaskIntoConstraints = false
        card.setContent(toolsStack, inset: 20)
        return card
    }

    // MARK: - State rendering

    private func getOpenApiBindAndPort() -> (String, Int) {
        let raw = context.currentRawConfig
        let openApi = raw["open_api"] as? [String: Any]
        let bind = (openApi?["bind"] as? String) ?? "127.0.0.1"
        let port = (openApi?["port"] as? Int) ?? 8770
        return (bind, port)
    }

    public func populateCurrentValues() {
        let raw = context.currentRawConfig
        let brain = raw["brain"] as? [String: Any]
        let mode = (brain?["mode"] as? String)?.lowercased() ?? "builtin"

        if mode == "external" {
            modeExternalRadio.state = .on
            modeBuiltinRadio.state = .off
            let ext = brain?["external"] as? [String: Any]
            let timeout = ext?["timeout_seconds"] as? Int ?? 120
            let action = ext?["timeout_action"] as? String ?? "skip"
            let queue = ext?["queue_minutes"] as? Int ?? 30
            timeoutField.stringValue = "\(timeout)"
            timeoutActionPopup.selectItem(withTitle: action)
            queueMinutesField.stringValue = "\(queue)"
            setExternalInputsEnabled(true)
        } else {
            modeBuiltinRadio.state = .on
            modeExternalRadio.state = .off
            timeoutField.stringValue = "120"
            timeoutActionPopup.selectItem(withTitle: "skip")
            queueMinutesField.stringValue = "30"
            setExternalInputsEnabled(false)
        }

        // Open API info
        let openApi = raw["open_api"] as? [String: Any]
        let isEnabled = (openApi?["enabled"] as? Bool) ?? false
        openApiEnabledCheckbox.state = isEnabled ? .on : .off
        openApiStateTag.status = isEnabled ? .normal : .inactive
        openApiStateTag.customText = isEnabled ? L10n.t(.mcpStatusEnabled) : L10n.t(.mcpStatusDisabled)

        let (bind, port) = getOpenApiBindAndPort()
        openApiUrlBox.stringValue = L10n.t(.uiMcp127, String(describing: bind), String(describing: port))
        updateTokenDisplay()
        updateTechDetails()
    }

    private func updateTokenDisplay() {
        if let token = currentToken, token.count >= 4 {
            let tail = String(token.suffix(4))
            openApiTokenBox.stringValue = L10n.t(.mcpTokenMaskedWithTail, String(describing: tail))
        } else {
            openApiTokenBox.stringValue = L10n.t(.mcpTokenMasked)
        }
    }

    private func setExternalInputsEnabled(_ enabled: Bool) {
        timeoutField.isEditable = enabled
        timeoutActionPopup.isEnabled = enabled
        queueMinutesField.isEditable = enabled
    }

    private func setStatus(_ label: NSTextField, text: String, color: NSColor) {
        label.stringValue = text
        label.textColor = color
    }

    // MARK: - Brain actions

    @objc private func handleBrainModeChanged() {
        let isExternal = (modeExternalRadio.state == .on)
        setExternalInputsEnabled(isExternal)
    }

    @objc private func handleSaveBrain() {
        let isExternal = (modeExternalRadio.state == .on)
        let res: Result<[String: Any], PageError>

        if isExternal {
            guard let timeout = Int(timeoutField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                setStatus(brainStatusMessageLabel, text: L10n.t(.mcpErrorTimeoutInt), color: McpPageTokens.errorText)
                return
            }
            guard let queue = Int(queueMinutesField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                setStatus(brainStatusMessageLabel, text: L10n.t(.mcpErrorQueueInt), color: McpPageTokens.errorText)
                return
            }
            let action = timeoutActionPopup.titleOfSelectedItem ?? "skip"
            res = McpPageLogic.updateBrainSettings(
                config: context.currentRawConfig,
                timeoutSeconds: timeout,
                timeoutAction: action,
                queueMinutes: queue
            )
        } else {
            res = McpPageLogic.updateBrainModeBuiltin(config: context.currentRawConfig)
        }

        switch res {
        case .success(let updated):
            do {
                try context.persistConfig(updated)
                setStatus(brainStatusMessageLabel, text: L10n.t(.uiMcp128), color: McpPageTokens.normalText)
                updateTechDetails()
            } catch {
                setStatus(brainStatusMessageLabel, text: L10n.t(.saveFailed, String(describing: error.localizedDescription)), color: McpPageTokens.errorText)
            }
        case .failure(let err):
            setStatus(brainStatusMessageLabel, text: L10n.t(.validationError, String(describing: err.message)), color: McpPageTokens.errorText)
        }
    }

    @objc private func handleGoToBrain() {
        if let shell = view.window?.contentViewController as? ShellViewController {
            shell.selectPage(id: "brain")
        }
    }

    // MARK: - Open API actions

    @objc private func handleOpenApiToggle() {
        let enabled = openApiEnabledCheckbox.state == .on
        let updated = McpPageLogic.updateOpenApiEnabled(config: context.currentRawConfig, enabled: enabled)
        do {
            // The HTTP listener starts with serve-core: saving restarts the core to apply it.
            try context.persistConfig(updated)
            setStatus(openApiStatusLabel, text: enabled ? L10n.t(.uiMcp129) : L10n.t(.uiMcp130), color: McpPageTokens.normalText)
        } catch {
            openApiEnabledCheckbox.state = enabled ? .off : .on
            setStatus(openApiStatusLabel, text: L10n.t(.saveFailed, String(describing: error.localizedDescription)), color: McpPageTokens.errorText)
        }
        populateCurrentValues()
    }

    @objc private func handleTestConnection() {
        setStatus(testConnStatusLabel, text: L10n.t(.mcpTestingConnection), color: McpPageTokens.textSecondary)
        loadMcpInfo()
    }

    public func loadOpenApiToken() {
        fetchToken(rotate: false, completion: nil)
    }

    private func fetchToken(rotate: Bool, completion: ((String?) -> Void)?) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var args = ["control", "open-api-token", "--json"]
            if rotate {
                args.append("--rotate")
            }
            let run = runControlCommand(executable: controlBinaryPath(), arguments: args, timeout: 10)
            var token: String? = nil
            if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8),
               let payload = try? McpPageLogic.parseTokenPayload(json: jsonStr) {
                token = payload.token
            }
            DispatchQueue.main.async {
                if let t = token {
                    self?.currentToken = t
                    self?.updateTokenDisplay()
                    self?.updateTechDetails()
                }
                completion?(token)
            }
        }
    }

    @objc private func handleCopyToken() {
        if let token = currentToken, !token.isEmpty {
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(token, forType: .string)
            setStatus(openApiStatusLabel, text: L10n.t(.mcpTokenCopied), color: McpPageTokens.normalText)
        } else {
            setStatus(openApiStatusLabel, text: L10n.t(.mcpReadingToken), color: McpPageTokens.textSecondary)
            fetchToken(rotate: false) { [weak self] token in
                guard let self = self else { return }
                if let t = token, !t.isEmpty {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString(t, forType: .string)
                    self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpTokenCopied), color: McpPageTokens.normalText)
                } else {
                    self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpFetchTokenFailed), color: McpPageTokens.errorText)
                }
            }
        }
    }

    @objc private func handleRotateToken() {
        let alert = NSAlert()
        alert.messageText = L10n.t(.uiMcp131)
        alert.informativeText = L10n.t(.uiMcp132)
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.t(.mcpRegenerate))
        alert.addButton(withTitle: L10n.t(.cancel))
        if alert.runModal() == .alertFirstButtonReturn {
            setStatus(openApiStatusLabel, text: L10n.t(.mcpRegeneratingToken), color: McpPageTokens.textSecondary)
            fetchToken(rotate: true) { [weak self] token in
                guard let self = self else { return }
                if token != nil {
                    self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpTokenRegeneratedSuccess), color: McpPageTokens.normalText)
                } else {
                    self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpRegenerateTokenFailed), color: McpPageTokens.errorText)
                }
            }
        }
    }

    @objc private func handleTestOpenApi() {
        setStatus(openApiStatusLabel, text: L10n.t(.mcpTestingOpenApi), color: McpPageTokens.textSecondary)
        lastOpenApiError = nil
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "open-api-check", "--json"], timeout: 10)
            DispatchQueue.main.async {
                guard let self = self else { return }
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8),
                   let payload = try? McpPageLogic.parseCheckPayload(json: jsonStr) {
                    self.lastOpenApiError = payload.error
                    if payload.ok {
                        self.setStatus(self.openApiStatusLabel, text: L10n.t(.uiMcp133, String(describing: payload.tools.count)), color: McpPageTokens.normalText)
                    } else if payload.busy {
                        self.setStatus(self.openApiStatusLabel, text: L10n.t(.uiMcp134), color: McpPageTokens.warningText)
                    } else if payload.error == "disabled" {
                        self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpOpenApiDisabledNotice), color: McpPageTokens.textSecondary)
                    } else {
                        self.setStatus(self.openApiStatusLabel, text: L10n.t(.uiResidualMcpTestFailedFmt, String(describing: payload.error ?? L10n.t(.uiResidualConnectFailed))), color: McpPageTokens.errorText)
                    }
                } else {
                    self.setStatus(self.openApiStatusLabel, text: L10n.t(.mcpOpenApiCheckFailed), color: McpPageTokens.errorText)
                }
                self.updateTechDetails()
            }
        }
    }

    // MARK: - Copy actions

    @objc private func handleCopyOpsSshConfig() {
        let ip = McpPageLogic.getPrimaryNonLoopbackIPv4()
        let host = ip ?? L10n.t(.mcpPlaceholderLocalAddress)
        let user = NSUserName()
        let home = NSHomeDirectory()
        let cmd = "ssh \(user)@\(host) nc -U \(home)/.wechat-rpa/worker.sock"

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(cmd, forType: .string)

        if ip == nil {
            setStatus(copyFeedbackLabel, text: L10n.t(.mcpCopiedSshPlaceholder), color: McpPageTokens.warningText)
        } else {
            setStatus(copyFeedbackLabel, text: L10n.t(.mcpCopiedSsh), color: McpPageTokens.normalText)
        }
    }

    @objc private func handleCopyOpenApiConfig() {
        let (bind, port) = getOpenApiBindAndPort()
        let token = currentToken ?? L10n.t(.mcpPlaceholderToken)
        let jsonConfig = "{\"url\":\"http://\(bind):\(port)/mcp\",\"headers\":{\"Authorization\":\"Bearer \(token)\"}}"

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(jsonConfig, forType: .string)

        setStatus(copyFeedbackLabel, text: L10n.t(.mcpCopiedOpenApiConfig), color: McpPageTokens.normalText)
    }

    @objc private func handleCopyTechDetails() {
        let text = techDetailsText()
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        setStatus(techDetailsFeedbackLabel, text: L10n.t(.mcpTechDetailsCopied), color: McpPageTokens.normalText)
    }

    // MARK: - MCP info

    public func loadMcpInfo() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "mcp-info", "--json"], timeout: 20)
            do {
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8) {
                    let payload = try McpPageLogic.parseMcpInfoPayload(json: jsonStr)
                    DispatchQueue.main.async {
                        guard let self = self else { return }
                        self.currentMcpRawJSON = jsonStr
                        self.applyMcpInfo(payload)
                        self.setStatus(self.testConnStatusLabel, text: L10n.t(.mcpTestConnComplete), color: McpPageTokens.normalText)
                        self.updateTechDetails()
                    }
                    return
                }
            } catch {
                // Command failed or not found
            }

            DispatchQueue.main.async {
                self?.currentMcpRawJSON = nil
                self?.applyFallbackMcpInfo()
                self?.updateTechDetails()
            }
        }
    }

    private func applyMcpInfo(_ payload: McpInfoPayload) {
        lastMcpPayload = payload
        renderEndpoints(payload.endpoints ?? [])
        renderTools(groups: payload.toolGroups ?? [])
    }

    private func renderEndpoints(_ endpoints: [McpEndpointInfo]) {
        for view in endpointsStack.arrangedSubviews {
            view.removeFromSuperview()
        }
        let header = makeEndpointHeaderRow()
        endpointsStack.addArrangedSubview(header)
        header.widthAnchor.constraint(equalTo: endpointsStack.widthAnchor).isActive = true
        if endpoints.isEmpty {
            let empty = NSTextField(labelWithString: L10n.t(.mcpPageEmptyEndpoints))
            empty.font = NSFont.systemFont(ofSize: 12)
            empty.textColor = McpPageTokens.textSecondary
            endpointsStack.addArrangedSubview(empty)
            return
        }
        for endpoint in endpoints {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .centerY
            row.spacing = 12
            row.translatesAutoresizingMaskIntoConstraints = false

            let name = NSTextField(labelWithString: endpoint.label)
            name.font = NSFont.systemFont(ofSize: 13)
            name.textColor = McpPageTokens.textPrimary
            name.lineBreakMode = .byTruncatingTail
            name.widthAnchor.constraint(equalToConstant: 140).isActive = true

            let value = NSTextField(labelWithString: endpoint.value)
            value.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
            value.textColor = McpPageTokens.textSecondary
            value.lineBreakMode = .byTruncatingTail
            value.toolTip = endpoint.value
            value.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)

            let tag = makeEndpointStateTag(endpoint)
            tag.translatesAutoresizingMaskIntoConstraints = false

            row.addArrangedSubview(name)
            row.addArrangedSubview(value)
            row.addArrangedSubview(tag)
            endpointsStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: endpointsStack.widthAnchor).isActive = true
            let separator = makeSeparator()
            endpointsStack.addArrangedSubview(separator)
            separator.widthAnchor.constraint(equalTo: endpointsStack.widthAnchor).isActive = true
        }
    }

    private func makeEndpointHeaderRow() -> NSView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 12
        row.translatesAutoresizingMaskIntoConstraints = false
        let entry = makeHelperLabel(L10n.t(.mcpColEntry))
        entry.widthAnchor.constraint(equalToConstant: 140).isActive = true
        let endpoint = makeHelperLabel(L10n.t(.mcpColEndpoint))
        let status = makeHelperLabel(L10n.t(.mcpLabelStatus))
        status.widthAnchor.constraint(equalToConstant: 90).isActive = true
        row.addArrangedSubview(entry)
        row.addArrangedSubview(endpoint)
        row.addArrangedSubview(status)
        let headerSeparator = makeSeparator()
        let wrapper = NSStackView(views: [row, headerSeparator])
        wrapper.orientation = .vertical
        wrapper.alignment = .leading
        wrapper.spacing = 6
        row.widthAnchor.constraint(equalTo: wrapper.widthAnchor).isActive = true
        headerSeparator.widthAnchor.constraint(equalTo: wrapper.widthAnchor).isActive = true
        return wrapper
    }

    private func makeSeparator() -> NSView {
        let line = NSView()
        line.wantsLayer = true
        line.layer?.backgroundColor = McpPageTokens.cardBorder.cgColor
        line.translatesAutoresizingMaskIntoConstraints = false
        line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return line
    }

    private func makeEndpointStateTag(_ endpoint: McpEndpointInfo) -> ThemeStatusTag {
        if endpoint.ok {
            return ThemeStatusTag(status: .normal, customText: L10n.t(.tagNormal))
        }
        if endpoint.id == "open_api" {
            let text = endpoint.detail.isEmpty ? L10n.t(.mcpStatusNotStarted) : endpoint.detail
            return ThemeStatusTag(status: .inactive, customText: text)
        }
        let text = endpoint.detail.isEmpty ? L10n.t(.tagError) : endpoint.detail
        return ThemeStatusTag(status: .error, customText: text)
    }

    private func renderTools(groups: [McpToolGroupInfo]) {
        for view in toolsStack.arrangedSubviews {
            view.removeFromSuperview()
        }
        if groups.isEmpty {
            let empty = NSTextField(wrappingLabelWithString: L10n.t(.mcpPageEmptyTools))
            empty.font = NSFont.systemFont(ofSize: 12)
            empty.textColor = McpPageTokens.textSecondary
            toolsStack.addArrangedSubview(empty)
            return
        }
        for group in groups {
            let title = NSTextField(labelWithString: group.title)
            title.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
            title.textColor = McpPageTokens.textPrimary
            let tools = NSTextField(wrappingLabelWithString: group.tools.joined(separator: ", "))
            tools.font = NSFont.systemFont(ofSize: 12)
            tools.textColor = McpPageTokens.textSecondary
            tools.isSelectable = true
            let box = NSStackView(views: [title, tools])
            box.orientation = .vertical
            box.alignment = .leading
            box.spacing = 4
            toolsStack.addArrangedSubview(box)
        }
    }

    private func applyFallbackMcpInfo() {
        lastMcpPayload = nil
        let home = NSHomeDirectory()
        let defaultEndpoints: [McpEndpointInfo] = [
            McpEndpointInfo(
                id: "tool_service",
                label: L10n.t(.mcpEndpointToolService),
                value: "\(home)/.wechat-rpa/bin/wechat-mcp",
                ok: FileManager.default.isExecutableFile(atPath: "\(home)/.wechat-rpa/bin/wechat-mcp"),
                detail: ""
            ),
            McpEndpointInfo(
                id: "core_socket",
                label: L10n.t(.uiMcp135),
                value: "\(home)/.wechat-rpa/core.sock",
                ok: FileManager.default.fileExists(atPath: "\(home)/.wechat-rpa/core.sock"),
                detail: ""
            ),
            McpEndpointInfo(
                id: "chat_history",
                label: L10n.t(.mcpEndpointChatHistory),
                value: "http://127.0.0.1:8765",
                ok: false,
                detail: ""
            ),
            McpEndpointInfo(
                id: "open_api",
                label: L10n.t(.mcpEndpointOpenApi),
                value: "http://127.0.0.1:8770/mcp",
                ok: false,
                detail: L10n.t(.mcpStatusNotStarted)
            )
        ]
        renderEndpoints(defaultEndpoints)

        // 默认工具分组（按三组标题）；放不进 L10n 的是工具名本身，标题仍走 L10n
        let defaultGroups: [McpToolGroupInfo] = [
            McpToolGroupInfo(title: L10n.t(.mcpGroupOpenApi), tools: [
                "next_turn", "reply", "get_turn_status", "resume_session",
                "history_list_contacts", "history_get_contact", "history_get_recent_messages",
                "history_get_history", "history_search_messages", "history_get_message", "history_get_media"
            ]),
            McpToolGroupInfo(title: L10n.t(.mcpGroupOps), tools: [
                "send_message", "send_file", "list_chats", "get_messages",
                "wait_for_message", "list_accounts", "list_conversations"
            ]),
            McpToolGroupInfo(title: L10n.t(.mcpGroupDesktop), tools: [
                "desktop_apps", "desktop_windows", "desktop_observe", "desktop_act", "desktop_stop"
            ])
        ]
        renderTools(groups: defaultGroups)

        setStatus(testConnStatusLabel, text: L10n.t(.contactsOffline), color: McpPageTokens.textSecondary)
    }

    // MARK: - Technical details

    private func techDetailsText() -> String {
        var lines: [String] = []
        let raw = context.currentRawConfig
        let brain = raw["brain"] as? [String: Any]
        let external = brain?["external"] as? [String: Any]
        let openApi = raw["open_api"] as? [String: Any]

        lines.append("[brain]")
        lines.append("brain.mode=\(brain?["mode"] as? String ?? "-")")
        lines.append("brain.external.timeout_seconds=\((external?["timeout_seconds"] as? Int).map { String($0) } ?? "-")")
        lines.append("brain.external.timeout_action=\(external?["timeout_action"] as? String ?? "-")")
        lines.append("brain.external.queue_minutes=\((external?["queue_minutes"] as? Int).map { String($0) } ?? "-")")
        lines.append("[open_api]")
        lines.append("open_api.enabled=\(openApi?["enabled"] as? Bool ?? false)")
        lines.append("open_api.bind=\(openApi?["bind"] as? String ?? "-")")
        lines.append("open_api.port=\((openApi?["port"] as? Int).map { String($0) } ?? "-")")
        lines.append("[worker_socket]")
        lines.append("\(NSHomeDirectory())/.wechat-rpa/worker.sock")
        lines.append("[mcp_endpoints]")
        if let payload = lastMcpPayload, let endpoints = payload.endpoints, !endpoints.isEmpty {
            for endpoint in endpoints {
                lines.append("\(endpoint.id)\t\(endpoint.value)\tok=\(endpoint.ok)\tdetail=\(endpoint.detail)")
            }
        } else {
            lines.append("-")
        }
        if let error = lastOpenApiError, !error.isEmpty {
            lines.append("[open_api_error]")
            lines.append(error)
        }
        return lines.joined(separator: "\n")
    }

    private func updateTechDetails() {
        let text = techDetailsText()
        techDetailsLabel.stringValue = text
        techDetailsLabel.toolTip = text
    }
}
