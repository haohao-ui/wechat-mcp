import Cocoa
import ApplicationServices

// wechat_click <x> <y> [<width> <height>]  — clicks inside the control centred on (x, y), screen points.
// Without a size the control is a 12×12 square. --plan-only prints the planned events and posts nothing.
// wechat_click key <cmd+v|cmd+a|cmd+c|return> — presses keys, only while WeChat is the frontmost app.
// Movement, press, keys and checks come from the shared action module (native/action).
var arguments = Array(CommandLine.arguments.dropFirst())
let planOnly = arguments.first == "--plan-only"
if planOnly { arguments.removeFirst() }

if arguments.first == "clear-input" {
    guard arguments.count == 1 else { exit(1) }
    runClearInput(planOnly: planOnly)
}

if arguments.first == "input-text" {
    guard arguments.count == 3 else { exit(1) }
    guard let x = Double(arguments[1]), let y = Double(arguments[2]), x.isFinite, y.isFinite else { exit(1) }
    runInputText(x: x, y: y, planOnly: planOnly)
}

if arguments.first == "key" {
    guard arguments.count == 2, Actions.keyCombos[arguments[1]] != nil else {
        fputs("wechat_click: unsupported_key\n", stderr); exit(1)
    }
    if planOnly {
        let sink = PlanSink()
        do { try Actions(sink: sink).key(arguments[1]) } catch { fputs("wechat_click: \(error)\n", stderr); exit(1) }
        print(String(decoding: try! JSONSerialization.data(withJSONObject: ["events": sink.events]), as: UTF8.self))
        exit(0)
    }
    guard AXIsProcessTrusted() else {
        fputs("wechat_click: accessibility_permission_missing_for_executing_process\n", stderr); exit(1)
    }
    // Keys go to whatever app is in front: never type unless it is WeChat.
    guard NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.tencent.xinWeChat" else {
        fputs("wechat_click: frontmost_not_wechat\n", stderr); exit(3)
    }
    do { try Actions().key(arguments[1]) } catch { fputs("wechat_click: \(error)\n", stderr); exit(1) }
    exit(0)
}
guard arguments.count == 2 || arguments.count == 4 else { exit(1) }
let numbers = arguments.compactMap(Double.init)
guard numbers.count == arguments.count, numbers.allSatisfy({ $0.isFinite }) else { exit(1) }
let x = numbers[0], y = numbers[1]
let width = numbers.count == 4 ? numbers[2] : 12, height = numbers.count == 4 ? numbers[3] : 12
guard width > 0, height > 0 else { exit(1) }
let target = CGRect(x: x - width/2, y: y - height/2, width: width, height: height)



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
