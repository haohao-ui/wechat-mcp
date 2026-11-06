// Actions.swift: Host app one-click actions defined in spec §5.4.
import Foundation
import AppKit
import ApplicationServices

public enum ActionError: Error, LocalizedError {
    case wechatNotRunning
    case windowNotFound
    case accessibilityFailed(String)
    case cancelled

    public var errorDescription: String? {
        switch self {
        case .wechatNotRunning: return "微信未运行"
        case .windowNotFound: return "未找到微信主窗口"
        case .accessibilityFailed(let msg): return "辅助功能操作失败: \(msg)"
        case .cancelled: return "操作已取消"
        }
    }
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
    /// 通过 AX 设置窗口的位置 (50, 50) 和大小 (880x640)，不点击窗口内容
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

        // 找到最适合的主窗口（通常是第一个或者尺寸比较大的普通窗口）
        guard let mainWindow = windows.first else {
            throw ActionError.windowNotFound
        }

        // 取消最小化
        AXUIElementSetAttributeValue(mainWindow, kAXMinimizedAttribute as CFString, kCFBooleanFalse)

        // 调整大小至 880x640
        var size = CGSize(width: 880, height: 640)
        if let sizeVal = AXValueCreate(.cgSize, &size) {
            AXUIElementSetAttributeValue(mainWindow, kAXSizeAttribute as CFString, sizeVal)
        }

        // 调整位置至屏幕可视区域内标准坐标 (如 x: 60, y: 60)
        var position = CGPoint(x: 60, y: 60)
        if let posVal = AXValueCreate(.cgPoint, &position) {
            AXUIElementSetAttributeValue(mainWindow, kAXPositionAttribute as CFString, posVal)
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
