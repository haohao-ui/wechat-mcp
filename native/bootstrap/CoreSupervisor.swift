import Foundation
import Darwin

/// 日志轮换管理器：单个文件最大 10MB，超过 10MB 就改名为 .1 到 .5，最多保留 5 份历史
public enum LogRotator {
    public static let defaultMaxBytes: UInt64 = 10 * 1024 * 1024 // 10MB
    public static let defaultMaxFiles: Int = 5
    private static let lock = NSLock()

    /// 检查指定日志文件大小，若超过 maxBytes 则执行轮换
    @discardableResult
    public static func rotateIfNeeded(
        fileURL: URL,
        maxBytes: UInt64 = defaultMaxBytes,
        maxFiles: Int = defaultMaxFiles
    ) -> Bool {
        lock.lock()
        defer { lock.unlock() }

        let fm = FileManager.default
        let path = fileURL.path
        guard fm.fileExists(atPath: path) else { return false }
        guard let attrs = try? fm.attributesOfItem(atPath: path),
              let size = attrs[.size] as? UInt64,
              size >= maxBytes else {
            return false
        }
        return rotateUnlocked(fileURL: fileURL, maxFiles: maxFiles)
    }

    /// 执行轮换：删除最老的 .maxFiles，将其余备份依次后移，当前文件命名为 .1，并创建新的空日志文件（0600权限）
    @discardableResult
    public static func rotate(fileURL: URL, maxFiles: Int = defaultMaxFiles) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return rotateUnlocked(fileURL: fileURL, maxFiles: maxFiles)
    }

    private static func rotateUnlocked(fileURL: URL, maxFiles: Int) -> Bool {
        let fm = FileManager.default
        let basePath = fileURL.path
        guard fm.fileExists(atPath: basePath) else { return false }

        // 1. 删除超出 maxFiles 的文件
        let oldestPath = "\(basePath).\(maxFiles)"
        if fm.fileExists(atPath: oldestPath) {
            try? fm.removeItem(atPath: oldestPath)
        }

        // 2. 倒序顺移历史备份文件: .(i) -> .(i+1)
        if maxFiles > 1 {
            for i in stride(from: maxFiles - 1, through: 1, by: -1) {
                let src = "\(basePath).\(i)"
                let dst = "\(basePath).\(i + 1)"
                if fm.fileExists(atPath: src) {
                    try? fm.removeItem(atPath: dst)
                    try? fm.moveItem(atPath: src, toPath: dst)
                }
            }
        }

        // 3. 将当前文件重命名为 .1
        let firstBackup = "\(basePath).1"
        try? fm.removeItem(atPath: firstBackup)
        do {
            try fm.moveItem(atPath: basePath, toPath: firstBackup)
            try? fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: firstBackup)
        } catch {
            return false
        }

        // 4. 创建新的空基准日志文件，权限严格设置为 0600
        fm.createFile(atPath: basePath, contents: Data(), attributes: [.posixPermissions: 0o600])
        return true
    }
}

/// 宿主日志管理器：写入 host.log 并在超过 10MB 时自动轮换
public final class HostLogger {
    public static let shared = HostLogger()
    private let lock = NSLock()
    public var logURL: URL

    public init(logURL: URL? = nil) {
        let home = FileManager.default.homeDirectoryForCurrentUser
        self.logURL = logURL ?? home.appendingPathComponent(".wechat-rpa/logs/host.log")
    }

    public func log(_ message: String) {
        lock.lock()
        defer { lock.unlock() }

        let logDir = logURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])

        // 写入前检查是否需要轮换
        LogRotator.rotateIfNeeded(fileURL: logURL)

        if !FileManager.default.fileExists(atPath: logURL.path) {
            FileManager.default.createFile(atPath: logURL.path, contents: Data(), attributes: [.posixPermissions: 0o600])
        } else {
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: logURL.path)
        }

        let df = ISO8601DateFormatter()
        let line = "\(df.string(from: Date())) [host] \(message)\n"
        if let data = line.data(using: .utf8) {
            if let handle = try? FileHandle(forWritingTo: logURL) {
                handle.seekToEndOfFile()
                handle.write(data)
                try? handle.close()
            }
        }
    }
}

public enum CoreProcessStatus: Equatable {
    case notRunning
    case running(pid: Int32)
    case stopping(pid: Int32)
    case restarting(at: Date)
    case gaveUp
    case configError

    public var rawStatusString: String {
        switch self {
        case .notRunning: return "not_running"
        case .running: return "running"
        case .stopping: return "stopping"
        case .restarting: return "restarting"
        case .gaveUp: return "gave_up"
        case .configError: return "config_error"
        }
    }
}

public enum CoreDecision: String, Codable, Equatable {
    case start
    case stop
    case keep
    case conflict
}

/// 纯函数：根据开关、Python launchd 状态与当前子进程状态计算决策
public func decideCoreAction(
    coreEnabled: Bool,
    pythonLoaded: Bool,
    status: CoreProcessStatus,
    now: Date = Date()
) -> CoreDecision {
    if case .stopping = status {
        return .keep
    }
    if coreEnabled && pythonLoaded {
        return .conflict
    }
    if !coreEnabled {
        if case .running = status {
            return .stop
        }
        return .keep
    }
    // coreEnabled == true && !pythonLoaded
    switch status {
    case .notRunning:
        return .start
    case .running:
        return .keep
    case .stopping:
        return .keep
    case .restarting(let at):
        return now >= at ? .start : .keep
    case .gaveUp, .configError:
        return .keep
    }
}

/// 检查 Python launchd 任务 local.wechat-rpa.bot 是否已加载
public func checkPythonLaunchdLoaded() -> Bool {
    let uid = geteuid()
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
    process.arguments = ["print", "gui/\(uid)/local.wechat-rpa.bot"]
    process.standardOutput = Pipe()
    process.standardError = Pipe()
    do {
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus == 0
    } catch {
        return false
    }
}

/// Local host adapter only: no core drain, worker exit or maintenance-lock evidence.
public enum CoreMaintenanceStatus: Equatable {
    case inactive
    case active(transactionID: String)
    case recoveryRequired
}

public enum CoreMaintenanceStopResult: Equatable {
    case stopped
    case notConfirmed
}

public enum CoreMaintenanceError: Error {
    case invalidTransaction, conflict, recoveryRequired, notConfirmed, ioFailure(Int32)
}

public final class CoreSupervisor {
    private let lock = NSLock()
    private let stopLock = NSLock()
    private let queue = DispatchQueue(label: "local.wechat-mcp.core-supervisor")
    private var process: Process?
    private var restartPolicy = RestartPolicy()
    private var manualStoppingPIDs = Set<Int32>()
    private let maintenanceURL: URL
    // Process identity retained after stop; a nil cache or a new PID is not exit proof.
    private var maintenanceStoppedProcess: Process?
    private var maintenanceStoppedTransaction: String?
    private var maintenanceCompletionFailed = false
    // Diagnostic evidence only: false means restoration durability could not be confirmed.
    private(set) var maintenanceRecoveryPersistenceConfirmed: Bool?
    // Internal syscall seams for isolated fault fixtures, not updater/control APIs.
    var maintenanceDirectoryOpen: (String) -> Int32 = { open($0, O_RDONLY | O_NOFOLLOW) }
    var maintenanceDirectorySync: (Int32) -> Int32 = { fsync($0) }
    var maintenanceMarkerUnlink: (String) -> Int32 = { unlink($0) }

    public private(set) var gaveUp: Bool = false
    public private(set) var isConfigError: Bool = false
    public private(set) var isStopping: Bool = false
    public private(set) var restartingAt: Date?
    public private(set) var lastExitCode: Int32?

    public var coreEnabledURL: URL
    public var binaryURL: URL
    public var configURL: URL
    public var logURL: URL
    public var journalURL: URL
    public var releasesDirURL: URL
    public var currentLinkURL: URL
    public var gracePeriod: TimeInterval
    public var pythonLoadedChecker: () -> Bool

    public init(
        coreEnabledURL: URL? = nil,
        maintenanceURL: URL? = nil,
        binaryURL: URL? = nil,
        configURL: URL? = nil,
        logURL: URL? = nil,
        journalURL: URL? = nil,
        releasesDirURL: URL? = nil,
        currentLinkURL: URL? = nil,
        gracePeriod: TimeInterval = 240.0,
        pythonLoadedChecker: @escaping () -> Bool = checkPythonLaunchdLoaded
    ) {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let baseRpa = home.appendingPathComponent(".wechat-rpa")
        self.coreEnabledURL = coreEnabledURL ?? baseRpa.appendingPathComponent("core.enabled")
        let stateDir = self.coreEnabledURL.deletingLastPathComponent()
        self.maintenanceURL = maintenanceURL ?? stateDir.appendingPathComponent("core.maintenance")
        self.binaryURL = binaryURL ?? baseRpa.appendingPathComponent("bin/wechat-mcp")
        self.configURL = configURL ?? home.appendingPathComponent(".WeChatRPABot/config.json")
        self.logURL = logURL ?? baseRpa.appendingPathComponent("logs/core.log")
        self.journalURL = journalURL ?? stateDir.appendingPathComponent("update-journal.json")
        self.releasesDirURL = releasesDirURL ?? baseRpa.appendingPathComponent("releases")
        self.currentLinkURL = currentLinkURL ?? baseRpa.appendingPathComponent("current")
        self.gracePeriod = gracePeriod
        self.pythonLoadedChecker = pythonLoadedChecker
    }

    public var maintenanceStatus: CoreMaintenanceStatus {
        lock.lock()
        defer { lock.unlock() }
        return readStartupDualGuardLocked()
    }

    /// 纯只读安全打开并读取私有文件（防 TOCTOU 与特殊文件阻塞，只有 ENOENT 视为不存在，其余非零错误均返回 recoveryRequired）
    private func readLimitedPrivateFile(url: URL, maxBytes: Int) -> (data: Data?, error: CoreMaintenanceError?) {
        let fd = open(url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        if fd < 0 {
            if errno == ENOENT {
                return (nil, nil)
            }
            return (nil, .recoveryRequired)
        }
        defer { close(fd) }

        var fileInfo = stat()
        guard fstat(fd, &fileInfo) == 0,
              (fileInfo.st_mode & S_IFMT) == S_IFREG,
              fileInfo.st_uid == geteuid(),
              (fileInfo.st_mode & 0o777) == 0o600,
              fileInfo.st_size <= maxBytes else {
            return (nil, .recoveryRequired)
        }

        let expectedCount = Int(fileInfo.st_size)
        var buffer = [UInt8](repeating: 0, count: expectedCount)
        let readBytes = buffer.withUnsafeMutableBytes { ptr -> Int in
            guard let base = ptr.baseAddress else { return -1 }
            return read(fd, base, ptr.count)
        }
        guard readBytes == expectedCount else {
            return (nil, .recoveryRequired)
        }
        return (Data(buffer), nil)
    }

    /// 精确核验 current 软链接物理目标必须落在 releasesDir 且等于预期 release_id 目录（拒绝外部目录逃逸或悬空）
    private func verifyCurrentLinkTarget(expectedReleaseID: String) -> Bool {
        var linkStat = stat()
        guard lstat(currentLinkURL.path, &linkStat) == 0,
              (linkStat.st_mode & S_IFMT) == S_IFLNK else {
            return false
        }
        guard let dest = try? FileManager.default.destinationOfSymbolicLink(atPath: currentLinkURL.path) else {
            return false
        }
        let targetURL = (dest as NSString).isAbsolutePath
            ? URL(fileURLWithPath: dest)
            : currentLinkURL.deletingLastPathComponent().appendingPathComponent(dest)

        let resolvedTarget = targetURL.standardized.resolvingSymlinksInPath().path
        let expectedURL = releasesDirURL.appendingPathComponent(expectedReleaseID).standardized.resolvingSymlinksInPath()
        guard resolvedTarget == expectedURL.path else {
            return false
        }
        var dirStat = stat()
        guard stat(resolvedTarget, &dirStat) == 0,
              (dirStat.st_mode & S_IFMT) == S_IFDIR else {
            return false
        }
        return true
    }

    /// 纯只读启动双检查 Guard：同时校验 core.maintenance 与 update-journal.json（零写入、零删除、Fail-Closed、防 TOCTOU）
    public func readStartupDualGuardLocked() -> CoreMaintenanceStatus {
        // 1. 检查 core.maintenance（通过受控 readLimitedPrivateFile，严格 36 字节 ASCII UUID）
        let (maintData, maintErr) = readLimitedPrivateFile(url: maintenanceURL, maxBytes: 128)
        if maintErr != nil {
            return .recoveryRequired
        }
        var activeTxID: String? = nil
        if let mData = maintData {
            guard mData.count == 36,
                  let rawUUID = String(data: mData, encoding: .utf8),
                  validTransaction(rawUUID) else {
                return .recoveryRequired
            }
            activeTxID = rawUUID
        }

        // 2. 检查 update-journal.json（受控打开 + fstat 校验）
        let (jData, jErr) = readLimitedPrivateFile(url: journalURL, maxBytes: 64 * 1024)
        if jErr != nil {
            return .recoveryRequired
        }

        if let jBytes = jData {
            guard let jObj = (try? JSONSerialization.jsonObject(with: jBytes, options: [])) as? [String: Any],
                  let schemaNum = jObj["journal_schema"] as? NSNumber,
                  CFGetTypeID(schemaNum) != CFBooleanGetTypeID(),
                  schemaNum.intValue == 1,
                  let stage = jObj["stage"] as? String,
                  let jTxID = jObj["transaction_id"] as? String,
                  let jTargetRel = jObj["target_release_id"] as? String,
                  let jTargetSeqNum = jObj["target_sequence"] as? NSNumber,
                  CFGetTypeID(jTargetSeqNum) != CFBooleanGetTypeID(),
                  jTargetSeqNum.int64Value >= 0 else {
                return .recoveryRequired
            }

            switch stage {
            case "RecoveryRequired":
                return .recoveryRequired

            case "Staged", "PublisherVerified", "LocalSigned", "Draining", "BackupVerified",
                 "Switching", "CandidateTesting", "CandidateTestingPassed", "RollbackStarted":
                return .active(transactionID: jTxID)

            case "RolledBack":
                // 真实回滚终态全要素校验
                let sourceRel = (jObj["source_release_id"] as? String) ?? "none"
                if sourceRel == "none" {
                    var lStat = stat()
                    if lstat(currentLinkURL.path, &lStat) == 0 {
                        return .recoveryRequired
                    }
                } else {
                    guard verifyCurrentLinkTarget(expectedReleaseID: sourceRel) else {
                        return .recoveryRequired
                    }
                }

                if let atid = activeTxID {
                    return .active(transactionID: atid)
                }
                return .inactive

            case "Committed":
                // 终态全要素二次只读核验：严禁仅凭 Committed 标签放行！
                // a. 读取并校验 current_release.json（严格 O_NOFOLLOW / 0600 / UID 匹配）
                let recordURL = journalURL.deletingLastPathComponent().appendingPathComponent("current_release.json")
                let (rData, rErr) = readLimitedPrivateFile(url: recordURL, maxBytes: 64 * 1024)
                guard rErr == nil, let rBytes = rData,
                      let rObj = (try? JSONSerialization.jsonObject(with: rBytes, options: [])) as? [String: Any],
                      let rSchema = rObj["record_schema"] as? NSNumber,
                      CFGetTypeID(rSchema) != CFBooleanGetTypeID(),
                      rSchema.intValue == 1,
                      let rTxID = rObj["tx_id"] as? String,
                      let rReleaseID = rObj["release_id"] as? String,
                      let rSeqNum = rObj["sequence"] as? NSNumber,
                      CFGetTypeID(rSeqNum) != CFBooleanGetTypeID(),
                      rSeqNum.int64Value >= 0 else {
                    return .recoveryRequired
                }

                guard rTxID == jTxID,
                      rReleaseID == jTargetRel,
                      rSeqNum.uint64Value == jTargetSeqNum.uint64Value else {
                    return .recoveryRequired
                }

                // 必需 digest 格式与匹配校验（严禁跳过）
                guard let jDigest = jObj["manifest_digest"] as? String,
                      jDigest.count == 64,
                      jDigest.allSatisfy({ ($0 >= "0" && $0 <= "9") || ($0 >= "a" && $0 <= "f") }),
                      let rDigest = rObj["manifest_digest"] as? String,
                      rDigest == jDigest else {
                    return .recoveryRequired
                }

                // b. 校验 current 软链接物理目标（拒绝外部目录逃逸或悬空）
                guard verifyCurrentLinkTarget(expectedReleaseID: jTargetRel) else {
                    return .recoveryRequired
                }

                if let atid = activeTxID {
                    return .active(transactionID: atid)
                }
                return .inactive

            default:
                // 未知 stage 必须 Fail-Closed
                return .recoveryRequired
            }
        } else {
            // journal 不存在 (ENOENT)
            if let atid = activeTxID {
                return .active(transactionID: atid)
            }
            return .inactive
        }
    }

    private func validTransaction(_ value: String) -> Bool {
        // Bounded ASCII UUIDs avoid arbitrary paths, unbounded records and ambiguous IDs.
        return value.utf8.count == 36 && UUID(uuidString: value) != nil
    }

    private func checkMaintenanceDirectory() throws {
        var info = stat()
        let parent = maintenanceURL.deletingLastPathComponent().path
        guard lstat(parent, &info) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        guard (info.st_mode & S_IFMT) == S_IFDIR,
              info.st_uid == geteuid(), (info.st_mode & 0o077) == 0 else {
            throw CoreMaintenanceError.recoveryRequired
        }
    }

    private func readMaintenanceLocked() -> CoreMaintenanceStatus {
        if maintenanceCompletionFailed { return .recoveryRequired }
        let fd = open(maintenanceURL.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        if fd < 0 { return errno == ENOENT ? .inactive : .recoveryRequired }
        defer { close(fd) }
        do { try checkMaintenanceDirectory() } catch { return .recoveryRequired }
        var info = stat()
        guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG,
              info.st_uid == geteuid(), (info.st_mode & 0o777) == 0o600,
              info.st_nlink == 1, info.st_size == 36 else { return .recoveryRequired }
        var bytes = [UInt8](repeating: 0, count: 37)
        let count = bytes.withUnsafeMutableBytes { read(fd, $0.baseAddress, $0.count) }
        guard count == 36, let id = String(bytes: bytes.prefix(count), encoding: .utf8),
              validTransaction(id) else { return .recoveryRequired }
        return .active(transactionID: id)
    }

    /// Persist suppression before any stop. This does not pause or drain core work.
    public func beginMaintenance(transactionID: String) throws {
        lock.lock()
        defer { lock.unlock() }
        guard validTransaction(transactionID) else { throw CoreMaintenanceError.invalidTransaction }
        switch readMaintenanceLocked() {
        case .active(let owner):
            guard owner == transactionID else { throw CoreMaintenanceError.conflict }
            return
        case .recoveryRequired: throw CoreMaintenanceError.recoveryRequired
        case .inactive: break
        }
        try publishMaintenanceMarkerLocked(transactionID: transactionID)
        maintenanceStoppedProcess = nil
        maintenanceStoppedTransaction = nil
    }

    private func publishMaintenanceMarkerLocked(transactionID: String) throws {
        try checkMaintenanceDirectory()
        let temporary = maintenanceURL.deletingLastPathComponent().appendingPathComponent(".maintenance-" + UUID().uuidString)
        let fd = open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        defer { close(fd); unlink(temporary.path) }
        let bytes = Array(transactionID.utf8)
        let written = bytes.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        guard written == bytes.count, fsync(fd) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        // Exclusive publication: never replace another transaction or corrupt marker.
        guard link(temporary.path, maintenanceURL.path) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        guard unlink(temporary.path) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        try syncMaintenanceDirectory()
    }

    private func syncMaintenanceDirectory() throws {
        let fd = maintenanceDirectoryOpen(maintenanceURL.deletingLastPathComponent().path)
        guard fd >= 0 else { throw CoreMaintenanceError.ioFailure(errno) }
        defer { close(fd) }
        guard maintenanceDirectorySync(fd) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
    }

    private func requireMaintenanceOwner(_ id: String) throws {
        guard validTransaction(id) else { throw CoreMaintenanceError.invalidTransaction }
        switch readMaintenanceLocked() {
        case .active(let owner) where owner == id: return
        case .recoveryRequired: throw CoreMaintenanceError.recoveryRequired
        default: throw CoreMaintenanceError.conflict
        }
    }

    /// Confirms only the exact child held by this supervisor, never all app processes.
    public func stopForMaintenance(transactionID: String) throws -> CoreMaintenanceStopResult {
        stopLock.lock()
        defer { stopLock.unlock() }
        lock.lock()
        do { try requireMaintenanceOwner(transactionID) } catch { lock.unlock(); throw error }
        let held = process ?? maintenanceStoppedProcess
        lock.unlock()
        guard let held = held else { return .notConfirmed }
        _ = stopProcessUnlocked()
        lock.lock()
        defer { lock.unlock() }
        try requireMaintenanceOwner(transactionID)
        guard !held.isRunning, process == nil || process === held else { return .notConfirmed }
        maintenanceStoppedProcess = held
        maintenanceStoppedTransaction = transactionID
        return .stopped
    }

    /// No implicit restart: only the existing supervisor reconcile may start afterwards.
    public func finishMaintenance(transactionID: String) throws {
        stopLock.lock()
        defer { stopLock.unlock() }
        lock.lock()
        defer { lock.unlock() }
        try requireMaintenanceOwner(transactionID)
        guard maintenanceStoppedTransaction == transactionID,
              let held = maintenanceStoppedProcess, !held.isRunning,
              process == nil || process === held else { throw CoreMaintenanceError.notConfirmed }
        var removed = false
        do {
            guard maintenanceMarkerUnlink(maintenanceURL.path) == 0 else { throw CoreMaintenanceError.ioFailure(errno) }
            removed = true
            try syncMaintenanceDirectory()
        } catch {
            // Unlink is not the commit point: directory persistence must succeed too.
            // Latch before recovery while holding the same lock as start/reconcile.
            maintenanceCompletionFailed = true
            maintenanceRecoveryPersistenceConfirmed = false
            do {
                if removed {
                    try publishMaintenanceMarkerLocked(transactionID: transactionID)
                } else {
                    try syncMaintenanceDirectory()
                }
                maintenanceRecoveryPersistenceConfirmed = true
            } catch {
                // Disk/permissions may prevent restoring any marker. A rebuilt instance
                // then needs the updater journal; memory alone cannot survive a restart.
                fputs("CoreSupervisor maintenance completion failed; local recovery required; marker durability unconfirmed\n", stderr)
            }
            throw error // Preserve the completion error, not a later restoration error.
        }
        maintenanceStoppedProcess = nil
        maintenanceStoppedTransaction = nil
    }

    public var currentStatus: CoreProcessStatus {
        lock.lock()
        defer { lock.unlock() }
        if isStopping {
            let pid = process?.processIdentifier ?? 0
            return .stopping(pid: pid)
        }
        if let proc = process, proc.isRunning {
            return .running(pid: proc.processIdentifier)
        }
        if isConfigError {
            return .configError
        }
        if gaveUp {
            return .gaveUp
        }
        if let at = restartingAt {
            if Date() < at {
                return .restarting(at: at)
            }
        }
        return .notRunning
    }

    public func reset() {
        lock.lock()
        defer { lock.unlock() }
        restartPolicy.reset()
        gaveUp = false
        isConfigError = false
        isStopping = false
        manualStoppingPIDs.removeAll()
        restartingAt = nil
        lastExitCode = nil
    }

    /// 核心生命周期协调函数：检查开关与互斥状态，执行启停
    @discardableResult
    public func checkAndReconcile(now: Date = Date()) -> CoreDecision {
        // startProcess rechecks under the same lock used to publish the marker.
        guard maintenanceStatus == .inactive else { return .keep }
        let enabled = FileManager.default.fileExists(atPath: coreEnabledURL.path)
        let pyLoaded = pythonLoadedChecker()
        let status = currentStatus
        let decision = decideCoreAction(coreEnabled: enabled, pythonLoaded: pyLoaded, status: status, now: now)

        switch decision {
        case .start:
            do {
                try startProcess()
            } catch {
                fputs("CoreSupervisor failed to start process: \(error.localizedDescription)\n", stderr)
            }
        case .stop, .conflict:
            if case .running = status {
                lock.lock()
                isStopping = true
                lock.unlock()

                queue.async { [weak self] in
                    guard let self = self else { return }
                    _ = self.stopProcess()
                    self.lock.lock()
                    self.isStopping = false
                    self.lock.unlock()
                }
            }
        case .keep:
            break
        }
        return decision
    }

    /// 启动 serve-core 子进程
    public func startProcess(
        binaryPath: String? = nil,
        arguments: [String]? = nil,
        logPath: String? = nil,
        customGracePeriod: TimeInterval? = nil
    ) throws {
        lock.lock()
        defer { lock.unlock() }

        guard readStartupDualGuardLocked() == .inactive else { throw CoreMaintenanceError.recoveryRequired }
        if let proc = process, proc.isRunning {
            return
        }

        let targetBinary = binaryPath != nil ? URL(fileURLWithPath: binaryPath!) : binaryURL
        let targetLog = logPath != nil ? URL(fileURLWithPath: logPath!) : logURL

        // 确保日志目录存在 (0700)
        let logDir = targetLog.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])

        // 启动前检查日志轮换：超过 10MB 时轮换为 .1 到 .5
        LogRotator.rotateIfNeeded(fileURL: targetLog)

        // 确保日志文件存在并设置 0600 权限
        if !FileManager.default.fileExists(atPath: targetLog.path) {
            FileManager.default.createFile(atPath: targetLog.path, contents: Data(), attributes: [.posixPermissions: 0o600])
        } else {
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: targetLog.path)
        }

        let logHandle = try FileHandle(forWritingTo: targetLog)
        try logHandle.seekToEnd()

        let proc = Process()
        proc.executableURL = targetBinary
        proc.arguments = arguments ?? ["serve-core", "--config", configURL.path]
        proc.standardOutput = logHandle
        proc.standardError = logHandle

        // 显式设置 LANG 和 LC_ALL 为 en_US.UTF-8
        var env = ProcessInfo.processInfo.environment
        env["LANG"] = "en_US.UTF-8"
        env["LC_ALL"] = "en_US.UTF-8"
        proc.environment = env

        proc.terminationHandler = { [weak self] p in
            try? logHandle.close()
            self?.handleProcessTermination(pid: p.processIdentifier, exitCode: p.terminationStatus, reason: p.terminationReason)
        }

        try proc.run()
        self.process = proc
        self.restartingAt = nil
    }

    private func handleProcessTermination(pid: Int32, exitCode: Int32, reason: Process.TerminationReason) {
        lock.lock()
        defer { lock.unlock() }

        self.process = nil
        self.lastExitCode = exitCode
        self.isStopping = false

        // 主动停止的进程（通过 stopProcess 终止）：不计入崩溃，状态回到可启动
        if manualStoppingPIDs.remove(pid) != nil {
            self.isConfigError = false
            self.restartingAt = nil
            return
        }

        // 退出码 0：正常退出，不计入崩溃
        if reason == .exit && exitCode == 0 {
            self.isConfigError = false
            self.restartingAt = nil
            return
        }

        // 退出码 2：配置错误，停止并不重启
        if reason == .exit && exitCode == 2 {
            self.isConfigError = true
            self.restartingAt = nil
            return
        }

        // 其他非零退出码或信号：按 RestartPolicy 退避
        let decision = restartPolicy.recordExit(at: Date())
        switch decision {
        case .giveUp:
            self.gaveUp = true
            self.restartingAt = nil
        case .restart(let after):
            self.restartingAt = Date().addingTimeInterval(after)
        }
    }

    /// 停止子进程：先发 SIGTERM，最多等 gracePeriod 秒，超时发 SIGKILL
    @discardableResult
    public func stopProcess(gracePeriod: TimeInterval? = nil) -> String {
        stopLock.lock()
        defer { stopLock.unlock() }

        return stopProcessUnlocked(gracePeriod: gracePeriod)
    }

    private func stopProcessUnlocked(gracePeriod: TimeInterval? = nil) -> String {
        let (proc, timeout) = { () -> (Process?, TimeInterval) in
            lock.lock()
            defer { lock.unlock() }
            return (self.process, gracePeriod ?? self.gracePeriod)
        }()

        guard let proc = proc, proc.isRunning else {
            lock.lock()
            self.isStopping = false
            lock.unlock()
            return "none"
        }

        let pid = proc.processIdentifier
        lock.lock()
        manualStoppingPIDs.insert(pid)
        lock.unlock()
        // 发送 SIGTERM
        kill(pid, SIGTERM)

        // 等待退出，最多 timeout 秒
        let deadline = Date().addingTimeInterval(timeout)
        var exited = false
        while Date() < deadline {
            if !proc.isRunning {
                exited = true
                break
            }
            usleep(25_000) // 25ms
        }

        if exited {
            lock.lock()
            self.isStopping = false
            lock.unlock()
            return "sigterm"
        }

        // 超时，发送 SIGKILL
        kill(pid, SIGKILL)
        while proc.isRunning {
            usleep(25_000)
        }
        lock.lock()
        self.isStopping = false
        lock.unlock()
        return "sigkill"
    }

    /// 便于测试：等待当前子进程退出
    public func waitUntilExit() {
        let proc = { () -> Process? in
            lock.lock()
            defer { lock.unlock() }
            return self.process
        }()
        proc?.waitUntilExit()
    }
}
