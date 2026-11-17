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
    /// 核心写入 ~/.wechat-rpa/folder-permission-needed.json 时的 detail（如 folder_access_blocked:Desktop）
    public var folderPermissionBlocked: String?

    public init(screenRecordingAuthorized: Bool, accessibilityTrusted: Bool, folderPermissionBlocked: String? = nil) {
        self.screenRecordingAuthorized = screenRecordingAuthorized
        self.accessibilityTrusted = accessibilityTrusted
        self.folderPermissionBlocked = folderPermissionBlocked
    }

    enum CodingKeys: String, CodingKey {
        case screenRecordingAuthorized = "screen_recording_authorized"
        case accessibilityTrusted = "accessibility_trusted"
        case folderPermissionBlocked = "folder_permission_blocked"
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
    public var coreEnabled: Bool?
    public var coreStatus: String?
    public var corePid: Int32?
    public var coreStoppedReason: String?

    public init(
        launchdLoaded: Bool,
        healthState: String? = nil,
        healthUpdatedAt: Double? = nil,
        now: Double = Date().timeIntervalSince1970,
        needsRecalibration: Bool? = nil,
        coreEnabled: Bool? = nil,
        coreStatus: String? = nil,
        corePid: Int32? = nil,
        coreStoppedReason: String? = nil
    ) {
        self.launchdLoaded = launchdLoaded
        self.healthState = healthState
        self.healthUpdatedAt = healthUpdatedAt
        self.now = now
        self.needsRecalibration = needsRecalibration
        self.coreEnabled = coreEnabled
        self.coreStatus = coreStatus
        self.corePid = corePid
        self.coreStoppedReason = coreStoppedReason
    }

    enum CodingKeys: String, CodingKey {
        case launchdLoaded = "launchd_loaded"
        case healthState = "health_state"
        case healthUpdatedAt = "health_updated_at"
        case now
        case needsRecalibration = "needs_recalibration"
        case coreEnabled = "core_enabled"
        case coreStatus = "core_status"
        case corePid = "core_pid"
        case coreStoppedReason = "core_stopped_reason"
    }
}

public struct SessionHealthInput: Codable {
    public var state: String
    public var reason: String?
    public var code: String?
    public var handoff: Bool?

    public init(state: String, reason: String? = nil, code: String? = nil, handoff: Bool? = nil) {
        self.state = state
        self.reason = reason
        self.code = code
        self.handoff = handoff
    }
}

public struct BrainInput: Codable {
    public var mode: String?
    public var pending: Int?
    public var lastPollAt: Double?
    public var now: Double?

    public init(mode: String? = "builtin", pending: Int? = nil, lastPollAt: Double? = nil, now: Double? = nil) {
        self.mode = mode
        self.pending = pending
        self.lastPollAt = lastPollAt
        self.now = now
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case pending
        case lastPollAt = "last_poll_at"
        case now
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
    public var brain: BrainInput?
    /// 模型配置状态，原样取自 ConfigFormLogic.maskConfig 的 model_state 字段；
    /// 字段缺失（旧版本）时为 nil，按「状态未知」处理，不报「模型未配置」。
    public var modelState: String?

    public init(
        permissions: PermissionsInput,
        desktopService: DesktopServiceInput,
        bot: BotInput,
        sessions: [String: SessionHealthInput],
        wechat: WechatInput,
        layout: LayoutInput,
        disk: DiskInput,
        brain: BrainInput? = nil,
        modelState: String? = nil
    ) {
        self.permissions = permissions
        self.desktopService = desktopService
        self.bot = bot
        self.sessions = sessions
        self.wechat = wechat
        self.layout = layout
        self.disk = disk
        self.brain = brain
        self.modelState = modelState
    }

    enum CodingKeys: String, CodingKey {
        case permissions
        case desktopService = "desktop_service"
        case bot
        case sessions
        case wechat
        case layout
        case disk
        case brain
        case modelState = "model_state"
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

public func sessionStateHumanReadable(
    state: String,
    reason: String? = nil,
    code: String? = nil,
    handoff: Bool? = nil
) -> String {
    if handoff == true || state == "handoff" {
        return L10n.t(.stateHandoff)
    }
    if state == "needs_recalibration" {
        return L10n.t(.stateNeedsRecalibration)
    }
    if state == "watching" || state == "listening" {
        return L10n.t(.stateListening)
    }
    if state == "replied" || state == "replying" {
        return L10n.t(.stateReplying)
    }
    if state == "waiting_for_wechat_window" {
        let combined = "\(reason ?? "") \(code ?? "")"
        if combined.contains("session_row_not_unique_or_missing") {
            return L10n.t(.stateWaitRowMissing)
        } else if combined.contains("window_blocked") || combined.contains("blocking") {
            return L10n.t(.stateWaitBlocked)
        } else {
            let detail = L10n.t(.stateWindowNotReady)
            return L10n.t(.stateWaitWindow, detail)
        }
    }
    return L10n.t(.modelStateUnknown)
}

public func formatBytes(_ bytes: Int64) -> String {
    let gb = Double(bytes) / Double(1024 * 1024 * 1024)
    if gb >= 1.0 {
        return String(format: "%.1f GB", gb)
    }
    let mb = Double(bytes) / Double(1024 * 1024)
    return String(format: "%.1f MB", mb)
}

/// 检查项的显示名（总览「系统检查与诊断」和诊断报告共用）。
public func statusItemTitle(key: String) -> String {
    switch key {
    case "permissions": return L10n.t(.itemPermissions)
    case "desktop_service": return L10n.t(.itemDesktopService)
    case "bot": return L10n.t(.itemBot)
    case "sessions": return L10n.t(.itemSessions)
    case "wechat": return L10n.t(.itemWechat)
    case "layout": return L10n.t(.itemLayout)
    case "disk": return L10n.t(.itemDisk)
    case "brain": return L10n.t(.itemReplyMode)
    default: return key
    }
}

private func pidSuffix(_ pid: Int32?) -> String {
    guard let pid = pid else { return "" }
    return " (PID: \(pid))"
}

public func evaluate(_ s: StatusInput) -> [StatusItem] {
    var items: [StatusItem] = []

    // 1. permissions
    let permItem: StatusItem
    if s.permissions.screenRecordingAuthorized && s.permissions.accessibilityTrusted {
        permItem = StatusItem(key: "permissions", level: .green, reason: L10n.t(.evPermOK), fix: nil)
    } else if !s.permissions.screenRecordingAuthorized && !s.permissions.accessibilityTrusted {
        permItem = StatusItem(key: "permissions", level: .red, reason: L10n.t(.evPermNone), fix: L10n.t(.evPermNoneFix))
    } else if !s.permissions.screenRecordingAuthorized {
        permItem = StatusItem(key: "permissions", level: .red, reason: L10n.t(.evPermScreen), fix: L10n.t(.evPermScreenFix))
    } else {
        permItem = StatusItem(key: "permissions", level: .red, reason: L10n.t(.evPermAccessibility), fix: L10n.t(.evPermAccessibilityFix))
    }
    if permItem.level == .green, let blocked = s.permissions.folderPermissionBlocked {
        // 附件保存在受保护文件夹而未授权：图片只能用缩略图，文件读不到
        items.append(StatusItem(
            key: "permissions", level: .red,
            reason: L10n.t(.evFolderBlocked, blocked),
            fix: L10n.t(.evFolderFix)))
    } else {
        items.append(permItem)
    }

    // 2. desktop_service
    let svcItem: StatusItem
    if s.desktopService.status == "running" && s.desktopService.socketExists {
        svcItem = StatusItem(key: "desktop_service", level: .green, reason: L10n.t(.evDesktopRunning, pidSuffix(s.desktopService.pid)), fix: nil)
    } else if s.desktopService.status == "idle" && s.desktopService.socketExists {
        svcItem = StatusItem(key: "desktop_service", level: .green, reason: L10n.t(.evDesktopIdle), fix: nil)
    } else if s.desktopService.status == "restarting" {
        svcItem = StatusItem(key: "desktop_service", level: .yellow, reason: L10n.t(.evDesktopRestarting), fix: L10n.t(.evDesktopRestartingFix))
    } else {
        let reason = L10n.t(.evDesktopStopped)
        svcItem = StatusItem(key: "desktop_service", level: .red, reason: reason, fix: L10n.t(.evDesktopStoppedFix))
    }
    items.append(svcItem)

    // 3. bot
    let botItem: StatusItem
    let isCore = s.bot.coreEnabled ?? false

    if isCore {
        if s.bot.launchdLoaded {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotConflict), fix: L10n.t(.evBotConflictFix))
        } else if s.bot.coreStatus == "stopping" {
            botItem = StatusItem(key: "bot", level: .yellow, reason: L10n.t(.evBotStopping), fix: nil)
        } else if s.bot.coreStatus == "restarting" {
            botItem = StatusItem(key: "bot", level: .yellow, reason: L10n.t(.evBotBackoff), fix: L10n.t(.evBotBackoffFix))
        } else if s.bot.coreStatus == "gave_up" {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotGaveUp), fix: L10n.t(.evBotGaveUpFix))
        } else if s.bot.coreStatus == "config_error" {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotConfigError), fix: L10n.t(.evBotConfigErrorFix))
        } else if s.bot.coreStatus == "stopped" || (s.bot.coreStatus != "running" && s.bot.corePid == nil) {
            let reason = L10n.t(.evBotNotRunning)
            botItem = StatusItem(key: "bot", level: .red, reason: reason, fix: L10n.t(.evBotRestartFix))
        } else {
            // Rust 核心运行中
            let pidStr = pidSuffix(s.bot.corePid)
            if s.bot.healthUpdatedAt == nil || s.bot.healthState == nil {
                botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotNoHealth, pidStr), fix: L10n.t(.evBotNoHealthFixCore))
            } else if (s.bot.now - (s.bot.healthUpdatedAt ?? 0)) > 120.0 {
                botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotStale, pidStr), fix: L10n.t(.evBotStaleFix))
            } else if s.bot.healthState == "needs_recalibration" || s.bot.needsRecalibration == true {
                botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotRecalibrate, pidStr), fix: L10n.t(.evBotRecalibrateFix))
            } else if s.bot.healthState == "paused" {
                botItem = StatusItem(key: "bot", level: .yellow, reason: L10n.t(.evBotPaused, pidStr), fix: L10n.t(.evBotPausedFix))
            } else {
                botItem = StatusItem(key: "bot", level: .green, reason: L10n.t(.evBotCoreRunning, pidStr, s.bot.healthState ?? "running"), fix: nil)
            }
        }
    } else {
        if !s.bot.launchdLoaded {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotPythonNotLoaded), fix: L10n.t(.evBotPythonNotLoadedFix))
        } else if s.bot.healthUpdatedAt == nil || s.bot.healthState == nil {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotNoHealth, ""), fix: L10n.t(.evBotNoHealthFixPython))
        } else if (s.bot.now - (s.bot.healthUpdatedAt ?? 0)) > 120.0 {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotStale, ""), fix: L10n.t(.evBotStaleFix))
        } else if s.bot.healthState == "needs_recalibration" || s.bot.needsRecalibration == true {
            botItem = StatusItem(key: "bot", level: .red, reason: L10n.t(.evBotRecalibrate, ""), fix: L10n.t(.evBotRecalibrateFix))
        } else if s.bot.healthState == "paused" {
            botItem = StatusItem(key: "bot", level: .yellow, reason: L10n.t(.evBotPaused, ""), fix: L10n.t(.evBotPausedFix))
        } else {
            botItem = StatusItem(key: "bot", level: .green, reason: L10n.t(.evBotPythonRunning, s.bot.healthState ?? "running"), fix: nil)
        }
    }
    items.append(botItem)

    // 4. sessions
    let sessionsItem: StatusItem
    var waitingSessions: [(name: String, code: String)] = []
    var recalibrationSessions: [String] = []
    var handoffSessions: [String] = []
    for (name, session) in s.sessions.sorted(by: { $0.key < $1.key }) {
        if session.state == "needs_recalibration" {
            recalibrationSessions.append(name)
        } else if session.state == "handoff" || session.handoff == true {
            handoffSessions.append(name)
        } else if session.state == "waiting_for_wechat_window" {
            let code = session.code ?? session.reason ?? "unknown"
            waitingSessions.append((name: name, code: code))
        }
    }
    if !recalibrationSessions.isEmpty {
        sessionsItem = StatusItem(key: "sessions", level: .red, reason: L10n.t(.evSessionsRecalibrate, recalibrationSessions.joined(separator: ", ")), fix: L10n.t(.evSessionsRecalibrateFix))
    } else if !handoffSessions.isEmpty {
        sessionsItem = StatusItem(key: "sessions", level: .yellow, reason: L10n.t(.evSessionsHandoff, handoffSessions.joined(separator: ", ")), fix: L10n.t(.evSessionsHandoffFix))
    } else if let firstWaiting = waitingSessions.first {
        let desc = sessionStateHumanReadable(state: "waiting_for_wechat_window", reason: firstWaiting.code, code: firstWaiting.code, handoff: false)
        sessionsItem = StatusItem(key: "sessions", level: .yellow, reason: L10n.t(.evSessionsWaiting, firstWaiting.name, desc), fix: L10n.t(.evSessionsWaitingFix))
    } else {
        sessionsItem = StatusItem(key: "sessions", level: .green, reason: L10n.t(.evSessionsOK, s.sessions.count), fix: nil)
    }
    items.append(sessionsItem)

    // 5. wechat
    let wechatItem: StatusItem
    if !s.wechat.isRunning {
        wechatItem = StatusItem(key: "wechat", level: .red, reason: L10n.t(.evWechatNotRunning), fix: L10n.t(.evWechatNotRunningFix))
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
            wechatItem = StatusItem(key: "wechat", level: .red, reason: L10n.t(.evWechatBlocked, blockingKind.rawValue), fix: L10n.t(.evWechatBlockedFix))
        } else if let mb = s.wechat.mainBounds {
            let dw = abs(mb.width - 880.0)
            let dh = abs(mb.height - 640.0)
            if dw > 2.0 || dh > 2.0 {
                wechatItem = StatusItem(key: "wechat", level: .yellow, reason: L10n.t(.evWechatSize, Int(mb.width), Int(mb.height)), fix: L10n.t(.evWechatRepositionFix))
            } else {
                wechatItem = StatusItem(key: "wechat", level: .green, reason: L10n.t(.evWechatReady), fix: nil)
            }
        } else {
            wechatItem = StatusItem(key: "wechat", level: .yellow, reason: L10n.t(.evWechatNoWindow), fix: L10n.t(.evWechatNoWindowFix))
        }
    }
    items.append(wechatItem)

    // 6. layout
    let layoutItem: StatusItem
    if s.layout.layoutExists {
        let vStr = s.layout.version != nil ? " (\(s.layout.version!))" : ""
        layoutItem = StatusItem(key: "layout", level: .green, reason: L10n.t(.evLayoutReady, vStr), fix: nil)
    } else {
        layoutItem = StatusItem(key: "layout", level: .red, reason: L10n.t(.evLayoutMissing), fix: L10n.t(.evLayoutMissingFix))
    }
    items.append(layoutItem)

    // 7. disk
    let diskItem: StatusItem
    let fiveGB: Int64 = 5 * 1024 * 1024 * 1024
    if s.disk.freeBytes < fiveGB {
        diskItem = StatusItem(key: "disk", level: .yellow, reason: L10n.t(.evDiskLow, formatBytes(s.disk.freeBytes)), fix: L10n.t(.evDiskLowFix))
    } else {
        diskItem = StatusItem(key: "disk", level: .green, reason: L10n.t(.evDiskOK, formatBytes(s.disk.freeBytes)), fix: nil)
    }
    items.append(diskItem)

    // 8. 回复方式（配置键仍叫 brain：内置回复 / 外部回复（MCP 接管））
    let brainMode = s.brain?.mode ?? "builtin"
    let brainItem: StatusItem
    if brainMode == "external" {
        let now = s.brain?.now ?? s.bot.now
        let lastPoll = s.brain?.lastPollAt
        let pending = s.brain?.pending ?? 0
        let isStale = (lastPoll == nil) || ((now - lastPoll!) > 60.0)
        let pendingText = pending > 0 ? L10n.t(.evExternalPending, pending) : ""

        if isStale {
            brainItem = StatusItem(key: "brain", level: .yellow, reason: L10n.t(.evExternalDisconnected, L10n.t(.replyModeExternal)) + pendingText, fix: L10n.t(.evExternalFix))
        } else {
            brainItem = StatusItem(key: "brain", level: .green, reason: L10n.t(.evExternalConnected, L10n.t(.replyModeExternal)) + pendingText, fix: nil)
        }
    } else {
        brainItem = StatusItem(key: "brain", level: .green, reason: L10n.t(.evBuiltinRunning, L10n.t(.replyModeBuiltin)), fix: nil)
    }
    items.append(brainItem)

    return items
}

// MARK: - 统一状态汇总
//
// 侧栏「核心服务」卡片、总览「需要处理」、会话页横幅、设置页「权限与连接」都只读 summarizeStatus 的结果，
// 不各自判断。优先级：离线 > 缺权限 > 模型未配置（> 手动暂停）。

public enum CoreServiceState: String, Equatable {
    case normal
    case offline
    case permissionMissing = "permission_missing"
    case modelUnconfigured = "model_unconfigured"
    case manuallyPaused = "manually_paused"
}

public enum ModelConfigState: String, Equatable {
    case configured
    case unconfigured
    /// 没设兼容接口，使用默认接入方式（codex-exec），不算「未配置」。
    case defaultBackend = "default"
    /// 拿不到 model_state（旧版本没有这个字段，或值不认识）。
    case unknown

    /// 宽松解析 T1 给出的 model_state 取值；不认识的一律算 unknown，不猜。
    public static func parse(_ raw: String?) -> ModelConfigState {
        guard let value = raw?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased(), !value.isEmpty else {
            return .unknown
        }
        switch value {
        case "configured", "ready", "ok":
            return .configured
        case "unconfigured", "not_configured", "missing", "missing_key", "incomplete":
            return .unconfigured
        case "default_codex", "default", "codex-exec", "codex_exec":
            return .defaultBackend
        default:
            return .unknown
        }
    }

    public var label: String {
        switch self {
        case .configured: return L10n.t(.modelConfigured)
        case .unconfigured: return L10n.t(.modelUnconfigured)
        case .defaultBackend: return L10n.t(.modelDefault)
        case .unknown: return L10n.t(.modelStateUnknown)
        }
    }

    public var level: StatusLevel? {
        switch self {
        case .configured: return .green
        case .unconfigured: return .yellow
        case .defaultBackend, .unknown: return nil
        }
    }
}

public enum StatusIssueKind: String, Equatable {
    case offline
    case screenRecording = "screen_recording"
    case accessibility
    case folderAccess = "folder_access"
    case modelUnconfigured = "model_unconfigured"
}

public enum StatusIssueAction: String, Equatable {
    case restartCore = "restart_core"
    case grantScreenRecording = "grant_screen_recording"
    case grantAccessibility = "grant_accessibility"
    case openFolderSettings = "open_folder_settings"
    case gotoModel = "goto_model"
}

public struct StatusIssue: Equatable {
    public let kind: StatusIssueKind
    public let level: StatusLevel
    /// 「需要处理」里的一行文字，例如「屏幕录制：缺失」。
    public let text: String
    public let action: StatusIssueAction
    public let actionTitle: String
}

public struct StatusSummary: Equatable {
    public let state: CoreServiceState
    public let level: StatusLevel
    /// 状态标签文字：正常 / 注意 / 异常。
    public let tagText: String
    /// 侧栏卡片上的一句话。
    public let cardText: String
    /// 会话页横幅文字；正常时为 nil（不显示横幅）。
    public let bannerText: String?
    /// 横幅 / 卡片对应的首要操作。
    public let primaryAction: StatusIssueAction?
    public let primaryActionTitle: String?
    /// 自动回复此刻是否实际停着（离线、缺屏幕录制/辅助功能、模型未配置、手动暂停）。
    public let autoReplyPaused: Bool
    /// 全部待处理项，按优先级排序：离线 → 权限 → 模型。
    public let issues: [StatusIssue]
    public let coreOffline: Bool
    /// 离线时的具体原因（取自检查项），其余情况为 nil。
    public let offlineReason: String?
    public let screenRecordingOK: Bool
    public let accessibilityOK: Bool
    public let folderBlockedDetail: String?
    public let modelState: ModelConfigState
}

public func statusTagText(_ level: StatusLevel) -> String {
    switch level {
    case .green: return L10n.t(.tagNormal)
    case .yellow: return L10n.t(.tagAttention)
    case .red: return L10n.t(.tagError)
    }
}

/// 核心服务是否离线：核心进程没在正常运行（未启动、停止中、崩溃退避、放弃重启、配置错误、
/// 与旧 Python 任务冲突、健康状态缺失或超过 120 秒未更新），或者权限齐全但桌面服务没在运行。
/// 缺权限导致桌面服务起不来不算离线——那是「缺权限」。
public func coreServiceOfflineReason(_ s: StatusInput, items: [StatusItem]) -> String? {
    let botReason = items.first(where: { $0.key == "bot" })?.reason
    let coreEnabled = s.bot.coreEnabled ?? false
    let healthMissing = s.bot.healthUpdatedAt == nil || s.bot.healthState == nil
    let healthStale = (s.bot.now - (s.bot.healthUpdatedAt ?? 0)) > 120.0

    var botOffline = false
    if coreEnabled {
        if s.bot.launchdLoaded {
            botOffline = true
        } else if let st = s.bot.coreStatus, ["stopping", "restarting", "gave_up", "config_error", "stopped"].contains(st) {
            botOffline = true
        } else if s.bot.coreStatus != "running" && s.bot.corePid == nil {
            botOffline = true
        } else if healthMissing || healthStale {
            botOffline = true
        }
    } else if !s.bot.launchdLoaded || healthMissing || healthStale {
        botOffline = true
    }
    if botOffline {
        return botReason ?? L10n.t(.evBotNotRunning)
    }

    let permissionsOK = s.permissions.screenRecordingAuthorized && s.permissions.accessibilityTrusted
    let desktopUp = s.desktopService.socketExists && (s.desktopService.status == "running" || s.desktopService.status == "idle")
    if permissionsOK && !desktopUp {
        return items.first(where: { $0.key == "desktop_service" })?.reason ?? L10n.t(.evDesktopStopped)
    }
    return nil
}

public func summarizeStatus(_ s: StatusInput, items: [StatusItem]? = nil) -> StatusSummary {
    let evaluated = items ?? evaluate(s)
    let offlineReason = coreServiceOfflineReason(s, items: evaluated)
    let screenOK = s.permissions.screenRecordingAuthorized
    let accessibilityOK = s.permissions.accessibilityTrusted
    let folderBlocked = s.permissions.folderPermissionBlocked
    let modelState = ModelConfigState.parse(s.modelState)
    // 外部回复（MCP 接管）时回复不由内置模型生成，模型未配置不会让自动回复停下。
    let modelBlocksReplies = modelState == .unconfigured && (s.brain?.mode ?? "builtin") != "external"
    let manuallyPaused = s.bot.healthState == "paused"

    var issues: [StatusIssue] = []
    if offlineReason != nil {
        issues.append(StatusIssue(kind: .offline, level: .red, text: L10n.t(.bannerOffline),
                                  action: .restartCore, actionTitle: L10n.t(.actionRestartCore)))
    }
    if !screenOK {
        issues.append(StatusIssue(kind: .screenRecording, level: .yellow,
                                  text: L10n.t(.issuePermissionMissing, L10n.t(.permScreen)),
                                  action: .grantScreenRecording, actionTitle: L10n.t(.actionGrant)))
    }
    if !accessibilityOK {
        issues.append(StatusIssue(kind: .accessibility, level: .yellow,
                                  text: L10n.t(.issuePermissionMissing, L10n.t(.permAccessibility)),
                                  action: .grantAccessibility, actionTitle: L10n.t(.actionGrant)))
    }
    if folderBlocked != nil {
        issues.append(StatusIssue(kind: .folderAccess, level: .yellow,
                                  text: L10n.t(.issueFolderBlocked, L10n.t(.permFolders)),
                                  action: .openFolderSettings, actionTitle: L10n.t(.actionGrant)))
    }
    if modelBlocksReplies {
        issues.append(StatusIssue(kind: .modelUnconfigured, level: .yellow, text: L10n.t(.bannerModelUnconfigured),
                                  action: .gotoModel, actionTitle: L10n.t(.actionGotoModel)))
    }

    let state: CoreServiceState
    let level: StatusLevel
    let cardText: String
    let bannerText: String?
    let primaryAction: StatusIssueAction?
    let primaryActionTitle: String?
    var paused = true

    if offlineReason != nil {
        state = .offline
        level = .red
        cardText = L10n.t(.cardOffline)
        bannerText = L10n.t(.bannerOffline)
        primaryAction = .restartCore
        primaryActionTitle = L10n.t(.actionRestartCore)
    } else if !screenOK || !accessibilityOK {
        state = .permissionMissing
        level = .yellow
        let name = !screenOK ? L10n.t(.permScreen) : L10n.t(.permAccessibility)
        cardText = L10n.t(.cardMissingPermission, name)
        bannerText = L10n.t(.bannerMissingPermission, name)
        primaryAction = !screenOK ? .grantScreenRecording : .grantAccessibility
        primaryActionTitle = L10n.t(.actionGrant)
    } else if let blocked = folderBlocked {
        // 文件夹受阻只影响附件保存，自动回复本身没有停。
        state = .permissionMissing
        level = .yellow
        cardText = L10n.t(.cardFolderBlocked)
        bannerText = L10n.t(.bannerFolderBlocked, blocked)
        primaryAction = .openFolderSettings
        primaryActionTitle = L10n.t(.actionGrant)
        paused = manuallyPaused || modelBlocksReplies
    } else if modelBlocksReplies {
        state = .modelUnconfigured
        level = .yellow
        cardText = L10n.t(.cardModelUnconfigured)
        bannerText = L10n.t(.bannerModelUnconfigured)
        primaryAction = .gotoModel
        primaryActionTitle = L10n.t(.actionGotoModel)
    } else if manuallyPaused {
        state = .manuallyPaused
        level = .yellow
        cardText = L10n.t(.cardManuallyPaused)
        bannerText = L10n.t(.bannerManuallyPaused)
        primaryAction = nil
        primaryActionTitle = nil
    } else {
        state = .normal
        level = .green
        cardText = L10n.t(.cardRunning)
        bannerText = nil
        primaryAction = nil
        primaryActionTitle = nil
        paused = false
    }

    return StatusSummary(
        state: state,
        level: level,
        tagText: statusTagText(level),
        cardText: cardText,
        bannerText: bannerText,
        primaryAction: primaryAction,
        primaryActionTitle: primaryActionTitle,
        autoReplyPaused: paused,
        issues: issues,
        coreOffline: offlineReason != nil,
        offlineReason: offlineReason,
        screenRecordingOK: screenOK,
        accessibilityOK: accessibilityOK,
        folderBlockedDetail: folderBlocked,
        modelState: modelState
    )
}

public func generateDiagnostics(items: [StatusItem], appVersion: String, hostInfo: String? = nil) -> String {
    var lines: [String] = []
    lines.append(L10n.t(.diagTitle))
    lines.append("\(L10n.t(.diagVersion)): \(appVersion)")
    if let host = hostInfo {
        lines.append("\(L10n.t(.diagHost)): \(host)")
    }
    lines.append("\(L10n.t(.diagTime)): \(ISO8601DateFormatter().string(from: Date()))")
    lines.append("----------------------------")
    for item in items {
        var line = "[\(statusItemTitle(key: item.key))] \(item.level.rawValue): \(item.reason)"
        if let fix = item.fix {
            line += L10n.t(.suggestionSuffix, fix)
        }
        lines.append(line)
    }
    lines.append("============================")
    return lines.joined(separator: "\n")
}

public func selectMainWindowBounds(_ windows: [(bounds: CGRect, onScreen: Bool)]) -> CGRect? {
    let visible = windows.filter { $0.onScreen }
    guard !visible.isEmpty else { return nil }
    return visible.max(by: { ($0.bounds.width * $0.bounds.height) < ($1.bounds.width * $1.bounds.height) })?.bounds
}
