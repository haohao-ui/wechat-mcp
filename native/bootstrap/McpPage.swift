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

public struct McpPageLogic {
    public static func updateBrainSettings(
        config: [String: Any],
        timeoutSeconds: Int,
        timeoutAction: String,
        queueMinutes: Int
    ) -> Result<[String: Any], String> {
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
            return .failure(error.localizedDescription)
        }
    }

    public static func updateBrainModeBuiltin(config: [String: Any]) -> Result<[String: Any], String> {
        let formInputs: [String: Any] = ["brain_mode": "builtin"]
        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: config, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(error.localizedDescription)
        }
    }

    public static func parseMcpInfoPayload(json: String) throws -> McpInfoPayload {
        guard let data = json.data(using: .utf8) else {
            throw NSError(domain: "McpPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 数据"])
        }
        let decoder = JSONDecoder()
        return try decoder.decode(McpInfoPayload.self, from: data)
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

    // UI - Endpoints
    private let endpointsStack = NSStackView()
    private let testConnButton = NSButton(title: "测试连接", target: nil, action: nil)
    private let testConnStatusLabel = NSTextField(labelWithString: "")

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
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 720, height: 740))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        populateCurrentValues()
        loadMcpInfo()
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
            [NSTextField(labelWithString: "等待回复 (timeout_seconds):"), timeoutField, NSTextField(labelWithString: "1–3600 秒 (默认 120)")],
            [NSTextField(labelWithString: "超时后动作 (timeout_action):"), timeoutActionPopup, NSTextField(labelWithString: "skip=跳过, handoff=转人工, builtin=改用内置")],
            [NSTextField(labelWithString: "最长排队 (queue_minutes):"), queueMinutesField, NSTextField(labelWithString: "1–1440 分钟 (默认 30)")]
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

        brainBox.contentView = brainStack
        contentStack.addArrangedSubview(brainBox)

        // 2. 连接端点与连通性
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
        cfgBox.title = "客户端配置示例 (Claude Desktop / Cursor 等)"
        cfgBox.boxType = .custom
        cfgBox.fillColor = NSColor.windowBackgroundColor
        cfgBox.cornerRadius = 4
        cfgBox.translatesAutoresizingMaskIntoConstraints = false

        let cfgText = """
        {
          "mcpServers": {
            "wechat": {
              "command": "~/.wechat-rpa/bin/wechat-mcp",
              "args": ["serve"]
            }
          }
        }
        """
        let cfgLabel = NSTextField(wrappingLabelWithString: cfgText)
        cfgLabel.font = NSFont.userFixedPitchFont(ofSize: 11)
        cfgLabel.isSelectable = true

        let copyBtn = NSButton(title: "复制配置", target: self, action: #selector(handleCopyConfig))
        let cfgStack = NSStackView(views: [cfgLabel, copyBtn])
        cfgStack.orientation = .vertical
        cfgStack.alignment = .leading
        cfgStack.spacing = 6
        cfgBox.contentView = cfgStack

        endStack.addArrangedSubview(cfgBox)
        endBox.contentView = endStack
        contentStack.addArrangedSubview(endBox)

        // 3. 可用工具按组列出
        let toolsBox = createSectionBox(title: "可用 MCP 工具")
        toolsStack.orientation = .vertical
        toolsStack.alignment = .leading
        toolsStack.spacing = 8
        toolsStack.translatesAutoresizingMaskIntoConstraints = false
        toolsBox.contentView = toolsStack
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
        let res: Result<[String: Any], String>

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
            brainStatusMessageLabel.stringValue = "校验错误: \(err)"
            brainStatusMessageLabel.textColor = .systemRed
        }
    }

    @objc private func handleTestConnection() {
        testConnStatusLabel.stringValue = "正在测试连接..."
        testConnStatusLabel.textColor = .systemBlue
        loadMcpInfo()
    }

    @objc private func handleCopyConfig() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        let configTemplate = """
        {
          "mcpServers": {
            "wechat": {
              "command": "~/.wechat-rpa/bin/wechat-mcp",
              "args": ["serve"]
            }
          }
        }
        """
        pasteboard.setString(configTemplate, forType: .string)
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

            let dot = NSTextField(labelWithString: ep.ok ? "●" : "●")
            dot.textColor = ep.ok ? .systemGreen : .systemRed
            let title = NSTextField(labelWithString: "\(ep.label):")
            title.font = NSFont.boldSystemFont(ofSize: 12)
            title.widthAnchor.constraint(equalToConstant: 120).isActive = true

            let val = NSTextField(labelWithString: "\(ep.value)  \(ep.detail)")
            val.font = NSFont.systemFont(ofSize: 11)
            val.textColor = .secondaryLabelColor

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
        let defaultEndpoints = [
            ("MCP 工具服务", "\(home)/.wechat-rpa/bin/wechat-mcp", FileManager.default.isExecutableFile(atPath: "\(home)/.wechat-rpa/bin/wechat-mcp")),
            ("外部大脑接口", "\(home)/.wechat-rpa/core.sock", FileManager.default.fileExists(atPath: "\(home)/.wechat-rpa/core.sock")),
            ("聊天历史查询", "http://127.0.0.1:8765", false)
        ]

        for (label, path, ok) in defaultEndpoints {
            let row = NSStackView()
            row.orientation = .horizontal
            row.spacing = 8

            let dot = NSTextField(labelWithString: "●")
            dot.textColor = ok ? .systemGreen : .systemRed
            let title = NSTextField(labelWithString: "\(label):")
            title.font = NSFont.boldSystemFont(ofSize: 12)
            title.widthAnchor.constraint(equalToConstant: 120).isActive = true

            let val = NSTextField(labelWithString: "\(path) \(ok ? "已就绪" : "未启动/未连接")")
            val.font = NSFont.systemFont(ofSize: 11)
            val.textColor = .secondaryLabelColor

            row.addArrangedSubview(dot)
            row.addArrangedSubview(title)
            row.addArrangedSubview(val)
            endpointsStack.addArrangedSubview(row)
        }

        testConnStatusLabel.stringValue = "控制命令不可用 (离线状态)"
        testConnStatusLabel.textColor = .secondaryLabelColor

        // 默认工具分组
        for v in toolsStack.arrangedSubviews { v.removeFromSuperview() }
        let defaultGroups = [
            ("外部大脑", ["next_turn", "reply", "resume_session"]),
            ("微信操作", ["send_message", "send_file", "list_chats"]),
            ("客户", ["get_customer_profile", "list_customers"]),
            ("聊天历史", ["history_list_contacts", "history_get_contact", "history_query_turns"])
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
