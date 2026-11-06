// StatusPanel.swift: AppKit status panel, action triggers, and config form interface.
import Foundation
import AppKit

public final class StatusPanelController: NSViewController {
    private let worker: SocketWorker
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

    private let configErrorLabel = NSTextField(wrappingLabelWithString: "")
    private let configSaveButton = NSButton()

    private var currentConfigURL: URL?
    private var rawConfigDict: [String: Any] = [:]
    private var latestItems: [StatusItem] = []
    private var latestBotState: String?

    init(worker: SocketWorker) {
        self.worker = worker
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
        let headerBox = createSectionBox(title: "微信客服助手 — 系统状态")
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

        let clipView = NSClipView()
        clipView.documentView = mainStack
        scrollView.contentView = clipView
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            mainStack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            mainStack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            mainStack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
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

    private func setupStatusSection(inside box: NSBox) {
        statusStack.orientation = .vertical
        statusStack.alignment = .leading
        statusStack.spacing = 8
        statusStack.translatesAutoresizingMaskIntoConstraints = false

        let keys = ["permissions", "desktop_service", "bot", "sessions", "wechat", "layout", "disk"]
        let titles = [
            "permissions": "权限状态",
            "desktop_service": "桌面服务",
            "bot": "机器人核心",
            "sessions": "各会话状态",
            "wechat": "微信客户端",
            "layout": "布局地图",
            "disk": "磁盘空间"
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

            row.addArrangedSubview(dot)
            row.addArrangedSubview(tLabel)
            row.addArrangedSubview(dLabel)
            statusStack.addArrangedSubview(row)

            statusItemViews[k] = (dot: dot, title: tLabel, detail: dLabel)
        }

        let refreshBtn = NSButton(title: "立即刷新", target: self, action: #selector(refreshClicked))
        statusStack.addArrangedSubview(refreshBtn)

        box.contentView = statusStack
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
            NSButton(title: "复制诊断信息", target: self, action: #selector(handleCopyDiagnostics))
        ])
        row2.orientation = .horizontal
        row2.spacing = 8

        actionMessageLabel.font = NSFont.systemFont(ofSize: 11)
        actionMessageLabel.textColor = .systemBlue

        stack.addArrangedSubview(row1)
        stack.addArrangedSubview(row2)
        stack.addArrangedSubview(actionMessageLabel)

        box.contentView = stack
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
            [NSTextField(labelWithString: "上下文消息条数:"), contextMsgField]
        ])
        grid.rowSpacing = 6
        grid.columnSpacing = 10
        stack.addArrangedSubview(grid)
        stack.addArrangedSubview(manageWindowCheck)

        configErrorLabel.font = NSFont.systemFont(ofSize: 11)
        configErrorLabel.textColor = .systemRed

        configSaveButton.title = "保存配置"
        configSaveButton.target = self
        configSaveButton.action = #selector(handleSaveConfig)

        let footerRow = NSStackView(views: [configSaveButton, configErrorLabel])
        footerRow.orientation = .horizontal
        footerRow.spacing = 10
        stack.addArrangedSubview(footerRow)

        box.contentView = stack
    }

    @objc private func refreshClicked() {
        refresh()
    }

    public func refresh() {
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
    }

    private func collectStatusInput() -> StatusInput {
        let probe = readDesktopProbe()

        // 1. Permissions
        let perm = PermissionsInput(
            screenRecordingAuthorized: probe.screen_recording_authorized,
            accessibilityTrusted: probe.accessibility_trusted
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
                    sessionsInput[name] = SessionHealthInput(
                        state: sDict["state"] as? String ?? "unknown",
                        reason: sDict["reason"] as? String,
                        code: sDict["code"] as? String
                    )
                }
            }
        }

        let botInput = BotInput(
            launchdLoaded: true, // 在本机默认已加载/运行
            healthState: healthState,
            healthUpdatedAt: healthUpdated,
            now: Date().timeIntervalSince1970
        )

        // 4. WeChat
        let isRunning = probe.wechat_process_count > 0
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.tencent.xinWeChat")
        let wechatPID = apps.first?.processIdentifier
        var mainRect: RectPayload? = nil
        if let firstWin = probe.wechat_windows.first {
            mainRect = RectPayload(x: firstWin.bounds.x, y: firstWin.bounds.y,
                                   width: firstWin.bounds.width, height: firstWin.bounds.height)
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
            disk: diskInput
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
            return
        }
        self.rawConfigDict = json
        populateConfigUI(json)
    }

    private func populateConfigUI(_ raw: [String: Any]) {
        let masked = ConfigFormLogic.maskConfig(raw)
        accountLabel.stringValue = masked["account_id"] as? String ?? "-"
        backendLabel.stringValue = masked["model_backend"] as? String ?? "-"
        modelLabel.stringValue = masked["model_name"] as? String ?? "-"
        keyStatusLabel.stringValue = masked["api_key_status"] as? String ?? "未配置"

        let editable = masked["editable"] as? [String: Any] ?? [:]
        if let allow = editable["allowlist"] {
            allowlistField.stringValue = formatListOrString(allow)
        }
        if let ignore = editable["ignorelist"] {
            ignorelistField.stringValue = formatListOrString(ignore)
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
        if let mw = editable["manage_window"] as? Bool {
            manageWindowCheck.state = mw ? .on : .off
        }
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

    @objc private func handleRestartBot() {
        actionMessageLabel.stringValue = "正在重启机器人..."
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

    @objc private func handleRestoreWechatWindow() {
        do {
            try HostActions.restoreWechatWindow(wechatPID: nil)
            actionMessageLabel.stringValue = "微信窗口已放回标准位置 (880x640)"
        } catch {
            actionMessageLabel.stringValue = "放回标准位置失败: \(error.localizedDescription)"
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
        let text = generateDiagnostics(items: latestItems, appVersion: "1.0.0")
        HostActions.copyDiagnostics(text)
        actionMessageLabel.stringValue = "诊断信息已复制到剪贴板"
    }

    @objc private func handleSaveConfig() {
        configErrorLabel.stringValue = ""
        actionMessageLabel.stringValue = "正在保存并校验配置..."

        var newConfig = rawConfigDict
        if let maxAtt = Int(maxAttachmentField.stringValue) {
            newConfig["max_attachment_mb"] = maxAtt
        }
        if let maxRep = Int(maxRepliesField.stringValue) {
            newConfig["max_replies_per_hour"] = maxRep
        }
        if let cd = Double(cooldownField.stringValue) {
            newConfig["cooldown_seconds"] = cd
        }
        if let tq = Double(quietSecField.stringValue) {
            newConfig["turn_quiet_seconds"] = tq
        }
        if let tm = Double(maxSecField.stringValue) {
            newConfig["turn_max_seconds"] = tm
        }
        if let cm = Int(contextMsgField.stringValue) {
            newConfig["context_messages"] = cm
        }
        newConfig["manage_window"] = (manageWindowCheck.state == .on)

        // allowlist / ignorelist 逗号解析
        let allowStr = allowlistField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if !allowStr.isEmpty {
            let items = allowStr.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            newConfig["allowlist"] = items.map { ["recipient": $0, "kind": "private"] }
        }

        guard let targetURL = currentConfigURL else {
            configErrorLabel.stringValue = "配置路径无效"
            return
        }

        do {
            let data = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])
            try ConfigFormLogic.saveConfig(
                targetPath: targetURL,
                newConfigData: data,
                checker: ConfigFormLogic.defaultCheckConfig,
                reloader: ConfigFormLogic.defaultReloadBot
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
}
