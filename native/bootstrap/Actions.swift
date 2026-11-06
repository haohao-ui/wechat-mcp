// Actions.swift: Host app one-click actions defined in spec §5.4.
import Foundation
import AppKit
import ApplicationServices

public enum ActionError: Error, LocalizedError {
    case wechatNotRunning
    case windowNotFound
    case multipleWindows(String)
    case accessibilityFailed(String)
    case cancelled

    public var errorDescription: String? {
        switch self {
        case .wechatNotRunning: return "微信未运行"
        case .windowNotFound: return "未找到微信主窗口"
        case .multipleWindows(let msg): return msg
        case .accessibilityFailed(let msg): return msg
        case .cancelled: return "操作已取消"
        }
    }
}

public struct WindowCandidate: Equatable {
    public let isMinimized: Bool
    public let isFullScreen: Bool
    public let frame: CGRect

    public init(isMinimized: Bool, isFullScreen: Bool, frame: CGRect) {
        self.isMinimized = isMinimized
        self.isFullScreen = isFullScreen
        self.frame = frame
    }
}

public enum WindowDecisionResult: Equatable {
    case adjust(targetOrigin: CGPoint, targetSize: CGSize)
    case rejected(String)
}

/// 纯函数：根据窗口列表和主窗口候选决定窗口调整策略（遵循 Worker.swift:140-170）
public func decideWindowAdjustment(
    windows: [WindowCandidate],
    mainWindow: WindowCandidate?,
    displayBounds: CGRect
) -> WindowDecisionResult {
    let nonMinimized = windows.filter { !$0.isMinimized }
    guard nonMinimized.count <= 1 else {
        return .rejected("微信有多个窗口，请先关闭其他窗口")
    }
    guard let main = mainWindow ?? nonMinimized.first else {
        return .rejected("未找到微信主窗口")
    }
    guard !main.isMinimized else {
        return .rejected("微信主窗口处于最小化状态")
    }
    guard !main.isFullScreen else {
        return .rejected("微信主窗口处于全屏状态")
    }

    let origin = main.frame.origin
    let bounds = displayBounds
    let targetX = max(bounds.minX, min(origin.x, bounds.maxX - 880))
    let targetY = max(bounds.minY + 25, min(origin.y, bounds.maxY - 640))
    return .adjust(
        targetOrigin: CGPoint(x: targetX, y: targetY),
        targetSize: CGSize(width: 880, height: 640)
    )
}

public struct HostActions {
    /// 1. 重启桌面服务 (子进程)
    /// 安全约束：机器人正在发送时先确认
    @discardableResult
    static func restartDesktopService(
        worker: SocketWorker,
        healthState: String?,
        confirmHandler: () -> Bool
    ) -> Bool {
        if healthState == "sending" {
            guard confirmHandler() else { return false }
        }
        worker.restartChild()
        return true
    }

    /// 2. 重启机器人
    /// 安全约束：机器人正在发送时先确认
    @discardableResult
    public static func restartBot(
        healthState: String?,
        confirmHandler: () -> Bool,
        runner: (() throws -> Void)? = nil
    ) throws -> Bool {
        if healthState == "sending" {
            guard confirmHandler() else { return false }
        }
        if let customRunner = runner {
            try customRunner()
        } else {
            let uid = geteuid()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
            process.arguments = ["kickstart", "-k", "gui/\(uid)/local.wechat-rpa.bot"]
            try process.run()
            process.waitUntilExit()
        }
        return true
    }

    /// 3. 暂停机器人：写入 <state_dir>/admin-pause.json
    public static func pauseBot(stateDir: URL) throws {
        try FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let pauseFile = stateDir.appendingPathComponent("admin-pause.json")
        let payload: [String: Any] = [
            "paused": true,
            "at": Date().timeIntervalSince1970
        ]
        let data = try JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted])
        try data.write(to: pauseFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pauseFile.path)
    }

    /// 3. 恢复机器人：删除 <state_dir>/admin-pause.json
    public static func resumeBot(stateDir: URL) throws {
        let pauseFile = stateDir.appendingPathComponent("admin-pause.json")
        if FileManager.default.fileExists(atPath: pauseFile.path) {
            try FileManager.default.removeItem(at: pauseFile)
        }
    }

    /// 4. 微信窗口放回标准位置
    /// 遵循 Worker.swift 规则：仅当且仅当恰有 1 个非最小化微信窗口时调整，使用 kAXMainWindowAttribute 并 clamp 到屏幕内
    public static func restoreWechatWindow(wechatPID: Int32?) throws {
        var targetPID: pid_t = 0
        if let pid = wechatPID, pid > 0 {
            targetPID = pid
        } else {
            let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.tencent.xinWeChat")
            guard let app = apps.first else {
                throw ActionError.wechatNotRunning
            }
            targetPID = app.processIdentifier
        }

        let appRef = AXUIElementCreateApplication(targetPID)
        var windowsVal: AnyObject?
        let axErr = AXUIElementCopyAttributeValue(appRef, kAXWindowsAttribute as CFString, &windowsVal)
        guard axErr == .success, let windows = windowsVal as? [AXUIElement], !windows.isEmpty else {
            throw ActionError.windowNotFound
        }

        var candidates: [(element: AXUIElement, candidate: WindowCandidate)] = []
        for win in windows {
            var minVal: AnyObject?
            let isMin = (AXUIElementCopyAttributeValue(win, kAXMinimizedAttribute as CFString, &minVal) == .success) && ((minVal as? Bool) == true)
            var fullVal: AnyObject?
            let isFull = (AXUIElementCopyAttributeValue(win, "AXFullScreen" as CFString, &fullVal) == .success) && ((fullVal as? Bool) == true)
            var posVal: AnyObject?
            var sizeVal: AnyObject?
            var pt = CGPoint.zero
            var sz = CGSize.zero
            if AXUIElementCopyAttributeValue(win, kAXPositionAttribute as CFString, &posVal) == .success, let pv = posVal {
                AXValueGetValue(pv as! AXValue, .cgPoint, &pt)
            }
            if AXUIElementCopyAttributeValue(win, kAXSizeAttribute as CFString, &sizeVal) == .success, let sv = sizeVal {
                AXValueGetValue(sv as! AXValue, .cgSize, &sz)
            }
            candidates.append((win, WindowCandidate(isMinimized: isMin, isFullScreen: isFull, frame: CGRect(origin: pt, size: sz))))
        }

        var mainVal: AnyObject?
        var mainElem: AXUIElement? = nil
        if AXUIElementCopyAttributeValue(appRef, kAXMainWindowAttribute as CFString, &mainVal) == .success, let mv = mainVal {
            mainElem = (mv as! AXUIElement)
        }

        let mainCandidate = candidates.first(where: { CFEqual($0.element, mainElem) })?.candidate

        var displayBounds = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let centerPt = CGPoint(
            x: (mainCandidate?.frame.origin.x ?? 0) + (mainCandidate?.frame.width ?? 880)/2,
            y: (mainCandidate?.frame.origin.y ?? 0) + (mainCandidate?.frame.height ?? 640)/2
        )
        var displayID = CGDirectDisplayID(0)
        var count: UInt32 = 0
        if CGGetDisplaysWithPoint(centerPt, 1, &displayID, &count) == .success, count == 1 {
            displayBounds = CGDisplayBounds(displayID)
        }

        let decision = decideWindowAdjustment(
            windows: candidates.map { $0.candidate },
            mainWindow: mainCandidate,
            displayBounds: displayBounds
        )

        switch decision {
        case .rejected(let reason):
            throw ActionError.multipleWindows(reason)
        case .adjust(var targetOrigin, var wantedSize):
            let targetWin = mainElem ?? candidates.first(where: { !$0.candidate.isMinimized })?.element
            guard let win = targetWin else { throw ActionError.windowNotFound }

            AXUIElementSetAttributeValue(win, kAXMinimizedAttribute as CFString, kCFBooleanFalse)

            if let pos = AXValueCreate(.cgPoint, &targetOrigin) {
                AXUIElementSetAttributeValue(win, kAXPositionAttribute as CFString, pos)
            }
            if let sz = AXValueCreate(.cgSize, &wantedSize) {
                AXUIElementSetAttributeValue(win, kAXSizeAttribute as CFString, sz)
            }
        }
    }

    /// 5. 打开附件 / 日志文件夹
    public static func openFolder(_ url: URL) {
        if !FileManager.default.fileExists(atPath: url.path) {
            try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        }
        NSWorkspace.shared.open(url)
    }

    /// 6. 复制诊断信息到剪贴板
    public static func copyDiagnostics(_ text: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(text, forType: .string)
    }
}
