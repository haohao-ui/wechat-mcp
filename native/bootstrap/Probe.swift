// Read-only bootstrap probe; compatible with the test host's Swift 5.7.
import Foundation
import AppKit
import ApplicationServices
import CoreGraphics

struct Rect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    init(_ rect: CGRect) {
        x = Double(rect.minX); y = Double(rect.minY)
        width = Double(rect.width); height = Double(rect.height)
    }
}
struct Display: Codable {
    let id: UInt32
    let main: Bool
    let bounds: Rect
    let pixels_width: Int
    let pixels_height: Int
}
struct Window: Codable {
    let id: UInt32
    let bounds: Rect
    let on_screen: Bool
}
struct Probe: Codable {
    let display_query_status: Int32
    let displays: [Display]
    let screen_recording_authorized: Bool
    let accessibility_trusted: Bool
    let wechat_process_count: Int
    let wechat_windows: [Window]
}

func readDesktopProbe() -> Probe {
var count: UInt32 = 0
var status = CGGetActiveDisplayList(0, nil, &count)
var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
if count > 0 { status = CGGetActiveDisplayList(count, &displays, &count) }
let displayInfo = displays.prefix(Int(count)).map { id in
    Display(id: id, main: CGDisplayIsMain(id) != 0, bounds: Rect(CGDisplayBounds(id)),
            pixels_width: CGDisplayPixelsWide(id), pixels_height: CGDisplayPixelsHigh(id))
}
let applications = NSRunningApplication.runningApplications(withBundleIdentifier: "com.tencent.xinWeChat")
let pids = Set(applications.map { $0.processIdentifier })
let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let matching = windows.compactMap { entry -> Window? in
    guard let pid = entry[kCGWindowOwnerPID as String] as? NSNumber, pids.contains(pid.int32Value),
          (entry[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
          let id = entry[kCGWindowNumber as String] as? NSNumber,
          let bounds = entry[kCGWindowBounds as String] as? [String: Any],
          let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary),
          rect.width > 300, rect.height > 200 else { return nil }
    return Window(id: id.uint32Value, bounds: Rect(rect),
                  on_screen: (entry[kCGWindowIsOnscreen as String] as? Bool) ?? false)
}
return Probe(display_query_status: status.rawValue, displays: displayInfo,
                   screen_recording_authorized: CGPreflightScreenCaptureAccess(),
                   accessibility_trusted: AXIsProcessTrusted(),
                   wechat_process_count: applications.count, wechat_windows: matching)
}
