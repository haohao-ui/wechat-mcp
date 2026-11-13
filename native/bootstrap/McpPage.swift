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

    public static func parseMcpInfoPayload(json: String) throws -> McpInfoPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 数据"])
        }
        let decoder = JSONDecoder()
        return try decoder.decode(McpInfoPayload.self, from: data)
    }

    public static func parseTokenPayload(json: String) throws -> OpenApiTokenPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 数据"])
        }
        return try JSONDecoder().decode(OpenApiTokenPayload.self, from: data)
    }

    public static func parseCheckPayload(json: String) throws -> OpenApiCheckPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 数据"])
        }
        return try JSONDecoder().decode(OpenApiCheckPayload.self, from: data)
    }

    public static func brainPagePointerText() -> String {
        return "角色定义、知识与话术请前往【大脑】页面管理；本页面仅管理大脑模式、外部超时与对外接口连接。"
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
    private let modeBuiltinRadio = NSButton(radioButtonWithTitle: "内置大脑 (调用配置好的大模型生成回复)", target: nil, action: nil)
    private let modeExternalRadio = NSButton(radioButtonWithTitle: "外部大脑 (MCP 协议接管消息轮询与回复决策)", target: nil, action: nil)

    // UI - External Brain Parameters
    private let timeoutField = NSTextField()
    private let timeoutActionPopup = NSPopUpButton()
    private let queueMinutesField = NSTextField()
    private let saveBrainButton = NSButton(title: "保存大脑设置", target: nil, action: nil)
    private let brainStatusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Open API Section
    private let openApiEnabledLabel = NSTextField(wrappingLabelWithString: "")
    private let openApiUrlLabel = NSTextField(labelWithString: "")
    private let openApiTokenMaskedLabel = NSTextField(labelWithString: "令牌: ••••")
    private let copyTokenButton = NSButton(title: "复制", target: nil, action: nil)
    private let rotateTokenButton = NSButton(title: "重新生成", target: nil, action: nil)
    private let testOpenApiButton = NSButton(title: "测试连接", target: nil, action: nil)
    private let openApiStatusLabel = NSTextField(wrappingLabelWithString: "")
    private var currentToken: String?

    // UI - Endpoints
    private let endpointsStack = NSStackView()
    private let testConnButton = NSButton(title: "测试连接", target: nil, action: nil)
    private let testConnStatusLabel = NSTextField(labelWithString: "")

    // UI - Copy Config
    private let copyOpsSshButton = NSButton(title: "复制运维 SSH 配置", target: nil, action: nil)
    private let copyOpenApiButton = NSButton(title: "复制对外接口配置", target: nil, action: nil)
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
        let brainBox = createSectionBox(title: "回复决策大脑")
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
            [NSTextField(labelWithString: "等待回复:"), timeoutField, NSTextField(labelWithString: "1–3600 秒 (默认 120)")],
            [NSTextField(labelWithString: "超时后动作:"), timeoutActionPopup, NSTextField(labelWithString: "skip=跳过, handoff=转人工, builtin=改用内置")],
            [NSTextField(labelWithString: "最长排队:"), queueMinutesField, NSTextField(labelWithString: "1–1440 分钟 (默认 30)")]
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
        let goToBrainBtn = NSButton(title: "前往大脑页面", target: self, action: #selector(handleGoToBrain))
        let pointerRow = NSStackView(views: [pointerLabel, goToBrainBtn])
        pointerRow.orientation = .horizontal
        pointerRow.spacing = 8
        brainStack.addArrangedSubview(pointerRow)

        brainBox.setContent(brainStack)
        contentStack.addArrangedSubview(brainBox)

        // 2. 对外接口区
        let openApiBox = createSectionBox(title: "对外接口")
        let openApiStack = NSStackView()
        openApiStack.orientation = .vertical
        openApiStack.alignment = .leading
        openApiStack.spacing = 10
        openApiStack.translatesAutoresizingMaskIntoConstraints = false

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
        let endBox = createSectionBox(title: "连接信息与入口")
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
        cfgBox.title = "客户端接入配置示例"
        cfgBox.boxType = .custom
        cfgBox.fillColor = NSColor.windowBackgroundColor
        cfgBox.cornerRadius = 4
        cfgBox.translatesAutoresizingMaskIntoConstraints = false

        let cfgText = """
        1. 运维 SSH 入口 (终端 / SSH 转发):
        ssh <用户>@<本机地址> nc -U ~/.wechat-rpa/worker.sock

        2. 对外接口配置 (Claude Desktop / Cursor 等 HTTP MCP 客户端):
        {
          "mcpServers": {
            "wechat-brain": {
              "url": "http://<bind>:<port>/mcp",
              "headers": {
                "Authorization": "Bearer <令牌>"
              }
            }
          }
        }
        """
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
        let toolsBox = createSectionBox(title: "可用 MCP 工具")
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
        if isEnabled {
            openApiEnabledLabel.stringValue = "开关状态: 已开启 (只读，请到「设置」页修改 open_api.enabled)"
            openApiEnabledLabel.textColor = .systemGreen
        } else {
            openApiEnabledLabel.stringValue = "开关状态: 已关闭 (只读，请到「设置」页修改 open_api.enabled)"
            openApiEnabledLabel.textColor = .secondaryLabelColor
        }

        let (bind, port) = getOpenApiBindAndPort()
        openApiUrlLabel.stringValue = "接口地址: http://\(bind):\(port)/mcp"
        updateTokenDisplay()
    }

    private func updateTokenDisplay() {
        if let token = currentToken, token.count >= 4 {
            let tail = String(token.suffix(4))
            openApiTokenMaskedLabel.stringValue = "令牌: ••••\(tail)"
        } else {
            openApiTokenMaskedLabel.stringValue = "令牌: ••••"
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
                brainStatusMessageLabel.stringValue = "等待回复必须为整数"
                brainStatusMessageLabel.textColor = .systemRed
                return
            }
            guard let queue = Int(queueMinutesField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                brainStatusMessageLabel.stringValue = "最长排队必须为整数"
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
                brainStatusMessageLabel.stringValue = "大脑设置已保存"
                brainStatusMessageLabel.textColor = .systemGreen
            } catch {
                brainStatusMessageLabel.stringValue = "保存失败: \(error.localizedDescription)"
                brainStatusMessageLabel.textColor = .systemRed
            }
        case .failure(let err):
            brainStatusMessageLabel.stringValue = "校验错误: \(err.message)"
            brainStatusMessageLabel.textColor = .systemRed
        }
    }

    @objc private func handleGoToBrain() {
        if let shell = view.window?.contentViewController as? ShellViewController {
            shell.selectPage(id: "brain")
        }
    }

    @objc private func handleTestConnection() {
        testConnStatusLabel.stringValue = "正在测试连接..."
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
            openApiStatusLabel.stringValue = "令牌已复制到剪贴板"
            openApiStatusLabel.textColor = .systemGreen
        } else {
            openApiStatusLabel.stringValue = "正在读取令牌..."
            openApiStatusLabel.textColor = .systemBlue
            fetchToken(rotate: false) { [weak self] token in
                if let t = token, !t.isEmpty {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString(t, forType: .string)
                    self?.openApiStatusLabel.stringValue = "令牌已复制到剪贴板"
                    self?.openApiStatusLabel.textColor = .systemGreen
                } else {
                    self?.openApiStatusLabel.stringValue = "获取令牌失败"
                    self?.openApiStatusLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleRotateToken() {
        let alert = NSAlert()
        alert.messageText = "重新生成对外接口令牌"
        alert.informativeText = "重新生成后将立即生效，当前连接的大脑将会断开。确定重新生成吗？"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "重新生成")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn {
            openApiStatusLabel.stringValue = "正在重新生成令牌..."
            openApiStatusLabel.textColor = .systemBlue
            fetchToken(rotate: true) { [weak self] token in
                if token != nil {
                    self?.openApiStatusLabel.stringValue = "令牌已重新生成并生效"
                    self?.openApiStatusLabel.textColor = .systemGreen
                } else {
                    self?.openApiStatusLabel.stringValue = "重新生成令牌失败"
                    self?.openApiStatusLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleTestOpenApi() {
        openApiStatusLabel.stringValue = "正在测试对外接口连接..."
        openApiStatusLabel.textColor = .systemBlue
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let run = runControlCommand(executable: controlBinaryPath(), arguments: ["control", "open-api-check", "--json"], timeout: 10)
            DispatchQueue.main.async {
                guard let self = self else { return }
                if run.status == 0, !run.timedOut, let jsonStr = String(data: run.output, encoding: .utf8),
                   let payload = try? McpPageLogic.parseCheckPayload(json: jsonStr) {
                    if payload.ok {
                        self.openApiStatusLabel.stringValue = "连接正常 (可用工具: \(payload.tools.count) 个)"
                        self.openApiStatusLabel.textColor = .systemGreen
                    } else if payload.busy {
                        self.openApiStatusLabel.stringValue = "已有大脑连接中"
                        self.openApiStatusLabel.textColor = .systemOrange
                    } else if payload.error == "disabled" {
                        self.openApiStatusLabel.stringValue = "对外接口未开启 (open_api.enabled=false)"
                        self.openApiStatusLabel.textColor = .secondaryLabelColor
                    } else {
                        self.openApiStatusLabel.stringValue = "测试失败: \(payload.error ?? "连接失败")"
                        self.openApiStatusLabel.textColor = .systemRed
                    }
                } else {
                    self.openApiStatusLabel.stringValue = "执行 open-api-check 失败"
                    self.openApiStatusLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleCopyOpsSshConfig() {
        let ip = McpPageLogic.getPrimaryNonLoopbackIPv4()
        let host = ip ?? "<本机地址>"
        let user = NSUserName()
        let home = NSHomeDirectory()
        let cmd = "ssh \(user)@\(host) nc -U \(home)/.wechat-rpa/worker.sock"

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(cmd, forType: .string)

        if ip == nil {
            copyFeedbackLabel.stringValue = "已复制运维 SSH 命令（未检测到非回环 IPv4，已使用 <本机地址> 占位，请自行替换）"
            copyFeedbackLabel.textColor = .systemOrange
        } else {
            copyFeedbackLabel.stringValue = "已复制运维 SSH 命令"
            copyFeedbackLabel.textColor = .systemGreen
        }
    }

    @objc private func handleCopyOpenApiConfig() {
        let (bind, port) = getOpenApiBindAndPort()
        let token = currentToken ?? "<令牌>"
        let jsonConfig = "{\"url\":\"http://\(bind):\(port)/mcp\",\"headers\":{\"Authorization\":\"Bearer \(token)\"}}"

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(jsonConfig, forType: .string)

        copyFeedbackLabel.stringValue = "已复制对外接口配置"
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
                        self?.testConnStatusLabel.stringValue = "连接测试完成"
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

            let isBusy = (ep.id == "open_api" && ep.detail == "已有大脑连接中")
            let dot = NSTextField(labelWithString: "●")
            if isBusy {
                dot.textColor = .systemOrange
            } else if ep.ok {
                dot.textColor = .systemGreen
            } else if ep.id == "open_api" && ep.detail == "已关闭" {
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
            ("MCP 工具服务", "\(home)/.wechat-rpa/bin/wechat-mcp", FileManager.default.isExecutableFile(atPath: "\(home)/.wechat-rpa/bin/wechat-mcp"), ""),
            ("外部大脑接口", "\(home)/.wechat-rpa/core.sock", FileManager.default.fileExists(atPath: "\(home)/.wechat-rpa/core.sock"), ""),
            ("聊天历史查询", "http://127.0.0.1:8765", false, ""),
            ("对外接口", "http://127.0.0.1:8770/mcp", false, "未启动/未连接")
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

            let text = detail.isEmpty ? (ok ? "已就绪" : "未启动/未连接") : detail
            let val = NSTextField(labelWithString: "\(path)  \(text)")
            val.font = NSFont.systemFont(ofSize: 11)
            val.textColor = .secondaryLabelColor

            row.addArrangedSubview(dot)
            row.addArrangedSubview(title)
            row.addArrangedSubview(val)
            endpointsStack.addArrangedSubview(row)
        }

        testConnStatusLabel.stringValue = "控制命令不可用 (离线状态)"
        testConnStatusLabel.textColor = .secondaryLabelColor

        // 默认工具分组（按三组标题：对外接口、运维、通用桌面）
        for v in toolsStack.arrangedSubviews { v.removeFromSuperview() }
        let defaultGroups = [
            ("对外接口", [
                "next_turn", "reply", "get_turn_status", "resume_session",
                "history_list_contacts", "history_get_contact", "history_get_recent_messages",
                "history_get_history", "history_search_messages", "history_get_message", "history_get_media"
            ]),
            ("运维", [
                "send_message", "send_file", "list_chats", "get_messages",
                "wait_for_message", "list_accounts", "list_conversations"
            ]),
            ("通用桌面", [
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
