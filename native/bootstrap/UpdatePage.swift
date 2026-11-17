// native/bootstrap/UpdatePage.swift
// Application update management, version verification, drain coordination, rollback, and failure recovery.
import Foundation
import AppKit

// MARK: - Shared Theme tokens
private enum UpdatePageTokens {
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
private enum UpdatePageStrings {
    static var pageTitle: String { L10n.t(.updatePagePageTitle) }
    static var sectionRollback: String { L10n.t(.updatePageSectionRollback) }
    static var sectionOperations: String { L10n.t(.updatePageSectionOperations) }
    static var sectionRecovery: String { L10n.t(.updatePageSectionRecovery) }
    static var rollbackAppNotice: String { L10n.t(.updatePageRollbackAppNotice) }
    static var noPreviousVersion: String { L10n.t(.updatePageNoPreviousVersion) }
    static var rollbackCheckboxPrompt: String { L10n.t(.updatePageRollbackCheckboxPrompt) }
    static var rollbackConfirmButton: String { L10n.t(.updatePageRollbackConfirmButton) }
    static var cancel: String { L10n.t(.cancel) }
    static var rollbackSuccess: String { L10n.t(.updatePageRollbackSuccess) }
    static var rollbackUnavailable: String { L10n.t(.updatePageRollbackUnavailable) }
    static var emptyCandidate: String { L10n.t(.updatePageEmptyCandidate) }
}

// MARK: - Update Data Models

public struct UpdateError: Error, CustomStringConvertible, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var description: String { message }
}

public struct RollbackStatusInfo: Codable, Equatable {
    public let version: String?
    public let installedAt: Double?
    public let canRollback: Bool
    public let reason: String?

    enum CodingKeys: String, CodingKey {
        case version
        case installedAt = "installed_at"
        case canRollback = "can_rollback"
        case reason
    }

    public init(version: String? = nil, installedAt: Double? = nil, canRollback: Bool = false, reason: String? = nil) {
        self.version = version
        self.installedAt = installedAt
        self.canRollback = canRollback
        self.reason = reason
    }

    public var localizedReason: String {
        guard let r = reason else { return canRollback ? L10n.t(.updateReasonSafe) : L10n.t(.updateReasonCannot) }
        switch r {
        case "no_previous_version": return L10n.t(.updatePageNoPreviousVersion)
        case "no_matching_completed_update": return L10n.t(.updateReasonNoMatching)
        case "previous_release_corrupt": return L10n.t(.updateReasonPrevCorrupt)
        case "backup_corrupt": return L10n.t(.updateReasonBackupCorrupt)
        case "rollback_not_supported_by_release": return L10n.t(.updateReasonNotSupported)
        case "database_schema_changed": return L10n.t(.updateReasonSchemaChanged)
        case "database_checkpoint_required": return L10n.t(.updateReasonCheckpointRequired)
        case "stop_core_first": return L10n.t(.updateReasonStopCoreFirst)
        case "recovery_required": return L10n.t(.updateReasonRecoveryRequired)
        case "install_root_unconfigured": return L10n.t(.updateReasonInstallRootUnconfigured)
        case "rollback_state_invalid": return L10n.t(.updateReasonRollbackStateInvalid)
        default: return r
        }
    }

    public var formattedInstalledAt: String {
        guard let ts = installedAt, ts > 0 else { return L10n.t(.statusUnknown) }
        let date = Date(timeIntervalSince1970: ts)
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return fmt.string(from: date)
    }
}

public struct UpdateReleaseInfo: Codable, Equatable {
    public let releaseId: String?
    public let sequence: UInt64?
    public let status: String?
    public let source: String?
    public let coreVersion: String?
    public let architecture: String
    public let componentsConsistent: Bool?

    enum CodingKeys: String, CodingKey {
        case releaseId = "release_id"
        case sequence
        case status
        case source
        case coreVersion = "core_version"
        case architecture
        case componentsConsistent = "components_consistent"
    }

    public init(
        releaseId: String? = nil,
        sequence: UInt64? = nil,
        status: String? = nil,
        source: String? = nil,
        coreVersion: String? = nil,
        architecture: String = "unknown",
        componentsConsistent: Bool? = nil
    ) {
        self.releaseId = releaseId
        self.sequence = sequence
        self.status = status
        self.source = source
        self.coreVersion = coreVersion
        self.architecture = architecture
        self.componentsConsistent = componentsConsistent
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.releaseId = try container.decodeIfPresent(String.self, forKey: .releaseId)
        self.sequence = try container.decodeIfPresent(UInt64.self, forKey: .sequence)
        self.status = try container.decodeIfPresent(String.self, forKey: .status)
        self.source = try container.decodeIfPresent(String.self, forKey: .source)
        self.coreVersion = try container.decodeIfPresent(String.self, forKey: .coreVersion)
        self.architecture = (try container.decodeIfPresent(String.self, forKey: .architecture)) ?? "unknown"
        self.componentsConsistent = try container.decodeIfPresent(Bool.self, forKey: .componentsConsistent)
    }

    /// 整套版本可信度判定：必须有明确已安装/已验证状态与合法发布源，缺失或 unknown 均不可信
    public var isTrustedInstalledRelease: Bool {
        guard let rid = releaseId, !rid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return false
        }
        guard let st = status?.lowercased(), st == "installed" || st == "verified" else {
            return false
        }
        guard let src = source?.lowercased(), src == "signed_package" else {
            return false
        }
        return true
    }

    /// 用户友好的整套版本描述（空值/null 均明确显示无法核验/未知，不当做未安装或解码失败）
    public var userFacingReleaseDescription: String {
        guard let rid = releaseId, !rid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return L10n.t(.updateReleaseDescUnknown)
        }
        return rid
    }

    /// 用户友好的核心编译版本描述（独立呈现）
    public var userFacingCoreVersionDescription: String {
        guard let cv = coreVersion, !cv.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return L10n.t(.updateCoreVersionUnknown)
        }
        return L10n.t(.updateCoreCompiledVersion, cv)
    }

    /// 用户友好的安装序列号描述（绝不拿 guard.highestAcceptedSequence 填补）
    public var userFacingSequenceDescription: String {
        if let s = sequence {
            return "\(s)"
        }
        return L10n.t(.updateSeqUnknown)
    }

    /// 用户友好的状态与来源描述：缺失或未知均明确标注不可信，不默认正常
    public var userFacingTrustDescription: String {
        let st = status?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let src = source?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if st.isEmpty && src.isEmpty {
            return L10n.t(.updateOldDataSourceUntrusted)
        }

        let stDesc: String
        switch st.lowercased() {
        case "unknown": stDesc = L10n.t(.updateStateUnknown)
        case "installed": stDesc = L10n.t(.updateStateInstalled)
        case "verified": stDesc = L10n.t(.updateStateVerified)
        case "": stDesc = L10n.t(.updateStateNone)
        default: stDesc = L10n.t(.updateStateUnknownFmt, st)
        }

        let srcDesc: String
        switch src.lowercased() {
        case "unknown": srcDesc = L10n.t(.updateSourceUnknown)
        case "signed_package": srcDesc = L10n.t(.updateSourceSignedPackage)
        case "": srcDesc = L10n.t(.updateSourceNone)
        default: srcDesc = L10n.t(.updateSourceUnknownFmt, src)
        }

        if isTrustedInstalledRelease {
            return "\(stDesc) (\(srcDesc))"
        } else {
            return L10n.t(.updateUntrustedStatusSourceFmt, stDesc, srcDesc)
        }
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

    public var userFacingGuardDescription: String {
        if let s = highestAcceptedSequence {
            return L10n.t(.updateHighWatermarkNotice, s)
        }
        return L10n.t(.updateNullValue)
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
            return L10n.t(.updateKeychainReady)
        }
        if let err = lastError, !err.isEmpty {
            return L10n.t(.updateSigningUnavailableFmt, err)
        }
        if !keychainAvailable {
            return L10n.t(.updateKeychainLocked)
        }
        return L10n.t(.updateCertNotFound)
    }
}

public enum UpdateClientError: LocalizedError, Equatable {
    case networkOffline(String)
    case commandFailed(code: Int32, message: String)
    case parseError(String)

    public var errorDescription: String? {
        switch self {
        case .networkOffline(let msg): return L10n.t(.updateErrNetworkOfflineFmt, msg)
        case .commandFailed(let code, let msg): return L10n.t(.updateErrCommandFailedFmt, code, msg)
        case .parseError(let msg): return L10n.t(.updateErrParseFmt, msg)
        }
    }
}

// MARK: - Update Page Pure Logic

public enum UpdatePageLogic {
    public static func parseStatusPayload(jsonString: String) throws -> UpdateStatusPayload {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError("Cannot encode string to UTF-8")
        }
        let decoder = JSONDecoder()
        return try decoder.decode(UpdateStatusPayload.self, from: data)
    }

    public static func parseCheckPayload(jsonString: String) throws -> UpdateCheckResult {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError("Cannot encode string to UTF-8")
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
        guard transaction.recoveryRequired else { return L10n.t(.updateRecoveryNormal) }
        let stage = transaction.stage ?? L10n.t(.statusUnknown)
        let err = transaction.error ?? L10n.t(.statusUnknown)
        let tx = transaction.transactionId ?? "-"

        return """
        \(L10n.t(.updateRecoveryReportTitle))
        \(L10n.t(.updateRecoveryTxFmt, tx))
        \(L10n.t(.updateRecoveryStageFmt, stage))
        \(L10n.t(.updateRecoveryReasonFmt, err))

        \(L10n.t(.updateRecoveryGuideHeader))
        \(L10n.t(.updateRecoveryGuideItem1))
        \(L10n.t(.updateRecoveryGuideItem2))
        \(L10n.t(.updateRecoveryGuideItem3))
        \(L10n.t(.updateRecoveryGuideItem4))
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
            let errStr = String(data: run.output, encoding: .utf8) ?? L10n.t(.statusUnknown)
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
            let errStr = String(data: run.output, encoding: .utf8) ?? L10n.t(.statusUnknown)
            throw UpdateClientError.commandFailed(code: run.status, message: errStr)
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try UpdatePageLogic.parseCheckPayload(jsonString: outStr)
    }

    public func fetchRollbackStatus() -> Result<RollbackStatusInfo, UpdateError> {
        let updater = updaterBinaryPath()
        guard FileManager.default.isExecutableFile(atPath: updater) else {
            return .failure(UpdateError(UpdatePageStrings.rollbackUnavailable))
        }
        let args = ["rollback-status", "--config", configPath]
        let run = executor(updater, args, 10.0)
        guard !run.timedOut else {
            return .failure(UpdateError(UpdatePageStrings.rollbackUnavailable))
        }
        guard run.status == 0, let jsonStr = String(data: run.output, encoding: .utf8), !jsonStr.isEmpty else {
            return .failure(UpdateError(UpdatePageStrings.rollbackUnavailable))
        }
        do {
            let decoder = JSONDecoder()
            let status = try decoder.decode(RollbackStatusInfo.self, from: run.output)
            return .success(status)
        } catch {
            return .failure(UpdateError(UpdatePageStrings.rollbackUnavailable))
        }
    }

    public func executeRollback(expectedVersion: String) -> Result<String, UpdateError> {
        let updater = updaterBinaryPath()
        let args = ["rollback", "--config", configPath, "--expected-version", expectedVersion]
        let run = executor(updater, args, 30.0)
        guard !run.timedOut else {
            return .failure(UpdateError(L10n.t(.updateRollbackTimeout)))
        }
        guard !run.output.isEmpty else {
            return .failure(UpdateError(L10n.t(.updateRollbackNoOutput)))
        }
        if run.status == 0 {
            return .success(UpdatePageStrings.rollbackSuccess)
        } else {
            return .failure(UpdateError(L10n.t(.updateRollbackExitCodeFmt, run.status)))
        }
    }
}

public func updaterBinaryPath() -> String {
    let corePath = controlBinaryPath()
    let dir = (corePath as NSString).deletingLastPathComponent
    let candidate = (dir as NSString).appendingPathComponent("wechat-updater")
    if FileManager.default.isExecutableFile(atPath: candidate) {
        return candidate
    }
    return "wechat-updater"
}

// MARK: - Update Page View Controller

public final class UpdatePageController: NSViewController {
    private let context: StatusPanelController
    private var client: UpdateClient

    // 页面状态
    private var currentStatusPayload: UpdateStatusPayload?
    private var currentCheckResult: UpdateCheckResult?
    private var currentRollbackStatus: RollbackStatusInfo?
    private var signingStatus = UpdateSigningStatus(keychainAvailable: true, localCertificateFound: true)
    private var isChecking = false

    // UI - 当前版本卡片 (全部使用 wrappingLabelWithString 并设受限宽度)
    private let currentReleaseLabel = NSTextField(wrappingLabelWithString: L10n.t(.loading))
    private let coreVersionLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnknown))
    private let currentSeqLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateSeqUnknown))
    private let guardSeqLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateNullValue))
    private let releaseTrustLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnverified))
    private let architectureLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnknown))
    private let consistencyBadge = NSTextField(wrappingLabelWithString: L10n.t(.updateReleaseDescUnknown))
    private let supervisorStatusLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusChecking))

    // UI - 回滚到上一版本卡片
    private let previousVersionLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusQuerying))
    private let rollbackEligibilityLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusChecking))
    private let rollbackButton = NSButton(title: L10n.t(.updatePageSectionRollback) + "…", target: nil, action: nil)
    private let rollbackReasonSideLabel = NSTextField(wrappingLabelWithString: "")

    // UI - 候选更新卡片
    private let candidateStatusLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateCandidateNotCheckedYet))
    private let candidateVersionLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidatePublishedLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidateSizeLabel = NSTextField(wrappingLabelWithString: "-")
    private let publisherVerificationBadge = NSTextField(wrappingLabelWithString: "-")
    private let localSigningLabel = NSTextField(wrappingLabelWithString: "-")
    private let compatibilityLabel = NSTextField(wrappingLabelWithString: "-")
    private let releaseNotesView = NSTextView()

    // UI - 流程流水线与进度
    private let pipelineStepsLabel = NSTextField(wrappingLabelWithString: L10n.t(.updatePipelineSteps))
    private let currentStageLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateStageIdle))
    private let progressBar = NSProgressIndicator()
    private let progressDetailLabel = NSTextField(wrappingLabelWithString: "")
    private let drainWarningLabel = NSTextField(wrappingLabelWithString: "")
    private let permissionsNoteLabel = NSTextField(wrappingLabelWithString: L10n.t(.updatePermissionsNotice))

    // UI - 操作按钮 (分行排布)
    private let checkButton = NSButton(title: L10n.t(.actionRecheck), target: nil, action: nil)
    private let refreshStatusButton = NSButton(title: L10n.t(.actionRefresh), target: nil, action: nil)
    private let downloadButton = NSButton(title: L10n.t(.updateBtnDownloadDisabled), target: nil, action: nil)
    private let installButton = NSButton(title: L10n.t(.updateBtnInstallDisabled), target: nil, action: nil)
    private let deferButton = NSButton(title: L10n.t(.updateBtnDeferDisabled), target: nil, action: nil)
    private let actionFeedbackLabel = NSTextField(wrappingLabelWithString: "")

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
        let headerLabel = NSTextField(labelWithString: UpdatePageStrings.pageTitle)
        headerLabel.font = NSFont.systemFont(ofSize: 18, weight: .bold)
        contentStack.addArrangedSubview(headerLabel)

        let introLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateIntroDesc))
        introLabel.font = NSFont.systemFont(ofSize: 12)
        introLabel.textColor = .secondaryLabelColor
        contentStack.addArrangedSubview(introLabel)

        // 2. 当前版本卡片
        let currentBox = createSectionBox(title: L10n.t(.updateSectionCurrentEnv))
        let currentGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.updateLabelCurrentSuite)), currentReleaseLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelCoreVersion)), coreVersionLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelInstallSeq)), currentSeqLabel],
            [NSTextField(labelWithString: "Downgrade guard:"), guardSeqLabel],
            [NSTextField(labelWithString: "Status & source:"), releaseTrustLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelHostArch)), architectureLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelConsistency)), consistencyBadge],
            [NSTextField(labelWithString: "Maintenance:"), supervisorStatusLabel]
        ])
        currentGrid.rowSpacing = 6
        currentGrid.columnSpacing = 12
        currentReleaseLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        coreVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        currentSeqLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        guardSeqLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        releaseTrustLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        architectureLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        consistencyBadge.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        supervisorStatusLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        currentBox.setContent(currentGrid)
        contentStack.addArrangedSubview(currentBox)

        // 3. 候选版本卡片
        let candidateBox = createSectionBox(title: L10n.t(.updateSectionCandidate))
        let candStack = NSStackView()
        candStack.orientation = .vertical
        candStack.alignment = .leading
        candStack.spacing = 8

        candStack.addArrangedSubview(candidateStatusLabel)
        candidateStatusLabel.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        candidateStatusLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 580).isActive = true

        let candGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.updateLabelTargetVersion)), candidateVersionLabel],
            [NSTextField(labelWithString: "Published at:"), candidatePublishedLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelPackageSize)), candidateSizeLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelPublisherVerify)), publisherVerificationBadge],
            [NSTextField(labelWithString: L10n.t(.updateLabelLocalSign)), localSigningLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelCompatibility)), compatibilityLabel]
        ])
        candGrid.rowSpacing = 6
        candGrid.columnSpacing = 12
        candidateVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        candidatePublishedLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        candidateSizeLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        publisherVerificationBadge.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        localSigningLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        compatibilityLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        candStack.addArrangedSubview(candGrid)

        let notesTitle = NSTextField(labelWithString: L10n.t(.updateLabelReleaseNotes))
        notesTitle.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        candStack.addArrangedSubview(notesTitle)

        let notesScroll = NSScrollView()
        notesScroll.translatesAutoresizingMaskIntoConstraints = false
        notesScroll.heightAnchor.constraint(equalToConstant: 90).isActive = true
        notesScroll.hasVerticalScroller = true
        notesScroll.borderType = .bezelBorder

        releaseNotesView.isEditable = false
        releaseNotesView.font = NSFont.systemFont(ofSize: 11)
        releaseNotesView.string = L10n.t(.updateReleaseNotesEmpty)
        notesScroll.documentView = releaseNotesView
        candStack.addArrangedSubview(notesScroll)

        candidateBox.setContent(candStack)
        contentStack.addArrangedSubview(candidateBox)

        // 4. 更新流程与进度流水线
        let pipelineBox = createSectionBox(title: L10n.t(.updateSectionProgress))
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
        let stagePrefix = NSTextField(labelWithString: "Stage:")
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
        recoveryBox.title = UpdatePageStrings.sectionRecovery
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

        // 6. 回滚到上一版本卡片
        let rollbackBox = createSectionBox(title: UpdatePageStrings.sectionRollback)
        let rbStack = NSStackView()
        rbStack.orientation = .vertical
        rbStack.alignment = .leading
        rbStack.spacing = 8

        let rbNoticeLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateRollbackNotice))
        rbNoticeLabel.font = NSFont.systemFont(ofSize: 11)
        rbNoticeLabel.textColor = .secondaryLabelColor
        rbStack.addArrangedSubview(rbNoticeLabel)

        let rbGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.updateLabelRollbackVersion)), previousVersionLabel],
            [NSTextField(labelWithString: L10n.t(.updateLabelRollbackEligibility)), rollbackEligibilityLabel]
        ])
        rbGrid.rowSpacing = 6
        rbGrid.columnSpacing = 12
        previousVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        rollbackEligibilityLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        rbStack.addArrangedSubview(rbGrid)

        rollbackButton.target = self
        rollbackButton.action = #selector(handleRollback)
        rollbackButton.isEnabled = false

        rollbackReasonSideLabel.font = NSFont.systemFont(ofSize: 11)
        rollbackReasonSideLabel.textColor = UpdatePageTokens.statusWarnText
        rollbackReasonSideLabel.lineBreakMode = .byTruncatingTail
        rollbackReasonSideLabel.maximumNumberOfLines = 1

        let rbBtnRow = NSStackView(views: [rollbackButton, rollbackReasonSideLabel])
        rbBtnRow.orientation = .horizontal
        rbBtnRow.spacing = 10
        rbBtnRow.alignment = .centerY
        rbStack.addArrangedSubview(rbBtnRow)

        rollbackBox.setContent(rbStack)
        contentStack.addArrangedSubview(rollbackBox)

        // 7. 操作按钮行 (分两行排布自适应宽度)
        let actionBox = createSectionBox(title: UpdatePageStrings.sectionOperations)
        let actStack = NSStackView()
        actStack.orientation = .vertical
        actStack.alignment = .leading
        actStack.spacing = 10

        // 第1行：状态检查与刷新查询操作
        let queryBtnRow = NSStackView()
        queryBtnRow.orientation = .horizontal
        queryBtnRow.spacing = 12

        checkButton.target = self
        checkButton.action = #selector(handleCheckUpdate)
        refreshStatusButton.target = self
        refreshStatusButton.action = #selector(handleRefreshStatus)

        queryBtnRow.addArrangedSubview(checkButton)
        queryBtnRow.addArrangedSubview(refreshStatusButton)
        actStack.addArrangedSubview(queryBtnRow)

        // 第2行：写操作受控入口 (本阶段永久禁用)
        let writeBtnRow = NSStackView()
        writeBtnRow.orientation = .horizontal
        writeBtnRow.spacing = 10

        downloadButton.target = self
        downloadButton.action = #selector(handleDisabledWriteAction)
        installButton.target = self
        installButton.action = #selector(handleDisabledWriteAction)
        deferButton.target = self
        deferButton.action = #selector(handleDisabledWriteAction)

        downloadButton.isEnabled = false
        installButton.isEnabled = false
        deferButton.isEnabled = false

        writeBtnRow.addArrangedSubview(downloadButton)
        writeBtnRow.addArrangedSubview(installButton)
        writeBtnRow.addArrangedSubview(deferButton)
        actStack.addArrangedSubview(writeBtnRow)

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
            rollbackBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
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
            actionFeedbackLabel.stringValue = L10n.t(.updateStatusFetchNoticeFmt, error.localizedDescription)
            actionFeedbackLabel.textColor = .secondaryLabelColor
        }
        loadRollbackStatus()
    }

    private func loadRollbackStatus() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let res = self.client.fetchRollbackStatus()
            DispatchQueue.main.async {
                self.applyRollbackStatusResult(res)
            }
        }
    }

    private func applyRollbackStatusResult(_ result: Result<RollbackStatusInfo, UpdateError>) {
        switch result {
        case .success(let info):
            self.currentRollbackStatus = info
            if let ver = info.version, !ver.isEmpty {
                self.previousVersionLabel.stringValue = ver
                self.previousVersionLabel.textColor = .labelColor
            } else {
                self.previousVersionLabel.stringValue = UpdatePageStrings.noPreviousVersion
                self.previousVersionLabel.textColor = .secondaryLabelColor
            }
            let reasonText = info.localizedReason
            self.rollbackEligibilityLabel.stringValue = reasonText
            if info.canRollback {
                self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusOkText
                self.rollbackReasonSideLabel.stringValue = ""
                self.rollbackButton.isEnabled = true
            } else {
                self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusWarnText
                self.rollbackReasonSideLabel.stringValue = reasonText
                self.rollbackReasonSideLabel.textColor = UpdatePageTokens.statusWarnText
                self.rollbackButton.isEnabled = false
            }
        case .failure:
            self.currentRollbackStatus = nil
            self.previousVersionLabel.stringValue = L10n.t(.statusUnknown)
            let reasonText = UpdatePageStrings.rollbackUnavailable
            self.rollbackEligibilityLabel.stringValue = reasonText
            self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusOffText
            self.rollbackReasonSideLabel.stringValue = reasonText
            self.rollbackReasonSideLabel.textColor = UpdatePageTokens.statusOffText
            self.rollbackButton.isEnabled = false
        }
    }

    public func applyStatusPayload(_ payload: UpdateStatusPayload) {
        self.currentStatusPayload = payload

        // 当前版本信息：严格展示实际读模型，不臆造，不用 core_version 或 guard 高水位填补 installed 字段
        currentReleaseLabel.stringValue = payload.currentRelease.userFacingReleaseDescription
        if payload.currentRelease.releaseId == nil || payload.currentRelease.releaseId?.isEmpty == true {
            currentReleaseLabel.textColor = .systemOrange
        } else {
            currentReleaseLabel.textColor = .labelColor
        }

        coreVersionLabel.stringValue = payload.currentRelease.userFacingCoreVersionDescription
        currentSeqLabel.stringValue = payload.currentRelease.userFacingSequenceDescription
        guardSeqLabel.stringValue = payload.guardInfo.userFacingGuardDescription

        releaseTrustLabel.stringValue = payload.currentRelease.userFacingTrustDescription
        if payload.currentRelease.isTrustedInstalledRelease {
            releaseTrustLabel.textColor = .systemGreen
        } else {
            releaseTrustLabel.textColor = .secondaryLabelColor
        }

        architectureLabel.stringValue = payload.currentRelease.architecture.isEmpty ? L10n.t(.statusUnknown) : payload.currentRelease.architecture

        if let c = payload.currentRelease.componentsConsistent {
            if c {
                consistencyBadge.stringValue = L10n.t(.updateConsistencyConsistent)
                consistencyBadge.textColor = .systemGreen
            } else {
                consistencyBadge.stringValue = L10n.t(.updateConsistencyInconsistent)
                consistencyBadge.textColor = .systemRed
            }
        } else {
            consistencyBadge.stringValue = L10n.t(.updateReleaseDescUnknown)
            consistencyBadge.textColor = .secondaryLabelColor
        }

        if payload.guardInfo.maintenanceActive {
            supervisorStatusLabel.stringValue = L10n.t(.updateSupervisorSuppressed)
            supervisorStatusLabel.textColor = .systemOrange
        } else {
            // 真实求实：不处于维护抑制态不等于运行正常
            supervisorStatusLabel.stringValue = L10n.t(.updateSupervisorUnsuppressed)
            supervisorStatusLabel.textColor = .secondaryLabelColor
        }

        // 事务与阶段
        updateTransactionUI(payload.transaction)
    }

    public func applyCheckResult(_ result: UpdateCheckResult) {
        self.currentCheckResult = result

        switch result.status {
        case .unconfigured:
            candidateStatusLabel.stringValue = L10n.t(.updateCandidateSourceUnconfigured)
            candidateStatusLabel.textColor = .systemOrange
            candidateVersionLabel.stringValue = L10n.t(.updateCandidateNoVersion)
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherNoRootKey)
            publisherVerificationBadge.textColor = .secondaryLabelColor
            localSigningLabel.stringValue = L10n.t(.updateSigningForbiddenNoSource)
            compatibilityLabel.stringValue = result.reason ?? L10n.t(.updateCompatibilityDefaultReason)
            releaseNotesView.string = L10n.t(.updateReleaseNotesUnconfigured)

        case .unverifiedTrustRoot:
            candidateStatusLabel.stringValue = L10n.t(.updateCandidateFoundUnverified)
            candidateStatusLabel.textColor = .systemOrange
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? L10n.t(.statusUnknown)
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                } else {
                    candidateSizeLabel.stringValue = L10n.t(.statusUnknown)
                }
                compatibilityLabel.stringValue = cand.compatible ? L10n.t(.updateCompatArchVerified) : L10n.t(.updateCompatIncompatible)
                releaseNotesView.string = cand.releaseNotes ?? L10n.t(.updateNoDetailedNotes)
            }
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherUnverifiedNoKey)
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = L10n.t(.updateSigningNeedVerify)

        case .alreadyInstalled:
            candidateStatusLabel.stringValue = L10n.t(.updateCandidateAlreadyLatest)
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
            }
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherApplied)
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = L10n.t(.updateLocalRunning)
            compatibilityLabel.stringValue = L10n.t(.updateCompatCurrentOk)
            releaseNotesView.string = L10n.t(.updateNoCandidateNotes)

        case .incompatible:
            candidateStatusLabel.stringValue = L10n.t(.updateCandidateIncompatibleEnv)
            candidateStatusLabel.textColor = .systemRed
            candidateVersionLabel.stringValue = "-"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherFailedCompat)
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = L10n.t(.updateSigningForbidden)
            compatibilityLabel.stringValue = result.reason ?? L10n.t(.updateCompatRequirementsNotMet)
            releaseNotesView.string = L10n.t(.updateCannotInstallOnHost)

        case .available:
            candidateStatusLabel.stringValue = L10n.t(.updateCandidateAvailableVerified)
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? "-"
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                }
                compatibilityLabel.stringValue = cand.compatible ? L10n.t(.updateCompatSystemCompatible) : L10n.t(.updateCompatSystemIncompatible)
                releaseNotesView.string = cand.releaseNotes ?? L10n.t(.updateNotesNone)
            }
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherSignatureVerified)
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
            currentStageLabel.stringValue = L10n.t(.updateRecoveryNeededFmt, transaction.stage ?? "Unknown")
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
            currentStageLabel.stringValue = L10n.t(.updateStageIdle)
            currentStageLabel.textColor = .secondaryLabelColor
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = ""
            drainWarningLabel.stringValue = ""

        case .inProgress:
            let stageName = transaction.stage ?? L10n.t(.updateStageExecuting)
            currentStageLabel.stringValue = L10n.t(.updateStageExternalTxFmt, stageName, "external")
            currentStageLabel.textColor = .systemBlue
            if stageName == "Draining" {
                drainWarningLabel.stringValue = L10n.t(.updateDrainWaitingNotice)
            } else {
                drainWarningLabel.stringValue = ""
            }

        case .committed:
            currentStageLabel.stringValue = L10n.t(.updateRecentCommitted)
            currentStageLabel.textColor = .systemGreen
            progressBar.doubleValue = 100.0
            progressDetailLabel.stringValue = L10n.t(.updateRecordApplied)
            drainWarningLabel.stringValue = ""

        case .rolledBack:
            currentStageLabel.stringValue = L10n.t(.updateRollbackRecordedSafe)
            currentStageLabel.textColor = .systemOrange
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = L10n.t(.updateKeepOldRecord)
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
        actionFeedbackLabel.stringValue = L10n.t(.updateCheckingStatus)
        actionFeedbackLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let res = try self.client.checkUpdate()
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.applyCheckResult(res)
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateCheckComplete)
                    self.actionFeedbackLabel.textColor = .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateCheckFailedFmt, error.localizedDescription)
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleRollback() {
        guard let info = currentRollbackStatus, info.canRollback, let ver = info.version else {
            actionFeedbackLabel.stringValue = L10n.t(.updateCannotRollbackFmt, currentRollbackStatus?.localizedReason ?? L10n.t(.updateNoRollbackDetected))
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        let alert = NSAlert()
        alert.messageText = L10n.t(.updateRollbackConfirmTitleFmt, ver)
        alert.informativeText = L10n.t(.updateRollbackConfirmBody)
        alert.alertStyle = .critical

        let checkbox = NSButton(checkboxWithTitle: UpdatePageStrings.rollbackCheckboxPrompt, target: nil, action: nil)
        checkbox.font = NSFont.systemFont(ofSize: 12)
        alert.accessoryView = checkbox
        alert.addButton(withTitle: UpdatePageStrings.rollbackConfirmButton)
        alert.addButton(withTitle: UpdatePageStrings.cancel)

        let resp = alert.runModal()
        guard resp == .alertFirstButtonReturn else { return }
        guard checkbox.state == .on else {
            actionFeedbackLabel.stringValue = L10n.t(.updateRollbackCancelledCheckbox)
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        actionFeedbackLabel.stringValue = L10n.t(.updateRollbackingFmt, ver)
        actionFeedbackLabel.textColor = .systemBlue
        rollbackButton.isEnabled = false

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let res = self.client.executeRollback(expectedVersion: ver)
            DispatchQueue.main.async {
                switch res {
                case .success(let msg):
                    self.actionFeedbackLabel.stringValue = msg
                    self.actionFeedbackLabel.textColor = .systemGreen
                    self.loadRollbackStatus()
                case .failure(let err):
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateRollbackFailedFmt, "\(err)")
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.rollbackButton.isEnabled = true
                }
            }
        }
    }

    /// 关键安全防线：所有写入操作全部重定向到此 handler，严格禁止任何模拟写入或状态修改
    @objc public func handleDisabledWriteAction() {
        actionFeedbackLabel.stringValue = L10n.t(.updateFeatureDisabledNotice)
        actionFeedbackLabel.textColor = .systemOrange
    }

    @objc private func handleRefreshStatus() {
        actionFeedbackLabel.stringValue = L10n.t(.updateRefreshingStatus)
        actionFeedbackLabel.textColor = .systemBlue
        refreshStatusButton.isEnabled = false

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let payload = try self.client.fetchStatus()
                DispatchQueue.main.async {
                    self.applyStatusPayload(payload)
                    self.refreshStatusButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateStatusRefreshed)
                    self.actionFeedbackLabel.textColor = .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.refreshStatusButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateStatusRefreshFailedFmt, error.localizedDescription)
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
        loadRollbackStatus()
    }

    // MARK: - Testing Hooks
    public func applyRollbackStatusResultForTesting(_ result: Result<RollbackStatusInfo, UpdateError>) {
        applyRollbackStatusResult(result)
    }

    public var isRollbackButtonEnabledForTesting: Bool {
        return rollbackButton.isEnabled
    }

    public var rollbackReasonTextForTesting: String {
        return rollbackReasonSideLabel.stringValue
    }

    public var rollbackSuccessMessageForTesting: String {
        return UpdatePageStrings.rollbackSuccess
    }
}
