// native/bootstrap/UpdatePage.swift
// Application update management, version verification, drain coordination, rollback, and failure recovery.
import Foundation
import AppKit

// MARK: - Design Tokens (待换成 Theme)
private enum UpdatePageTokens {
    static let brand = NSColor(red: 0x12/255.0, green: 0xA3/255.0, blue: 0x7A/255.0, alpha: 1.0)
    static let brandDeep = NSColor(red: 0x0A/255.0, green: 0x5B/255.0, blue: 0x6B/255.0, alpha: 1.0)
    static let statusOkText = NSColor(red: 0x12/255.0, green: 0x79/255.0, blue: 0x54/255.0, alpha: 1.0)
    static let statusOkBg = NSColor(red: 0xE4/255.0, green: 0xF5/255.0, blue: 0xEF/255.0, alpha: 1.0)
    static let statusWarnText = NSColor(red: 0x92/255.0, green: 0x5C/255.0, blue: 0x07/255.0, alpha: 1.0)
    static let statusWarnBg = NSColor(red: 0xFB/255.0, green: 0xF1/255.0, blue: 0xE2/255.0, alpha: 1.0)
    static let statusErrText = NSColor(red: 0xD3/255.0, green: 0x2F/255.0, blue: 0x2F/255.0, alpha: 1.0)
    static let statusErrBg = NSColor(red: 0xFC/255.0, green: 0xEB/255.0, blue: 0xEB/255.0, alpha: 1.0)
    static let statusOffText = NSColor(red: 0x6C/255.0, green: 0x70/255.0, blue: 0x74/255.0, alpha: 1.0)
    static let statusOffBg = NSColor(red: 0xF0/255.0, green: 0xF1/255.0, blue: 0xF2/255.0, alpha: 1.0)
    static let fieldBorder = NSColor(red: 0xC9/255.0, green: 0xCD/255.0, blue: 0xD4/255.0, alpha: 1.0)
}

// MARK: - Strings (待迁入 L10n)
private enum UpdatePageStrings {
    static let pageTitle = "应用版本更新"
    static let sectionRollback = "回滚到上一版本"
    static let sectionOperations = "操作与维护"
    static let sectionRecovery = "故障恢复指引"
    static let noPreviousVersion = "无可回滚的上一版本"
    static let rollbackCheckboxPrompt = "我已知晓回滚将恢复至选定版本并覆盖当前程序"
    static let rollbackConfirmButton = "确认回滚"
    static let cancel = "取消"
    static let rollbackSuccess = "已成功回滚至上一版本，重启服务后生效"
    static let rollbackUnavailable = "暂不可用（更新回滚服务尚未合入或不可用）"
    static let emptyCandidate = "当前已是最新版本，未检测到可用更新"
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
        guard let r = reason else { return canRollback ? "可安全回滚" : "暂不可回滚" }
        switch r {
        case "no_previous_version": return "无可回滚的上一版本"
        case "no_matching_completed_update": return "未找到匹配的已完成更新记录"
        case "previous_release_corrupt": return "上一版本程序文件损坏或被篡改"
        case "backup_corrupt": return "数据库在线备份损坏或不完整"
        case "rollback_not_supported_by_release": return "该发布版本不支持就地回滚"
        case "database_schema_changed": return "数据库结构已发生变化，拒绝回滚"
        case "database_checkpoint_required": return "存在未提交事务，需完全停止服务"
        case "stop_core_first": return "核心服务运行中，需先停止服务"
        case "recovery_required": return "存在中断的更新事务，需执行受控恢复"
        case "install_root_unconfigured": return "未配置程序安装目录"
        case "rollback_state_invalid": return "回滚状态无效"
        default: return "暂不可用 (\(r))"
        }
    }

    public var formattedInstalledAt: String {
        guard let ts = installedAt, ts > 0 else { return "未知" }
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

    public func fetchRollbackStatus() -> Result<RollbackStatusInfo, UpdateError> {
        let updater = updaterBinaryPath()
        let args = ["rollback-status", "--config", configPath]
        let run = executor(updater, args, 10.0)
        guard !run.timedOut else {
            return .failure(UpdateError("查询回滚状态超时"))
        }
        guard let jsonStr = String(data: run.output, encoding: .utf8), !jsonStr.isEmpty else {
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
            return .failure(UpdateError("执行回滚操作超时"))
        }
        guard !run.output.isEmpty else {
            return .failure(UpdateError("回滚失败：无输出"))
        }
        if run.status == 0 {
            return .success(UpdatePageStrings.rollbackSuccess)
        } else {
            return .failure(UpdateError("回滚未完成 (退出码 \(run.status))"))
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
    private let currentReleaseLabel = NSTextField(wrappingLabelWithString: "加载中...")
    private let coreVersionLabel = NSTextField(wrappingLabelWithString: "未知")
    private let currentSeqLabel = NSTextField(wrappingLabelWithString: "未知 (未标记)")
    private let guardSeqLabel = NSTextField(wrappingLabelWithString: "无 (null)")
    private let releaseTrustLabel = NSTextField(wrappingLabelWithString: "未校验")
    private let architectureLabel = NSTextField(wrappingLabelWithString: "未知")
    private let consistencyBadge = NSTextField(wrappingLabelWithString: "未校验 (未知)")
    private let supervisorStatusLabel = NSTextField(wrappingLabelWithString: "检查中...")

    // UI - 回滚到上一版本卡片
    private let previousVersionLabel = NSTextField(wrappingLabelWithString: "查询中...")
    private let previousInstalledAtLabel = NSTextField(wrappingLabelWithString: "-")
    private let rollbackEligibilityLabel = NSTextField(wrappingLabelWithString: "检查中...")
    private let rollbackButton = NSButton(title: "回滚到上一版本…", target: nil, action: nil)

    // UI - 候选更新卡片
    private let candidateStatusLabel = NSTextField(wrappingLabelWithString: "尚未检查更新")
    private let candidateVersionLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidatePublishedLabel = NSTextField(wrappingLabelWithString: "-")
    private let candidateSizeLabel = NSTextField(wrappingLabelWithString: "-")
    private let publisherVerificationBadge = NSTextField(wrappingLabelWithString: "-")
    private let localSigningLabel = NSTextField(wrappingLabelWithString: "-")
    private let compatibilityLabel = NSTextField(wrappingLabelWithString: "-")
    private let releaseNotesView = NSTextView()

    // UI - 流程流水线与进度
    private let pipelineStepsLabel = NSTextField(wrappingLabelWithString: "检查 → 下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检")
    private let currentStageLabel = NSTextField(wrappingLabelWithString: "空闲")
    private let progressBar = NSProgressIndicator()
    private let progressDetailLabel = NSTextField(wrappingLabelWithString: "")
    private let drainWarningLabel = NSTextField(wrappingLabelWithString: "")
    private let permissionsNoteLabel = NSTextField(wrappingLabelWithString: "提示：专用机应用更新后将由本地自签名钥匙串重签名，若系统权限重置需在「设置」中重新授权。")

    // UI - 操作按钮 (分行排布)
    private let checkButton = NSButton(title: "检查更新", target: nil, action: nil)
    private let refreshStatusButton = NSButton(title: "刷新状态", target: nil, action: nil)
    private let downloadButton = NSButton(title: "下载更新 (未启用)", target: nil, action: nil)
    private let installButton = NSButton(title: "安装并重启 (未启用)", target: nil, action: nil)
    private let deferButton = NSButton(title: "延期更新 (未启用)", target: nil, action: nil)
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

        let introLabel = NSTextField(wrappingLabelWithString: "管理微信客服助手整套原生组件与核心服务更新，支持更新检查、候选验证、受控维护以及回滚至上一稳定版本。")
        introLabel.font = NSFont.systemFont(ofSize: 12)
        introLabel.textColor = .secondaryLabelColor
        contentStack.addArrangedSubview(introLabel)

        // 2. 当前版本卡片
        let currentBox = createSectionBox(title: "当前运行环境 (真实读模型)")
        let currentGrid = NSGridView(views: [
            [NSTextField(labelWithString: "当前整套版本:"), currentReleaseLabel],
            [NSTextField(labelWithString: "核心编译版本:"), coreVersionLabel],
            [NSTextField(labelWithString: "整套安装序列:"), currentSeqLabel],
            [NSTextField(labelWithString: "防降级高水位:"), guardSeqLabel],
            [NSTextField(labelWithString: "发布状态与来源:"), releaseTrustLabel],
            [NSTextField(labelWithString: "宿主运行架构:"), architectureLabel],
            [NSTextField(labelWithString: "组件一致性:"), consistencyBadge],
            [NSTextField(labelWithString: "维护抑制状态:"), supervisorStatusLabel]
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
        let candidateBox = createSectionBox(title: "候选版本与发布说明")
        let candStack = NSStackView()
        candStack.orientation = .vertical
        candStack.alignment = .leading
        candStack.spacing = 8

        candStack.addArrangedSubview(candidateStatusLabel)
        candidateStatusLabel.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        candidateStatusLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 580).isActive = true

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
        candidateVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        candidatePublishedLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        candidateSizeLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        publisherVerificationBadge.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        localSigningLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
        compatibilityLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 460).isActive = true
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

        let rbGrid = NSGridView(views: [
            [NSTextField(labelWithString: "可回滚上一版本:"), previousVersionLabel],
            [NSTextField(labelWithString: "原安装时间:"), previousInstalledAtLabel],
            [NSTextField(labelWithString: "回滚资格与原因:"), rollbackEligibilityLabel]
        ])
        rbGrid.rowSpacing = 6
        rbGrid.columnSpacing = 12
        previousVersionLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        previousInstalledAtLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        rollbackEligibilityLabel.widthAnchor.constraint(lessThanOrEqualToConstant: 440).isActive = true
        rbStack.addArrangedSubview(rbGrid)

        rollbackButton.target = self
        rollbackButton.action = #selector(handleRollback)
        rollbackButton.isEnabled = false
        rbStack.addArrangedSubview(rollbackButton)

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
            actionFeedbackLabel.stringValue = "状态获取提示: \(error.localizedDescription)"
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
            self.previousInstalledAtLabel.stringValue = info.formattedInstalledAt
            self.rollbackEligibilityLabel.stringValue = info.localizedReason
            if info.canRollback {
                self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusOkText
                self.rollbackButton.isEnabled = true
            } else {
                self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusWarnText
                self.rollbackButton.isEnabled = false
            }
        case .failure(let err):
            self.currentRollbackStatus = nil
            self.previousVersionLabel.stringValue = "未知"
            self.previousInstalledAtLabel.stringValue = "-"
            self.rollbackEligibilityLabel.stringValue = err.message
            self.rollbackEligibilityLabel.textColor = UpdatePageTokens.statusOffText
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
            currentStageLabel.stringValue = "空闲"
            currentStageLabel.textColor = .secondaryLabelColor
            progressBar.doubleValue = 0.0
            progressDetailLabel.stringValue = ""
            drainWarningLabel.stringValue = ""

        case .inProgress:
            let stageName = transaction.stage ?? "执行中"
            currentStageLabel.stringValue = "状态: \(stageName) (外部事务)"
            currentStageLabel.textColor = .systemBlue
            if stageName == "Draining" {
                drainWarningLabel.stringValue = "⚠️ 外部更新事务处于排空等待态；延期请由管理员命令行协调。"
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
        actionFeedbackLabel.stringValue = "正在查询更新状态..."
        actionFeedbackLabel.textColor = .systemBlue

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let res = try self.client.checkUpdate()
                DispatchQueue.main.async {
                    self.isChecking = false
                    self.checkButton.isEnabled = true
                    self.applyCheckResult(res)
                    self.actionFeedbackLabel.stringValue = "检查完成。"
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

    @objc private func handleRollback() {
        guard let info = currentRollbackStatus, info.canRollback, let ver = info.version else {
            actionFeedbackLabel.stringValue = "当前不可回滚：\(currentRollbackStatus?.localizedReason ?? "未检测到可回滚版本")"
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        let alert = NSAlert()
        alert.messageText = "确认回滚到上一版本（\(ver)）？"
        alert.informativeText = "回滚将停止核心服务并将应用程序切换回上一版本，业务数据库保留。确认继续吗？"
        alert.alertStyle = .critical

        let checkbox = NSButton(checkboxWithTitle: UpdatePageStrings.rollbackCheckboxPrompt, target: nil, action: nil)
        checkbox.font = NSFont.systemFont(ofSize: 12)
        alert.accessoryView = checkbox
        alert.addButton(withTitle: UpdatePageStrings.rollbackConfirmButton)
        alert.addButton(withTitle: UpdatePageStrings.cancel)

        let resp = alert.runModal()
        guard resp == .alertFirstButtonReturn else { return }
        guard checkbox.state == .on else {
            actionFeedbackLabel.stringValue = "回滚已取消：请勾选确认方可执行回滚。"
            actionFeedbackLabel.textColor = .systemRed
            return
        }

        actionFeedbackLabel.stringValue = "正在执行版本回滚到 \(ver)..."
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
                    self.actionFeedbackLabel.stringValue = "回滚失败: \(err)"
                    self.actionFeedbackLabel.textColor = .systemRed
                    self.rollbackButton.isEnabled = true
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
                    self.actionFeedbackLabel.stringValue = "状态已刷新。"
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
        loadRollbackStatus()
    }
}
