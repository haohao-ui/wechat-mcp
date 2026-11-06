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
}
