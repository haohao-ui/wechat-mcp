import Foundation

public enum RestartDecision: Equatable {
    case restart(after: TimeInterval)
    case giveUp
}

public struct RestartPolicy {
    private var exitHistory: [Date] = []

    public init() {}

    public mutating func recordExit(at date: Date = Date()) -> RestartDecision {
        // 滚动 1 小时窗口：移除超过 3600 秒的历史退出记录
        exitHistory = exitHistory.filter { date.timeIntervalSince($0) <= 3600 }
        exitHistory.append(date)

        let count = exitHistory.count
        if count >= 5 {
            return .giveUp
        } else if count == 1 {
            return .restart(after: 5.0)
        } else if count == 2 {
            return .restart(after: 15.0)
        } else {
            return .restart(after: 60.0)
        }
    }

    public mutating func reset() {
        exitHistory.removeAll()
    }
}

public enum WorkerStatus: Equatable {
    case running(pid: Int32)
    case restarting(at: Date)
    case stopped(reason: String)
    /// Listening on the socket between per-connection children: healthy, not stopped.
    case idle
}

public struct ChildExitHandler {
    public private(set) var restartPolicy = RestartPolicy()
    public private(set) var gaveUp: Bool = false
    public var isGivenUp: Bool { gaveUp }
    public private(set) var status: WorkerStatus = .stopped(reason: "服务未启动")

    public var isAcceptingConnections: Bool {
        if gaveUp { return false }
        if case .restarting(let at) = status {
            return Date() >= at
        }
        return true
    }

    public init() {}

    public mutating func markReadyToAccept() {
        if !gaveUp {
            status = .idle
        }
    }

    public mutating func markRunning(pid: Int32) {
        status = .running(pid: pid)
    }

    public mutating func handleExit(status exitStatus: Int32,
                                    reason: Process.TerminationReason,
                                    at date: Date = Date()) -> RestartDecision? {
        let isAbnormal = (reason == .uncaughtSignal || exitStatus != 0)
        if !isAbnormal {
            // 正常退出（reason == .exit 且 status == 0）：
            // 不记录到 RestartPolicy，状态重置为等待客户端连接
            status = .idle
            return nil
        }

        let decision = restartPolicy.recordExit(at: date)
        switch decision {
        case .giveUp:
            gaveUp = true
            status = .stopped(reason: "连续崩溃达到上限（1小时内5次），已停止自动重启")
        case .restart(let after):
            status = .restarting(at: date.addingTimeInterval(after))
        }
        return decision
    }

    public mutating func reset() {
        restartPolicy.reset()
        gaveUp = false
        status = .idle
    }
}
