import Cocoa
import ApplicationServices

guard CommandLine.arguments.count >= 3 else {
    exit(1)
}

guard let x = Double(CommandLine.arguments[1]),
      let y = Double(CommandLine.arguments[2]), x.isFinite, y.isFinite else {
    exit(1)
}

guard AXIsProcessTrusted() else {
    fputs("wechat_click: accessibility_permission_missing_for_executing_process\n", stderr)
    exit(1)
}

let pt = CGPoint(x: x, y: y)
let debugDirectory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/diagnostics")
let debugEnabled = FileManager.default.fileExists(atPath: debugDirectory.appendingPathComponent("capture.enabled").path)
var audit: [String: Any] = ["x":x, "y":y, "accessibility_trusted":AXIsProcessTrusted(),
                           "post_event_allowed":CGPreflightPostEventAccess(),
                           "frontmost_app":NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "unknown"]
if debugEnabled {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    if let hit = windows.first(where: { entry in
        guard let bounds = entry[kCGWindowBounds as String] as? [String:Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { return false }
        return rect.width > 20 && rect.height > 20 && rect.contains(pt)
    }) {
        audit["hit_owner"] = hit[kCGWindowOwnerName as String] as? String ?? "unknown"
        audit["hit_window_id"] = (hit[kCGWindowNumber as String] as? NSNumber)?.uint32Value ?? 0
    }
}
let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: pt, mouseButton: .left)
let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: pt, mouseButton: .left)

guard let down = down, let up = up else {
    fputs("wechat_click: event_creation_failed\n", stderr)
    exit(1)
}
let origin = CGEvent(source: nil)?.location ?? pt
for step in 1...12 {
    let t = Double(step) / 12
    let progress = t * t * (3 - 2 * t)
    let position = CGPoint(x: origin.x + (pt.x-origin.x)*progress, y: origin.y + (pt.y-origin.y)*progress)
    guard let move = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: position, mouseButton: .left) else {
        fputs("wechat_click: movement_event_creation_failed\n", stderr)
        exit(1)
    }
    move.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.015)
}
down.post(tap: .cghidEventTap)
Thread.sleep(forTimeInterval: 0.06)
up.post(tap: .cghidEventTap)
if debugEnabled {
    if let location = CGEvent(source: nil)?.location {
        audit["pointer_after_x"] = Double(location.x)
        audit["pointer_after_y"] = Double(location.y)
    }
    do {
        let info = try FileManager.default.attributesOfItem(atPath: debugDirectory.path)
        guard (info[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
              ((info[.posixPermissions] as? NSNumber)?.intValue ?? 0o777) & 0o077 == 0 else { exit(1) }
        let output = debugDirectory.appendingPathComponent("click.json")
        try JSONSerialization.data(withJSONObject: audit).write(to: output, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions:0o600], ofItemAtPath: output.path)
    } catch {
        fputs("wechat_click: diagnostic_write_failed\n", stderr)
        exit(1)
    }
}
