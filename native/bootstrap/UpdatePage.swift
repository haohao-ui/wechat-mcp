// native/bootstrap/UpdatePage.swift
// Application update management, version verification, drain coordination, and failure recovery.
// STRICT READ-ONLY PHASE: All real downloads, installs, deferrals, and rollbacks are disabled.
import Foundation
import AppKit

// MARK: - Update Data Models

public struct UpdateReleaseInfo: Codable, Equatable {
    public let releaseId: String
    public let sequence: UInt64?
    public let architecture: String
    public let componentsConsistent: Bool?

    enum CodingKeys: String, CodingKey {
        case releaseId = "release_id"
        case sequence
        case architecture
        case componentsConsistent = "components_consistent"
    }

    public init(releaseId: String, sequence: UInt64?, architecture: String, componentsConsistent: Bool?) {
        self.releaseId = releaseId
        self.sequence = sequence
        self.architecture = architecture
        self.componentsConsistent = componentsConsistent
    }
}

public struct UpdateCandidateSummary: Codable, Equatable {
    public let releaseId: String
    public let sequence: UInt64

    enum CodingKeys: String, CodingKey {
        case releaseId = "release_id"
        case sequence
    }

    public init(releaseId: String, sequence: UInt64) {
        self.releaseId = releaseId
        self.sequence = sequence
    }
}

public enum UpdateTransactionStatus: String, Codable, Equatable {
    case idle
    case inProgress = "in_progress"
    case committed
    case rolledBack = "rolled_back"
    case recoveryRequired = "recovery_required"
}

public struct UpdateTransactionInfo: Codable, Equatable {
    public let status: UpdateTransactionStatus
    public let transactionId: String?
    public let stage: String?
    public let error: String?
    public let recoveryRequired: Bool

    enum CodingKeys: String, CodingKey {
        case status
        case transactionId = "transaction_id"
        case stage
        case error
        case recoveryRequired = "recovery_required"
    }

    public init(
        status: UpdateTransactionStatus,
        transactionId: String? = nil,
        stage: String? = nil,
        error: String? = nil,
        recoveryRequired: Bool = false
    ) {
        self.status = status
        self.transactionId = transactionId
        self.stage = stage
        self.error = error
        self.recoveryRequired = recoveryRequired
    }
}

public struct UpdateGuardInfo: Codable, Equatable {
    public let highestAcceptedSequence: UInt64?
    public let maintenanceActive: Bool

    enum CodingKeys: String, CodingKey {
        case highestAcceptedSequence = "highest_accepted_sequence"
        case maintenanceActive = "maintenance_active"
    }

    public init(highestAcceptedSequence: UInt64?, maintenanceActive: Bool) {
        self.highestAcceptedSequence = highestAcceptedSequence
        self.maintenanceActive = maintenanceActive
    }
}

public struct UpdateStatusPayload: Codable, Equatable {
    public let currentRelease: UpdateReleaseInfo
    public let candidateRelease: UpdateCandidateSummary?
    public let transaction: UpdateTransactionInfo
    public let guardInfo: UpdateGuardInfo

    enum CodingKeys: String, CodingKey {
        case currentRelease = "current_release"
        case candidateRelease = "candidate_release"
        case transaction
        case guardInfo = "guard"
    }

    public init(
        currentRelease: UpdateReleaseInfo,
        candidateRelease: UpdateCandidateSummary? = nil,
        transaction: UpdateTransactionInfo,
        guardInfo: UpdateGuardInfo
    ) {
        self.currentRelease = currentRelease
        self.candidateRelease = candidateRelease
        self.transaction = transaction
        self.guardInfo = guardInfo
    }
}

public enum UpdateCheckStatus: String, Codable, Equatable {
    case unconfigured
    case unverifiedTrustRoot = "unverified_trust_root"
    case alreadyInstalled = "already_installed"
    case incompatible
    case available
}

public struct UpdateCandidateDetail: Codable, Equatable {
    public let releaseId: String
    public let sequence: UInt64
    public let publishedAt: String?
    public let sizeBytes: UInt64?
    public let releaseNotes: String?
    public let compatible: Bool
    public let requiresLocalSigning: Bool

    enum CodingKeys: String, CodingKey {
        case releaseId = "release_id"
        case sequence
        case publishedAt = "published_at"
        case sizeBytes = "size_bytes"
        case releaseNotes = "release_notes"
        case compatible
        case requiresLocalSigning = "requires_local_signing"
    }

    public init(
        releaseId: String,
        sequence: UInt64,
        publishedAt: String? = nil,
        sizeBytes: UInt64? = nil,
        releaseNotes: String? = nil,
        compatible: Bool = true,
        requiresLocalSigning: Bool = true
    ) {
        self.releaseId = releaseId
        self.sequence = sequence
        self.publishedAt = publishedAt
        self.sizeBytes = sizeBytes
        self.releaseNotes = releaseNotes
        self.compatible = compatible
        self.requiresLocalSigning = requiresLocalSigning
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.releaseId = try container.decode(String.self, forKey: .releaseId)
        self.sequence = try container.decode(UInt64.self, forKey: .sequence)
        self.publishedAt = try container.decodeIfPresent(String.self, forKey: .publishedAt)
        self.sizeBytes = try container.decodeIfPresent(UInt64.self, forKey: .sizeBytes)
        self.releaseNotes = try container.decodeIfPresent(String.self, forKey: .releaseNotes)
        self.compatible = (try container.decodeIfPresent(Bool.self, forKey: .compatible)) ?? true
        self.requiresLocalSigning = (try container.decodeIfPresent(Bool.self, forKey: .requiresLocalSigning)) ?? true
    }
}

public struct UpdateVerificationInfo: Codable, Equatable {
    public let publisherVerified: Bool
    public let keyId: String?

    enum CodingKeys: String, CodingKey {
        case publisherVerified = "publisher_verified"
        case keyId = "key_id"
    }

    public init(publisherVerified: Bool, keyId: String? = nil) {
        self.publisherVerified = publisherVerified
        self.keyId = keyId
    }
}

public struct UpdateCheckResult: Codable, Equatable {
    public let status: UpdateCheckStatus
    public let reason: String?
    public let candidate: UpdateCandidateDetail?
    public let verification: UpdateVerificationInfo?

    public init(
        status: UpdateCheckStatus,
        reason: String? = nil,
        candidate: UpdateCandidateDetail? = nil,
        verification: UpdateVerificationInfo? = nil
    ) {
        self.status = status
        self.reason = reason
        self.candidate = candidate
        self.verification = verification
    }
}

public struct UpdateSigningStatus: Equatable {
    public let keychainAvailable: Bool
    public let localCertificateFound: Bool
    public let lastError: String?

    public init(keychainAvailable: Bool, localCertificateFound: Bool, lastError: String? = nil) {
        self.keychainAvailable = keychainAvailable
        self.localCertificateFound = localCertificateFound
        self.lastError = lastError
    }

    public var canPerformLocalSigning: Bool {
        return keychainAvailable && localCertificateFound
    }

    public var userFacingDescription: String {
        if canPerformLocalSigning {
            return "专用机自签名证书与钥匙串就绪 (待发布者验证通过后重签)"
        }
        if let err = lastError, !err.isEmpty {
            return "签名不可用: \(err)"
        }
        if !keychainAvailable {
            return "钥匙串处于锁定状态或未解锁"
        }
        return "专用机本地自签名证书未找到"
    }
}

public enum UpdateClientError: LocalizedError, Equatable {
    case networkOffline(String)
    case commandFailed(code: Int32, message: String)
    case parseError(String)

    public var errorDescription: String? {
        switch self {
        case .networkOffline(let msg): return "网络连接失败: \(msg)"
        case .commandFailed(let code, let msg): return "命令执行错误 [\(code)]: \(msg)"
        case .parseError(let msg): return "数据解析失败: \(msg)"
        }
    }
}

// MARK: - Update Page Pure Logic

public enum UpdatePageLogic {
    public static func parseStatusPayload(jsonString: String) throws -> UpdateStatusPayload {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError("无法将字符串转为 UTF-8 字节")
        }
        let decoder = JSONDecoder()
        return try decoder.decode(UpdateStatusPayload.self, from: data)
    }

    public static func parseCheckPayload(jsonString: String) throws -> UpdateCheckResult {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError("无法将字符串转为 UTF-8 字节")
        }
        let decoder = JSONDecoder()
        return try decoder.decode(UpdateCheckResult.self, from: data)
    }

    /// 严格只读阶段：无论如何均不可安装
    public static func canInstall(
        checkResult: UpdateCheckResult?,
        transaction: UpdateTransactionStatus
    ) -> Bool {
        // 当前阶段未放行真实安装，硬编码返回 false 防止任何非受控写入
        return false
    }

    public static func recoveryGuide(for transaction: UpdateTransactionInfo) -> String {
        guard transaction.recoveryRequired else { return "系统状态正常，无需恢复。" }
        let stage = transaction.stage ?? "未知阶段"
        let err = transaction.error ?? "未知错误"
        let tx = transaction.transactionId ?? "无事务ID"

        return """
        【更新异常中断，需要人工恢复】
        - 事务 ID: \(tx)
        - 中断阶段: \(stage)
        - 错误原因: \(err)

        只读说明与恢复指引:
        1. 核心自动处理已被安全抑制，原旧版本及客户会话未受污染。
        2. 当前界面处于严格只读受控阶段，禁止通过界面清除维护标志或篡改状态。
        3. 请核对 ~/.wechat-rpa/logs/core.log 与 host.log 获取详细故障上下文。
        4. 需恢复旧版时，请由管理员核实独立 update-journal.json 后通过专用命令行执行受控回退。
        """
    }

    public static func formatBytes(_ bytes: UInt64) -> String {
        let kb = Double(bytes) / 1024.0
        if kb < 1024.0 {
            return String(format: "%.1f KB", kb)
        }
        let mb = kb / 1024.0
        return String(format: "%.2f MB", mb)
    }
}

// MARK: - Update Client

public final class UpdateClient {
    public typealias CommandExecutor = (String, [String], Double) -> ControlRun

    private let configPath: String
    private let executor: CommandExecutor

    public init(configPath: String? = nil, executor: @escaping CommandExecutor = runControlCommand) {
        self.configPath = configPath ?? (FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".WeChatRPABot/config.json").path)
        self.executor = executor
    }

    public func fetchStatus() throws -> UpdateStatusPayload {
        let args = ["control", "update-status", "--config", configPath]
        let run = executor(controlBinaryPath(), args, 10.0)
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "未知错误"
            throw UpdateClientError.commandFailed(code: run.status, message: errStr)
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try UpdatePageLogic.parseStatusPayload(jsonString: outStr)
    }

    public func checkUpdate(manifestPath: String? = nil) throws -> UpdateCheckResult {
        var args = ["control", "update-check", "--config", configPath]
        if let mp = manifestPath, !mp.isEmpty {
            args.append("--manifest")
            args.append(mp)
        }
        let run = executor(controlBinaryPath(), args, 15.0)
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "未知错误"
            throw UpdateClientError.commandFailed(code: run.status, message: errStr)
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try UpdatePageLogic.parseCheckPayload(jsonString: outStr)
    }
}

// MARK: - Update Page View Controller

public final class UpdatePageController: NSViewController {
    private let context: StatusPanelController
    private var client: UpdateClient

    // 页面状态（只读）
    private var currentStatusPayload: UpdateStatusPayload?
    private var currentCheckResult: UpdateCheckResult?
    private var signingStatus = UpdateSigningStatus(keychainAvailable: true, localCertificateFound: true)
    private var isChecking = false

    // UI - 当前版本卡片
    private let currentReleaseLabel = NSTextField(labelWithString: "加载中...")
    private let currentSeqLabel = NSTextField(labelWithString: "未校验")
    private let architectureLabel = NSTextField(labelWithString: "未知")
    private let consistencyBadge = NSTextField(labelWithString: "未校验 (未知)")
    private let supervisorStatusLabel = NSTextField(labelWithString: "检查中...")

    // UI - 候选更新卡片
    private let candidateStatusLabel = NSTextField(wrappingLabelWithString: "尚未检查更新")
    private let candidateVersionLabel = NSTextField(labelWithString: "-")
    private let candidatePublishedLabel = NSTextField(labelWithString: "-")
    private let candidateSizeLabel = NSTextField(labelWithString: "-")
    private let publisherVerificationBadge = NSTextField(labelWithString: "-")
    private let localSigningLabel = NSTextField(labelWithString: "-")
    private let compatibilityLabel = NSTextField(wrappingLabelWithString: "-")
    private let releaseNotesView = NSTextView()

    // UI - 流程流水线与进度
    private let pipelineStepsLabel = NSTextField(labelWithString: "检查 → 下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检")
    private let currentStageLabel = NSTextField(labelWithString: "只读监控中")
    private let progressBar = NSProgressIndicator()
    private let progressDetailLabel = NSTextField(labelWithString: "")
    private let drainWarningLabel = NSTextField(wrappingLabelWithString: "")
    private let permissionsNoteLabel = NSTextField(wrappingLabelWithString: "提示：专用机应用更新后将由本地自签名钥匙串重签名，若系统权限重置需在「设置」中重新授权。")

    // UI - 操作按钮 (只读阶段全部禁用写操作)
    private let checkButton = NSButton(title: "检查更新", target: nil, action: nil)
    private let downloadButton = NSButton(title: "下载更新 (本阶段未支持)", target: nil, action: nil)
    private let installButton = NSButton(title: "安装并重启 (本阶段未支持)", target: nil, action: nil)
    private let deferButton = NSButton(title: "延期更新 (本阶段未支持)", target: nil, action: nil)
    private let refreshStatusButton = NSButton(title: "刷新状态", target: nil, action: nil)
    private let actionFeedbackLabel = NSTextField(wrappingLabelWithString: "当前处于只读状态查看阶段；真实下载、安装、排空延期与恢复写入均已安全禁用。")

    // UI - 恢复卡片 (当出现 recovery_required 时高亮)
    private let recoveryBox = NSBox()
    private let recoveryDetailsLabel = NSTextField(wrappingLabelWithString: "")

    // UI - 整个内容 Stack
    public let contentStack = NSStackView()

    public init(context: StatusPanelController, client: UpdateClient? = nil) {
        self.context = context
        self.client = client ?? UpdateClient()
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
        loadInitialStatus()
    }

    private func setupUI() {
        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true

        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = 16
        contentStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. 标题
        let headerLabel = NSTextField(labelWithString: "应用版本更新 (只读状态)")
        headerLabel.font = NSFont.systemFont(ofSize: 18, weight: .bold)
        contentStack.addArrangedSubview(headerLabel)

        let introLabel = NSTextField(wrappingLabelWithString: "管理微信客服助手整套原生组件与核心服务更新。当前处于 I6 限定审查阶段，界面严格以真实读模型为准，真实更新写入处于 unsupported 禁用状态。")
        introLabel.font = NSFont.systemFont(ofSize: 12)
        introLabel.textColor = .secondaryLabelColor
        contentStack.addArrangedSubview(introLabel)

        // 2. 当前版本卡片
        let currentBox = createSectionBox(title: "当前运行环境 (真实读模型)")
        let currentGrid = NSGridView(views: [
            [NSTextField(labelWithString: "当前整套版本:"), currentReleaseLabel],
            [NSTextField(labelWithString: "序列号 (Sequence):"), currentSeqLabel],
            [NSTextField(labelWithString: "宿主运行架构:"), architectureLabel],
            [NSTextField(labelWithString: "组件一致性:"), consistencyBadge],
            [NSTextField(labelWithString: "维护抑制状态:"), supervisorStatusLabel]
        ])
        currentGrid.rowSpacing = 6
        currentGrid.columnSpacing = 12
        currentBox.setContent(currentGrid)
        contentStack.addArrangedSubview(currentBox)

        // 3. 候选版本卡片
        let candidateBox = createSectionBox(title: "候选版本与发布说明")
        let candStack = NSStackView()
        candStack.orientation = .vertical
        candStack.alignment = .leading
        candStack.spacing = 8

        candStack.addArrangedSubview(candidateStatusLabel)
        candidateStatusLabel.font = NSFont.systemFont(ofSize: 13, weight: .semibold)

        let candGrid = NSGridView(views: [
            [NSTextField(labelWithString: "目标版本:"), candidateVersionLabel],
            [NSTextField(labelWithString: "发布时间:"), candidatePublishedLabel],
            [NSTextField(labelWithString: "更新包大小:"), candidateSizeLabel],
            [NSTextField(labelWithString: "发布者验证:"), publisherVerificationBadge],
            [NSTextField(labelWithString: "本机重签名:"), localSigningLabel],
            [NSTextField(labelWithString: "兼容性说明:"), compatibilityLabel]
        ])
        candGrid.rowSpacing = 6
        candGrid.columnSpacing = 12
        candStack.addArrangedSubview(candGrid)

        let notesTitle = NSTextField(labelWithString: "版本发布说明:")
        notesTitle.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        candStack.addArrangedSubview(notesTitle)

        let notesScroll = NSScrollView()
        notesScroll.translatesAutoresizingMaskIntoConstraints = false
        notesScroll.heightAnchor.constraint(equalToConstant: 90).isActive = true
        notesScroll.hasVerticalScroller = true
        notesScroll.borderType = .bezelBorder

        releaseNotesView.isEditable = false
        releaseNotesView.font = NSFont.systemFont(ofSize: 11)
        releaseNotesView.string = "（暂无发布说明）"
        notesScroll.documentView = releaseNotesView
        candStack.addArrangedSubview(notesScroll)

        candidateBox.setContent(candStack)
        contentStack.addArrangedSubview(candidateBox)

        // 4. 更新流程与进度流水线
        let pipelineBox = createSectionBox(title: "更新阶段与进度")
        let pipeStack = NSStackView()
        pipeStack.orientation = .vertical
        pipeStack.alignment = .leading
        pipeStack.spacing = 8

        pipelineStepsLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .medium)
        pipelineStepsLabel.textColor = .secondaryLabelColor
        pipeStack.addArrangedSubview(pipelineStepsLabel)

        let stageRow = NSStackView()
        stageRow.orientation = .horizontal
        stageRow.spacing = 10
        let stagePrefix = NSTextField(labelWithString: "当前执行阶段:")
        stagePrefix.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        currentStageLabel.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        currentStageLabel.textColor = .systemBlue
        stageRow.addArrangedSubview(stagePrefix)
        stageRow.addArrangedSubview(currentStageLabel)
        pipeStack.addArrangedSubview(stageRow)

        progressBar.translatesAutoresizingMaskIntoConstraints = false
        progressBar.isIndeterminate = false
        progressBar.minValue = 0.0
        progressBar.maxValue = 100.0
        progressBar.doubleValue = 0.0
        progressBar.widthAnchor.constraint(equalToConstant: 360).isActive = true
        pipeStack.addArrangedSubview(progressBar)

        progressDetailLabel.font = NSFont.systemFont(ofSize: 11)
        progressDetailLabel.textColor = .secondaryLabelColor
        pipeStack.addArrangedSubview(progressDetailLabel)

        drainWarningLabel.font = NSFont.systemFont(ofSize: 12)
        drainWarningLabel.textColor = .systemOrange
        pipeStack.addArrangedSubview(drainWarningLabel)

        permissionsNoteLabel.font = NSFont.systemFont(ofSize: 11)
        permissionsNoteLabel.textColor = .secondaryLabelColor
        pipeStack.addArrangedSubview(permissionsNoteLabel)

        pipelineBox.setContent(pipeStack)
        contentStack.addArrangedSubview(pipelineBox)

        // 5. 失败恢复指引卡片
        recoveryBox.title = "⚠️ 故障恢复指引 (只读状态)"
        recoveryBox.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)
        recoveryBox.boxType = .primary
        recoveryBox.translatesAutoresizingMaskIntoConstraints = false
        recoveryDetailsLabel.font = NSFont.systemFont(ofSize: 12)
        recoveryDetailsLabel.textColor = .systemRed
        let recStack = NSStackView()
        recStack.orientation = .vertical
        recStack.alignment = .leading
        recStack.spacing = 8
        recStack.addArrangedSubview(recoveryDetailsLabel)
        recoveryBox.setContent(recStack)
        recoveryBox.isHidden = true
        contentStack.addArrangedSubview(recoveryBox)

        // 6. 操作按钮行 (严格只读，所有写操作永久禁用)
        let actionBox = createSectionBox(title: "操作与维护 (只读受控)")
        let actStack = NSStackView()
        actStack.orientation = .vertical
        actStack.alignment = .leading
        actStack.spacing = 10

        let btnRow = NSStackView()
        btnRow.orientation = .horizontal
        btnRow.spacing = 10

        checkButton.target = self
        checkButton.action = #selector(handleCheckUpdate)
        downloadButton.target = self
        downloadButton.action = #selector(handleDisabledWriteAction)
        installButton.target = self
        installButton.action = #selector(handleDisabledWriteAction)
        deferButton.target = self
        deferButton.action = #selector(handleDisabledWriteAction)
        refreshStatusButton.target = self
        refreshStatusButton.action = #selector(handleRefreshStatus)

        // 核心约束：下载/安装/延期在生产代码中始终禁用
        downloadButton.isEnabled = false
        installButton.isEnabled = false
        deferButton.isEnabled = false

        btnRow.addArrangedSubview(checkButton)
        btnRow.addArrangedSubview(downloadButton)
        btnRow.addArrangedSubview(installButton)
        btnRow.addArrangedSubview(deferButton)
        btnRow.addArrangedSubview(refreshStatusButton)
        actStack.addArrangedSubview(btnRow)

        actionFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        actionFeedbackLabel.textColor = .secondaryLabelColor
        actStack.addArrangedSubview(actionFeedbackLabel)

        actionBox.setContent(actStack)
        contentStack.addArrangedSubview(actionBox)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: view.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),

            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),

            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),

            currentBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            candidateBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            pipelineBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            recoveryBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            actionBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
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

    // MARK: - Status Loading & Presentation

    public func loadInitialStatus() {
        do {
            let payload = try client.fetchStatus()
            applyStatusPayload(payload)
        } catch {
            actionFeedbackLabel.stringValue = "状态获取提示: \(error.localizedDescription)"
            actionFeedbackLabel.textColor = .secondaryLabelColor
        }
    }

    public func applyStatusPayload(_ payload: UpdateStatusPayload) {
        self.currentStatusPayload = payload

        // 当前版本信息：严格展示实际读模型，不臆造
        currentReleaseLabel.stringValue = payload.currentRelease.releaseId.isEmpty ? "未知" : payload.currentRelease.releaseId
        if let s = payload.currentRelease.sequence {
            currentSeqLabel.stringValue = "\(s)"
        } else {
            currentSeqLabel.stringValue = "未标记 (null)"
        }
        architectureLabel.stringValue = payload.currentRelease.architecture.isEmpty ? "未知" : payload.currentRelease.architecture

        if let c = payload.currentRelease.componentsConsistent {
            if c {
                consistencyBadge.stringValue = "整套组件版本一致"
                consistencyBadge.textColor = .systemGreen
            } else {
                consistencyBadge.stringValue = "⚠️ 组件版本不一致"
                consistencyBadge.textColor = .systemRed
            }
        } else {
            consistencyBadge.stringValue = "未校验 (未知)"
            consistencyBadge.textColor = .secondaryLabelColor
        }

        if payload.guardInfo.maintenanceActive {
            supervisorStatusLabel.stringValue = "处于维护抑制态 (核心启动被抑制)"
            supervisorStatusLabel.textColor = .systemOrange
        } else {
            // 真实求实：不处于维护抑制态不等于运行正常
            supervisorStatusLabel.stringValue = "未处于维护抑制态 (运行健康度未经验证)"
            supervisorStatusLabel.textColor = .secondaryLabelColor
        }

        // 事务与阶段
        updateTransactionUI(payload.transaction)
    }

    public func applyCheckResult(_ result: UpdateCheckResult) {
        self.currentCheckResult = result

        switch result.status {
        case .unconfigured:
            candidateStatusLabel.stringValue = "更新源尚未配置"
            candidateStatusLabel.textColor = .systemOrange
            candidateVersionLabel.stringValue = "无可用版本 (null)"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "未配置公钥根"
            publisherVerificationBadge.textColor = .secondaryLabelColor
            localSigningLabel.stringValue = "未配置发布源，禁止重签"
            compatibilityLabel.stringValue = result.reason ?? "生产环境下需配置授权更新清单；禁止任意未经验证的网络下载"
            releaseNotesView.string = "系统未配置已批准的更新清单或分发源。为保障微信业务与专用机安全，禁止使用任意 URL 下载执行内容。"

        case .unverifiedTrustRoot:
            candidateStatusLabel.stringValue = "已发现候选版本（来源尚未验证）"
            candidateStatusLabel.textColor = .systemOrange
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? "未知"
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                } else {
                    candidateSizeLabel.stringValue = "未知"
                }
                compatibilityLabel.stringValue = cand.compatible ? "架构兼容 (发布者未验证)" : "架构或系统不兼容"
                releaseNotesView.string = cand.releaseNotes ?? "（无详细发布说明）"
            }
            publisherVerificationBadge.stringValue = "尚未验证（无可信根公钥配置）"
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = "禁止重签 (需要发布者验证通过)"

        case .alreadyInstalled:
            candidateStatusLabel.stringValue = "当前已是最新版本"
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
            }
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "已生效"
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = "已在运行"
            compatibilityLabel.stringValue = "当前版本已与本地环境兼容"
            releaseNotesView.string = "没有待更新的候选版本。"

        case .incompatible:
            candidateStatusLabel.stringValue = "候选版本不兼容当前环境"
            candidateStatusLabel.textColor = .systemRed
            candidateVersionLabel.stringValue = "-"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "未通过兼容性核验"
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = "禁止重签"
            compatibilityLabel.stringValue = result.reason ?? "不满足最低系统或架构要求"
            releaseNotesView.string = "该版本无法安装在当前目标专用机上。"

        case .available:
            candidateStatusLabel.stringValue = "发现可用版本并通过完整验证"
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? "-"
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                }
                compatibilityLabel.stringValue = cand.compatible ? "系统环境兼容" : "不兼容"
                releaseNotesView.string = cand.releaseNotes ?? "（暂无说明）"
            }
            publisherVerificationBadge.stringValue = "发布者签名已验证"
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = signingStatus.userFacingDescription
        }

        // 始终确保写按钮禁用
        downloadButton.isEnabled = false
        installButton.isEnabled = false
        deferButton.isEnabled = false
    }

    private func updateTransactionUI(_ transaction: UpdateTransactionInfo) {
        if transaction.recoveryRequired {
            currentStageLabel.stringValue = "故障需要恢复 (\(transaction.stage ?? "Unknown"))"
            currentStageLabel.textColor = .systemRed
            recoveryBox.isHidden = false
            recoveryDetailsLabel.stringValue = UpdatePageLogic.recoveryGuide(for: transaction)
            downloadButton.isEnabled = false
            installButton.isEnabled = false
            deferButton.isEnabled = false
            return
        }

        recoveryBox.isHidden = true
        switch transaction.status {
        case .idle:
            currentStageLabel.stringValue = "空闲 (只读状态)"
            currentStageLabel.textColor = .secondaryLabelColor
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = ""
            drainWarningLabel.stringValue = ""

        case .inProgress:
            let stageName = transaction.stage ?? "执行中"
            currentStageLabel.stringValue = "状态: \(stageName) (外部事务)"
            currentStageLabel.textColor = .systemBlue
            if stageName == "Draining" {
                drainWarningLabel.stringValue = "⚠️ 外部更新事务处于排空等待态；当前界面为只读，延期请由管理员命令行协调。"
            } else {
                drainWarningLabel.stringValue = ""
            }

        case .committed:
            currentStageLabel.stringValue = "最近记录已提交"
            currentStageLabel.textColor = .systemGreen
            progressBar.doubleValue = 100.0
            progressDetailLabel.stringValue = "已记录生效"
            drainWarningLabel.stringValue = ""

        case .rolledBack:
            currentStageLabel.stringValue = "已安全记录回退状态"
            currentStageLabel.textColor = .systemOrange
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = "保持旧版本记录"
            drainWarningLabel.stringValue = ""

        case .recoveryRequired:
            break
        }

        downloadButton.isEnabled = false
        installButton.isEnabled = false
        deferButton.isEnabled = false
    }

    /// 供测试夹具显式注入合成状态，用于离屏截图与 UI 验证，与生产 handler 严格隔离
    public func applySyntheticState(
        stage: String,
        progress: Double,
        progressDetail: String,
        drainWarning: String? = nil,
        recoveryInfo: UpdateTransactionInfo? = nil
    ) {
        currentStageLabel.stringValue = stage
        progressBar.doubleValue = progress
        progressDetailLabel.stringValue = progressDetail
        if let dw = drainWarning {
            drainWarningLabel.stringValue = dw
        } else {
            drainWarningLabel.stringValue = ""
        }
        if let rec = recoveryInfo, rec.recoveryRequired {
            recoveryBox.isHidden = false
            recoveryDetailsLabel.stringValue = UpdatePageLogic.recoveryGuide(for: rec)
        } else {
            recoveryBox.isHidden = true
        }
    }

    // MARK: - Actions

    @objc private func handleCheckUpdate() {
        guard !isChecking else { return }
        isChecking = true
        checkButton.isEnabled = false
        actionFeedbackLabel.stringValue = "正在查询更新状态 (只读查询)..."
        actionFeedbackLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let res = try self.client.checkUpdate()
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.applyCheckResult(res)
                    self.actionFeedbackLabel.stringValue = "检查完成 (只读呈现)。"
                    self.actionFeedbackLabel.textColor = .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = "检查失败: \(error.localizedDescription)"
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
    }

    /// 关键安全防线：所有写入操作全部重定向到此 handler，严格禁止任何模拟写入或状态修改
    @objc public func handleDisabledWriteAction() {
        actionFeedbackLabel.stringValue = "操作未启用: 当前阶段真实更新下载/安装/延期入口为只读禁用状态 (unsupported)，不进行任何写入或业务状态修改。"
        actionFeedbackLabel.textColor = .systemOrange
    }

    @objc private func handleRefreshStatus() {
        loadInitialStatus()
        actionFeedbackLabel.stringValue = "状态已刷新 (只读)。"
        actionFeedbackLabel.textColor = .secondaryLabelColor
    }
}
