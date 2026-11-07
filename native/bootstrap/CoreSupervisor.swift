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

public final class CoreSupervisor {
    private let lock = NSLock()
    private let stopLock = NSLock()
    private let queue = DispatchQueue(label: "local.wechat-mcp.core-supervisor")
    private var process: Process?
    private var restartPolicy = RestartPolicy()
    private var manualStoppingPIDs = Set<Int32>()

    public private(set) var gaveUp: Bool = false
    public private(set) var isConfigError: Bool = false
    public private(set) var isStopping: Bool = false
    public private(set) var restartingAt: Date?
    public private(set) var lastExitCode: Int32?

    public var coreEnabledURL: URL
    public var binaryURL: URL
    public var configURL: URL
    public var logURL: URL
    public var gracePeriod: TimeInterval
    public var pythonLoadedChecker: () -> Bool

    public init(
        coreEnabledURL: URL? = nil,
        binaryURL: URL? = nil,
        configURL: URL? = nil,
        logURL: URL? = nil,
        gracePeriod: TimeInterval = 240.0,
        pythonLoadedChecker: @escaping () -> Bool = checkPythonLaunchdLoaded
    ) {
        let home = FileManager.default.homeDirectoryForCurrentUser
        self.coreEnabledURL = coreEnabledURL ?? home.appendingPathComponent(".wechat-rpa/core.enabled")
        self.binaryURL = binaryURL ?? home.appendingPathComponent(".wechat-rpa/bin/wechat-mcp")
        self.configURL = configURL ?? home.appendingPathComponent(".WeChatRPABot/config.json")
        self.logURL = logURL ?? home.appendingPathComponent(".wechat-rpa/logs/core.log")
        self.gracePeriod = gracePeriod
        self.pythonLoadedChecker = pythonLoadedChecker
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
