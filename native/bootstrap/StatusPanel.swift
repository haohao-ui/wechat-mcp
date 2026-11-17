// StatusPanel.swift: AppKit status panel, action triggers, and config form interface.
import Foundation
import AppKit

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
    private let manageWindowCheck = NSButton(checkboxWithTitle: L10n.t(.manageWindow), target: nil, action: nil)
    private let brainModePopup = NSPopUpButton()
    private let brainTimeoutField = NSTextField()
    private let brainTimeoutActionPopup = NSPopUpButton()
    private let resumeHandoffButton = NSButton(title: L10n.t(.actionResumeAutoReply), target: nil, action: nil)
    /// Sessions the core marked as handed to a person (health.json sessions[*].state == "handoff").
    private var handoffSessionNames: [String] = []

    private let configErrorLabel = NSTextField(wrappingLabelWithString: "")
    private let configSaveButton = NSButton()

    private var currentConfigURL: URL?
    private var rawConfigDict: [String: Any] = [:]
    private var latestItems: [StatusItem] = []
    private var latestBotState: String?

    public var isBotRunning: Bool {
        if case .running = coreSupervisor.currentStatus { return true }
        return coreSupervisor.pythonLoadedChecker()
    }
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
        let headerBox = createSectionBox(title: L10n.t(.appName) + " — " + currentVersion)
        setupStatusSection(inside: headerBox)
        mainStack.addArrangedSubview(headerBox)

        // 2. One-click Actions
        let actionBox = createSectionBox(title: L10n.t(.legacyActions))
        setupActionSection(inside: actionBox)
        mainStack.addArrangedSubview(actionBox)

        // 3. Configuration Form
        let configBox = createSectionBox(title: L10n.t(.legacyConfig))
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
            "permissions": L10n.t(.itemPermissions),
            "desktop_service": L10n.t(.itemDesktopService),
            "bot": L10n.t(.itemBot),
            "sessions": L10n.t(.itemSessions),
            "wechat": L10n.t(.itemWechat),
            "layout": L10n.t(.itemLayout),
            "disk": L10n.t(.itemDisk),
            "brain": L10n.t(.itemReplyMode)
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

            let dLabel = NSTextField(wrappingLabelWithString: L10n.t(.loading))
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
            NSButton(title: L10n.t(.actionRecheck), target: self, action: #selector(refreshClicked)),
            NSButton(title: L10n.t(.permScreen), target: self, action: #selector(requestScreen)),
            NSButton(title: L10n.t(.permAccessibility), target: self, action: #selector(requestAccessibility))
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
            NSButton(title: L10n.t(.actionRestartDesktopService), target: self, action: #selector(handleRestartDesktopService)),
            NSButton(title: L10n.t(.actionRestartBot), target: self, action: #selector(handleRestartBot)),
            pauseResumeButton,
            NSButton(title: L10n.t(.actionRepositionWechat), target: self, action: #selector(handleRestoreWechatWindow))
        ])
        row1.orientation = .horizontal
        row1.spacing = 8

        pauseResumeButton.title = L10n.t(.actionPauseAutoReply)
        pauseResumeButton.target = self
        pauseResumeButton.action = #selector(handleTogglePause)

        let row2 = NSStackView(views: [
            NSButton(title: L10n.t(.openAttachments), target: self, action: #selector(handleOpenAttachments)),
            NSButton(title: L10n.t(.openLogs), target: self, action: #selector(handleOpenLogs)),
            NSButton(title: L10n.t(.copyDiagnostics), target: self, action: #selector(handleCopyDiagnostics)),
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
            NSTextField(labelWithString: L10n.t(.labelAccount)), accountLabel,
            NSTextField(labelWithString: L10n.t(.labelModelName)), backendLabel, modelLabel,
            NSTextField(labelWithString: L10n.t(.labelApiKey)), keyStatusLabel
        ])
        roRow.orientation = .horizontal
        roRow.spacing = 8
        stack.addArrangedSubview(roRow)

        // Editable form grid
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.allowlist)), allowlistField],
            [NSTextField(labelWithString: L10n.t(.ignorelist)), ignorelistField],
            [NSTextField(labelWithString: L10n.t(.fieldAttachment) + " (1–200 MB)"), maxAttachmentField],
            [NSTextField(labelWithString: L10n.t(.fieldMaxReplies)), maxRepliesField],
            [NSTextField(labelWithString: L10n.t(.fieldCooldown)), cooldownField],
            [NSTextField(labelWithString: L10n.t(.fieldQuiet)), quietSecField],
            [NSTextField(labelWithString: L10n.t(.fieldTurnMax)), maxSecField],
            [NSTextField(labelWithString: L10n.t(.fieldContext)), contextMsgField],
            [NSTextField(labelWithString: L10n.t(.itemReplyMode)), brainModePopup],
            [NSTextField(labelWithString: L10n.t(.externalTimeoutLabel, L10n.t(.replyModeExternal))), brainTimeoutField],
            [NSTextField(labelWithString: L10n.t(.timeoutAction)), brainTimeoutActionPopup]
        ])
        grid.rowSpacing = 6
        grid.columnSpacing = 10
        stack.addArrangedSubview(grid)
        stack.addArrangedSubview(manageWindowCheck)
        brainModePopup.addItems(withTitles: [L10n.t(.replyModeBuiltin), L10n.t(.replyModeExternal)])
        brainTimeoutActionPopup.addItems(withTitles: [L10n.t(.skipTurn), L10n.t(.fallbackToBuiltin, L10n.t(.replyModeBuiltin)), L10n.t(.actionHandoff)])
        brainTimeoutField.placeholderString = L10n.t(.defaultPlaceholder, "120")

        configErrorLabel.font = NSFont.systemFont(ofSize: 11)
        configErrorLabel.textColor = .systemRed

        configSaveButton.title = L10n.t(.actionSaveConfig)
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
        for (field, value) in defaults { field.placeholderString = L10n.t(.defaultPlaceholder, String(value)) }
        ignorelistField.placeholderString = L10n.t(.ignorePlaceholder)
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
        publishStatus(input, items: items)

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
                text += L10n.t(.suggestionSuffix, fix)
            }
            viewGroup.detail.stringValue = text
        }

        if input.bot.healthState == "paused" {
            pauseResumeButton.title = L10n.t(.actionResumeAutoReply)
        } else {
            pauseResumeButton.title = L10n.t(.actionPauseAutoReply)
        }
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
            stoppedReason = L10n.t(.evBackoffEta, Int(at.timeIntervalSince1970))
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
            coreReason = L10n.t(.evBackoffEta, Int(at.timeIntervalSince1970))
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
            brain: brainInput,
            modelState: ConfigFormLogic.maskConfig(rawConfigDict)["model_state"] as? String
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
            configErrorLabel.stringValue = L10n.t(.configUnavailable)
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
        keyStatusLabel.stringValue = masked["api_key_status"] as? String ?? L10n.t(.modelUnconfigured)

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
            alert.messageText = L10n.t(.confirmBusy)
            alert.informativeText = L10n.t(.confirmBusyBody, actionName)
            alert.addButton(withTitle: L10n.t(.continueAction, actionName))
            alert.addButton(withTitle: L10n.t(.cancel))
            return alert.runModal() == .alertFirstButtonReturn
        }
        return true
    }

    @objc private func handleRestartDesktopService() {
        actionMessageLabel.stringValue = L10n.t(.loading)
        let ok = HostActions.restartDesktopService(worker: worker, healthState: latestBotState) { [weak self] in
            return self?.confirmIfSending(actionName: L10n.t(.actionRestartDesktopService)) ?? false
        }
        if ok {
            actionMessageLabel.stringValue = L10n.t(.actionRequested)
        } else {
            actionMessageLabel.stringValue = L10n.t(.actionCancelled)
        }
        refresh()
    }

    @objc private func handleRestartBot() {
        actionMessageLabel.stringValue = L10n.t(.loading)
        if FileManager.default.fileExists(atPath: coreSupervisor.coreEnabledURL.path) {
            coreSupervisor.reset()
            coreSupervisor.stopProcess()
            coreSupervisor.checkAndReconcile()
            actionMessageLabel.stringValue = L10n.t(.actionRequested)
            refresh()
            return
        }
        do {
            let ok = try HostActions.restartBot(healthState: latestBotState, confirmHandler: { [weak self] in
                return self?.confirmIfSending(actionName: L10n.t(.actionRestartBot)) ?? false
            })
            if ok {
                actionMessageLabel.stringValue = L10n.t(.actionRequested)
            } else {
                actionMessageLabel.stringValue = L10n.t(.actionCancelled)
            }
        } catch {
            actionMessageLabel.stringValue = L10n.t(.actionFailed)
        }
        refresh()
    }

    @objc private func handleTogglePause() {
        let stateDir = getStateDir()
        do {
            if latestBotState == "paused" {
                try HostActions.resumeBot(stateDir: stateDir)
                actionMessageLabel.stringValue = L10n.t(.actionRequested)
            } else {
                try HostActions.pauseBot(stateDir: stateDir)
                actionMessageLabel.stringValue = L10n.t(.actionRequested)
            }
        } catch {
            actionMessageLabel.stringValue = L10n.t(.actionFailed)
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
        actionMessageLabel.stringValue = L10n.t(.loading)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var failed: [String] = []
            for name in names {
                let result = runControlCommand(executable: bin.path, arguments: ["control", "resume-session", name, "--config", config], timeout: 15)
                if result.status != 0 || result.timedOut { failed.append(name) }
            }
            DispatchQueue.main.async {
                self?.actionMessageLabel.stringValue = failed.isEmpty ? L10n.t(.actionRequested) : L10n.t(.actionFailed)
                self?.refresh()
            }
        }
    }

    @objc private func handleRestoreWechatWindow() {
        do {
            try HostActions.restoreWechatWindow(wechatPID: nil)
            actionMessageLabel.stringValue = L10n.t(.actionRequested)
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
        actionMessageLabel.stringValue = L10n.t(.copiedDiagnostics)
    }

    @objc private func handleSaveConfig() {
        configErrorLabel.stringValue = ""
        actionMessageLabel.stringValue = L10n.t(.savingConfig)

        guard let targetURL = currentConfigURL else {
            configErrorLabel.stringValue = L10n.t(.configUnavailable)
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
            configErrorLabel.stringValue = L10n.t(.validationError, L10n.t(.valueUnavailable))
            actionMessageLabel.stringValue = L10n.t(.saveRejected)
            return
        }

        do {
            let data = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])
            try ConfigFormLogic.saveConfig(
                targetPath: targetURL,
                newConfigData: data,
                checker: ConfigFormLogic.defaultCheckConfig,
                reloader: nil
            )
            rawConfigDict = newConfig
            actionMessageLabel.stringValue = L10n.t(.savedNeedsRestart)
            configErrorLabel.stringValue = ""
            refresh()
        } catch {
            configErrorLabel.stringValue = L10n.t(.saveFailed, L10n.t(.valueUnavailable))
            actionMessageLabel.stringValue = L10n.t(.saveRejected)
        }
    }

    public static let statusDidChange = Notification.Name("HostStatusDidChange")
    public private(set) var statusSummary: StatusSummary?
    public private(set) var statusInput: StatusInput?

    private func publishStatus(_ input: StatusInput, items: [StatusItem]) {
        statusInput = input
        statusSummary = summarizeStatus(input, items: items)
        NotificationCenter.default.post(name: Self.statusDidChange, object: self)
    }

    public func collectLatestStatus() -> ([StatusItem], StatusInput) {
        let input = collectStatusInput()
        let items = evaluate(input)
        latestItems = items
        latestBotState = input.bot.healthState
        publishStatus(input, items: items)
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

    public func persistConfig(_ newConfig: [String: Any]) throws {
        guard let targetURL = currentConfigURL else {
            throw ConfigFormError.writeFailed(L10n.t(.configUnavailable))
        }
        let data = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])
        try ConfigFormLogic.saveConfig(
            targetPath: targetURL,
            newConfigData: data,
            checker: ConfigFormLogic.defaultCheckConfig,
            reloader: nil
        )
        rawConfigDict = newConfig
        refresh()
    }
}

/// A document view whose origin is the top-left corner, so scrolled content starts at the top.
public class FlippedView: NSView {
    public override var isFlipped: Bool { true }
}
