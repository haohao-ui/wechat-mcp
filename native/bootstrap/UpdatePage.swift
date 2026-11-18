// native/bootstrap/UpdatePage.swift
// Application update management, version verification, drain coordination, and failure recovery.
// Verified downloads are enabled only when the actual updater reports support.
// Installation and deferral stay disabled until production transactions are ready;
// package rollback (restore the previous installed .app) is available through the updater.
import Foundation
import AppKit
import Darwin

/// Bounded updater/control execution for this page. No child or pipe can keep
/// the UI waiting indefinitely after its deadline, and output is memory-bounded.
public func runUpdatePageCommand(executable: String, arguments: [String], timeout: Double) -> ControlRun {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    process.standardInput = FileHandle.nullDevice
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = pipe
    do { try process.run() } catch { return ControlRun(status: -1, output: Data(), timedOut: false) }
    try? pipe.fileHandleForWriting.close()
    let fd = pipe.fileHandleForReading.fileDescriptor
    _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
    let deadline = ProcessInfo.processInfo.systemUptime + max(0.05, timeout)
    var output = Data()
    var eof = false
    var failed = false
    var timedOut = false
    var buffer = [UInt8](repeating: 0, count: 16384)
    while true {
        var count = 0
        repeat {
            count = Darwin.read(fd, &buffer, buffer.count)
            if count > 0 {
                if output.count + count > 4 * 1024 * 1024 { failed = true; break }
                output.append(contentsOf: buffer.prefix(count))
            } else if count == 0 { eof = true }
            else if errno != EAGAIN && errno != EINTR { failed = true }
        } while count > 0 || (count < 0 && errno == EINTR)
        if failed || (!process.isRunning && eof) { break }
        if ProcessInfo.processInfo.systemUptime >= deadline { timedOut = true; break }
        usleep(10_000)
    }
    if process.isRunning {
        process.terminate()
        let grace = ProcessInfo.processInfo.systemUptime + 0.5
        while process.isRunning && ProcessInfo.processInfo.systemUptime < grace { usleep(10_000) }
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        let killDeadline = ProcessInfo.processInfo.systemUptime + 2
        while process.isRunning && ProcessInfo.processInfo.systemUptime < killDeadline { usleep(10_000) }
    }
    try? pipe.fileHandleForReading.close()
    if failed { return ControlRun(status: -1, output: Data(L10n.t(.uiUpdate136).utf8), timedOut: timedOut) }
    return ControlRun(status: process.isRunning ? -1 : process.terminationStatus, output: output, timedOut: timedOut)
}

// MARK: - Update Data Models

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
        return L10n.t(.updateCoreCompiledVersion, String(describing: cv))
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
        case "unknown": stDesc = L10n.t(.modelStateUnknown)
        case "installed": stDesc = L10n.t(.updateStateInstalled)
        case "verified": stDesc = L10n.t(.updateStateVerified)
        case "": stDesc = L10n.t(.updateStateNone)
        default: stDesc = L10n.t(.updateStateUnknownFmt, String(describing: st))
        }

        let srcDesc: String
        switch src.lowercased() {
        case "unknown": srcDesc = L10n.t(.updateSourceUnknown)
        case "signed_package": srcDesc = L10n.t(.updateSourceSignedPackage)
        case "": srcDesc = L10n.t(.updateSourceNone)
        default: srcDesc = L10n.t(.updateSourceUnknownFmt, String(describing: src))
        }

        if isTrustedInstalledRelease {
            return "\(stDesc) (\(srcDesc))"
        } else {
            return L10n.t(.uiUpdate137, String(describing: stDesc), String(describing: srcDesc))
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
            return L10n.t(.uiUpdate138, String(describing: s))
        }
        return L10n.t(.updateNullValue)
    }
}

public struct UpdateCachedCandidate: Codable, Equatable {
    public let candidateId: String
    public let releaseId: String
    public let sequence: UInt64
    public let manifestSha256: String
    enum CodingKeys: String, CodingKey {
        case sequence
        case candidateId = "candidate_id"
        case releaseId = "release_id"
        case manifestSha256 = "manifest_sha256"
    }
}

public struct UpdateDownloadCache: Codable, Equatable {
    public let status: String
    public let candidates: [UpdateCachedCandidate]
}

public struct UpdateStatusPayload: Codable, Equatable {
    public let currentRelease: UpdateReleaseInfo
    public let candidateRelease: UpdateCandidateSummary?
    public let transaction: UpdateTransactionInfo
    public let guardInfo: UpdateGuardInfo
    public let downloadCache: UpdateDownloadCache?

    enum CodingKeys: String, CodingKey {
        case currentRelease = "current_release"
        case candidateRelease = "candidate_release"
        case transaction
        case guardInfo = "guard"
        case downloadCache = "download_cache"
    }

    public init(
        currentRelease: UpdateReleaseInfo,
        candidateRelease: UpdateCandidateSummary? = nil,
        transaction: UpdateTransactionInfo,
        guardInfo: UpdateGuardInfo,
        downloadCache: UpdateDownloadCache? = nil
    ) {
        self.currentRelease = currentRelease
        self.candidateRelease = candidateRelease
        self.transaction = transaction
        self.guardInfo = guardInfo
        self.downloadCache = downloadCache
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
        if !container.contains(.publishedAt) {
            self.publishedAt = nil
        } else if try container.decodeNil(forKey: .publishedAt) {
            self.publishedAt = nil
        } else if let text = try? container.decode(String.self, forKey: .publishedAt) {
            self.publishedAt = text
        } else {
            let timestamp = try container.decode(UInt64.self, forKey: .publishedAt)
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "zh_CN")
            formatter.timeZone = TimeZone(secondsFromGMT: 0)
            formatter.dateFormat = "yyyy-MM-dd HH:mm 'UTC'"
            self.publishedAt = formatter.string(from: Date(timeIntervalSince1970: TimeInterval(timestamp)))
        }
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
    public let manifestSha256: String?

    enum CodingKeys: String, CodingKey {
        case status, reason, candidate, verification
        case manifestSha256 = "manifest_sha256"
    }

    public init(
        status: UpdateCheckStatus,
        reason: String? = nil,
        candidate: UpdateCandidateDetail? = nil,
        verification: UpdateVerificationInfo? = nil,
        manifestSha256: String? = nil
    ) {
        self.status = status
        self.reason = reason
        self.candidate = candidate
        self.verification = verification
        self.manifestSha256 = manifestSha256
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
            return L10n.t(.updateSigningUnavailableFmt, String(describing: err))
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
        case .networkOffline(let msg): return L10n.t(.updateErrNetworkOfflineFmt, String(describing: msg))
        case .commandFailed(let code, let msg): return L10n.t(.uiUpdate139, String(describing: code), String(describing: msg))
        case .parseError(let msg): return L10n.t(.updateErrParseFmt, String(describing: msg))
        }
    }
}

// MARK: - Rollback copy (暂用中文硬编码，后续统一迁入 L10n)
//
// 说明：这些文案只描述界面行为与更新器返回的应用自有原因，任何失败路径都不得
// 拼接命令原始输出、密钥或第三方原文。
private enum RollbackStrings {
    static let sectionTitle = L10n.t(.updatePageSectionRollback)
    static let notice = L10n.t(.updateRollbackNotice)
    static let labelVersion = L10n.t(.updateLabelRollbackVersion)
    static let labelEligibility = L10n.t(.updateLabelRollbackEligibility)
    static let buttonTitle = L10n.t(.uiUpdate140)
    static let buttonTooltip = L10n.t(.uiUpdate141)
    static let querying = L10n.t(.uiUpdate142)
    static let unknown = L10n.t(.statusUnknown)
    static let noPreviousVersion = L10n.t(.updatePageNoPreviousVersion)
    static let noDetectedVersion = L10n.t(.updateNoRollbackDetected)
    static let unavailable = L10n.t(.updatePageRollbackUnavailable)
    static let timeout = L10n.t(.updateRollbackTimeout)
    static let noOutput = L10n.t(.updateRollbackNoOutput)
    static let success = L10n.t(.updatePageRollbackSuccess)
    static let confirmButton = L10n.t(.brainPageRollbackConfirmButton)
    static let cancel = L10n.t(.cancel)
    static let confirmCheckbox = L10n.t(.updatePageRollbackCheckboxPrompt)
    static let cancelled = L10n.t(.brainRollbackCancelledCheckbox)
    static let eligible = L10n.t(.updateReasonSafe)
    static let notEligible = L10n.t(.updateReasonCannot)
    static let installedAtPrefix = L10n.t(.uiUpdate143)
    static let noMatchingCompleted = L10n.t(.updateReasonNoMatching)
    static let previousCorrupt = L10n.t(.updateReasonPrevCorrupt)
    static let backupCorrupt = L10n.t(.updateReasonBackupCorrupt)
    static let notSupportedByRelease = L10n.t(.updateReasonNotSupported)
    static let schemaChanged = L10n.t(.updateReasonSchemaChanged)
    static let checkpointRequired = L10n.t(.updateReasonCheckpointRequired)
    static let stopCoreFirst = L10n.t(.updateReasonStopCoreFirst)
    static let recoveryRequired = L10n.t(.updateReasonRecoveryRequired)
    static let installRootUnconfigured = L10n.t(.updateReasonInstallRootUnconfigured)
    static let rollbackStateInvalid = L10n.t(.updateReasonRollbackStateInvalid)

    static func confirmTitle(_ version: String) -> String {
        return L10n.t(.updateRollbackConfirmTitleFmt, String(describing: version))
    }

    static let confirmBody = L10n.t(.updateRollbackConfirmBody)

    static func inProgress(_ version: String) -> String {
        return L10n.t(.updateRollbackingFmt, String(describing: version))
    }

    static func failed(_ reason: String) -> String {
        return L10n.t(.updateRollbackFailedFmt, String(describing: reason))
    }

    static func cannotRollback(_ reason: String) -> String {
        return L10n.t(.uiUpdate144, String(describing: reason))
    }

    static func exitCode(_ code: Int32) -> String {
        return L10n.t(.uiUpdate145, String(describing: code))
    }

    /// 更新器返回的原因是应用自有文案（见 updater/app_rollback.rs）；这里再做一次
    /// 去掉控制字符并限长，避免任何原始输出原样进入界面。
    static func safeReasonText(_ raw: String) -> String {
        let cleaned = raw.components(separatedBy: CharacterSet.controlCharacters).joined()
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleaned.isEmpty else { return notEligible }
        return cleaned.count > 160 ? String(cleaned.prefix(160)) + "…" : cleaned
    }
}

// MARK: - Rollback Data Models

public struct UpdateError: Error, CustomStringConvertible, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var description: String { message }
}

/// “整包回滚到上一版本”状态，字段与 `wechat-updater rollback-status` 的 JSON 契约一致。
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

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.version = try container.decodeIfPresent(String.self, forKey: .version)
        self.installedAt = try container.decodeIfPresent(Double.self, forKey: .installedAt)
        self.canRollback = (try container.decodeIfPresent(Bool.self, forKey: .canRollback)) ?? false
        self.reason = try container.decodeIfPresent(String.self, forKey: .reason)
    }

    /// 面向用户的原因说明：已知稳定码映射为固定中文；其余按应用自有文案清洗后展示。
    public var localizedReason: String {
        guard let raw = reason?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty else {
            return canRollback ? RollbackStrings.eligible : RollbackStrings.notEligible
        }
        switch raw {
        case "no_previous_version": return RollbackStrings.noPreviousVersion
        case "no_matching_completed_update": return RollbackStrings.noMatchingCompleted
        case "previous_release_corrupt": return RollbackStrings.previousCorrupt
        case "backup_corrupt": return RollbackStrings.backupCorrupt
        case "rollback_not_supported_by_release": return RollbackStrings.notSupportedByRelease
        case "database_schema_changed": return RollbackStrings.schemaChanged
        case "database_checkpoint_required": return RollbackStrings.checkpointRequired
        case "stop_core_first": return RollbackStrings.stopCoreFirst
        case "recovery_required": return RollbackStrings.recoveryRequired
        case "install_root_unconfigured": return RollbackStrings.installRootUnconfigured
        case "rollback_state_invalid": return RollbackStrings.rollbackStateInvalid
        default: return RollbackStrings.safeReasonText(raw)
        }
    }

    public var formattedInstalledAt: String {
        guard let ts = installedAt, ts > 0 else { return RollbackStrings.unknown }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "zh_CN")
        formatter.timeZone = TimeZone.current
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(ts)))
    }
}

// MARK: - Update Page Pure Logic

public enum UpdatePageLogic {
    public static func parseStatusPayload(jsonString: String) throws -> UpdateStatusPayload {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError(L10n.t(.uiUpdate146))
        }
        let decoder = JSONDecoder()
        return try decoder.decode(UpdateStatusPayload.self, from: data)
    }

    public static func parseCheckPayload(jsonString: String) throws -> UpdateCheckResult {
        guard let data = jsonString.data(using: .utf8) else {
            throw UpdateClientError.parseError(L10n.t(.uiUpdate146))
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
        let stage = transaction.stage ?? L10n.t(.uiUpdate147)
        let err = transaction.error ?? L10n.t(.brainErrorUnknown)
        let tx = transaction.transactionId ?? L10n.t(.uiUpdate148)

        return L10n.t(.uiResidualUpdateRecoveryGuide, String(describing: tx), String(describing: stage), String(describing: err))
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

public struct UpdateDownloadReceipt: Decodable, Equatable {
    public let status: String
    public let candidateId: String
    public let releaseId: String
    public let sequence: UInt64
    public let manifestSha256: String
    enum CodingKeys: String, CodingKey {
        case status, sequence
        case candidateId = "candidate_id"
        case releaseId = "release_id"
        case manifestSha256 = "manifest_sha256"
    }
}

private struct UpdaterCapabilityResponse: Decodable {
    struct Capabilities: Decodable { let schema: UInt32; let download: Bool }
    let status: String
    let maintenanceActive: Bool?
    let capabilities: Capabilities?
    enum CodingKeys: String, CodingKey {
        case status, capabilities
        case maintenanceActive = "maintenance_active"
    }
    var canDownload: Bool {
        capabilities?.schema == 1 && capabilities?.download == true && maintenanceActive == false && ["idle", "committed"].contains(status)
    }
}

// MARK: - Update Client

public final class UpdateClient {
    public typealias CommandExecutor = (String, [String], Double) -> ControlRun

    private let configPath: String
    private let executor: CommandExecutor

    public init(configPath: String? = nil, executor: @escaping CommandExecutor = runUpdatePageCommand) {
        self.configPath = configPath ?? (FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".WeChatRPABot/config.json").path)
        self.executor = executor
    }

    public func fetchStatus() throws -> UpdateStatusPayload {
        let args = ["control", "update-status", "--config", configPath]
        let run = executor(controlBinaryPath(), args, 10.0)
        guard run.status == 0 && !run.timedOut else {
            let errStr = String(data: run.output, encoding: .utf8) ?? L10n.t(.brainErrorUnknown)
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
        guard run.status == 0 && !run.timedOut else {
            let errStr = String(data: run.output, encoding: .utf8) ?? L10n.t(.brainErrorUnknown)
            throw UpdateClientError.commandFailed(code: run.status, message: errStr)
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try UpdatePageLogic.parseCheckPayload(jsonString: outStr)
    }
    private func updaterPath() throws -> String {
        let path = URL(fileURLWithPath: controlBinaryPath()).deletingLastPathComponent().appendingPathComponent("wechat-updater").path
        var file = stat()
        var parent = stat()
        let directory = (path as NSString).deletingLastPathComponent
        guard lstat(path, &file) == 0, file.st_mode & S_IFMT == S_IFREG,
              file.st_uid == geteuid(), file.st_mode & 0o022 == 0,
              lstat(directory, &parent) == 0, parent.st_mode & S_IFMT == S_IFDIR,
              parent.st_uid == geteuid(), parent.st_mode & 0o022 == 0,
              FileManager.default.isExecutableFile(atPath: path) else {
            throw UpdateClientError.parseError(L10n.t(.uiUpdate149))
        }
        return path
    }

    public func supportsDownload() throws -> Bool {
        let run = executor(try updaterPath(), ["status", "--config", configPath], 5)
        guard run.status == 0 && !run.timedOut else { return false }
        return (try JSONDecoder().decode(UpdaterCapabilityResponse.self, from: run.output)).canDownload
    }

    public func downloadUpdate(expected: UpdateCheckResult) throws -> UpdateDownloadReceipt {
        guard expected.status == .available, expected.verification?.publisherVerified == true,
              let digest = expected.manifestSha256, digest.count == 64,
              digest.allSatisfy({ $0.isHexDigit }), let candidate = expected.candidate,
              try supportsDownload() else {
            throw UpdateClientError.parseError(L10n.t(.uiUpdate150))
        }
        let run = executor(try updaterPath(), ["download", "--config", configPath], 660)
        guard run.status == 0 && !run.timedOut else {
            let text = String(data: run.output, encoding: .utf8) ?? ""
            let message = text.contains("download_cache_quota_exceeded") ? L10n.t(.uiUpdate151) : text.contains("concurrent_download_in_progress") ? L10n.t(.uiUpdate152) : L10n.t(.uiUpdate153)
            throw UpdateClientError.commandFailed(code: run.status, message: message)
        }
        let receipt = try JSONDecoder().decode(UpdateDownloadReceipt.self, from: run.output)
        guard receipt.status == "downloaded", UUID(uuidString: receipt.candidateId)?.uuidString.lowercased() == receipt.candidateId,
              receipt.releaseId == candidate.releaseId, receipt.sequence == candidate.sequence,
              receipt.manifestSha256 == digest else {
            throw UpdateClientError.parseError(L10n.t(.uiUpdate154))
        }
        return receipt
    }

    /// 查询「整包回滚到上一版本」是否可用。失败一律收敛为固定文案，
    /// 不向上传递命令原始输出或第三方错误文本。
    public func fetchRollbackStatus() -> Result<RollbackStatusInfo, UpdateError> {
        let executable: String
        do { executable = try updaterPath() } catch {
            return .failure(UpdateError(RollbackStrings.unavailable))
        }
        let run = executor(executable, ["rollback-status", "--config", configPath], 10.0)
        guard !run.timedOut, run.status == 0 else {
            return .failure(UpdateError(RollbackStrings.unavailable))
        }
        do {
            return .success(try JSONDecoder().decode(RollbackStatusInfo.self, from: run.output))
        } catch {
            return .failure(UpdateError(RollbackStrings.unavailable))
        }
    }

    /// 执行整包回滚：由更新器直接换回上一个已安装的 .app。
    /// `expectedVersion` 取自已展示的回滚状态，防止查询后状态漂移导致误回滚。
    public func executeRollback(expectedVersion: String) -> Result<String, UpdateError> {
        guard !expectedVersion.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return .failure(UpdateError(RollbackStrings.unavailable))
        }
        let executable: String
        do { executable = try updaterPath() } catch {
            return .failure(UpdateError(RollbackStrings.unavailable))
        }
        let args = ["rollback", "--config", configPath, "--expected-version", expectedVersion]
        let run = executor(executable, args, 30.0)
        guard !run.timedOut else { return .failure(UpdateError(RollbackStrings.timeout)) }
        guard run.status == 0 else { return .failure(UpdateError(RollbackStrings.exitCode(run.status))) }
        guard !run.output.isEmpty else { return .failure(UpdateError(RollbackStrings.noOutput)) }
        return .success(RollbackStrings.success)
    }

}

// MARK: - Update Page View Controller

public final class UpdatePageController: NSViewController {
    private let context: StatusPanelController
    private var client: UpdateClient

    // 页面状态（只读）
    private var currentStatusPayload: UpdateStatusPayload?
    private var currentCheckResult: UpdateCheckResult?
    private var downloadSupported = false
    private var isDownloading = false
    private var downloadedReceipt: UpdateDownloadReceipt?
    private var signingStatus = UpdateSigningStatus(keychainAvailable: true, localCertificateFound: true)
    private var isChecking = false
    // 回滚入口状态
    private var currentRollbackStatus: RollbackStatusInfo?
    private var isRollingBack = false

    // UI - 当前版本卡片 (全部使用 wrappingLabelWithString 并设受限宽度)
    private let currentReleaseLabel = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate155))
    private let coreVersionLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnknown))
    private let currentSeqLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateSeqUnknown))
    private let guardSeqLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateNullValue))
    private let releaseTrustLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnverified))
    private let architectureLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusUnknown))
    private let consistencyBadge = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate156))
    private let supervisorStatusLabel = NSTextField(wrappingLabelWithString: L10n.t(.statusChecking))

    // UI - 候选更新卡片
    private let candidateStatusLabel = NSTextField(wrappingLabelWithString: L10n.t(.updateCandidateNotCheckedYet))
    private let candidateVersionLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidatePublishedLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidateSizeLabel = NSTextField(wrappingLabelWithString: "-")
    private let publisherVerificationBadge = NSTextField(wrappingLabelWithString: "-")
    private let localSigningLabel = NSTextField(wrappingLabelWithString: "-")
    private let compatibilityLabel = NSTextField(wrappingLabelWithString: "-")
    private let releaseNotesView = NSTextView()
    private let candidateEmptyLabel = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate157))
    private let candidateDetailsContainer = NSStackView()
    private let readOnlyBadge = NSTextField(labelWithString: L10n.t(.uiUpdate158))

    // UI - 流程流水线与进度
    private let pipelineStepsLabel = NSTextField(wrappingLabelWithString: L10n.t(.updatePipelineSteps))
    private let currentStageLabel = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate159))
    private let progressBar = NSProgressIndicator()
    private let progressDetailLabel = NSTextField(wrappingLabelWithString: "")
    private let drainWarningLabel = NSTextField(wrappingLabelWithString: "")
    private let permissionsNoteLabel = NSTextField(wrappingLabelWithString: L10n.t(.updatePermissionsNotice))

    // UI - 操作按钮 (只读阶段全部禁用写操作，分行排布)
    private let checkButton = NSButton(title: L10n.t(.uiUpdate160), target: nil, action: nil)
    private let refreshStatusButton = NSButton(title: L10n.t(.uiUpdate161), target: nil, action: nil)
    private let downloadButton = NSButton(title: L10n.t(.uiUpdate162), target: nil, action: nil)
    private let installButton = NSButton(title: L10n.t(.uiUpdate163), target: nil, action: nil)
    private let deferButton = NSButton(title: L10n.t(.uiUpdate164), target: nil, action: nil)
    private let writeControlsNote = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate165))
    private let actionFeedbackLabel = NSTextField(wrappingLabelWithString: L10n.t(.uiUpdate166))

    // UI - 回滚到上一版本（整包换回上一个 .app，无可用备份时禁用并写明原因）
    private let rollbackSection = NSStackView()
    private let previousVersionLabel = NSTextField(wrappingLabelWithString: RollbackStrings.querying)
    private let rollbackEligibilityLabel = NSTextField(wrappingLabelWithString: RollbackStrings.querying)
    private let rollbackButton = NSButton(title: RollbackStrings.buttonTitle, target: nil, action: nil)
    private let rollbackReasonSideLabel = NSTextField(wrappingLabelWithString: "")

    // UI - 恢复卡片 (当出现 recovery_required 时高亮)
    private let recoveryBox = NSBox()
    private let technicalDetails = NSStackView()
    private let transactionDetails = NSStackView()
    private let detailsButton = NSButton(checkboxWithTitle: L10n.t(.uiUpdate167), target: nil, action: nil)
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
        loadRollbackStatus()
    }

    private func setupUI() {
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = 20
        contentStack.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scroll.documentView = document
        view.addSubview(scroll)

        checkButton.target = self
        checkButton.action = #selector(handleCheckUpdate)
        refreshStatusButton.target = self
        refreshStatusButton.action = #selector(handleRefreshStatus)
        let queries = NSStackView(views: [checkButton, refreshStatusButton])
        queries.spacing = 12
        contentStack.addArrangedSubview(queries)

        let card = createSectionBox(title: L10n.t(.uiUpdate168))
        let body = NSStackView()
        body.orientation = .vertical
        body.alignment = .leading
        body.spacing = 16
        currentReleaseLabel.font = NSFont.systemFont(ofSize: 22, weight: .semibold)
        body.addArrangedSubview(currentReleaseLabel)
        candidateStatusLabel.font = NSFont.systemFont(ofSize: 15, weight: .semibold)
        body.addArrangedSubview(candidateStatusLabel)
        candidateEmptyLabel.textColor = .secondaryLabelColor
        body.addArrangedSubview(candidateEmptyLabel)

        candidateDetailsContainer.orientation = .vertical
        candidateDetailsContainer.alignment = .leading
        candidateDetailsContainer.spacing = 10
        let summary = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.updateLabelTargetVersion)), candidateVersionLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate169)), candidatePublishedLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate170)), compatibilityLabel]
        ])
        summary.rowSpacing = 8
        summary.columnSpacing = 16
        candidateDetailsContainer.addArrangedSubview(summary)
        let notes = NSScrollView()
        notes.translatesAutoresizingMaskIntoConstraints = false
        notes.hasVerticalScroller = true
        notes.drawsBackground = false
        releaseNotesView.isEditable = false
        releaseNotesView.font = NSFont.systemFont(ofSize: 13)
        releaseNotesView.string = L10n.t(.updateReleaseNotesEmpty)
        releaseNotesView.isVerticallyResizable = true
        releaseNotesView.autoresizingMask = [.width]
        releaseNotesView.textContainer?.widthTracksTextView = true
        notes.documentView = releaseNotesView
        candidateDetailsContainer.addArrangedSubview(notes)
        notes.heightAnchor.constraint(equalToConstant: 110).isActive = true
        notes.widthAnchor.constraint(equalTo: candidateDetailsContainer.widthAnchor).isActive = true
        candidateDetailsContainer.isHidden = true
        body.addArrangedSubview(candidateDetailsContainer)

        // Keep the existing read-only capability boundary; never simulate an update.
        downloadButton.target = self
        installButton.target = self
        deferButton.target = self
        for button in [downloadButton, installButton, deferButton] {
            button.action = #selector(handleDisabledWriteAction)
            button.isEnabled = false
            button.toolTip = L10n.t(.uiUpdate171)
        }
        downloadButton.action = #selector(handleDownloadUpdate)
        let writes = NSStackView(views: [downloadButton])
        writes.spacing = 12
        body.addArrangedSubview(writes)
        actionFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        actionFeedbackLabel.textColor = .secondaryLabelColor
        body.addArrangedSubview(actionFeedbackLabel)

        // 回滚入口：整包换回上一个已安装的 .app；没有可用备份时按钮禁用并写明原因
        rollbackSection.orientation = .vertical
        rollbackSection.alignment = .leading
        rollbackSection.spacing = 10
        let rollbackTitleLabel = NSTextField(labelWithString: RollbackStrings.sectionTitle)
        rollbackTitleLabel.font = NSFont.systemFont(ofSize: 13, weight: .bold)
        rollbackSection.addArrangedSubview(rollbackTitleLabel)
        let rollbackNoticeLabel = NSTextField(wrappingLabelWithString: RollbackStrings.notice)
        rollbackNoticeLabel.font = NSFont.systemFont(ofSize: 11)
        rollbackNoticeLabel.textColor = .secondaryLabelColor
        rollbackSection.addArrangedSubview(rollbackNoticeLabel)

        previousVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        rollbackEligibilityLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        let rollbackGrid = NSGridView(views: [
            [NSTextField(labelWithString: RollbackStrings.labelVersion), previousVersionLabel],
            [NSTextField(labelWithString: RollbackStrings.labelEligibility), rollbackEligibilityLabel]
        ])
        rollbackGrid.rowSpacing = 8
        rollbackGrid.columnSpacing = 16
        rollbackSection.addArrangedSubview(rollbackGrid)

        rollbackButton.target = self
        rollbackButton.action = #selector(handleRollback)
        rollbackButton.isEnabled = false
        rollbackButton.toolTip = RollbackStrings.buttonTooltip
        rollbackReasonSideLabel.font = NSFont.systemFont(ofSize: 11)
        rollbackReasonSideLabel.textColor = .systemOrange
        rollbackReasonSideLabel.lineBreakMode = .byTruncatingTail
        rollbackReasonSideLabel.maximumNumberOfLines = 1
        let rollbackRow = NSStackView(views: [rollbackButton, rollbackReasonSideLabel])
        rollbackRow.orientation = .horizontal
        rollbackRow.spacing = 12
        rollbackRow.alignment = .centerY
        rollbackSection.addArrangedSubview(rollbackRow)
        body.addArrangedSubview(rollbackSection)

        transactionDetails.orientation = .vertical
        transactionDetails.alignment = .leading
        transactionDetails.spacing = 8
        transactionDetails.addArrangedSubview(currentStageLabel)
        progressBar.minValue = 0
        progressBar.maxValue = 100
        progressBar.widthAnchor.constraint(equalToConstant: 280).isActive = true
        transactionDetails.addArrangedSubview(progressBar)
        transactionDetails.addArrangedSubview(progressDetailLabel)
        transactionDetails.addArrangedSubview(drainWarningLabel)
        transactionDetails.isHidden = true
        body.addArrangedSubview(transactionDetails)

        recoveryBox.title = L10n.t(.uiUpdate172)
        recoveryDetailsLabel.textColor = .systemRed
        recoveryBox.setContent(recoveryDetailsLabel)
        recoveryBox.isHidden = true
        body.addArrangedSubview(recoveryBox)

        detailsButton.target = self
        detailsButton.action = #selector(toggleTechnicalDetails)
        body.addArrangedSubview(detailsButton)
        technicalDetails.orientation = .vertical
        technicalDetails.alignment = .leading
        technicalDetails.spacing = 10
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.uiUpdate173)), coreVersionLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate174)), currentSeqLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate175)), guardSeqLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate176)), releaseTrustLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate177)), architectureLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate178)), consistencyBadge],
            [NSTextField(labelWithString: L10n.t(.uiUpdate179)), supervisorStatusLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate180)), candidateSizeLabel],
            [NSTextField(labelWithString: L10n.t(.uiUpdate181)), publisherVerificationBadge],
            [NSTextField(labelWithString: L10n.t(.uiUpdate182)), localSigningLabel]
        ])
        grid.rowSpacing = 8
        grid.columnSpacing = 16
        technicalDetails.addArrangedSubview(grid)
        let unavailableActions = NSStackView(views: [installButton, deferButton])
        unavailableActions.spacing = 12
        technicalDetails.addArrangedSubview(unavailableActions)
        technicalDetails.addArrangedSubview(pipelineStepsLabel)
        technicalDetails.addArrangedSubview(permissionsNoteLabel)
        technicalDetails.addArrangedSubview(writeControlsNote)
        technicalDetails.isHidden = true
        body.addArrangedSubview(technicalDetails)
        card.setContent(body, inset: 16)
        contentStack.addArrangedSubview(card)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            document.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scroll.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 8),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24),
            card.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            candidateDetailsContainer.widthAnchor.constraint(equalTo: body.widthAnchor),
            technicalDetails.widthAnchor.constraint(equalTo: body.widthAnchor),
            rollbackSection.widthAnchor.constraint(equalTo: body.widthAnchor)
        ])
    }

    @objc private func toggleTechnicalDetails() {
        technicalDetails.isHidden = detailsButton.state != .on
    }

    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)
        box.boxType = .primary
        box.contentViewMargins = NSSize(width: 12, height: 10)
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    // MARK: - Status Loading & Presentation

    public func loadInitialStatus() {
        do {
            let payload = try client.fetchStatus()
            applyStatusPayload(payload)
        } catch {
            actionFeedbackLabel.stringValue = L10n.t(.updateStatusFetchNoticeFmt, String(describing: error.localizedDescription))
            actionFeedbackLabel.textColor = .secondaryLabelColor
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
                consistencyBadge.stringValue = L10n.t(.uiUpdate183)
                consistencyBadge.textColor = .systemRed
            }
        } else {
            consistencyBadge.stringValue = L10n.t(.uiUpdate156)
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
        refreshDownloadState()
    }

    public func applyCheckResult(_ result: UpdateCheckResult) {
        if (result.status == .available || result.status == .alreadyInstalled),
           result.verification?.publisherVerified != true {
            applyCheckResult(UpdateCheckResult(status: .unverifiedTrustRoot,
                reason: L10n.t(.uiUpdate184), candidate: result.candidate,
                verification: result.verification))
            return
        }
        self.currentCheckResult = result

        let showsCandidateDetails = result.status != .unconfigured
        candidateDetailsContainer.isHidden = !showsCandidateDetails
        candidateEmptyLabel.isHidden = showsCandidateDetails
        let friendlyReason = friendlyCheckReason(result.reason)
        if case .unconfigured = result.status {
            candidateEmptyLabel.stringValue = friendlyReason
        }

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
            compatibilityLabel.stringValue = friendlyReason
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
            candidateStatusLabel.stringValue = L10n.t(.uiUpdate185)
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
            }
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = L10n.t(.uiUpdate186)
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = L10n.t(.uiUpdate187)
            compatibilityLabel.stringValue = L10n.t(.uiUpdate188)
            releaseNotesView.string = L10n.t(.updateNoCandidateNotes)

        case .incompatible:
            candidateStatusLabel.stringValue = L10n.t(.uiUpdate189)
            candidateStatusLabel.textColor = .systemRed
            candidateVersionLabel.stringValue = "-"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = L10n.t(.uiUpdate190)
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = L10n.t(.updateSigningForbidden)
            compatibilityLabel.stringValue = result.reason ?? L10n.t(.updateCompatRequirementsNotMet)
            releaseNotesView.string = L10n.t(.updateCannotInstallOnHost)

        case .available:
            candidateStatusLabel.stringValue = L10n.t(.uiUpdate191)
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? "-"
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                }
                compatibilityLabel.stringValue = cand.compatible ? L10n.t(.uiUpdate192) : L10n.t(.updateCompatSystemIncompatible)
                releaseNotesView.string = cand.releaseNotes ?? L10n.t(.updateNotesNone)
            }
            publisherVerificationBadge.stringValue = L10n.t(.updatePublisherSignatureVerified)
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = L10n.t(.uiUpdate193)
        }

        refreshDownloadState()
        installButton.isEnabled = false
        deferButton.isEnabled = false
    }

    private func refreshDownloadState() {
        let safeStatus = currentStatusPayload.map { !$0.guardInfo.maintenanceActive && !$0.transaction.recoveryRequired && [.idle, .committed].contains($0.transaction.status) } ?? false
        let verified = currentCheckResult?.status == .available && currentCheckResult?.verification?.publisherVerified == true
        let digest = currentCheckResult?.manifestSha256
        let validDigest = digest?.count == 64 && digest?.allSatisfy({ $0.isHexDigit }) == true
        let downloaded = downloadedReceipt != nil && downloadedReceipt?.manifestSha256 == digest
        let cached = currentStatusPayload?.downloadCache?.status == "cached_not_revalidated" && currentStatusPayload?.downloadCache?.candidates.contains(where: { $0.manifestSha256 == digest }) == true
        downloadButton.isEnabled = downloadSupported && safeStatus && verified && validDigest && !isChecking && !isDownloading && !downloaded
        downloadButton.title = isDownloading ? L10n.t(.uiUpdate194) : downloaded ? L10n.t(.uiUpdate195) : (cached && verified) ? L10n.t(.uiUpdate196) : L10n.t(.uiUpdate162)
        if downloaded && currentCheckResult?.status == .available {
            compatibilityLabel.stringValue = L10n.t(.uiUpdate197)
            localSigningLabel.stringValue = L10n.t(.uiUpdate198)
        } else if cached && verified {
            compatibilityLabel.stringValue = L10n.t(.uiUpdate199)
            localSigningLabel.stringValue = L10n.t(.uiUpdate200)
        }
        downloadButton.toolTip = downloadSupported ? L10n.t(.uiUpdate201) : L10n.t(.uiUpdate202)
        checkButton.isEnabled = !isChecking && !isDownloading
        refreshStatusButton.isEnabled = !isDownloading
        if downloadSupported {
            readOnlyBadge.stringValue = L10n.t(.uiUpdate203)
            writeControlsNote.stringValue = L10n.t(.uiUpdate204)
        } else {
            readOnlyBadge.stringValue = L10n.t(.uiUpdate205)
            writeControlsNote.stringValue = L10n.t(.uiUpdate206)
        }
    }

    private func friendlyCheckReason(_ reason: String?) -> String {
        guard let reason, !reason.isEmpty else {
            return L10n.t(.uiUpdate207)
        }
        if reason == "distribution_source_not_configured" {
            return L10n.t(.uiUpdate208)
        }
        return reason
    }

    private func updateTransactionUI(_ transaction: UpdateTransactionInfo) {
        transactionDetails.isHidden = transaction.status == .idle && !transaction.recoveryRequired
        progressBar.isIndeterminate = transaction.status == .inProgress
        if progressBar.isIndeterminate { progressBar.startAnimation(nil) } else { progressBar.stopAnimation(nil) }
        if transaction.recoveryRequired {
            currentStageLabel.stringValue = L10n.t(.updateRecoveryNeededFmt, String(describing: transaction.stage ?? "Unknown"))
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
            currentStageLabel.stringValue = L10n.t(.uiUpdate209)
            currentStageLabel.textColor = .secondaryLabelColor
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = ""
            drainWarningLabel.stringValue = ""

        case .inProgress:
            let stageName = transaction.stage ?? L10n.t(.updateStageExecuting)
            currentStageLabel.stringValue = L10n.t(.uiUpdate210, String(describing: stageName))
            currentStageLabel.textColor = .systemBlue
            if stageName == "Draining" {
                drainWarningLabel.stringValue = L10n.t(.uiUpdate211)
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

    // MARK: - Rollback (整包回滚到上一版本)

    /// 异步查询回滚可用性；结果只写入回滚卡片，异常不外抛。
    private func loadRollbackStatus() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let result = self.client.fetchRollbackStatus()
            DispatchQueue.main.async {
                self.applyRollbackStatusResult(result)
            }
        }
    }

    /// 把回滚查询结果映射到卡片：可用时启用按钮，不可用/失败时禁用并写明原因。
    private func applyRollbackStatusResult(_ result: Result<RollbackStatusInfo, UpdateError>) {
        switch result {
        case .success(let info):
            currentRollbackStatus = info
            if let version = info.version?.trimmingCharacters(in: .whitespacesAndNewlines), !version.isEmpty {
                previousVersionLabel.stringValue = version
                previousVersionLabel.textColor = .labelColor
                // 安装时间不可信（整包备份没有可靠的安装回执），界面上不展示，也不放进 tooltip。
                previousVersionLabel.toolTip = nil
            } else {
                previousVersionLabel.stringValue = RollbackStrings.noPreviousVersion
                previousVersionLabel.textColor = .secondaryLabelColor
                previousVersionLabel.toolTip = nil
            }
            let reasonText = info.localizedReason
            rollbackEligibilityLabel.stringValue = reasonText
            if info.canRollback && !isRollingBack {
                rollbackEligibilityLabel.textColor = .systemGreen
                rollbackReasonSideLabel.stringValue = ""
                rollbackReasonSideLabel.toolTip = nil
                rollbackButton.isEnabled = true
            } else {
                rollbackEligibilityLabel.textColor = .systemOrange
                rollbackReasonSideLabel.stringValue = reasonText
                rollbackReasonSideLabel.textColor = .systemOrange
                rollbackReasonSideLabel.toolTip = reasonText
                rollbackButton.isEnabled = false
            }
        case .failure(let error):
            currentRollbackStatus = nil
            previousVersionLabel.stringValue = RollbackStrings.unknown
            previousVersionLabel.textColor = .secondaryLabelColor
            previousVersionLabel.toolTip = nil
            rollbackEligibilityLabel.stringValue = error.message
            rollbackEligibilityLabel.textColor = .secondaryLabelColor
            rollbackReasonSideLabel.stringValue = error.message
            rollbackReasonSideLabel.textColor = .secondaryLabelColor
            rollbackReasonSideLabel.toolTip = error.message
            rollbackButton.isEnabled = false
        }
    }

    @objc private func handleRollback() {
        guard !isRollingBack else { return }
        guard let info = currentRollbackStatus, info.canRollback,
              let version = info.version?.trimmingCharacters(in: .whitespacesAndNewlines),
              !version.isEmpty else {
            let reasonText = currentRollbackStatus?.localizedReason ?? RollbackStrings.noDetectedVersion
            rollbackReasonSideLabel.stringValue = reasonText
            rollbackReasonSideLabel.textColor = .systemOrange
            rollbackButton.isEnabled = false
            actionFeedbackLabel.stringValue = RollbackStrings.cannotRollback(reasonText)
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        // 二次确认：破坏性整包替换，必须显式勾选后才执行
        let alert = NSAlert()
        alert.messageText = RollbackStrings.confirmTitle(version)
        alert.informativeText = RollbackStrings.confirmBody
        alert.alertStyle = .critical
        let checkbox = NSButton(checkboxWithTitle: RollbackStrings.confirmCheckbox, target: nil, action: nil)
        checkbox.font = NSFont.systemFont(ofSize: 12)
        alert.accessoryView = checkbox
        alert.addButton(withTitle: RollbackStrings.confirmButton)
        alert.addButton(withTitle: RollbackStrings.cancel)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        guard checkbox.state == .on else {
            actionFeedbackLabel.stringValue = RollbackStrings.cancelled
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        isRollingBack = true
        rollbackButton.isEnabled = false
        actionFeedbackLabel.stringValue = RollbackStrings.inProgress(version)
        actionFeedbackLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            let result = self.client.executeRollback(expectedVersion: version)
            DispatchQueue.main.async {
                self.isRollingBack = false
                switch result {
                case .success(let message):
                    self.actionFeedbackLabel.stringValue = message
                    self.actionFeedbackLabel.textColor = .systemGreen
                    self.rollbackReasonSideLabel.stringValue = ""
                    self.rollbackReasonSideLabel.toolTip = nil
                    // 回滚后重新查询：替换完成后备份状态会随之变化
                    self.loadRollbackStatus()
                case .failure(let error):
                    self.actionFeedbackLabel.stringValue = RollbackStrings.failed(error.message)
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.rollbackReasonSideLabel.stringValue = error.message
                    self.rollbackReasonSideLabel.textColor = .systemRed
                    self.rollbackReasonSideLabel.toolTip = error.message
                    self.rollbackButton.isEnabled = self.currentRollbackStatus?.canRollback == true
                }
            }
        }
    }

    // MARK: - Testing Hooks (回滚卡片离屏夹具)

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
        return RollbackStrings.success
    }

    // MARK: - Actions

    @objc private func handleCheckUpdate() {
        guard !isChecking && !isDownloading else { return }
        isChecking = true
        downloadSupported = false
        refreshDownloadState()
        actionFeedbackLabel.stringValue = L10n.t(.uiUpdate212)
        actionFeedbackLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let res = try self.client.checkUpdate()
                let supportsDownload = res.status == .available && res.verification?.publisherVerified == true ? ((try? self.client.supportsDownload()) ?? false) : false
                DispatchQueue.main.async {
                    self.downloadSupported = supportsDownload
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.applyCheckResult(res)
                    self.actionFeedbackLabel.stringValue = res.status == .incompatible ? L10n.t(.uiUpdate213) : L10n.t(.uiUpdate214)
                    self.actionFeedbackLabel.textColor = res.status == .incompatible ? .systemRed : .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.downloadSupported = false
                    self.refreshDownloadState()
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateCheckFailedFmt, String(describing: error.localizedDescription))
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleDownloadUpdate() {
        guard downloadButton.isEnabled, !isDownloading, let expected = currentCheckResult else { return }
        isDownloading = true
        refreshDownloadState()
        actionFeedbackLabel.stringValue = L10n.t(.uiUpdate215)
        actionFeedbackLabel.textColor = .systemBlue
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let receipt = try self.client.downloadUpdate(expected: expected)
                DispatchQueue.main.async {
                    self.isDownloading = false
                    if self.currentCheckResult?.manifestSha256 == receipt.manifestSha256 {
                        self.downloadedReceipt = receipt
                        self.actionFeedbackLabel.stringValue = L10n.t(.uiUpdate216)
                        self.actionFeedbackLabel.textColor = .systemGreen
                    } else {
                        self.actionFeedbackLabel.stringValue = L10n.t(.uiUpdate217)
                        self.actionFeedbackLabel.textColor = .systemOrange
                    }
                    self.refreshDownloadState()
                }
            } catch {
                DispatchQueue.main.async {
                    self.isDownloading = false
                    self.actionFeedbackLabel.stringValue = L10n.t(.uiUpdate218, String(describing: error.localizedDescription))
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.refreshDownloadState()
                }
            }
        }
    }

    /// 关键安全防线：尚未接入生产事务的安装/延期操作保持禁用，不模拟安装成功
    @objc public func handleDisabledWriteAction() {
        actionFeedbackLabel.stringValue = L10n.t(.uiUpdate219)
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
                    self.actionFeedbackLabel.stringValue = L10n.t(.uiUpdate220)
                    self.actionFeedbackLabel.textColor = .secondaryLabelColor
                    self.loadRollbackStatus()
                }
            } catch {
                DispatchQueue.main.async {
                    self.refreshStatusButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = L10n.t(.updateStatusRefreshFailedFmt, String(describing: error.localizedDescription))
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.loadRollbackStatus()
                }
            }
        }
    }
}
