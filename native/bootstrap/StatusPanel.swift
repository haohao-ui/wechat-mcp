// StatusPanel.swift: AppKit status panel, action triggers, and config form interface.
import Foundation
import AppKit

let hostStatusDidRefreshNotification = Notification.Name("WeChatHostStatusDidRefresh")

/// 获取宿主程序当前版本号：优先读取 ~/.wechat-rpa/VERSION，其次读取 Info.plist 中的版本信息
public func hostAppVersion() -> String {
    let home = FileManager.default.homeDirectoryForCurrentUser
    let versionFile = home.appendingPathComponent(".wechat-rpa/VERSION")
    if let data = try? Data(contentsOf: versionFile),
       let str = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
       !str.isEmpty {
        return str
    }
    if let info = Bundle.main.infoDictionary,
       let build = info["CFBundleVersion"] as? String,
       !build.isEmpty, build != "1" {
        let short = info["CFBundleShortVersionString"] as? String ?? "0.1.0"
        return "\(short) (\(build))"
    }
    return Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.0"
}

public final class StatusPanelController: NSViewController {
    private let worker: SocketWorker
    private let coreSupervisor: CoreSupervisor
    private var timer: Timer?

    // UI Elements - Status
    private let statusStack = NSStackView()
    private var statusItemViews: [String: (dot: NSTextField, title: NSTextField, detail: NSTextField)] = [:]

    // UI Elements - Actions
    private let pauseResumeButton = NSButton()
    private let actionMessageLabel = NSTextField(labelWithString: "")

    // UI Elements - Config Form
    private let accountLabel = NSTextField(labelWithString: "-")
    private let backendLabel = NSTextField(labelWithString: "-")
    private let modelLabel = NSTextField(labelWithString: "-")
    private let keyStatusLabel = NSTextField(labelWithString: "-")

    private let allowlistField = NSTextField()
    private let ignorelistField = NSTextField()
    private let maxAttachmentField = NSTextField()
    private let maxRepliesField = NSTextField()
    private let cooldownField = NSTextField()
    private let quietSecField = NSTextField()
    private let maxSecField = NSTextField()
    private let contextMsgField = NSTextField()
    private let manageWindowCheck = NSButton(checkboxWithTitle: "自动管理微信主窗口 (调整至 880x640)", target: nil, action: nil)
    private let brainModePopup = NSPopUpButton()
    private let brainTimeoutField = NSTextField()
    private let brainTimeoutActionPopup = NSPopUpButton()
    private let resumeHandoffButton = NSButton(title: "恢复自动回复", target: nil, action: nil)
    /// Sessions the core marked as handed to a person (health.json sessions[*].state == "handoff").
    private var handoffSessionNames: [String] = []

    private let configErrorLabel = NSTextField(wrappingLabelWithString: "")
    private let configSaveButton = NSButton()

    private var currentConfigURL: URL?
    private var rawConfigDict: [String: Any] = [:]
    private var latestItems: [StatusItem] = []
    private var latestBotState: String?

    public var currentRawConfig: [String: Any] { rawConfigDict }
    public var logDirectoryURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/logs", isDirectory: true)
    }

    init(worker: SocketWorker, coreSupervisor: CoreSupervisor = CoreSupervisor()) {
        self.worker = worker
        self.coreSupervisor = coreSupervisor
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 720, height: 760))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        loadInitialConfig()
        refresh()

        // 5秒定时刷新
        timer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { [weak self] _ in
            self?.refresh()
        }
    }

    deinit {
        timer?.invalidate()
    }

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 16
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. Title & Header
        let currentVersion = hostAppVersion()
        let headerBox = createSectionBox(title: "微信客服助手 — 系统状态 (版本: \(currentVersion))")
        setupStatusSection(inside: headerBox)
        mainStack.addArrangedSubview(headerBox)

        // 2. One-click Actions
        let actionBox = createSectionBox(title: "一键操作")
        setupActionSection(inside: actionBox)
        mainStack.addArrangedSubview(actionBox)

        // 3. Configuration Form
        let configBox = createSectionBox(title: "常用配置")
        setupConfigSection(inside: configBox)
        mainStack.addArrangedSubview(configBox)

        // The scroll document must be flipped (content starts at the top) and sized by its stack.
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(mainStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            mainStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
            mainStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            mainStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            mainStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            headerBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            actionBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            configBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 14, weight: .bold)
        box.boxType = .primary
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    /// Pins `content` inside the box with margins, so the box takes its height from the content.
    private func embed(_ content: NSView, in box: NSBox) {
        guard let holder = box.contentView else { return }
        content.translatesAutoresizingMaskIntoConstraints = false
        holder.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: holder.leadingAnchor, constant: 12),
            content.trailingAnchor.constraint(equalTo: holder.trailingAnchor, constant: -12),
            content.topAnchor.constraint(equalTo: holder.topAnchor, constant: 10),
            content.bottomAnchor.constraint(equalTo: holder.bottomAnchor, constant: -10)
        ])
    }

    @objc private func requestScreen() {
        // macOS shows the system prompt only once per app; afterwards open the settings pane directly.
        if !CGRequestScreenCaptureAccess() {
            openPrivacyPane("Privacy_ScreenCapture")
        }
        refresh()
    }

    @objc private func requestAccessibility() {
        if !AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary) {
            openPrivacyPane("Privacy_Accessibility")
        }
        refresh()
    }

    private func openPrivacyPane(_ anchor: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)") {
            NSWorkspace.shared.open(url)
        }
    }

    private func setupStatusSection(inside box: NSBox) {
        statusStack.orientation = .vertical
        statusStack.alignment = .leading
        statusStack.spacing = 8
        statusStack.translatesAutoresizingMaskIntoConstraints = false

        let keys = ["permissions", "desktop_service", "bot", "brain", "sessions", "wechat", "layout", "disk"]
        let titles = [
            "permissions": "权限状态",
            "desktop_service": "桌面服务",
            "bot": "机器人核心",
            "sessions": "各会话状态",
            "wechat": "微信客户端",
            "layout": "布局地图",
            "disk": "磁盘空间",
            "brain": "大脑状态"
        ]

        for k in keys {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .firstBaseline
            row.spacing = 8

            let dot = NSTextField(labelWithString: "●")
            dot.font = NSFont.systemFont(ofSize: 14)
            dot.textColor = .systemGray

            let tLabel = NSTextField(labelWithString: titles[k] ?? k)
            tLabel.font = NSFont.systemFont(ofSize: 12, weight: .semibold)
            tLabel.setContentHuggingPriority(.required, for: .horizontal)

            let dLabel = NSTextField(wrappingLabelWithString: "正在检查...")
            dLabel.font = NSFont.systemFont(ofSize: 12)
            dLabel.textColor = .secondaryLabelColor
            dLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            tLabel.widthAnchor.constraint(equalToConstant: 80).isActive = true

            row.addArrangedSubview(dot)
            row.addArrangedSubview(tLabel)
            row.addArrangedSubview(dLabel)
            statusStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: statusStack.widthAnchor).isActive = true

            statusItemViews[k] = (dot: dot, title: tLabel, detail: dLabel)
        }

        let buttons = NSStackView(views: [
            NSButton(title: "立即刷新", target: self, action: #selector(refreshClicked)),
            NSButton(title: "请求屏幕录制权限", target: self, action: #selector(requestScreen)),
            NSButton(title: "请求辅助功能权限", target: self, action: #selector(requestAccessibility))
        ])
        buttons.orientation = .horizontal
        buttons.spacing = 8
        statusStack.addArrangedSubview(buttons)

        embed(statusStack, in: box)
    }

    private func setupActionSection(inside box: NSBox) {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let row1 = NSStackView(views: [
            NSButton(title: "重启桌面服务", target: self, action: #selector(handleRestartDesktopService)),
            NSButton(title: "重启机器人", target: self, action: #selector(handleRestartBot)),
            pauseResumeButton,
            NSButton(title: "放回标准位置", target: self, action: #selector(handleRestoreWechatWindow))
        ])
        row1.orientation = .horizontal
        row1.spacing = 8

        pauseResumeButton.title = "暂停机器人"
        pauseResumeButton.target = self
        pauseResumeButton.action = #selector(handleTogglePause)

        let row2 = NSStackView(views: [
            NSButton(title: "打开附件文件夹", target: self, action: #selector(handleOpenAttachments)),
            NSButton(title: "打开日志文件夹", target: self, action: #selector(handleOpenLogs)),
            NSButton(title: "复制诊断信息", target: self, action: #selector(handleCopyDiagnostics)),
            resumeHandoffButton
        ])
        resumeHandoffButton.target = self
        resumeHandoffButton.action = #selector(handleResumeHandoff)
        resumeHandoffButton.isEnabled = false
        row2.orientation = .horizontal
        row2.spacing = 8

        actionMessageLabel.font = NSFont.systemFont(ofSize: 11)
        actionMessageLabel.textColor = .systemBlue

        stack.addArrangedSubview(row1)
        stack.addArrangedSubview(row2)
        stack.addArrangedSubview(actionMessageLabel)

        embed(stack, in: box)
    }

    private func setupConfigSection(inside box: NSBox) {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        // Read-only info row
        let roRow = NSStackView(views: [
            NSTextField(labelWithString: "账号:"), accountLabel,
            NSTextField(labelWithString: "模型:"), backendLabel, modelLabel,
            NSTextField(labelWithString: "密钥:"), keyStatusLabel
        ])
        roRow.orientation = .horizontal
        roRow.spacing = 8
        stack.addArrangedSubview(roRow)

        // Editable form grid
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: "允许名单 (allowlist):"), allowlistField],
            [NSTextField(labelWithString: "忽略名单 (ignorelist):"), ignorelistField],
            [NSTextField(labelWithString: "附件上限 MB (1~200):"), maxAttachmentField],
            [NSTextField(labelWithString: "每小时回复上限:"), maxRepliesField],
            [NSTextField(labelWithString: "冷却时间 (秒):"), cooldownField],
            [NSTextField(labelWithString: "静默等待 (秒):"), quietSecField],
            [NSTextField(labelWithString: "轮次最长时间 (秒):"), maxSecField],
            [NSTextField(labelWithString: "上下文消息条数:"), contextMsgField],
            [NSTextField(labelWithString: "回复由:"), brainModePopup],
            [NSTextField(labelWithString: "外部大脑超时 (秒):"), brainTimeoutField],
            [NSTextField(labelWithString: "超时后:"), brainTimeoutActionPopup]
        ])
        grid.rowSpacing = 6
        grid.columnSpacing = 10
        stack.addArrangedSubview(grid)
        stack.addArrangedSubview(manageWindowCheck)
        brainModePopup.addItems(withTitles: ["内置大脑", "外部大脑 (MCP next_turn / reply)"])
        brainTimeoutActionPopup.addItems(withTitles: ["这一轮不回复", "改用内置大脑", "转人工"])
        brainTimeoutField.placeholderString = "留空使用默认值 120"

        configErrorLabel.font = NSFont.systemFont(ofSize: 11)
        configErrorLabel.textColor = .systemRed

        configSaveButton.title = "保存配置"
        configSaveButton.target = self
        configSaveButton.action = #selector(handleSaveConfig)

        let footerRow = NSStackView(views: [configSaveButton, configErrorLabel])
        footerRow.orientation = .horizontal
        footerRow.spacing = 10
        stack.addArrangedSubview(footerRow)

        // Empty fields keep the bot's own defaults (scripts/wechat_bot/service.py load_config).
        let defaults: [(NSTextField, String)] = [
            (maxAttachmentField, "50"), (maxRepliesField, "20"), (cooldownField, "15"),
            (quietSecField, "10"), (maxSecField, "60"), (contextMsgField, "5")
        ]
        for (field, value) in defaults { field.placeholderString = "留空使用默认值 \(value)" }
        ignorelistField.placeholderString = "每行一个：名称, private 或 group"
        setConfigFormEnabled(false)

        embed(stack, in: box)
    }

    @objc private func refreshClicked() {
        refresh()
    }

    public func refresh() {
        // Permissions granted after launch: start the desktop service now (start() is idempotent).
        let granted = readDesktopProbe()
        if granted.screen_recording_authorized && granted.accessibility_trusted {
            try? worker.start()
        }
        coreSupervisor.checkAndReconcile()
        let input = collectStatusInput()
        let items = evaluate(input)
        latestItems = items
        latestBotState = input.bot.healthState

        for item in items {
            guard let viewGroup = statusItemViews[item.key] else { continue }
            switch item.level {
            case .green:
                viewGroup.dot.textColor = .systemGreen
                viewGroup.dot.stringValue = "●"
            case .yellow:
                viewGroup.dot.textColor = .systemOrange
                viewGroup.dot.stringValue = "▲"
            case .red:
                viewGroup.dot.textColor = .systemRed
                viewGroup.dot.stringValue = "✖"
            }

            var text = item.reason
            if let fix = item.fix, !fix.isEmpty {
                text += " (建议: \(fix))"
            }
            viewGroup.detail.stringValue = text
        }

        if input.bot.healthState == "paused" {
            pauseResumeButton.title = "恢复机器人"
        } else {
            pauseResumeButton.title = "暂停机器人"
        }
        NotificationCenter.default.post(name: hostStatusDidRefreshNotification, object: self,
                                        userInfo: ["items": items, "input": input])
    }

    private func collectStatusInput() -> StatusInput {
        let probe = readDesktopProbe()

        // 1. Permissions
        // 核心在附件因文件夹权限失败时写这个标记，权限恢复后自动删除
        let folderMarker = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".wechat-rpa/folder-permission-needed.json")
        var folderBlocked: String? = nil
        if let data = try? Data(contentsOf: folderMarker),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            folderBlocked = (obj["detail"] as? String) ?? "unknown"
        }
        let perm = PermissionsInput(
            screenRecordingAuthorized: probe.screen_recording_authorized,
            accessibilityTrusted: probe.accessibility_trusted,
            folderPermissionBlocked: folderBlocked
        )

        // 2. Desktop Service
        var svcStatus = "stopped"
        var svcPid: Int32? = nil
        var stoppedReason: String? = nil
        switch worker.status {
        case .running(let pid):
            svcStatus = "running"
            svcPid = pid
        case .restarting(let at):
            svcStatus = "restarting"
            stoppedReason = "退避重试中 (预计 \(Int(at.timeIntervalSince1970)) 重试)"
        case .stopped(let reason):
            svcStatus = "stopped"
            stoppedReason = reason
        case .idle:
            svcStatus = "idle"
        }
        let sockPath = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".wechat-rpa/worker.sock").path
        let sockExists = FileManager.default.fileExists(atPath: sockPath)
        let desktopSvc = DesktopServiceInput(
            socketExists: sockExists,
            status: svcStatus,
            pid: svcPid,
            stoppedReason: stoppedReason
        )

        // 3. Bot & Health
        let configPath = getConfigURL()
        var stateDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/WeChatRPABot", isDirectory: true)
        if FileManager.default.fileExists(atPath: configPath.path),
           let cfgData = try? Data(contentsOf: configPath),
           let cfgObj = try? JSONSerialization.jsonObject(with: cfgData, options: []) as? [String: Any],
           let customStateDir = cfgObj["state_dir"] as? String {
            stateDir = URL(fileURLWithPath: customStateDir)
        }

        let healthFile = stateDir.appendingPathComponent("health.json")
        var healthState: String? = nil
        var healthUpdated: Double? = nil
        var sessionsInput: [String: SessionHealthInput] = [:]

        if FileManager.default.fileExists(atPath: healthFile.path),
           let hData = try? Data(contentsOf: healthFile),
           let hObj = try? JSONSerialization.jsonObject(with: hData, options: []) as? [String: Any] {
            healthState = hObj["state"] as? String
            healthUpdated = hObj["updated_at"] as? Double
            if let sessObj = hObj["sessions"] as? [String: [String: Any]] {
                for (name, sDict) in sessObj {
                    let state = sDict["state"] as? String ?? "unknown"
                    sessionsInput[name] = SessionHealthInput(
                        state: state,
                        reason: sDict["reason"] as? String,
                        code: sDict["code"] as? String,
                        handoff: state == "handoff"
                    )
                }
            }
        }

        var brainInput: BrainInput? = nil
        if let hData = try? Data(contentsOf: healthFile),
           let hObj = try? JSONSerialization.jsonObject(with: hData, options: []) as? [String: Any],
           let b = hObj["brain"] as? [String: Any] {
            brainInput = BrainInput(mode: b["mode"] as? String ?? "builtin",
                                    pending: b["pending"] as? Int,
                                    lastPollAt: b["last_poll_at"] as? Double,
                                    now: Date().timeIntervalSince1970)
        }
        handoffSessionNames = sessionsInput.filter { $0.value.handoff == true }.map { $0.key }.sorted()
        resumeHandoffButton.isEnabled = !handoffSessionNames.isEmpty

        let coreEnabled = FileManager.default.fileExists(atPath: coreSupervisor.coreEnabledURL.path)
        let pyLoaded = coreSupervisor.pythonLoadedChecker()
        let coreSt = coreSupervisor.currentStatus
        var corePid: Int32? = nil
        var coreReason: String? = nil
        if case .running(let pid) = coreSt {
            corePid = pid
        } else if case .restarting(let at) = coreSt {
            coreReason = "退避重试中 (预计 \(Int(at.timeIntervalSince1970)) 重试)"
        }

        let botInput = BotInput(
            launchdLoaded: pyLoaded,
            healthState: healthState,
            healthUpdatedAt: healthUpdated,
            now: Date().timeIntervalSince1970,
            coreEnabled: coreEnabled,
            coreStatus: coreSt.rawStatusString,
            corePid: corePid,
            coreStoppedReason: coreReason
        )

        // 4. WeChat
        let isRunning = probe.wechat_process_count > 0
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.tencent.xinWeChat")
        let wechatPID = apps.first?.processIdentifier
        var mainRect: RectPayload? = nil
        let windowTuples = probe.wechat_windows.map {
            (bounds: CGRect(x: $0.bounds.x, y: $0.bounds.y, width: $0.bounds.width, height: $0.bounds.height),
             onScreen: $0.on_screen)
        }
        if let chosen = selectMainWindowBounds(windowTuples) {
            mainRect = RectPayload(x: chosen.origin.x, y: chosen.origin.y,
                                   width: chosen.width, height: chosen.height)
        }


        let rawWinList = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        let rawCodables = rawWinList.map { dict -> [String: AnyCodable] in
            var res: [String: AnyCodable] = [:]
            for (k, v) in dict { res[k] = AnyCodable(v) }
            return res
        }

        let wechatInput = WechatInput(
            isRunning: isRunning,
            version: nil,
            pid: wechatPID,
            mainBounds: mainRect,
            rawWindows: rawCodables,
            focusedIdentifier: nil
        )

        // 5. Layout
        let layoutDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".wechat-rpa/layout", isDirectory: true)
        let layoutFiles = (try? FileManager.default.contentsOfDirectory(atPath: layoutDir.path)) ?? []
        let layoutExists = !layoutFiles.filter { $0.hasPrefix("wechat-") && $0.hasSuffix(".json") }.isEmpty
        let layoutInput = LayoutInput(layoutExists: layoutExists)

        // 6. Disk
        var freeBytes: Int64 = 100 * 1024 * 1024 * 1024
        if let attrs = try? FileManager.default.attributesOfFileSystem(forPath: stateDir.path),
           let freeNum = attrs[.systemFreeSize] as? NSNumber {
            freeBytes = freeNum.int64Value
        }
        let diskInput = DiskInput(freeBytes: freeBytes, attachmentBytes: 0)

        return StatusInput(
            permissions: perm,
            desktopService: desktopSvc,
            bot: botInput,
            sessions: sessionsInput,
            wechat: wechatInput,
            layout: layoutInput,
            disk: diskInput,
            brain: brainInput
        )
    }

    private func getConfigURL() -> URL {
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".WeChatRPABot/config.json")
    }

    private func getStateDir() -> URL {
        if let custom = rawConfigDict["state_dir"] as? String {
            return URL(fileURLWithPath: custom)
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/WeChatRPABot", isDirectory: true)
    }

    private func loadInitialConfig() {
        let url = getConfigURL()
        currentConfigURL = url
        guard FileManager.default.fileExists(atPath: url.path),
              let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data, options: []) as? [String: Any] else {
            setConfigFormEnabled(false)
            configErrorLabel.stringValue = "未找到机器人配置文件（\(url.path)），无法保存"
            return
        }
        self.rawConfigDict = json
        populateConfigUI(json)
        setConfigFormEnabled(true)
        configErrorLabel.stringValue = ""
    }

    private func setConfigFormEnabled(_ enabled: Bool) {
        for field in [allowlistField, ignorelistField, maxAttachmentField, maxRepliesField,
                      cooldownField, quietSecField, maxSecField, contextMsgField] {
            field.isEnabled = enabled
        }
        manageWindowCheck.isEnabled = enabled
        brainModePopup.isEnabled = enabled
        brainTimeoutField.isEnabled = enabled
        brainTimeoutActionPopup.isEnabled = enabled
        configSaveButton.isEnabled = enabled
    }

    private func populateConfigUI(_ raw: [String: Any]) {
        let masked = ConfigFormLogic.maskConfig(raw)
        accountLabel.stringValue = masked["account_id"] as? String ?? "-"
        backendLabel.stringValue = masked["model_backend"] as? String ?? "-"
        modelLabel.stringValue = masked["model_name"] as? String ?? "-"
        keyStatusLabel.stringValue = masked["api_key_status"] as? String ?? "未配置"

        let editable = masked["editable"] as? [String: Any] ?? [:]
        if let allow = editable["allowlist"] as? [[String: Any]] {
            allowlistField.stringValue = ConfigFormLogic.formatContactText(allow)
        } else if let allowArr = editable["allowlist"] {
            allowlistField.stringValue = formatListOrString(allowArr)
        }

        if let ignore = editable["ignorelist"] as? [[String: Any]] {
            ignorelistField.stringValue = ConfigFormLogic.formatContactText(ignore)
        } else if let ignoreArr = editable["ignorelist"] {
            ignorelistField.stringValue = formatListOrString(ignoreArr)
        }

        if let att = editable["max_attachment_mb"] {
            maxAttachmentField.stringValue = "\(att)"
        }
        if let rep = editable["max_replies_per_hour"] {
            maxRepliesField.stringValue = "\(rep)"
        }
        if let cd = editable["cooldown_seconds"] {
            cooldownField.stringValue = "\(cd)"
        }
        if let tq = editable["turn_quiet_seconds"] {
            quietSecField.stringValue = "\(tq)"
        }
        if let tm = editable["turn_max_seconds"] {
            maxSecField.stringValue = "\(tm)"
        }
        if let cm = editable["context_messages"] {
            contextMsgField.stringValue = "\(cm)"
        }
        // The bot defaults manage_window to true when the key is absent.
        let mw = editable["manage_window"] as? Bool ?? true
        manageWindowCheck.state = mw ? .on : .off
        let brain = editable["brain"] as? [String: Any] ?? [:]
        brainModePopup.selectItem(at: (brain["mode"] as? String) == "external" ? 1 : 0)
        let ext = brain["external"] as? [String: Any] ?? [:]
        brainTimeoutField.stringValue = (ext["timeout_seconds"]).map { "\($0)" } ?? ""
        brainTimeoutActionPopup.selectItem(at: ["skip", "builtin", "handoff"].firstIndex(of: ext["timeout_action"] as? String ?? "skip") ?? 0)
    }

    private func formatListOrString(_ val: Any) -> String {
        if let arr = val as? [Any] {
            if let dicts = arr as? [[String: Any]] {
                let recips = dicts.compactMap { $0["recipient"] as? String }
                return recips.joined(separator: ", ")
            }
            let strs = arr.map { "\($0)" }
            return strs.joined(separator: ", ")
        }
        return "\(val)"
    }

    // --- Action Handlers ---

    private func confirmIfSending(actionName: String) -> Bool {
        if latestBotState == "sending" {
            let alert = NSAlert()
            alert.messageText = "机器人正在发送回复"
            alert.informativeText = "机器人当前正在处理并发送消息，执行「\(actionName)」可能会打断发送。确定要继续吗？"
            alert.addButton(withTitle: "继续\(actionName)")
            alert.addButton(withTitle: "取消")
            return alert.runModal() == .alertFirstButtonReturn
        }
        return true
    }

    @objc private func handleRestartDesktopService() {
        actionMessageLabel.stringValue = "正在重启桌面服务..."
        let ok = HostActions.restartDesktopService(worker: worker, healthState: latestBotState) { [weak self] in
            return self?.confirmIfSending(actionName: "重启桌面服务") ?? false
        }
        if ok {
            actionMessageLabel.stringValue = "桌面服务已触发重启"
        } else {
            actionMessageLabel.stringValue = "操作已取消"
        }
        refresh()
    }

    /// The Rust core is supervised in-process: restart it the same way as 「重启机器人」 (a planned
    /// stop, not counted as a crash). The legacy launchd label `local.wechat-rpa.bot` no longer
    /// exists, so kickstarting it left saved settings without effect.
    fileprivate func restartCoreForConfig() throws {
        guard FileManager.default.fileExists(atPath: coreSupervisor.coreEnabledURL.path) else {
            try ConfigFormLogic.defaultReloadBot()
            return
        }
        let supervisor = coreSupervisor
        DispatchQueue.global(qos: .userInitiated).async {
            supervisor.reset()
            supervisor.stopProcess()
            supervisor.checkAndReconcile()
        }
    }

    @objc private func handleRestartBot() {
        actionMessageLabel.stringValue = "正在重启机器人..."
        if FileManager.default.fileExists(atPath: coreSupervisor.coreEnabledURL.path) {
            coreSupervisor.reset()
            coreSupervisor.stopProcess()
            coreSupervisor.checkAndReconcile()
            actionMessageLabel.stringValue = "已触发 Rust 核心重启"
            refresh()
            return
        }
        do {
            let ok = try HostActions.restartBot(healthState: latestBotState, confirmHandler: { [weak self] in
                return self?.confirmIfSending(actionName: "重启机器人") ?? false
            })
            if ok {
                actionMessageLabel.stringValue = "已触发机器人重启"
            } else {
                actionMessageLabel.stringValue = "操作已取消"
            }
        } catch {
            actionMessageLabel.stringValue = "重启机器人失败: \(error.localizedDescription)"
        }
        refresh()
    }

    @objc private func handleTogglePause() {
        let stateDir = getStateDir()
        do {
            if latestBotState == "paused" {
                try HostActions.resumeBot(stateDir: stateDir)
                actionMessageLabel.stringValue = "机器人已恢复运行"
            } else {
                try HostActions.pauseBot(stateDir: stateDir)
                actionMessageLabel.stringValue = "机器人已暂停"
            }
        } catch {
            actionMessageLabel.stringValue = "操作失败: \(error.localizedDescription)"
        }
        refresh()
    }

    /// Hands the listed sessions back to automatic replies through `wechat-mcp control resume-session`
    /// (the running core is told over core.sock; without a core the handoff marker file is removed).
    @objc private func handleResumeHandoff() {
        let names = handoffSessionNames
        guard !names.isEmpty else { return }
        let bin = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/bin/wechat-mcp")
        let config = getConfigURL().path
        actionMessageLabel.stringValue = "正在恢复自动回复：\(names.joined(separator: "、"))"
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var failed: [String] = []
            for name in names {
                let process = Process()
                process.executableURL = bin
                process.arguments = ["control", "resume-session", name, "--config", config]
                var env = ProcessInfo.processInfo.environment
                env["LANG"] = "en_US.UTF-8"; env["LC_ALL"] = "en_US.UTF-8"
                process.environment = env
                do { try process.run(); process.waitUntilExit() } catch { failed.append(name); continue }
                if process.terminationStatus != 0 { failed.append(name) }
            }
            DispatchQueue.main.async {
                self?.actionMessageLabel.stringValue = failed.isEmpty ? "已恢复自动回复" : "恢复失败：\(failed.joined(separator: "、"))"
                self?.refresh()
            }
        }
    }

    @objc private func handleRestoreWechatWindow() {
        do {
            try HostActions.restoreWechatWindow(wechatPID: nil)
            actionMessageLabel.stringValue = "微信窗口已放回标准位置 (880x640)"
        } catch {
            actionMessageLabel.stringValue = error.localizedDescription
        }
        refresh()
    }

    @objc private func handleOpenAttachments() {
        let stateDir = getStateDir()
        let attDir = stateDir.appendingPathComponent("attachments", isDirectory: true)
        HostActions.openFolder(attDir)
    }

    @objc private func handleOpenLogs() {
        let logsDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".wechat-rpa/logs", isDirectory: true)
        HostActions.openFolder(logsDir)
    }

    @objc private func handleCopyDiagnostics() {
        let text = generateDiagnostics(items: latestItems, appVersion: hostAppVersion())
        HostActions.copyDiagnostics(text)
        actionMessageLabel.stringValue = "诊断信息已复制到剪贴板"
    }

    @objc private func handleSaveConfig() {
        configErrorLabel.stringValue = ""
        actionMessageLabel.stringValue = "正在保存并校验配置..."

        guard let targetURL = currentConfigURL else {
            configErrorLabel.stringValue = "配置路径无效"
            return
        }

        var formInputs: [String: Any] = [:]
        formInputs["allowlist"] = allowlistField.stringValue
        formInputs["ignorelist"] = ignorelistField.stringValue
        formInputs["max_attachment_mb"] = maxAttachmentField.stringValue
        formInputs["max_replies_per_hour"] = maxRepliesField.stringValue
        formInputs["cooldown_seconds"] = cooldownField.stringValue
        formInputs["turn_quiet_seconds"] = quietSecField.stringValue
        formInputs["turn_max_seconds"] = maxSecField.stringValue
        formInputs["context_messages"] = contextMsgField.stringValue
        formInputs["manage_window"] = (manageWindowCheck.state == .on)
        formInputs["brain_mode"] = brainModePopup.indexOfSelectedItem == 1 ? "external" : "builtin"
        if brainModePopup.indexOfSelectedItem == 1 {
            formInputs["brain_timeout_seconds"] = brainTimeoutField.stringValue
            formInputs["brain_timeout_action"] = ["skip", "builtin", "handoff"][max(0, brainTimeoutActionPopup.indexOfSelectedItem)]
        }

        let newConfig: [String: Any]
        do {
            newConfig = try ConfigFormLogic.buildUpdatedConfig(original: rawConfigDict, formInputs: formInputs)
        } catch {
            configErrorLabel.stringValue = "表单格式错误: \(error.localizedDescription)"
            actionMessageLabel.stringValue = "配置保存被拒绝"
            return
        }

        do {
            let data = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])
            try ConfigFormLogic.saveConfig(
                targetPath: targetURL,
                newConfigData: data,
                checker: ConfigFormLogic.defaultCheckConfig,
                reloader: { [weak self] in try self?.restartCoreForConfig() }
            )
            rawConfigDict = newConfig
            actionMessageLabel.stringValue = "配置保存成功，机器人已重启生效"
            configErrorLabel.stringValue = ""
            refresh()
        } catch {
            configErrorLabel.stringValue = "保存失败: \(error.localizedDescription)"
            actionMessageLabel.stringValue = "配置保存被拒绝"
        }
    }

    public func collectLatestStatus() -> ([StatusItem], StatusInput) {
        let input = collectStatusInput()
        let items = evaluate(input)
        latestItems = items
        latestBotState = input.bot.healthState
        return (items, input)
    }

    public func pauseResumeBot() {
        handleTogglePause()
    }

    public func restartDesktopService() {
        handleRestartDesktopService()
    }

    public func restartBot() {
        handleRestartBot()
    }

    public func repositionWechat() {
        handleRestoreWechatWindow()
    }

    public func resumeHandoff(sessionName: String? = nil) {
        if let name = sessionName {
            handoffSessionNames = [name]
        }
        handleResumeHandoff()
    }

    public func openAttachments() {
        handleOpenAttachments()
    }

    public func openLogs() {
        handleOpenLogs()
    }

    public func copyDiagnostics() {
        handleCopyDiagnostics()
    }

    /// Saves the config; `restartCore` restarts serve-core so settings read only at start (brain,
    /// open_api, pacing) take effect. Allowlist edits are re-read live and need no restart.
    public func persistConfig(_ newConfig: [String: Any], restartCore: Bool = true) throws {
        guard let targetURL = currentConfigURL else {
            throw ConfigFormError.writeFailed("配置路径无效")
        }
        let data = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])
        try ConfigFormLogic.saveConfig(
            targetPath: targetURL,
            newConfigData: data,
            checker: ConfigFormLogic.defaultCheckConfig,
            reloader: restartCore ? { [weak self] in try self?.restartCoreForConfig() } : nil
        )
        rawConfigDict = newConfig
        refresh()
    }
}

/// A document view whose origin is the top-left corner, so scrolled content starts at the top.
public class FlippedView: NSView {
    public override var isFlipped: Bool { true }
}
