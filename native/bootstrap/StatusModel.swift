// StatusModel.swift: Pure logic evaluating host app status items and generating diagnostics.
import Foundation
import CoreGraphics

public enum StatusLevel: String, Codable, Equatable {
    case green
    case yellow
    case red
}

public struct StatusItem: Codable, Equatable {
    public let key: String
    public let level: StatusLevel
    public let reason: String
    public let fix: String?

    public init(key: String, level: StatusLevel, reason: String, fix: String? = nil) {
        self.key = key
        self.level = level
        self.reason = reason
        self.fix = fix
    }
}

public struct PermissionsInput: Codable {
    public var screenRecordingAuthorized: Bool
    public var accessibilityTrusted: Bool

    public init(screenRecordingAuthorized: Bool, accessibilityTrusted: Bool) {
        self.screenRecordingAuthorized = screenRecordingAuthorized
        self.accessibilityTrusted = accessibilityTrusted
    }

    enum CodingKeys: String, CodingKey {
        case screenRecordingAuthorized = "screen_recording_authorized"
        case accessibilityTrusted = "accessibility_trusted"
    }
}

public struct DesktopServiceInput: Codable {
    public var socketExists: Bool
    public var status: String
    public var pid: Int32?
    public var restartingAt: Double?
    public var stoppedReason: String?

    public init(socketExists: Bool, status: String, pid: Int32? = nil, restartingAt: Double? = nil, stoppedReason: String? = nil) {
        self.socketExists = socketExists
        self.status = status
        self.pid = pid
        self.restartingAt = restartingAt
        self.stoppedReason = stoppedReason
    }

    enum CodingKeys: String, CodingKey {
        case socketExists = "socket_exists"
        case status
        case pid
        case restartingAt = "restarting_at"
        case stoppedReason = "stopped_reason"
    }
}

public struct BotInput: Codable {
    public var launchdLoaded: Bool
    public var healthState: String?
    public var healthUpdatedAt: Double?
    public var now: Double
    public var needsRecalibration: Bool?

    public init(launchdLoaded: Bool, healthState: String? = nil, healthUpdatedAt: Double? = nil, now: Double = Date().timeIntervalSince1970, needsRecalibration: Bool? = nil) {
        self.launchdLoaded = launchdLoaded
        self.healthState = healthState
        self.healthUpdatedAt = healthUpdatedAt
        self.now = now
        self.needsRecalibration = needsRecalibration
    }

    enum CodingKeys: String, CodingKey {
        case launchdLoaded = "launchd_loaded"
        case healthState = "health_state"
        case healthUpdatedAt = "health_updated_at"
        case now
        case needsRecalibration = "needs_recalibration"
    }
}

public struct SessionHealthInput: Codable {
    public var state: String
    public var reason: String?
    public var code: String?

    public init(state: String, reason: String? = nil, code: String? = nil) {
        self.state = state
        self.reason = reason
        self.code = code
    }
}

public struct RectPayload: Codable {
    public var x: Double
    public var y: Double
    public var width: Double
    public var height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }

    public var cgRect: CGRect {
        CGRect(x: x, y: y, width: width, height: height)
    }
}

public struct WechatInput: Codable {
    public var isRunning: Bool
    public var version: String?
    public var pid: Int32?
    public var mainBounds: RectPayload?
    public var rawWindows: [[String: AnyCodable]]?
    public var focusedIdentifier: String?

    public init(isRunning: Bool, version: String? = nil, pid: Int32? = nil, mainBounds: RectPayload? = nil, rawWindows: [[String: AnyCodable]]? = nil, focusedIdentifier: String? = nil) {
        self.isRunning = isRunning
        self.version = version
        self.pid = pid
        self.mainBounds = mainBounds
        self.rawWindows = rawWindows
        self.focusedIdentifier = focusedIdentifier
    }

    enum CodingKeys: String, CodingKey {
        case isRunning = "is_running"
        case version
        case pid
        case mainBounds = "main_bounds"
        case rawWindows = "windows"
        case focusedIdentifier = "focused_identifier"
    }
}

public struct LayoutInput: Codable {
    public var layoutExists: Bool
    public var version: String?

    public init(layoutExists: Bool, version: String? = nil) {
        self.layoutExists = layoutExists
        self.version = version
    }

    enum CodingKeys: String, CodingKey {
        case layoutExists = "layout_exists"
        case version
    }
}

public struct DiskInput: Codable {
    public var freeBytes: Int64
    public var attachmentBytes: Int64?

    public init(freeBytes: Int64, attachmentBytes: Int64? = nil) {
        self.freeBytes = freeBytes
        self.attachmentBytes = attachmentBytes
    }

    enum CodingKeys: String, CodingKey {
        case freeBytes = "free_bytes"
        case attachmentBytes = "attachment_bytes"
    }
}

public struct StatusInput: Codable {
    public var permissions: PermissionsInput
    public var desktopService: DesktopServiceInput
    public var bot: BotInput
    public var sessions: [String: SessionHealthInput]
    public var wechat: WechatInput
    public var layout: LayoutInput
    public var disk: DiskInput

    public init(
        permissions: PermissionsInput,
        desktopService: DesktopServiceInput,
        bot: BotInput,
        sessions: [String: SessionHealthInput],
        wechat: WechatInput,
        layout: LayoutInput,
        disk: DiskInput
    ) {
        self.permissions = permissions
        self.desktopService = desktopService
        self.bot = bot
        self.sessions = sessions
        self.wechat = wechat
        self.layout = layout
        self.disk = disk
    }

    enum CodingKeys: String, CodingKey {
        case permissions
        case desktopService = "desktop_service"
        case bot
        case sessions
        case wechat
        case layout
        case disk
    }
}

// AnyCodable helper for parsing arbitrary dictionary representation of windows
public struct AnyCodable: Codable {
    public let value: Any

    public init(_ value: Any) {
        self.value = value
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let b = try? container.decode(Bool.self) {
            self.value = b
        } else if let i = try? container.decode(Int.self) {
            self.value = i
        } else if let d = try? container.decode(Double.self) {
            self.value = d
        } else if let s = try? container.decode(String.self) {
            self.value = s
        } else if let arr = try? container.decode([AnyCodable].self) {
            self.value = arr.map { $0.value }
        } else if let dict = try? container.decode([String: AnyCodable].self) {
            var res: [String: Any] = [:]
            for (k, v) in dict { res[k] = v.value }
            self.value = res
        } else {
            self.value = ()
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        if let b = value as? Bool {
            try container.encode(b)
        } else if let i = value as? Int {
            try container.encode(i)
        } else if let d = value as? Double {
            try container.encode(d)
        } else if let s = value as? String {
            try container.encode(s)
        } else {
            try container.encodeNil()
        }
    }
}

public func formatBytes(_ bytes: Int64) -> String {
    let gb = Double(bytes) / Double(1024 * 1024 * 1024)
    if gb >= 1.0 {
        return String(format: "%.1f GB", gb)
    }
    let mb = Double(bytes) / Double(1024 * 1024)
    return String(format: "%.1f MB", mb)
}

public func evaluate(_ s: StatusInput) -> [StatusItem] {
    var items: [StatusItem] = []

    // 1. permissions
    let permItem: StatusItem
    if s.permissions.screenRecordingAuthorized && s.permissions.accessibilityTrusted {
        permItem = StatusItem(key: "permissions", level: .green, reason: "屏幕录制与辅助功能已授权", fix: nil)
    } else if !s.permissions.screenRecordingAuthorized && !s.permissions.accessibilityTrusted {
        permItem = StatusItem(key: "permissions", level: .red, reason: "屏幕录制与辅助功能未授权", fix: "请在「系统设置 → 隐私与安全性」中授予屏幕录制和辅助功能权限")
    } else if !s.permissions.screenRecordingAuthorized {
        permItem = StatusItem(key: "permissions", level: .red, reason: "屏幕录制未授权", fix: "请在「系统设置 → 隐私与安全性」中授予屏幕录制权限")
    } else {
        permItem = StatusItem(key: "permissions", level: .red, reason: "辅助功能未授权", fix: "请在「系统设置 → 隐私与安全性」中授予辅助功能权限")
    }
    items.append(permItem)

    // 2. desktop_service
    let svcItem: StatusItem
    if s.desktopService.status == "running" && s.desktopService.socketExists {
        let pidStr = s.desktopService.pid != nil ? " (PID: \(s.desktopService.pid!))" : ""
        svcItem = StatusItem(key: "desktop_service", level: .green, reason: "桌面服务运行中\(pidStr)", fix: nil)
    } else if s.desktopService.status == "restarting" {
        svcItem = StatusItem(key: "desktop_service", level: .yellow, reason: "桌面服务正在退避重试", fix: "等待自动重试或点击重启服务")
    } else {
        let reason = s.desktopService.stoppedReason ?? "桌面服务未运行"
        svcItem = StatusItem(key: "desktop_service", level: .red, reason: reason, fix: "点击「重启桌面服务」")
    }
    items.append(svcItem)

    // 3. bot
    let botItem: StatusItem
    if !s.bot.launchdLoaded {
        botItem = StatusItem(key: "bot", level: .red, reason: "机器人服务未加载 (launchd 缺失)", fix: "检查并加载 local.wechat-rpa.bot")
    } else if s.bot.healthUpdatedAt == nil || s.bot.healthState == nil {
        botItem = StatusItem(key: "bot", level: .red, reason: "未检测到机器人健康状态 (health.json 缺失)", fix: "启动机器人服务")
    } else if (s.bot.now - (s.bot.healthUpdatedAt ?? 0)) > 120.0 {
        botItem = StatusItem(key: "bot", level: .red, reason: "机器人健康状态超过 120 秒未更新", fix: "检查机器人进程或点击「重启机器人」")
    } else if s.bot.healthState == "needs_recalibration" || s.bot.needsRecalibration == true {
        botItem = StatusItem(key: "bot", level: .red, reason: "机器人需要重新校准 (needs_recalibration)", fix: "运行 control calibrate 重新定位")
    } else if s.bot.healthState == "paused" {
        botItem = StatusItem(key: "bot", level: .yellow, reason: "机器人已暂停", fix: "点击「恢复机器人」")
    } else {
        botItem = StatusItem(key: "bot", level: .green, reason: "机器人正常运行中 (\(s.bot.healthState ?? "running"))", fix: nil)
    }
    items.append(botItem)

    // 4. sessions
    let sessionsItem: StatusItem
    var waitingSessions: [(name: String, code: String)] = []
    var recalibrationSessions: [String] = []
    for (name, session) in s.sessions.sorted(by: { $0.key < $1.key }) {
        if session.state == "needs_recalibration" {
            recalibrationSessions.append(name)
        } else if session.state == "waiting_for_wechat_window" {
            let code = session.code ?? session.reason ?? "unknown"
            waitingSessions.append((name: name, code: code))
        }
    }
    if !recalibrationSessions.isEmpty {
        sessionsItem = StatusItem(key: "sessions", level: .red, reason: "会话【\(recalibrationSessions.joined(separator: ", "))】需要重新校准", fix: "运行校准生成新地图")
    } else if let firstWaiting = waitingSessions.first {
        sessionsItem = StatusItem(key: "sessions", level: .yellow, reason: "会话【\(firstWaiting.name)】处于 waiting_for_wechat_window: \(firstWaiting.code)", fix: "检查微信窗口是否可见且未被遮挡")
    } else {
        sessionsItem = StatusItem(key: "sessions", level: .green, reason: "所有会话状态正常 (\(s.sessions.count) 个)", fix: nil)
    }
    items.append(sessionsItem)

    // 5. wechat
    let wechatItem: StatusItem
    if !s.wechat.isRunning {
        wechatItem = StatusItem(key: "wechat", level: .red, reason: "微信进程未运行", fix: "请打开微信客户端")
    } else {
        // 解析窗口
        var windowDicts: [[String: Any]] = []
        if let raw = s.wechat.rawWindows {
            for row in raw {
                var d: [String: Any] = [:]
                for (k, v) in row { d[k] = v.value }
                windowDicts.append(d)
            }
        }
        let winInfos = windowInfos(windowDicts)
        let mainSize = CGSize(
            width: s.wechat.mainBounds?.width ?? 880,
            height: s.wechat.mainBounds?.height ?? 640
        )
        let blocking = blockingScreen(
            windows: winInfos,
            wechatPID: s.wechat.pid ?? 0,
            mainSize: mainSize,
            focusedIdentifier: s.wechat.focusedIdentifier
        )

        if let blockingKind = blocking {
            wechatItem = StatusItem(key: "wechat", level: .red, reason: "有其他微信窗口挡在主窗口前面 (\(blockingKind.rawValue))", fix: "关闭遮挡窗口，或点击「微信窗口放回标准位置」")
        } else if let mb = s.wechat.mainBounds {
            let dw = abs(mb.width - 880.0)
            let dh = abs(mb.height - 640.0)
            if dw > 2.0 || dh > 2.0 {
                wechatItem = StatusItem(key: "wechat", level: .yellow, reason: "微信主窗口尺寸 (\(Int(mb.width))x\(Int(mb.height))) 与标准 (880x640) 不符", fix: "点击「微信窗口放回标准位置」")
            } else {
                wechatItem = StatusItem(key: "wechat", level: .green, reason: "微信主窗口就绪且无遮挡", fix: nil)
            }
        } else {
            wechatItem = StatusItem(key: "wechat", level: .yellow, reason: "未检测到微信主窗口", fix: "打开微信主窗口并点击「微信窗口放回标准位置」")
        }
    }
    items.append(wechatItem)

    // 6. layout
    let layoutItem: StatusItem
    if s.layout.layoutExists {
        let vStr = s.layout.version != nil ? " (\(s.layout.version!))" : ""
        layoutItem = StatusItem(key: "layout", level: .green, reason: "当前版本布局地图就绪\(vStr)", fix: nil)
    } else {
        layoutItem = StatusItem(key: "layout", level: .red, reason: "当前微信版本没有布局地图", fix: "运行校准以生成布局地图")
    }
    items.append(layoutItem)

    // 7. disk
    let diskItem: StatusItem
    let fiveGB: Int64 = 5 * 1024 * 1024 * 1024
    if s.disk.freeBytes < fiveGB {
        diskItem = StatusItem(key: "disk", level: .yellow, reason: "磁盘剩余空间不足 5GB (剩余 \(formatBytes(s.disk.freeBytes)))", fix: "清理附件或日志以释放空间")
    } else {
        diskItem = StatusItem(key: "disk", level: .green, reason: "磁盘空间充足 (剩余 \(formatBytes(s.disk.freeBytes)))", fix: nil)
    }
    items.append(diskItem)

    return items
}

public func generateDiagnostics(items: [StatusItem], appVersion: String, hostInfo: String? = nil) -> String {
    var lines: [String] = []
    lines.append("=== 微信客服助手 诊断报告 ===")
    lines.append("版本: \(appVersion)")
    if let host = hostInfo {
        lines.append("宿主环境: \(host)")
    }
    lines.append("生成时间: \(ISO8601DateFormatter().string(from: Date()))")
    lines.append("----------------------------")
    for item in items {
        let name: String
        switch item.key {
        case "permissions": name = "权限"
        case "desktop_service": name = "桌面服务"
        case "bot": name = "机器人"
        case "sessions": name = "各会话"
        case "wechat": name = "微信"
        case "layout": name = "布局地图"
        case "disk": name = "磁盘"
        default: name = item.key
        }
        var line = "[\(name)] \(item.level.rawValue): \(item.reason)"
        if let fix = item.fix {
            line += " (建议: \(fix))"
        }
        lines.append(line)
    }
    lines.append("============================")
    return lines.joined(separator: "\n")
}
