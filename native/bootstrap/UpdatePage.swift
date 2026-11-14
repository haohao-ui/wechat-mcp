// native/bootstrap/UpdatePage.swift
// Application update management, version verification, drain coordination, and failure recovery.
// Verified downloads are enabled only when the actual updater reports support.
// Installation, deferral and rollback stay disabled until production transactions are ready.
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
    if failed { return ControlRun(status: -1, output: Data("更新器输出或管道超出限制".utf8), timedOut: timedOut) }
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
            return "整套版本无法核验 (未知)"
        }
        return rid
    }

    /// 用户友好的核心编译版本描述（独立呈现）
    public var userFacingCoreVersionDescription: String {
        guard let cv = coreVersion, !cv.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return "未知 (未报告)"
        }
        return "\(cv) (核心编译版)"
    }

    /// 用户友好的安装序列号描述（绝不拿 guard.highestAcceptedSequence 填补）
    public var userFacingSequenceDescription: String {
        if let s = sequence {
            return "\(s)"
        }
        return "未知 (未标记)"
    }

    /// 用户友好的状态与来源描述：缺失或未知均明确标注不可信，不默认正常
    public var userFacingTrustDescription: String {
        let st = status?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let src = source?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if st.isEmpty && src.isEmpty {
            return "未提供状态/来源 (旧数据源，不可信)"
        }

        let stDesc: String
        switch st.lowercased() {
        case "unknown": stDesc = "状态未知"
        case "installed": stDesc = "已安装"
        case "verified": stDesc = "已验证"
        case "": stDesc = "无状态"
        default: stDesc = "未知状态 [\(st)]"
        }

        let srcDesc: String
        switch src.lowercased() {
        case "unknown": srcDesc = "来源未知"
        case "signed_package": srcDesc = "签名发布包"
        case "": srcDesc = "无来源"
        default: srcDesc = "未知来源 [\(src)]"
        }

        if isTrustedInstalledRelease {
            return "\(stDesc) (\(srcDesc))"
        } else {
            return "\(stDesc) / \(srcDesc) (不可信)"
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
            return "\(s) (防降级高水位，非安装版本)"
        }
        return "无 (null)"
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

        只读说明与受控恢复指引:
        1. 维护标志文件已记录；实际核心进程存活状态与业务数据完整性未知。
        2. 当前界面处于严格只读受控阶段，禁止通过界面清除维护标志或进行破坏性写入。
        3. 真实版本回退能力当前处于 unsupported 禁用状态，不可直接回退；需要专门受控恢复。
        4. 请核对 ~/.wechat-rpa/logs/core.log 与 host.log 获取详细故障上下文并联系管理员。
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
        guard run.status == 0 && !run.timedOut else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "未知错误"
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
            throw UpdateClientError.parseError("可信更新器不可用")
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
            throw UpdateClientError.parseError("更新清单尚未验证或更新器暂不支持下载")
        }
        let run = executor(try updaterPath(), ["download", "--config", configPath], 660)
        guard run.status == 0 && !run.timedOut else {
            let text = String(data: run.output, encoding: .utf8) ?? ""
            let message = text.contains("download_cache_quota_exceeded") ? "更新缓存已满，请清理未使用候选后重试" : text.contains("concurrent_download_in_progress") ? "已有更新包正在下载，请稍后重试" : "下载或校验未完成，请重试"
            throw UpdateClientError.commandFailed(code: run.status, message: message)
        }
        let receipt = try JSONDecoder().decode(UpdateDownloadReceipt.self, from: run.output)
        guard receipt.status == "downloaded", UUID(uuidString: receipt.candidateId)?.uuidString.lowercased() == receipt.candidateId,
              receipt.releaseId == candidate.releaseId, receipt.sequence == candidate.sequence,
              receipt.manifestSha256 == digest else {
            throw UpdateClientError.parseError("更新源或下载结果已变化，请重新检查更新")
        }
        return receipt
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

    // UI - 当前版本卡片 (全部使用 wrappingLabelWithString 并设受限宽度)
    private let currentReleaseLabel = NSTextField(wrappingLabelWithString: "加载中...")
    private let coreVersionLabel = NSTextField(wrappingLabelWithString: "未知")
    private let currentSeqLabel = NSTextField(wrappingLabelWithString: "未知 (未标记)")
    private let guardSeqLabel = NSTextField(wrappingLabelWithString: "无 (null)")
    private let releaseTrustLabel = NSTextField(wrappingLabelWithString: "未校验")
    private let architectureLabel = NSTextField(wrappingLabelWithString: "未知")
    private let consistencyBadge = NSTextField(wrappingLabelWithString: "未校验 (未知)")
    private let supervisorStatusLabel = NSTextField(wrappingLabelWithString: "检查中...")

    // UI - 候选更新卡片
    private let candidateStatusLabel = NSTextField(wrappingLabelWithString: "尚未检查更新")
    private let candidateVersionLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidatePublishedLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidateSizeLabel = NSTextField(wrappingLabelWithString: "-")
    private let publisherVerificationBadge = NSTextField(wrappingLabelWithString: "-")
    private let localSigningLabel = NSTextField(wrappingLabelWithString: "-")
    private let compatibilityLabel = NSTextField(wrappingLabelWithString: "-")
    private let releaseNotesView = NSTextView()
    private let candidateEmptyLabel = NSTextField(wrappingLabelWithString: "尚未检查更新。点击「检查更新」读取授权清单状态；未配置更新源时不会联网或下载。")
    private let candidateDetailsContainer = NSStackView()
    private let readOnlyBadge = NSTextField(labelWithString: "只读")

    // UI - 流程流水线与进度
    private let pipelineStepsLabel = NSTextField(wrappingLabelWithString: "检查 → 下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检")
    private let currentStageLabel = NSTextField(wrappingLabelWithString: "只读监控中")
    private let progressBar = NSProgressIndicator()
    private let progressDetailLabel = NSTextField(wrappingLabelWithString: "")
    private let drainWarningLabel = NSTextField(wrappingLabelWithString: "")
    private let permissionsNoteLabel = NSTextField(wrappingLabelWithString: "提示：专用机应用更新后将由本地自签名钥匙串重签名，若系统权限重置需在「设置」中重新授权。")

    // UI - 操作按钮 (只读阶段全部禁用写操作，分行排布)
    private let checkButton = NSButton(title: "检查更新", target: nil, action: nil)
    private let refreshStatusButton = NSButton(title: "刷新状态", target: nil, action: nil)
    private let downloadButton = NSButton(title: "下载更新", target: nil, action: nil)
    private let installButton = NSButton(title: "安装并重启", target: nil, action: nil)
    private let deferButton = NSButton(title: "延期更新", target: nil, action: nil)
    private let writeControlsNote = NSTextField(wrappingLabelWithString: "受控写入（当前未启用）：下载、安装、延期由真实更新事务控制，界面不会模拟成功。")
    private let actionFeedbackLabel = NSTextField(wrappingLabelWithString: "当前处于只读状态查看阶段；真实下载、安装、排空延期与恢复写入均已安全禁用。")

    // UI - 恢复卡片 (当出现 recovery_required 时高亮)
    private let recoveryBox = NSBox()
    private let technicalDetails = NSStackView()
    private let transactionDetails = NSStackView()
    private let detailsButton = NSButton(checkboxWithTitle: "显示技术详情", target: nil, action: nil)
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

        let card = createSectionBox(title: "版本与更新")
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
            [NSTextField(labelWithString: "目标版本:"), candidateVersionLabel],
            [NSTextField(labelWithString: "发布时间"), candidatePublishedLabel],
            [NSTextField(labelWithString: "适用环境"), compatibilityLabel]
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
        releaseNotesView.string = "（暂无发布说明）"
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
            button.toolTip = "当前仅支持检查状态，此操作尚未启用"
        }
        downloadButton.action = #selector(handleDownloadUpdate)
        let writes = NSStackView(views: [downloadButton])
        writes.spacing = 12
        body.addArrangedSubview(writes)
        actionFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        actionFeedbackLabel.textColor = .secondaryLabelColor
        body.addArrangedSubview(actionFeedbackLabel)

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

        recoveryBox.title = "需要人工恢复"
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
            [NSTextField(labelWithString: "核心编译版本"), coreVersionLabel],
            [NSTextField(labelWithString: "整套安装序列"), currentSeqLabel],
            [NSTextField(labelWithString: "防降级高水位"), guardSeqLabel],
            [NSTextField(labelWithString: "发布状态与来源"), releaseTrustLabel],
            [NSTextField(labelWithString: "宿主运行架构"), architectureLabel],
            [NSTextField(labelWithString: "组件一致性"), consistencyBadge],
            [NSTextField(labelWithString: "维护抑制状态"), supervisorStatusLabel],
            [NSTextField(labelWithString: "更新包大小"), candidateSizeLabel],
            [NSTextField(labelWithString: "发布者验证"), publisherVerificationBadge],
            [NSTextField(labelWithString: "本机重签名"), localSigningLabel]
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
            technicalDetails.widthAnchor.constraint(equalTo: body.widthAnchor)
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
            actionFeedbackLabel.stringValue = "状态获取提示: \(error.localizedDescription)"
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

        architectureLabel.stringValue = payload.currentRelease.architecture.isEmpty ? "未知" : payload.currentRelease.architecture

        if let c = payload.currentRelease.componentsConsistent {
            if c {
                consistencyBadge.stringValue = "整套组件版本一致"
                consistencyBadge.textColor = .systemGreen
            } else {
                consistencyBadge.stringValue = "组件版本不一致"
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
        refreshDownloadState()
    }

    public func applyCheckResult(_ result: UpdateCheckResult) {
        if (result.status == .available || result.status == .alreadyInstalled),
           result.verification?.publisherVerified != true {
            applyCheckResult(UpdateCheckResult(status: .unverifiedTrustRoot,
                reason: "响应未提供发布者验签成功证明", candidate: result.candidate,
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
            candidateStatusLabel.stringValue = "更新源尚未配置"
            candidateStatusLabel.textColor = .systemOrange
            candidateVersionLabel.stringValue = "无可用版本 (null)"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "未配置公钥根"
            publisherVerificationBadge.textColor = .secondaryLabelColor
            localSigningLabel.stringValue = "未配置发布源，禁止重签"
            compatibilityLabel.stringValue = friendlyReason
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
            candidateStatusLabel.stringValue = "此版本已有匹配的安装记录"
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
            }
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "清单签名与安装记录已核对"
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = "运行状态需单独核验"
            compatibilityLabel.stringValue = "清单与当前环境兼容"
            releaseNotesView.string = "没有待更新的候选版本。"

        case .incompatible:
            candidateStatusLabel.stringValue = "更新检查未通过"
            candidateStatusLabel.textColor = .systemRed
            candidateVersionLabel.stringValue = "-"
            candidatePublishedLabel.stringValue = "-"
            candidateSizeLabel.stringValue = "-"
            publisherVerificationBadge.stringValue = "签名或环境检查未通过"
            publisherVerificationBadge.textColor = .systemRed
            localSigningLabel.stringValue = "禁止重签"
            compatibilityLabel.stringValue = result.reason ?? "不满足最低系统或架构要求"
            releaseNotesView.string = "该版本无法安装在当前目标专用机上。"

        case .available:
            candidateStatusLabel.stringValue = "发现新版本，清单签名已验证"
            candidateStatusLabel.textColor = .systemGreen
            if let cand = result.candidate {
                candidateVersionLabel.stringValue = "\(cand.releaseId) (seq: \(cand.sequence))"
                candidatePublishedLabel.stringValue = cand.publishedAt ?? "-"
                if let sz = cand.sizeBytes {
                    candidateSizeLabel.stringValue = UpdatePageLogic.formatBytes(sz)
                }
                compatibilityLabel.stringValue = cand.compatible ? "清单与当前系统兼容，安装包待校验" : "不兼容"
                releaseNotesView.string = cand.releaseNotes ?? "（暂无说明）"
            }
            publisherVerificationBadge.stringValue = "发布者签名已验证"
            publisherVerificationBadge.textColor = .systemGreen
            localSigningLabel.stringValue = "安装包与本机签名尚未校验"
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
        downloadButton.title = isDownloading ? "下载中…" : downloaded ? "已下载" : (cached && verified) ? "校验本地更新包" : "下载更新"
        if downloaded && currentCheckResult?.status == .available {
            compatibilityLabel.stringValue = "更新包已下载，安装前将继续验证"
            localSigningLabel.stringValue = "安装包摘要已验证，尚未解包与本机签名"
        } else if cached && verified {
            compatibilityLabel.stringValue = "本机有候选缓存，需重新校验更新包"
            localSigningLabel.stringValue = "缓存尚未重新校验，不代表已安装"
        }
        downloadButton.toolTip = downloadSupported ? "下载到本机私有缓存，不会停止业务或安装" : "当前更新器未报告可用的下载能力"
        checkButton.isEnabled = !isChecking && !isDownloading
        refreshStatusButton.isEnabled = !isDownloading
        if downloadSupported {
            readOnlyBadge.stringValue = "可下载"
            writeControlsNote.stringValue = "支持已验证清单的受限下载；安装、延期和恢复尚未启用。"
        } else {
            readOnlyBadge.stringValue = "仅检查"
            writeControlsNote.stringValue = "下载需由更新器报告支持；安装、延期和恢复尚未启用。"
        }
    }

    private func friendlyCheckReason(_ reason: String?) -> String {
        guard let reason, !reason.isEmpty else {
            return "尚未配置授权更新清单；当前不会联网、下载或安装。"
        }
        if reason == "distribution_source_not_configured" {
            return "尚未配置授权更新源；当前仅展示本地状态，不会联网、下载或安装。"
        }
        return reason
    }

    private func updateTransactionUI(_ transaction: UpdateTransactionInfo) {
        transactionDetails.isHidden = transaction.status == .idle && !transaction.recoveryRequired
        progressBar.isIndeterminate = transaction.status == .inProgress
        if progressBar.isIndeterminate { progressBar.startAnimation(nil) } else { progressBar.stopAnimation(nil) }
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
                drainWarningLabel.stringValue = "外部更新事务处于排空等待态；当前界面为只读，延期请由管理员命令行协调。"
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
        guard !isChecking && !isDownloading else { return }
        isChecking = true
        downloadSupported = false
        refreshDownloadState()
        actionFeedbackLabel.stringValue = "正在查询更新状态 (只读查询)..."
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
                    self.actionFeedbackLabel.stringValue = res.status == .incompatible ? "检查未通过，请查看提示。" : "清单检查完成，本次未下载或安装。"
                    self.actionFeedbackLabel.textColor = res.status == .incompatible ? .systemRed : .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.downloadSupported = false
                    self.refreshDownloadState()
                    self.actionFeedbackLabel.stringValue = "检查失败: \(error.localizedDescription)"
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleDownloadUpdate() {
        guard downloadButton.isEnabled, !isDownloading, let expected = currentCheckResult else { return }
        isDownloading = true
        refreshDownloadState()
        actionFeedbackLabel.stringValue = "正在下载并校验更新包，不改变业务运行状态。"
        actionFeedbackLabel.textColor = .systemBlue
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let receipt = try self.client.downloadUpdate(expected: expected)
                DispatchQueue.main.async {
                    self.isDownloading = false
                    if self.currentCheckResult?.manifestSha256 == receipt.manifestSha256 {
                        self.downloadedReceipt = receipt
                        self.actionFeedbackLabel.stringValue = "更新包已下载并校验，尚未安装。"
                        self.actionFeedbackLabel.textColor = .systemGreen
                    } else {
                        self.actionFeedbackLabel.stringValue = "下载结果与当前清单不同，请重新检查更新。"
                        self.actionFeedbackLabel.textColor = .systemOrange
                    }
                    self.refreshDownloadState()
                }
            } catch {
                DispatchQueue.main.async {
                    self.isDownloading = false
                    self.actionFeedbackLabel.stringValue = "下载未完成：\(error.localizedDescription)"
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.refreshDownloadState()
                }
            }
        }
    }

    /// 关键安全防线：尚未接入生产事务的安装/延期操作保持禁用，不模拟安装成功
    @objc public func handleDisabledWriteAction() {
        actionFeedbackLabel.stringValue = "操作未启用: 当前阶段安装/延期/恢复入口尚未启用 (unsupported)，不进行任何写入或业务状态修改。"
        actionFeedbackLabel.textColor = .systemOrange
    }

    @objc private func handleRefreshStatus() {
        actionFeedbackLabel.stringValue = "正在刷新状态..."
        actionFeedbackLabel.textColor = .systemBlue
        refreshStatusButton.isEnabled = false

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let payload = try self.client.fetchStatus()
                DispatchQueue.main.async {
                    self.applyStatusPayload(payload)
                    self.refreshStatusButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = "状态已刷新 (只读)。"
                    self.actionFeedbackLabel.textColor = .secondaryLabelColor
                }
            } catch {
                DispatchQueue.main.async {
                    self.refreshStatusButton.isEnabled = true
                    self.actionFeedbackLabel.stringValue = "状态刷新失败: \(error.localizedDescription)"
                    self.actionFeedbackLabel.textColor = .systemRed
                }
            }
        }
    }
}
