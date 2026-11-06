// Screens.swift: Screen identification skeleton.
import Foundation
import CoreGraphics

public enum ScreenKind: String, Codable, Equatable {
    case main
    case contextMenu = "context_menu"
    case savePanel = "save_panel"
    case otherWechatWindow = "other_wechat_window"
    case unknown
}

public struct WindowInfo: Equatable {
    public let owner: String
    public let pid: Int32
    public let layer: Int
    public let bounds: CGRect
    public let onScreen: Bool

    public init(owner: String, pid: Int32, layer: Int, bounds: CGRect, onScreen: Bool) {
        self.owner = owner
        self.pid = pid
        self.layer = layer
        self.bounds = bounds
        self.onScreen = onScreen
    }
}

public struct FocusInfo: Equatable {
    public let frontmostBundle: String?
    public let focusedIdentifier: String?

    public init(frontmostBundle: String?, focusedIdentifier: String?) {
        self.frontmostBundle = frontmostBundle
        self.focusedIdentifier = focusedIdentifier
    }
}

private func parsePID(_ value: Any?) -> Int32? {
    if let n = value as? Int32 { return n }
    if let n = value as? Int { return Int32(n) }
    if let n = value as? NSNumber { return n.int32Value }
    return nil
}

private func parseLayer(_ value: Any?) -> Int? {
    if let n = value as? Int { return n }
    if let n = value as? NSNumber { return n.intValue }
    if let n = value as? Int32 { return Int(n) }
    return nil
}

private func parseBool(_ value: Any?) -> Bool? {
    if let b = value as? Bool { return b }
    if let n = value as? NSNumber { return n.boolValue }
    return nil
}

private func parseBounds(_ value: Any?) -> CGRect? {
    guard let dict = value as? [String: Any] else { return nil }

    if let rect = CGRect(dictionaryRepresentation: dict as CFDictionary) {
        if rect.width > 0 && rect.height > 0 && rect.width.isFinite && rect.height.isFinite {
            return rect
        }
    }

    var xVal: Double?
    if let num = dict["x"] as? NSNumber { xVal = num.doubleValue }
    else if let num = dict["X"] as? NSNumber { xVal = num.doubleValue }

    var yVal: Double?
    if let num = dict["y"] as? NSNumber { yVal = num.doubleValue }
    else if let num = dict["Y"] as? NSNumber { yVal = num.doubleValue }

    var wVal: Double?
    if let num = dict["w"] as? NSNumber { wVal = num.doubleValue }
    else if let num = dict["width"] as? NSNumber { wVal = num.doubleValue }
    else if let num = dict["Width"] as? NSNumber { wVal = num.doubleValue }

    var hVal: Double?
    if let num = dict["h"] as? NSNumber { hVal = num.doubleValue }
    else if let num = dict["height"] as? NSNumber { hVal = num.doubleValue }
    else if let num = dict["Height"] as? NSNumber { hVal = num.doubleValue }

    guard let x = xVal, let y = yVal, let w = wVal, let h = hVal,
          w > 0, h > 0, w.isFinite, h.isFinite else {
        return nil
    }

    return CGRect(x: x, y: y, width: w, height: h)
}

public func windowInfos(_ rows: [[String: Any]]) -> [WindowInfo] {
    return rows.compactMap { row in
        guard let owner = (row[kCGWindowOwnerName as String] as? String) ?? (row["owner"] as? String) else {
            return nil
        }
        guard let pid = parsePID(row[kCGWindowOwnerPID as String]) ?? parsePID(row["pid"]) else {
            return nil
        }
        guard let layer = parseLayer(row[kCGWindowLayer as String]) ?? parseLayer(row["layer"]) else {
            return nil
        }
        guard let bounds = parseBounds(row[kCGWindowBounds as String]) ?? parseBounds(row["bounds"]) else {
            return nil
        }
        guard let onScreen = parseBool(row[kCGWindowIsOnscreen as String]) ??
                             parseBool(row["on_screen"]) ??
                             parseBool(row["onScreen"]) else {
            return nil
        }

        return WindowInfo(owner: owner, pid: pid, layer: layer, bounds: bounds, onScreen: onScreen)
    }
}

private func isWechatBundle(_ bundle: String?) -> Bool {
    guard let b = bundle else { return false }
    return b == "com.tencent.xinWeChat" || b == "com.tencent.WeChat" || b == "WeChat" || b == "微信"
}

public func identifyScreen(windows: [WindowInfo], focus: FocusInfo, wechatPID: Int32, mainSize: CGSize) -> ScreenKind {
    // 1. focus.focusedIdentifier == "saveAsNameTextField" -> save_panel
    if focus.focusedIdentifier == "saveAsNameTextField" {
        return .savePanel
    }

    // 2. 微信进程有 layer != 0、宽 20..600、高 20..900 的屏上窗口 -> context_menu
    let hasContextMenu = windows.contains { w in
        w.pid == wechatPID &&
        w.onScreen &&
        w.layer != 0 &&
        w.bounds.width >= 20.0 && w.bounds.width <= 600.0 &&
        w.bounds.height >= 20.0 && w.bounds.height <= 900.0
    }
    if hasContextMenu {
        return .contextMenu
    }

    // 微信进程 layer 0 的屏上窗口
    let layer0 = windows.filter { w in
        w.pid == wechatPID && w.layer == 0 && w.onScreen
    }

    // 3. 前台为微信，且微信进程 layer 0 的屏上窗口恰好 1 个、尺寸与 mainSize 相差 <= 2 点 -> main
    if isWechatBundle(focus.frontmostBundle) && layer0.count == 1 {
        let win = layer0[0]
        let dw = abs(win.bounds.width - mainSize.width)
        let dh = abs(win.bounds.height - mainSize.height)
        if dw <= 2.0 && dh <= 2.0 {
            return .main
        }
    }

    // 4. 微信进程 layer 0 的屏上窗口多于 1 个 -> other_wechat_window
    if layer0.count > 1 {
        return .otherWechatWindow
    }

    // 5. 其余 -> unknown
    return .unknown
}

public /// CGWindowLevelForKey(.popUpMenuWindow): context menus; status items sit at 25.
let popUpMenuLayer = 101

func blockingScreen(windows: [WindowInfo], wechatPID: Int32, mainSize: CGSize, focusedIdentifier: String?) -> ScreenKind? {
    // 1. focusedIdentifier == "saveAsNameTextField" -> .savePanel
    if focusedIdentifier == "saveAsNameTextField" {
        return .savePanel
    }

    // 2. 微信进程有弹出菜单层（kCGPopUpMenuWindowLevel = 101）、宽 20..600、高 20..900 的屏上窗口 -> .contextMenu
    //    The menu-bar status item (layer 25, e.g. 52x24 at y=0) is always on screen and is not a menu.
    let hasContextMenu = windows.contains { w in
        w.pid == wechatPID &&
        w.onScreen &&
        w.layer == popUpMenuLayer &&
        w.bounds.width >= 20.0 && w.bounds.width <= 600.0 &&
        w.bounds.height >= 20.0 && w.bounds.height <= 900.0
    }
    if hasContextMenu {
        return .contextMenu
    }

    // 3. Another WeChat layer-0 window in front of the main window and overlapping it -> .otherWechatWindow.
    //    CGWindowList is front to back. A window behind the main one (device 2026-11-06: the 360x124
    //    download window left by "下载到...") is neither visible nor in the way and does not block.
    let layer0 = windows.filter { w in w.pid == wechatPID && w.layer == 0 && w.onScreen }
    if layer0.count > 1 {
        guard let mainIndex = layer0.firstIndex(where: { w in
            abs(w.bounds.width - mainSize.width) <= 2.0 && abs(w.bounds.height - mainSize.height) <= 2.0
        }) else { return .otherWechatWindow }
        let main = layer0[mainIndex].bounds
        if layer0[..<mainIndex].contains(where: { $0.bounds.intersects(main) }) {
            return .otherWechatWindow
        }
    }

    // 4. 否则 -> nil
    return nil
}
