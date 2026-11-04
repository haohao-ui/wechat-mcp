// Explicit live acceptance: two clicks in a verified foreground WeChat title bar. Never sends text.
import AppKit
import ApplicationServices

enum AcceptanceError: Error { case failed(String) }
func require(_ condition: Bool, _ reason: String) throws {
    if !condition { throw AcceptanceError.failed(reason) }
}
func run(_ executable: URL, _ arguments: [String] = []) throws -> Data {
    let process = Process(), output = Pipe()
    process.executableURL = executable; process.arguments = arguments
    process.standardOutput = output; process.standardError = FileHandle.standardError
    try process.run()
    let data = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    try require(process.terminationStatus == 0, "helper_failed")
    return data
}
func visibleOverlayWindows() -> Int {
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String:Any]] ?? []
    return rows.filter { ($0[kCGWindowOwnerName as String] as? String) == "wechat_overlay" }.count
}
func hitPID(_ point: CGPoint) throws -> pid_t {
    var element: AXUIElement?
    let status = AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &element)
    try require(status == .success && element != nil, "accessibility_hit_failed")
    var pid: pid_t = 0
    try require(AXUIElementGetPid(element!, &pid) == .success, "hit_pid_failed")
    return pid
}
func capture(_ rect: CGRect, _ path: URL) throws {
    guard let image = CGWindowListCreateImage(rect, .optionOnScreenOnly, kCGNullWindowID, [.bestResolution]) else {
        throw AcceptanceError.failed("capture_failed")
    }
    let bitmap = NSBitmapImageRep(cgImage: image)
    guard let data = bitmap.representation(using: .png, properties: [:]) else { throw AcceptanceError.failed("png_failed") }
    try data.write(to: path, options: .atomic)
    try FileManager.default.setAttributes([.posixPermissions:0o600], ofItemAtPath:path.path)
}

do {
    try require(CommandLine.arguments.count == 2, "helpers_path_required")
    try require(AXIsProcessTrusted() && CGPreflightScreenCaptureAccess(), "gui_permissions_missing")
    let directory = URL(fileURLWithPath: CommandLine.arguments[1])
    let evidence = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/overlay-acceptance")
    try FileManager.default.createDirectory(at:evidence, withIntermediateDirectories:true, attributes:[.posixPermissions:0o700])
    let info = try FileManager.default.attributesOfItem(atPath:evidence.path)
    try require((info[.ownerAccountID] as? NSNumber)?.uint32Value == getuid() &&
                (info[.posixPermissions] as? NSNumber)?.intValue == 0o700, "unsafe_evidence_directory")
    for name in ["render.json", "pointer-click.png", "badge-click.png"] {
        let path = evidence.appendingPathComponent(name)
        if FileManager.default.fileExists(atPath:path.path) { try FileManager.default.removeItem(at:path) }
    }
    let windowData = try run(directory.appendingPathComponent("wechat_window"))
    guard let window = try JSONSerialization.jsonObject(with:windowData) as? [String:Any],
          let x = window["x"] as? Double, let y = window["y"] as? Double,
          let w = window["width"] as? Double, let h = window["height"] as? Double,
          window["on_screen"] as? Bool == true else { throw AcceptanceError.failed("window_not_visible") }
    try require(w >= 700 && h >= 500, "window_too_small_for_safe_title_click")
    guard let foreground = NSWorkspace.shared.frontmostApplication,
          ["com.tencent.xinWeChat", "com.tencent.WeChat"].contains(foreground.bundleIdentifier ?? "") else {
        throw AcceptanceError.failed("wechat_not_foreground")
    }
    let overlay = Process(), input = Pipe(), output = Pipe()
    overlay.executableURL = directory.appendingPathComponent("wechat_overlay")
    overlay.arguments = ["--render-evidence", evidence.path]
    overlay.standardInput = input; overlay.standardOutput = output; overlay.standardError = FileHandle.standardError
    try overlay.run()
    var inputClosed = false
    defer {
        if !inputClosed { input.fileHandleForWriting.closeFile() }
        if overlay.isRunning { overlay.terminate() }
    }
    func event(_ value: [String:Any]) throws {
        var data = try JSONSerialization.data(withJSONObject:value); data.append(10)
        input.fileHandleForWriting.write(data)
        var line = Data()
        let deadline = Date().addingTimeInterval(3)
        while line.last != 10 {
            try require(Date() < deadline && line.count < 32, "overlay_ack_timeout_or_oversize")
            var fd = pollfd(fd:output.fileHandleForReading.fileDescriptor, events:Int16(POLLIN), revents:0)
            let ready = poll(&fd, 1, 100)
            if ready == 0 { continue }
            try require(ready > 0, "overlay_poll_failed")
            let byte = output.fileHandleForReading.readData(ofLength:1)
            try require(!byte.isEmpty, "overlay_closed")
            line.append(byte)
        }
        try require(String(decoding:line, as:UTF8.self) == "{\"ok\":true}\n", "overlay_ack_failed")
    }
    try event(["event":"window", "x":x,"y":y,"width":w,"height":h])
    Thread.sleep(forTimeInterval:0.12)
    let shown = visibleOverlayWindows()
    try require(shown == 2, "expected_two_overlay_panels")
    let topStrip = CGRect(x:x, y:max(0,y-40), width:w, height:100)
    try capture(topStrip, evidence.appendingPathComponent("located.png"))
    var clicks: [[String:Any]] = []
    for offset in [450.0, 510.0] {
        let point = CGPoint(x:x+offset, y:y+26)
        try require(try hitPID(point) == foreground.processIdentifier, "title_target_not_wechat")
        try event(["event":"move", "x":point.x,"y":point.y])
        let process = Process()
        process.executableURL = directory.appendingPathComponent("wechat_click")
        process.arguments = [String(Double(point.x)), String(Double(point.y))]
        process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.standardError
        try process.run()
        var positions: [CGPoint] = []
        while process.isRunning {
            if let p = CGEvent(source:nil)?.location { positions.append(p) }
            Thread.sleep(forTimeInterval:0.01)
        }
        process.waitUntilExit()
        try require(process.terminationStatus == 0, "actual_click_failed")
        try event(["event":"click", "x":point.x,"y":point.y])
        Thread.sleep(forTimeInterval:0.16)
        guard let actual = CGEvent(source:nil)?.location else { throw AcceptanceError.failed("no_pointer") }
        let distance = hypot(actual.x-point.x, actual.y-point.y)
        let distinct = Set(positions.map { "\(Int($0.x)),\(Int($0.y))" }).count
        let foregroundUnchanged = NSWorkspace.shared.frontmostApplication?.processIdentifier == foreground.processIdentifier
        let passesThrough = try hitPID(point) == foreground.processIdentifier
        try require(distance <= 2 && foregroundUnchanged && passesThrough, "click_focus_or_hit_mismatch")
        try require(distinct >= 3, "smooth_movement_not_observed")
        try capture(topStrip, evidence.appendingPathComponent("click-\(Int(offset)).png"))
        clicks.append(["x":point.x,"y":point.y,"actual_x":actual.x,"actual_y":actual.y,
                       "distinct_pointer_positions":distinct,"frontmost_unchanged":foregroundUnchanged,
                       "hit_passes_to_wechat":passesThrough,"overlay_panels":visibleOverlayWindows()])
    }
    try event(["event":"hide"])
    Thread.sleep(forTimeInterval:0.08)
    let hidden = visibleOverlayWindows()
    try require(hidden == 0, "overlay_not_hidden_for_capture")
    try capture(topStrip, evidence.appendingPathComponent("hidden.png"))
    try event(["event":"resume"])
    Thread.sleep(forTimeInterval:0.08)
    try require(visibleOverlayWindows() == 2, "overlay_not_resumed")
    Thread.sleep(forTimeInterval:2.2)
    let idle = visibleOverlayWindows()
    try require(idle == 0, "overlay_not_hidden_when_idle")
    let renderData = try Data(contentsOf:evidence.appendingPathComponent("render.json"))
    guard let render = try JSONSerialization.jsonObject(with:renderData) as? [String:Any] else {
        throw AcceptanceError.failed("render_evidence_missing")
    }
    try require((render["click_pulse"] as? Double ?? 0) > 0 && render["pointer_visible"] as? Bool == true &&
                render["badge_visible"] as? Bool == true && render["can_become_key"] as? Bool == false &&
                render["pointer_ignores_mouse"] as? Bool == true, "click_render_state_invalid")
    input.fileHandleForWriting.closeFile()
    inputClosed = true
    let deadline = Date().addingTimeInterval(2)
    while overlay.isRunning && Date() < deadline { Thread.sleep(forTimeInterval:0.02) }
    let exited = !overlay.isRunning
    try require(exited, "overlay_did_not_exit_after_eof")
    let report: [String:Any] = ["status":"passed", "window_id":window["window_id"] ?? 0,
        "visible_panels":shown,"clicks":clicks,"hidden_panels":hidden,"idle_panels":idle,
        "overlay_exited":exited,"message_send_calls":0,"display_count":NSScreen.screens.count,
        "click_render_verified":true]
    let result = try JSONSerialization.data(withJSONObject:report,options:[.sortedKeys])
    try result.write(to:evidence.appendingPathComponent("result.json"),options:.atomic)
    try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:evidence.appendingPathComponent("result.json").path)
    print(String(decoding:result,as:UTF8.self))
} catch {
    let data = try! JSONSerialization.data(withJSONObject:["status":"failed", "error":String(describing:error),"message_send_calls":0])
    print(String(decoding:data,as:UTF8.self)); exit(1)
}
