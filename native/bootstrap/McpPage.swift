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

public final class McpPageController: NSViewController {
    private let context: StatusPanelController

    // UI - Brain Selection
    private let modeBuiltinRadio = NSButton(radioButtonWithTitle: L10n.t(.uiMcp115), target: nil, action: nil)
    private let modeExternalRadio = NSButton(radioButtonWithTitle: L10n.t(.uiMcp116), target: nil, action: nil)

    // UI - External Brain Parameters
    private let timeoutField = NSTextField()
    private let timeoutActionPopup = NSPopUpButton()
    private let queueMinutesField = NSTextField()
    private let saveBrainButton = NSButton(title: L10n.t(.uiMcp117), target: nil, action: nil)
    private let brainStatusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Open API Section
    private let openApiEnabledLabel = NSTextField(wrappingLabelWithString: "")
    private let openApiEnabledCheckbox = NSButton(checkboxWithTitle: L10n.t(.uiMcp118), target: nil, action: nil)
    private let openApiUrlLabel = NSTextField(labelWithString: "")
    private let openApiTokenMaskedLabel = NSTextField(labelWithString: L10n.t(.mcpTokenMasked))
    private let copyTokenButton = NSButton(title: L10n.t(.actionCopy), target: nil, action: nil)
    private let rotateTokenButton = NSButton(title: L10n.t(.mcpRegenerate), target: nil, action: nil)
    private let testOpenApiButton = NSButton(title: L10n.t(.modelPageButtonTest), target: nil, action: nil)
    private let openApiStatusLabel = NSTextField(wrappingLabelWithString: "")
    private var currentToken: String?

    // UI - Endpoints
    private let endpointsStack = NSStackView()
    private let testConnButton = NSButton(title: L10n.t(.modelPageButtonTest), target: nil, action: nil)
    private let testConnStatusLabel = NSTextField(labelWithString: "")

    // UI - Copy Config
    private let copyOpsSshButton = NSButton(title: L10n.t(.mcpCopyOpsSshBtn), target: nil, action: nil)
    private let copyOpenApiButton = NSButton(title: L10n.t(.mcpCopyOpenApiBtn), target: nil, action: nil)
    private let copyFeedbackLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Tools
    private let toolsStack = NSStackView()

    public init(context: StatusPanelController) {
        self.context = context
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

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let contentStack = NSStackView()
        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = 16
        contentStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. 回复决策（大脑模式）
        let brainBox = createSectionBox(title: L10n.t(.uiMcp119))
        let brainStack = NSStackView()
        brainStack.orientation = .vertical
        brainStack.alignment = .leading
        brainStack.spacing = 10
        brainStack.translatesAutoresizingMaskIntoConstraints = false

        modeBuiltinRadio.target = self
        modeBuiltinRadio.action = #selector(handleBrainModeChanged)
        modeExternalRadio.target = self
        modeExternalRadio.action = #selector(handleBrainModeChanged)

        brainStack.addArrangedSubview(modeBuiltinRadio)
        brainStack.addArrangedSubview(modeExternalRadio)

        // 外部大脑参数
        let extParamsGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.uiMcp120)), timeoutField, NSTextField(labelWithString: L10n.t(.mcpFieldTimeoutHelper))],
            [NSTextField(labelWithString: L10n.t(.uiMcp121)), timeoutActionPopup, NSTextField(labelWithString: L10n.t(.mcpFieldTimeoutActionHelper))],
            [NSTextField(labelWithString: L10n.t(.uiMcp122)), queueMinutesField, NSTextField(labelWithString: L10n.t(.mcpFieldQueueMinutesHelper))]
        ])
        extParamsGrid.rowSpacing = 6
        extParamsGrid.columnSpacing = 10
        timeoutField.widthAnchor.constraint(equalToConstant: 80).isActive = true
        queueMinutesField.widthAnchor.constraint(equalToConstant: 80).isActive = true

        timeoutActionPopup.addItems(withTitles: ["skip", "handoff", "builtin"])

        brainStack.addArrangedSubview(extParamsGrid)

        let btnRow = NSStackView()
        btnRow.orientation = .horizontal
        btnRow.spacing = 10
        saveBrainButton.target = self
        saveBrainButton.action = #selector(handleSaveBrain)
        btnRow.addArrangedSubview(saveBrainButton)
        btnRow.addArrangedSubview(brainStatusMessageLabel)
        brainStack.addArrangedSubview(btnRow)

        let pointerLabel = NSTextField(wrappingLabelWithString: McpPageLogic.brainPagePointerText())
        pointerLabel.font = NSFont.systemFont(ofSize: 11)
        pointerLabel.textColor = .secondaryLabelColor
        let goToBrainBtn = NSButton(title: L10n.t(.uiMcp123), target: self, action: #selector(handleGoToBrain))
        let pointerRow = NSStackView(views: [pointerLabel, goToBrainBtn])
        pointerRow.orientation = .horizontal
        pointerRow.spacing = 8
        brainStack.addArrangedSubview(pointerRow)

        brainBox.setContent(brainStack)
        contentStack.addArrangedSubview(brainBox)

        // 2. 对外接口区
        let openApiBox = createSectionBox(title: L10n.t(.mcpEndpointOpenApi))
        let openApiStack = NSStackView()
        openApiStack.orientation = .vertical
        openApiStack.alignment = .leading
        openApiStack.spacing = 10
        openApiStack.translatesAutoresizingMaskIntoConstraints = false

        openApiEnabledCheckbox.target = self
        openApiEnabledCheckbox.action = #selector(handleOpenApiToggle)
        openApiStack.addArrangedSubview(openApiEnabledCheckbox)
        openApiStack.addArrangedSubview(openApiEnabledLabel)
        openApiStack.addArrangedSubview(openApiUrlLabel)

        let tokenRow = NSStackView()
        tokenRow.orientation = .horizontal
        tokenRow.spacing = 10
        tokenRow.alignment = .centerY

        copyTokenButton.target = self
        copyTokenButton.action = #selector(handleCopyToken)
        rotateTokenButton.target = self
        rotateTokenButton.action = #selector(handleRotateToken)

        tokenRow.addArrangedSubview(openApiTokenMaskedLabel)
        tokenRow.addArrangedSubview(copyTokenButton)
        tokenRow.addArrangedSubview(rotateTokenButton)
        openApiStack.addArrangedSubview(tokenRow)

        let openApiCheckRow = NSStackView()
        openApiCheckRow.orientation = .horizontal
        openApiCheckRow.spacing = 10
        openApiCheckRow.alignment = .centerY

        testOpenApiButton.target = self
        testOpenApiButton.action = #selector(handleTestOpenApi)

        openApiCheckRow.addArrangedSubview(testOpenApiButton)
        openApiCheckRow.addArrangedSubview(openApiStatusLabel)
        openApiStack.addArrangedSubview(openApiCheckRow)

        openApiBox.setContent(openApiStack)
        contentStack.addArrangedSubview(openApiBox)

        // 3. 连接端点与连通性
        let endBox = createSectionBox(title: L10n.t(.mcpSectionEndpoints))
        let endStack = NSStackView()
        endStack.orientation = .vertical
        endStack.alignment = .leading
        endStack.spacing = 10
        endStack.translatesAutoresizingMaskIntoConstraints = false

        endpointsStack.orientation = .vertical
        endpointsStack.alignment = .leading
        endpointsStack.spacing = 6
        endStack.addArrangedSubview(endpointsStack)

        let testRow = NSStackView()
        testRow.orientation = .horizontal
        testRow.spacing = 10
        testConnButton.target = self
        testConnButton.action = #selector(handleTestConnection)
        testRow.addArrangedSubview(testConnButton)
        testRow.addArrangedSubview(testConnStatusLabel)
        endStack.addArrangedSubview(testRow)

        // 配置示例说明
        let cfgBox = NSBox()
        cfgBox.title = L10n.t(.mcpBoxConfigExample)
        cfgBox.boxType = .custom
        cfgBox.fillColor = NSColor.windowBackgroundColor
        cfgBox.cornerRadius = 4
        cfgBox.translatesAutoresizingMaskIntoConstraints = false

        let cfgText = L10n.t(.uiResidualMcpConfigExample)
        let cfgLabel = NSTextField(wrappingLabelWithString: cfgText)
        cfgLabel.font = NSFont.userFixedPitchFont(ofSize: 11)
        cfgLabel.isSelectable = true

        let copyBtnRow = NSStackView()
        copyBtnRow.orientation = .horizontal
        copyBtnRow.spacing = 10
        copyOpsSshButton.target = self
        copyOpsSshButton.action = #selector(handleCopyOpsSshConfig)
        copyOpenApiButton.target = self
        copyOpenApiButton.action = #selector(handleCopyOpenApiConfig)
        copyBtnRow.addArrangedSubview(copyOpsSshButton)
        copyBtnRow.addArrangedSubview(copyOpenApiButton)

        let cfgStack = NSStackView(views: [cfgLabel, copyBtnRow, copyFeedbackLabel])
        cfgStack.orientation = .vertical
        cfgStack.alignment = .leading
        cfgStack.spacing = 6
        cfgBox.setContent(cfgStack)

        endStack.addArrangedSubview(cfgBox)
        endBox.setContent(endStack)
        contentStack.addArrangedSubview(endBox)

        // 4. 可用工具按组列出
        let toolsBox = createSectionBox(title: L10n.t(.mcpSectionTools))
        toolsStack.orientation = .vertical
        toolsStack.alignment = .leading
        toolsStack.spacing = 8
        toolsStack.translatesAutoresizingMaskIntoConstraints = false
        toolsBox.setContent(toolsStack)
        contentStack.addArrangedSubview(toolsBox)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 16),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -16),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            brainBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            openApiBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            endBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            toolsBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            cfgBox.widthAnchor.constraint(equalTo: endStack.widthAnchor)
        ])
    }

    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)
        box.boxType = .primary
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

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

        // Open API Info
        let openApi = raw["open_api"] as? [String: Any]
        let isEnabled = (openApi?["enabled"] as? Bool) ?? false
        openApiEnabledCheckbox.state = isEnabled ? .on : .off
        if isEnabled {
            openApiEnabledLabel.stringValue = L10n.t(.uiMcp125)
            openApiEnabledLabel.textColor = .systemGreen
        } else {
            openApiEnabledLabel.stringValue = L10n.t(.uiMcp126)
            openApiEnabledLabel.textColor = .secondaryLabelColor
        }

        let (bind, port) = getOpenApiBindAndPort()
        openApiUrlLabel.stringValue = L10n.t(.uiMcp127, String(describing: bind), String(describing: port))
        updateTokenDisplay()
    }

    private func updateTokenDisplay() {
        if let token = currentToken, token.count >= 4 {
            let tail = String(token.suffix(4))
            openApiTokenMaskedLabel.stringValue = L10n.t(.mcpTokenMaskedWithTail, String(describing: tail))
        } else {
            openApiTokenMaskedLabel.stringValue = L10n.t(.mcpTokenMasked)
        }
    }

    private func setExternalInputsEnabled(_ enabled: Bool) {
        timeoutField.isEnabled = enabled
        timeoutActionPopup.isEnabled = enabled
        queueMinutesField.isEnabled = enabled
    }

    @objc private func handleBrainModeChanged() {
        let isExternal = (modeExternalRadio.state == .on)
        setExternalInputsEnabled(isExternal)
    }

    @objc private func handleSaveBrain() {
        let isExternal = (modeExternalRadio.state == .on)
        let res: Result<[String: Any], PageError>

        if isExternal {
            guard let timeout = Int(timeoutField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                brainStatusMessageLabel.stringValue = L10n.t(.mcpErrorTimeoutInt)
                brainStatusMessageLabel.textColor = .systemRed
                return
            }
            guard let queue = Int(queueMinutesField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                brainStatusMessageLabel.stringValue = L10n.t(.mcpErrorQueueInt)
                brainStatusMessageLabel.textColor = .systemRed
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
                brainStatusMessageLabel.stringValue = L10n.t(.uiMcp128)
                brainStatusMessageLabel.textColor = .systemGreen
            } catch {
                brainStatusMessageLabel.stringValue = L10n.t(.saveFailed, String(describing: error.localizedDescription))
                brainStatusMessageLabel.textColor = .systemRed
            }
        case .failure(let err):
            brainStatusMessageLabel.stringValue = L10n.t(.validationError, String(describing: err.message))
            brainStatusMessageLabel.textColor = .systemRed
        }
    }

    @objc private func handleGoToBrain() {
        if let shell = view.window?.contentViewController as? ShellViewController {
            shell.selectPage(id: "brain")
        }
    }

    @objc private func handleOpenApiToggle() {
        let enabled = openApiEnabledCheckbox.state == .on
        let updated = McpPageLogic.updateOpenApiEnabled(config: context.currentRawConfig, enabled: enabled)
        do {
            // The HTTP listener starts with serve-core: saving restarts the core to apply it.
            try context.persistConfig(updated)
            openApiStatusLabel.stringValue = enabled ? L10n.t(.uiMcp129) : L10n.t(.uiMcp130)
            openApiStatusLabel.textColor = .systemGreen
        } catch {
            openApiEnabledCheckbox.state = enabled ? .off : .on
            openApiStatusLabel.stringValue = L10n.t(.saveFailed, String(describing: error.localizedDescription))
            openApiStatusLabel.textColor = .systemRed
        }
        populateCurrentValues()
    }

    @objc private func handleTestConnection() {
        testConnStatusLabel.stringValue = L10n.t(.mcpTestingConnection)
        testConnStatusLabel.textColor = .systemBlue
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
            openApiStatusLabel.stringValue = L10n.t(.mcpTokenCopied)
            openApiStatusLabel.textColor = .systemGreen
        } else {
            openApiStatusLabel.stringValue = L10n.t(.mcpReadingToken)
            openApiStatusLabel.textColor = .systemBlue
            fetchToken(rotate: false) { [weak self] token in
                if let t = token, !t.isEmpty {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString(t, forType: .string)
                    self?.openApiStatusLabel.stringValue = L10n.t(.mcpTokenCopied)
                    self?.openApiStatusLabel.textColor = .systemGreen
                } else {
                    self?.openApiStatusLabel.stringValue = L10n.t(.mcpFetchTokenFailed)
                    self?.openApiStatusLabel.textColor = .systemRed
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
            openApiStatusLabel.stringValue = L10n.t(.mcpRegeneratingToken)
            openApiStatusLabel.textColor = .systemBlue
            fetchToken(rotate: true) { [weak self] token in
                if token != nil {
                    self?.openApiStatusLabel.stringValue = L10n.t(.mcpTokenRegeneratedSuccess)
                    self?.openApiStatusLabel.textColor = .systemGreen
                } else {
                    self?.openApiStatusLabel.stringValue = L10n.t(.mcpRegenerateTokenFailed)
                    self?.openApiStatusLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleTestOpenApi() {
        openApiStatusLabel.stringValue = L10n.t(.mcpTestingOpenApi)
        openApiStatusLabel.textColor = .systemBlue
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "open-api-check", "--json"], timeout: 10)
            DispatchQueue.main.async {
                guard let self = self else { return }
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8),
                   let payload = try? McpPageLogic.parseCheckPayload(json: jsonStr) {
                    if payload.ok {
                        self.openApiStatusLabel.stringValue = L10n.t(.uiMcp133, String(describing: payload.tools.count))
                        self.openApiStatusLabel.textColor = .systemGreen
                    } else if payload.busy {
                        self.openApiStatusLabel.stringValue = L10n.t(.uiMcp134)
                        self.openApiStatusLabel.textColor = .systemOrange
                    } else if payload.error == "disabled" {
                        self.openApiStatusLabel.stringValue = L10n.t(.mcpOpenApiDisabledNotice)
                        self.openApiStatusLabel.textColor = .secondaryLabelColor
                    } else {
                        self.openApiStatusLabel.stringValue = L10n.t(.uiResidualMcpTestFailedFmt, String(describing: payload.error ?? L10n.t(.uiResidualConnectFailed)))
                        self.openApiStatusLabel.textColor = .systemRed
                    }
                } else {
                    self.openApiStatusLabel.stringValue = L10n.t(.mcpOpenApiCheckFailed)
                    self.openApiStatusLabel.textColor = .systemRed
                }
            }
        }
    }

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
            copyFeedbackLabel.stringValue = L10n.t(.mcpCopiedSshPlaceholder)
            copyFeedbackLabel.textColor = .systemOrange
        } else {
            copyFeedbackLabel.stringValue = L10n.t(.mcpCopiedSsh)
            copyFeedbackLabel.textColor = .systemGreen
        }
    }

    @objc private func handleCopyOpenApiConfig() {
        let (bind, port) = getOpenApiBindAndPort()
        let token = currentToken ?? L10n.t(.mcpPlaceholderToken)
        let jsonConfig = "{\"url\":\"http://\(bind):\(port)/mcp\",\"headers\":{\"Authorization\":\"Bearer \(token)\"}}"

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(jsonConfig, forType: .string)

        copyFeedbackLabel.stringValue = L10n.t(.mcpCopiedOpenApiConfig)
        copyFeedbackLabel.textColor = .systemGreen
    }

    public func loadMcpInfo() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "mcp-info", "--json"], timeout: 20)
            do {
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8) {
                    let payload = try McpPageLogic.parseMcpInfoPayload(json: jsonStr)
                    DispatchQueue.main.async {
                        self?.applyMcpInfo(payload)
                        self?.testConnStatusLabel.stringValue = L10n.t(.mcpTestConnComplete)
                        self?.testConnStatusLabel.textColor = .systemGreen
                    }
                    return
                }
            } catch {
                // Command failed or not found
            }

            DispatchQueue.main.async {
                self?.applyFallbackMcpInfo()
            }
        }
    }

    private func applyMcpInfo(_ payload: McpInfoPayload) {
        // Endpoints
        for v in endpointsStack.arrangedSubviews { v.removeFromSuperview() }
        let endpoints = payload.endpoints ?? []
        for ep in endpoints {
            let row = NSStackView()
            row.orientation = .horizontal
            row.spacing = 8

            let isBusy = (ep.id == "open_api" && ep.detail == L10n.t(.uiMcp134))
            let dot = NSTextField(labelWithString: "●")
            if isBusy {
                dot.textColor = .systemOrange
            } else if ep.ok {
                dot.textColor = .systemGreen
            } else if ep.id == "open_api" && ep.detail == L10n.t(.mcpStatusClosed) {
                dot.textColor = .secondaryLabelColor
            } else {
                dot.textColor = .systemRed
            }

            let title = NSTextField(labelWithString: "\(ep.label):")
            title.font = NSFont.boldSystemFont(ofSize: 12)
            title.widthAnchor.constraint(equalToConstant: 120).isActive = true

            let val = NSTextField(labelWithString: "\(ep.value)  \(ep.detail)")
            val.font = NSFont.systemFont(ofSize: 11)
            if isBusy {
                val.textColor = .systemOrange
            } else {
                val.textColor = .secondaryLabelColor
            }

            row.addArrangedSubview(dot)
            row.addArrangedSubview(title)
            row.addArrangedSubview(val)
            endpointsStack.addArrangedSubview(row)
        }

        // Tools
        for v in toolsStack.arrangedSubviews { v.removeFromSuperview() }
        let groups = payload.toolGroups ?? []
        for grp in groups {
            let gTitle = NSTextField(labelWithString: "【\(grp.title)】")
            gTitle.font = NSFont.boldSystemFont(ofSize: 12)
            let gTools = NSTextField(wrappingLabelWithString: grp.tools.joined(separator: ", "))
            gTools.font = NSFont.systemFont(ofSize: 11)
            gTools.textColor = .secondaryLabelColor

            let gBox = NSStackView(views: [gTitle, gTools])
            gBox.orientation = .vertical
            gBox.alignment = .leading
            gBox.spacing = 2
            toolsStack.addArrangedSubview(gBox)
        }
    }

    private func applyFallbackMcpInfo() {
        // 离线默认展示
        for v in endpointsStack.arrangedSubviews { v.removeFromSuperview() }
        let home = NSHomeDirectory()
        let defaultEndpoints: [(String, String, Bool, String)] = [
            (L10n.t(.mcpEndpointToolService), "\(home)/.wechat-rpa/bin/wechat-mcp", FileManager.default.isExecutableFile(atPath: "\(home)/.wechat-rpa/bin/wechat-mcp"), ""),
            (L10n.t(.uiMcp135), "\(home)/.wechat-rpa/core.sock", FileManager.default.fileExists(atPath: "\(home)/.wechat-rpa/core.sock"), ""),
            (L10n.t(.mcpEndpointChatHistory), "http://127.0.0.1:8765", false, ""),
            (L10n.t(.mcpEndpointOpenApi), "http://127.0.0.1:8770/mcp", false, L10n.t(.mcpStatusNotStarted))
        ]

        for (label, path, ok, detail) in defaultEndpoints {
            let row = NSStackView()
            row.orientation = .horizontal
            row.spacing = 8

            let dot = NSTextField(labelWithString: "●")
            dot.textColor = ok ? .systemGreen : .systemRed
            let title = NSTextField(labelWithString: "\(label):")
            title.font = NSFont.boldSystemFont(ofSize: 12)
            title.widthAnchor.constraint(equalToConstant: 120).isActive = true

            let text = detail.isEmpty ? (ok ? L10n.t(.mcpStatusReady) : L10n.t(.mcpStatusNotStarted)) : detail
            let val = NSTextField(labelWithString: "\(path)  \(text)")
            val.font = NSFont.systemFont(ofSize: 11)
            val.textColor = .secondaryLabelColor

            row.addArrangedSubview(dot)
            row.addArrangedSubview(title)
            row.addArrangedSubview(val)
            endpointsStack.addArrangedSubview(row)
        }

        testConnStatusLabel.stringValue = L10n.t(.contactsOffline)
        testConnStatusLabel.textColor = .secondaryLabelColor

        // 默认工具分组（按三组标题：对外接口、运维、通用桌面）
        for v in toolsStack.arrangedSubviews { v.removeFromSuperview() }
        let defaultGroups = [
            (L10n.t(.mcpEndpointOpenApi), [
                "next_turn", "reply", "get_turn_status", "resume_session",
                "history_list_contacts", "history_get_contact", "history_get_recent_messages",
                "history_get_history", "history_search_messages", "history_get_message", "history_get_media"
            ]),
            (L10n.t(.mcpGroupOps), [
                "send_message", "send_file", "list_chats", "get_messages",
                "wait_for_message", "list_accounts", "list_conversations"
            ]),
            (L10n.t(.mcpGroupDesktop), [
                "desktop_apps", "desktop_windows", "desktop_observe", "desktop_act", "desktop_stop"
            ])
        ]
        for (gName, tools) in defaultGroups {
            let gTitle = NSTextField(labelWithString: "【\(gName)】")
            gTitle.font = NSFont.boldSystemFont(ofSize: 12)
            let gTools = NSTextField(wrappingLabelWithString: tools.joined(separator: ", "))
            gTools.font = NSFont.systemFont(ofSize: 11)
            gTools.textColor = .secondaryLabelColor

            let gBox = NSStackView(views: [gTitle, gTools])
            gBox.orientation = .vertical
            gBox.alignment = .leading
            gBox.spacing = 2
            toolsStack.addArrangedSubview(gBox)
        }
    }
}
