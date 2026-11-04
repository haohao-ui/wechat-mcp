import Foundation
import AppKit
import ApplicationServices
import CoreGraphics

func wechatApplication() throws -> NSRunningApplication {
    let applications = NSRunningApplication.runningApplications(withBundleIdentifier: "com.tencent.xinWeChat")
    guard applications.count == 1, let application = applications.first else {
        throw RuntimeFailure("wechat_process_missing_or_ambiguous")
    }
    return application
}

func wechatWindows(_ application: NSRunningApplication) throws -> [RuntimeWindow] {
    guard let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        throw RuntimeFailure("window_query_failed")
    }
    return windows.compactMap { entry -> RuntimeWindow? in
        guard (entry[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == application.processIdentifier,
              (entry[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
              let number = entry[kCGWindowNumber as String] as? NSNumber,
              let bounds = entry[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary),
              rect.width > 325, rect.height > 200 else { return nil }
        return RuntimeWindow(window_id: number.uint32Value, pid: application.processIdentifier,
                             x: rect.minX, y: rect.minY, width: rect.width, height: rect.height,
                             on_screen: (entry[kCGWindowIsOnscreen as String] as? Bool) ?? false)
    }.sorted {
        if $0.on_screen != $1.on_screen { return $0.on_screen }
        let left = $0.width * $0.height, right = $1.width * $1.height
        return left == right ? $0.window_id < $1.window_id : left > right
    }
}

func currentWechatWindow() throws -> RuntimeWindow {
    let application = try wechatApplication()
    guard !application.isHidden else { throw RuntimeFailure("wechat_hidden") }
    guard let window = try wechatWindows(application).first else { throw RuntimeFailure("wechat_window_missing") }
    guard window.on_screen else { throw RuntimeFailure("wechat_window_not_observable") }
    return window
}

/// Inspect only roles and attribute support; never include names, values or screenshots.
func runtimeDiagnostics() throws -> [String: Any] {
    let trusted = AXIsProcessTrusted()
    var result: [String: Any] = [
        "protocol_version": runtimeProtocolVersion,
        "accessibility_trusted": trusted,
        "screen_recording_authorized": CGPreflightScreenCaptureAccess(),
        "identity_verified": false,
        "delivery_receipts": false,
        "capture_backend": "screencapturekit",
        "ocr_backend": "vision",
    ]
    let application: NSRunningApplication
    do { application = try wechatApplication() }
    catch {
        result["process_state"] = "missing_or_ambiguous"
        return result
    }
    result["process_state"] = "running"
    result["wechat_frontmost"] = NSWorkspace.shared.frontmostApplication?.processIdentifier == application.processIdentifier
    result["content_readability"] = "not_probed"
    result["window_candidates"] = try wechatWindows(application).map { try jsonObject($0) }
    result["client_version"] = application.bundleURL.flatMap { Bundle(url: $0) }?
        .object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown"
    do {
        result["window"] = try jsonObject(currentWechatWindow())
        result["window_state"] = "observable"
    } catch {
        result["window_state"] = (error as? RuntimeFailure)?.description ?? "unavailable"
    }
    guard trusted else { result["ax_probe_state"] = "permission_missing"; return result }
    let app = AXUIElementCreateApplication(application.processIdentifier)
    AXUIElementSetMessagingTimeout(app, 0.1)
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success else {
        result["ax_probe_state"] = "unavailable"
        return result
    }
    var roles: [String: Int] = [:]
    var count = 0
    var textControls = 0
    var truncated = false
    let deadline = ProcessInfo.processInfo.systemUptime + 2
    func visit(_ element: AXUIElement, _ depth: Int) {
        guard count < 500, depth <= 12, ProcessInfo.processInfo.systemUptime < deadline else {
            truncated = true; return
        }
        count += 1
        var roleValue: CFTypeRef?
        if AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &roleValue) == .success,
           let role = roleValue as? String {
            roles[role, default: 0] += 1
            if role == "AXTextArea" || role == "AXTextField" {
                var settable = DarwinBoolean(false)
                if AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &settable) == .success,
                   settable.boolValue { textControls += 1 }
            }
        }
        var children: CFTypeRef?
        if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children) == .success {
            for child in children as? [AXUIElement] ?? [] { visit(child, depth + 1) }
        }
    }
    for window in value as? [AXUIElement] ?? [] { visit(window, 0) }
    result["ax_probe_state"] = "observed"
    result["ax_roles"] = roles
    result["ax_nodes"] = count
    result["ax_settable_text_controls"] = textControls
    result["ax_probe_truncated"] = truncated
    return result
}
