// Contacts.swift
// Contacts layout checks, window picking, and read-only operations for WeChat bot worker.
import Foundation
import CoreGraphics
import AppKit
import Vision

#if !BOT_WORKER
public enum Failure: Error {
    case invalid(String)
}
#endif

// MARK: - Pure functions

/// Verifies whether clicking `point` is permitted in the contacts view.
/// Forbidden areas (e.g. profile bottom action buttons y >= 440) are strictly rejected.
/// Only coordinates within the `list` region are allowed for list navigation clicks.
public func contactsClickAllowed(point: WindowPoint, layout: WechatLayout) -> Bool {
    guard let spec = layout.contacts else { return false }
    if spec.profileForbidden.contains(point) {
        return false
    }
    if !spec.list.contains(point) {
        return false
    }
    return true
}

public func contactsClickAllowed(point: (Double, Double), layout: WechatLayout) -> Bool {
    return contactsClickAllowed(point: WindowPoint(x: point.0, y: point.1), layout: layout)
}

/// Identifies the contacts manager window among candidate windows.
/// Prefers a window titled "通讯录管理" (or size near 720x560), never selecting the main WeChat window (880x640).
public func pickManagerWindow(_ windows: [[String: Any]]) -> [String: Any]? {
    var candidates: [(win: [String: Any], score: Int)] = []

    for w in windows {
        let title = (w["title"] as? String) ?? (w[kCGWindowName as String] as? String) ?? ""
        var width = w["width"] as? Double ?? 0
        var height = w["height"] as? Double ?? 0
        if width == 0 || height == 0 {
            if let bounds = w[kCGWindowBounds as String] as? [String: Any],
               let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) {
                width = Double(rect.width)
                height = Double(rect.height)
            }
        }

        // Exclude the 880x640 main window if it does not have the manager title
        if width.rounded() == 880 && height.rounded() == 640 && !title.contains("通讯录管理") {
            continue
        }

        var score = 0
        if title.contains("通讯录管理") || title.contains("Contacts Manager") {
            score += 100
        }
        if abs(width - 720) <= 40 && abs(height - 560) <= 40 {
            score += 50
        }

        if score > 0 {
            candidates.append((win: w, score: score))
        }
    }

    candidates.sort { $0.score > $1.score }
    return candidates.first?.win
}

/// 「通讯录管理」左栏筛选的点击位置（管理窗口内坐标）。
/// 「全部」用布局里的固定位置；标签必须由调用方传入从左栏识别到的 y（标签间距与表格行高不同，不能按序号推算），
/// 且必须在「标签」标题下方、窗口底边以内，否则返回 nil（拒绝点击）。
public func managerFilterPoint(kind: String, y: Double?, windowHeight: Double, spec: ContactsManagerFilterSpec) -> WindowPoint? {
    switch kind {
    case "all":
        return WindowPoint(x: spec.x, y: spec.allY)
    case "tag":
        guard let y = y, y >= spec.tagY + 20, y <= windowHeight - 20 else { return nil }
        return WindowPoint(x: spec.x, y: y)
    default:
        return nil
    }
}

/// Formats OCR lines ensuring none intersect the forbidden profile region.
public func filterProfileForbiddenLines(lines: [[String: Any]], forbiddenRect: WindowRect) -> [[String: Any]] {
    return lines.filter { line in
        guard let x = line["x"] as? Double,
              let y = line["y"] as? Double else { return true }
        let w = (line["w"] as? Double) ?? 0
        let h = (line["h"] as? Double) ?? 0
        let center = WindowPoint(x: x + w / 2.0, y: y + h / 2.0)
        return !forbiddenRect.contains(center)
    }
}

/// Performs on-device Vision text recognition returning lines in window/crop coordinates.
public func recognizeContactLines(image: CGImage, offsetX: Double = 0, offsetY: Double = 0, timeout: Double = 12) throws -> [[String: Any]] {
    let semaphore = DispatchSemaphore(value: 0)
    var lines: [[String: Any]] = []
    var failure: Error?

    DispatchQueue.global(qos: .userInitiated).async {
        let request = VNRecognizeTextRequest()
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        let handler = VNImageRequestHandler(cgImage: image, options: [:])
        do {
            try handler.perform([request])
            let imageWidth = Double(image.width)
            let imageHeight = Double(image.height)
            for observation in request.results ?? [] {
                guard let candidate = observation.topCandidates(1).first else { continue }
                let box = observation.boundingBox
                let x = offsetX + box.origin.x * imageWidth
                let y = offsetY + (1.0 - box.origin.y - box.height) * imageHeight
                let w = box.width * imageWidth
                let h = box.height * imageHeight
                lines.append([
                    "text": candidate.string,
                    "x": x,
                    "y": y,
                    "w": w,
                    "h": h
                ])
            }
        } catch {
            failure = error
        }
        semaphore.signal()
    }

    guard semaphore.wait(timeout: .now() + timeout) == .success else {
        throw Failure.invalid("vision_timeout")
    }
    if let failure = failure {
        throw failure
    }
    return lines.sorted { ($0["y"] as? Double ?? 0) < ($1["y"] as? Double ?? 0) }
}

public func isContactsOperation(_ operation: String) -> Bool {
    return operation.hasPrefix("contacts_")
}

// MARK: - Worker Execution (BOT_WORKER)

#if BOT_WORKER

func findManagerWindow(wechatPID: Int32? = nil) -> [String: Any]? {
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let choices: [[String: Any]] = rows.compactMap { row in
        guard let owner = row[kCGWindowOwnerName as String] as? String, ["WeChat", "微信"].contains(owner),
              let id = row[kCGWindowNumber as String] as? UInt32,
              let pid = row[kCGWindowOwnerPID as String] as? Int32,
              let bounds = row[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary), rect.width > 0, rect.height > 0 else { return nil }
        if let targetPID = wechatPID, pid != targetPID { return nil }
        let title = (row[kCGWindowName as String] as? String) ?? ""
        return [
            "window_id": id,
            "pid": Int(pid),
            "x": Double(rect.minX),
            "y": Double(rect.minY),
            "width": Double(rect.width),
            "height": Double(rect.height),
            "title": title,
            "on_screen": true
        ]
    }
    return pickManagerWindow(choices)
}

func closeManagerWindow(_ window: [String: Any]) throws -> Bool {
    guard AXIsProcessTrusted() else { throw Failure.invalid("accessibility_not_trusted") }
    let pid = pid_t(window["pid"] as! Int)
    let ax = AXUIElementCreateApplication(pid)
    var listed: CFTypeRef?
    guard AXUIElementCopyAttributeValue(ax, kAXWindowsAttribute as CFString, &listed) == .success,
          let windows = listed as? [AXUIElement] else {
        throw Failure.invalid("contacts_manager_missing")
    }
    let targetX = window["x"] as? Double ?? 0
    let targetY = window["y"] as? Double ?? 0
    let targetW = window["width"] as? Double ?? 0
    let targetH = window["height"] as? Double ?? 0

    for element in windows {
        var titleRef: CFTypeRef?
        let title = (AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &titleRef) == .success ? (titleRef as? String) : nil) ?? ""
        let origin = axValue(element, kAXPositionAttribute as String, .cgPoint, CGPoint.zero) ?? CGPoint.zero
        let size = axValue(element, kAXSizeAttribute as String, .cgSize, CGSize.zero) ?? CGSize.zero
        let matchesTitle = title.contains("通讯录管理") || title.contains("Contacts Manager")
        let matchesBounds = abs(origin.x - targetX) < 10 && abs(origin.y - targetY) < 10 && abs(size.width - targetW) < 10 && abs(size.height - targetH) < 10

        if matchesTitle || matchesBounds {
            if let closeBtn = axElement(element, kAXCloseButtonAttribute as String) {
                try press(closeBtn)
                return true
            }
        }
    }
    throw Failure.invalid("contacts_manager_missing")
}

func executeContactsOperation(operation: String, params: [String: Any], manage: Bool) throws -> [String: Any] {
    let layout = try currentLayout()
    let spec = try layout.contactsSpec()

    if operation == "contacts_enter" {
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        let navPoint = WindowPoint(x: spec.navContacts.x, y: spec.navContacts.y)
        if spec.profileForbidden.contains(navPoint) { throw Failure.invalid("contacts_forbidden_click") }
        try act { try botActions.click(CGRect(x: wx + spec.navContacts.x - 5, y: wy + spec.navContacts.y - 5, width: 10, height: 10)) }
        pauseUI(0.2)
        let listRect = CGRect(x: wx + spec.list.x, y: wy + spec.list.y, width: spec.list.w, height: spec.list.h)
        let linesPerTick = Int32(layout.scroll.linesPerTick)
        try act { try botActions.scroll(in: listRect, lines: linesPerTick, ticks: 10) }
        pauseUI(0.2)
        return ["ok": true]
    }

    if operation == "contacts_headers" {
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let normImage = try checkedScreen(window, regions: ["session_list"])  // 通讯录页无输入框/标题锚点
        let listCrop = CGRect(x: spec.list.x, y: spec.list.y, width: spec.list.w, height: spec.list.h)
        guard let cropped = normImage.cropping(to: listCrop) else { throw Failure.invalid("bitmap_failed") }
        let lines = try recognizeContactLines(image: cropped, offsetX: spec.list.x, offsetY: spec.list.y)
        return ["lines": lines]
    }

    if operation == "contacts_click_list" {
        guard let x = (params["x"] as? NSNumber)?.doubleValue,
              let y = (params["y"] as? NSNumber)?.doubleValue else {
            throw Failure.invalid("invalid_coordinates")
        }
        let point = WindowPoint(x: x, y: y)
        if spec.profileForbidden.contains(point) {
            throw Failure.invalid("contacts_forbidden_click")
        }
        if !spec.list.contains(point) {
            throw Failure.invalid("contacts_click_outside_list")
        }
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        try act { try botActions.click(CGRect(x: wx + x - 5, y: wy + y - 5, width: 10, height: 10)) }
        pauseUI(0.2)
        let normImage = try checkedScreen(window, regions: ["session_list"])  // 通讯录页无输入框/标题锚点
        let listCrop = CGRect(x: spec.list.x, y: spec.list.y, width: spec.list.w, height: spec.list.h)
        guard let cropped = normImage.cropping(to: listCrop) else { throw Failure.invalid("bitmap_failed") }
        let lines = try recognizeContactLines(image: cropped, offsetX: spec.list.x, offsetY: spec.list.y)
        return ["lines": lines]
    }

    if operation == "contacts_scroll_list" {
        guard let direction = params["direction"] as? String, direction == "up" || direction == "down" else {
            throw Failure.invalid("invalid_scroll_request")
        }
        let ticks = (params["ticks"] as? Int) ?? 1
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        let listRect = CGRect(x: wx + spec.list.x, y: wy + spec.list.y, width: spec.list.w, height: spec.list.h)
        let linesPerTick = Int32(layout.scroll.linesPerTick)
        try act { try botActions.scroll(in: listRect, lines: direction == "up" ? linesPerTick : -linesPerTick, ticks: ticks) }
        pauseUI(0.2)
        let normImage = try checkedScreen(window, regions: ["session_list"])  // 通讯录页无输入框/标题锚点
        let listCrop = CGRect(x: spec.list.x, y: spec.list.y, width: spec.list.w, height: spec.list.h)
        guard let cropped = normImage.cropping(to: listCrop) else { throw Failure.invalid("bitmap_failed") }
        let lines = try recognizeContactLines(image: cropped, offsetX: spec.list.x, offsetY: spec.list.y)
        return ["lines": lines]
    }

    if operation == "contacts_open_row" {
        guard let y = (params["y"] as? NSNumber)?.doubleValue else {
            throw Failure.invalid("invalid_row_coordinate")
        }
        let point = WindowPoint(x: 180.0, y: y)
        if spec.profileForbidden.contains(point) {
            throw Failure.invalid("contacts_forbidden_click")
        }
        if !spec.list.contains(point) {
            throw Failure.invalid("contacts_click_outside_list")
        }
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        try act { try botActions.click(CGRect(x: wx + 180.0 - 10, y: wy + y - 5, width: 20, height: 10)) }
        pauseUI(0.3)
        let normImage = try checkedScreen(window, regions: ["session_list"])  // 通讯录页无输入框/标题锚点
        let profileCrop = CGRect(x: spec.profile.x, y: spec.profile.y, width: spec.profile.w, height: spec.profile.h)
        guard let cropped = normImage.cropping(to: profileCrop) else { throw Failure.invalid("bitmap_failed") }
        let rawLines = try recognizeContactLines(image: cropped, offsetX: spec.profile.x, offsetY: spec.profile.y)
        let safeLines = filterProfileForbiddenLines(lines: rawLines, forbiddenRect: spec.profileForbidden)
        return ["lines": safeLines]
    }

    if operation == "contacts_manager_open" {
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        let btnPoint = WindowPoint(x: spec.managerButton.x, y: spec.managerButton.y)
        if spec.profileForbidden.contains(btnPoint) { throw Failure.invalid("contacts_forbidden_click") }
        try act { try botActions.click(CGRect(x: wx + spec.managerButton.x - 5, y: wy + spec.managerButton.y - 5, width: 10, height: 10)) }

        let deadline = ProcessInfo.processInfo.systemUptime + 2.5
        var mgrWin: [String: Any]? = nil
        while ProcessInfo.processInfo.systemUptime < deadline {
            pauseUI(0.15)
            if let found = findManagerWindow(wechatPID: Int32(window["pid"] as? Int ?? 0)) {
                mgrWin = found
                break
            }
        }
        guard let mgr = mgrWin else {
            throw Failure.invalid("contacts_manager_missing")
        }
        return [
            "window": [
                "x": mgr["x"] ?? 0,
                "y": mgr["y"] ?? 0,
                "w": mgr["width"] ?? 0,
                "h": mgr["height"] ?? 0
            ]
        ]
    }

    if operation == "contacts_manager_page" {
        guard let mgr = findManagerWindow() else {
            throw Failure.invalid("contacts_manager_missing")
        }
        let rawImage = try windowImage(mgr)
        let lines = try recognizeContactLines(image: rawImage, offsetX: 0, offsetY: 0)
        return [
            "lines": lines,
            "window": [
                "x": mgr["x"] ?? 0,
                "y": mgr["y"] ?? 0,
                "w": mgr["width"] ?? 0,
                "h": mgr["height"] ?? 0
            ]
        ]
    }

    if operation == "contacts_manager_scroll" {
        guard let direction = params["direction"] as? String, direction == "up" || direction == "down" else {
            throw Failure.invalid("invalid_scroll_request")
        }
        let ticks = (params["ticks"] as? Int) ?? 1
        guard let mgr = findManagerWindow() else {
            throw Failure.invalid("contacts_manager_missing")
        }
        let mx = mgr["x"] as? Double ?? 0
        let my = mgr["y"] as? Double ?? 0
        let mw = mgr["width"] as? Double ?? 720
        let mh = mgr["height"] as? Double ?? 560
        let tableRect = CGRect(x: mx + 200, y: my + 96, width: mw - 200, height: mh - 120)
        let linesPerTick = Int32(layout.scroll.linesPerTick)
        try act { try botActions.scroll(in: tableRect, lines: direction == "up" ? linesPerTick : -linesPerTick, ticks: ticks) }
        pauseUI(0.2)
        let rawImage = try windowImage(mgr)
        let lines = try recognizeContactLines(image: rawImage, offsetX: 0, offsetY: 0)
        return [
            "lines": lines,
            "window": [
                "x": mgr["x"] ?? 0,
                "y": mgr["y"] ?? 0,
                "w": mgr["width"] ?? 0,
                "h": mgr["height"] ?? 0
            ]
        ]
    }

    if operation == "contacts_manager_filter" {
        guard let kind = params["kind"] as? String, kind == "all" || kind == "tag" else {
            throw Failure.invalid("invalid_filter_kind")
        }
        guard let mgr = findManagerWindow() else {
            throw Failure.invalid("contacts_manager_missing")
        }
        let mx = mgr["x"] as? Double ?? 0
        let my = mgr["y"] as? Double ?? 0
        let mh = mgr["height"] as? Double ?? 560
        guard let point = managerFilterPoint(kind: kind, y: (params["y"] as? NSNumber)?.doubleValue, windowHeight: mh, spec: spec.manager.filter) else {
            throw Failure.invalid("contacts_filter_position_invalid")
        }
        try act { try botActions.click(CGRect(x: mx + point.x - 5, y: my + point.y - 5, width: 10, height: 10)) }
        pauseUI(0.2)
        let rawImage = try windowImage(mgr)
        let lines = try recognizeContactLines(image: rawImage, offsetX: 0, offsetY: 0)
        return [
            "lines": lines,
            "window": [
                "x": mgr["x"] ?? 0,
                "y": mgr["y"] ?? 0,
                "w": mgr["width"] ?? 0,
                "h": mgr["height"] ?? 0
            ]
        ]
    }

    if operation == "contacts_manager_close" {
        guard let mgr = findManagerWindow() else {
            throw Failure.invalid("contacts_manager_missing")
        }
        let closed = try closeManagerWindow(mgr)
        pauseUI(0.2)
        return ["closed": closed]
    }

    if operation == "contacts_leave" {
        // 「通讯录管理」窗口若还开着（例如同步中断），会被界面检查当成"别的微信窗口"挡住、机器人停摆：先关掉它
        if let mgr = findManagerWindow() {
            _ = try? closeManagerWindow(mgr)
            pauseUI(0.3)
        }
        let (window, _) = try readyWindow(manage: manage)
        try screenGate(window)
        let wx = window["x"] as! Double, wy = window["y"] as! Double
        let navPoint = WindowPoint(x: spec.navChats.x, y: spec.navChats.y)
        if spec.profileForbidden.contains(navPoint) { throw Failure.invalid("contacts_forbidden_click") }
        try act { try botActions.click(CGRect(x: wx + spec.navChats.x - 5, y: wy + spec.navChats.y - 5, width: 10, height: 10)) }
        pauseUI(0.2)
        return ["ok": true]
    }

    throw Failure.invalid("unsupported_operation")
}

#endif
