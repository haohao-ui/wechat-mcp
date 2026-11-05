import Cocoa
import ApplicationServices

// wechat_click <x> <y> [<width> <height>]  — clicks inside the control centred on (x, y), screen points.
// Without a size the control is a 12×12 square. --plan-only prints the planned events and posts nothing.
// Movement, press and checks come from the shared action module (native/action).
var arguments = Array(CommandLine.arguments.dropFirst())
let planOnly = arguments.first == "--plan-only"
if planOnly { arguments.removeFirst() }
guard arguments.count == 2 || arguments.count == 4 else { exit(1) }
let numbers = arguments.compactMap(Double.init)
guard numbers.count == arguments.count, numbers.allSatisfy({ $0.isFinite }) else { exit(1) }
let x = numbers[0], y = numbers[1]
let width = numbers.count == 4 ? numbers[2] : 12, height = numbers.count == 4 ? numbers[3] : 12
guard width > 0, height > 0 else { exit(1) }
let target = CGRect(x: x - width/2, y: y - height/2, width: width, height: height)

/// Records what would be posted (pointer follows our own moves; no time passes).
final class PlanSink: EventSink {
    var position = CGEvent(source: nil)?.location ?? .zero
    var events: [[String: Any]] = []
    func cursor() -> CGPoint { position }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws {
        let names: [CGEventType: String] = [.mouseMoved: "moved", .leftMouseDown: "left_down", .leftMouseUp: "left_up"]
        events.append(["type": names[type] ?? "other", "x": Double(point.x), "y": Double(point.y)])
        position = point
    }
    func scroll(lines: Int32, at point: CGPoint) throws {}
    func sleep(_ seconds: Double) {}
}

if planOnly {
    let sink = PlanSink()
    do { try Actions(sink: sink).click(target) } catch { fputs("wechat_click: \(error)\n", stderr); exit(1) }
    let data = try! JSONSerialization.data(withJSONObject: ["events": sink.events])
    print(String(decoding: data, as: UTF8.self))
    exit(0)
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
do {
    try Actions().click(target)
} catch {
    fputs("wechat_click: \(error)\n", stderr)
    exit(1)
}
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
