import AppKit
import ApplicationServices
import CryptoKit
import Vision
import ImageIO
import Darwin
import ScreenCaptureKit

private let maximumControls = 256
private let maximumDepth = 12
private let captureTimeout: TimeInterval = 5
private let axTimeout: Float = 2
private let maximumImagePixels = 16 * 1024 * 1024
private let recognitionTimeout: TimeInterval = 12
private let coldRecognitionTimeout: TimeInterval = 60
private var recognizerWarmed = false

func attribute(_ element: AXUIElement, _ key: String, deadline: InteractionDeadline? = nil) -> CFTypeRef? {
    if let deadline = deadline {
        guard deadline.remaining > 0 else { return nil }
        AXUIElementSetMessagingTimeout(element,min(axTimeout,Float(deadline.remaining)))
    }
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success ? value : nil
}

func axRect(_ element: AXUIElement, deadline: InteractionDeadline? = nil) -> WindowRect? {
    guard let pos = attribute(element,kAXPositionAttribute,deadline:deadline), CFGetTypeID(pos) == AXValueGetTypeID(),
          let size = attribute(element,kAXSizeAttribute,deadline:deadline), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero, dimensions = CGSize.zero
    guard AXValueGetValue(pos as! AXValue,.cgPoint,&point), AXValueGetValue(size as! AXValue,.cgSize,&dimensions) else { return nil }
    return WindowRect(x:point.x,y:point.y,w:dimensions.width,h:dimensions.height)
}

func runningApp(_ bundle: String) throws -> NSRunningApplication {
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier:bundle).filter { !$0.isTerminated }
    guard apps.count == 1 else { throw InteractionFailure("application_not_unique_or_missing") }
    return apps[0]
}

func windowInfo(_ app: NSRunningApplication, id: UInt32?) throws -> [String:Any] {
    let windows = (CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements], kCGNullWindowID) as? [[String:Any]] ?? []).filter {
        ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == app.processIdentifier
            && ($0[kCGWindowLayer as String] as? Int) == 0
    }
    let choices = windows.filter { id == nil || ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == id }
    guard choices.count == 1 else { throw InteractionFailure("window_not_unique_or_missing: specify window_id") }
    return choices[0]
}

func boundsOf(_ info: [String:Any]) throws -> WindowRect {
    guard let raw = info[kCGWindowBounds as String] as? [String:Any], let r = CGRect(dictionaryRepresentation:raw as CFDictionary),
          r.width > 0, r.height > 0 else { throw InteractionFailure("window_bounds_unavailable") }
    return WindowRect(x:r.minX,y:r.minY,w:r.width,h:r.height)
}

func captureWindow(_ id: UInt32, deadline: InteractionDeadline) throws -> CGImage {
    try deadline.check()
    let waitSeconds = min(captureTimeout,deadline.remaining)
    #if compiler(>=5.9)
    if #available(macOS 14, *) {
        let done = DispatchSemaphore(value:0)
        var image: CGImage?
        SCShareableContent.getExcludingDesktopWindows(false,onScreenWindowsOnly:true) { content,_ in
            guard let window = content?.windows.first(where: { $0.windowID == id }) else { done.signal(); return }
            let filter = SCContentFilter(desktopIndependentWindow:window)
            let config = SCStreamConfiguration()
            config.width = Int((window.frame.width * Double(filter.pointPixelScale)).rounded())
            config.height = Int((window.frame.height * Double(filter.pointPixelScale)).rounded())
            config.showsCursor = false
            SCScreenshotManager.captureImage(contentFilter:filter,configuration:config) { result,_ in image = result; done.signal() }
        }
        guard done.wait(timeout:.now()+waitSeconds) == .success, let result = image else { throw InteractionFailure("screen_capture_failed") }
        return result
    } else {
        guard let image = CGWindowListCreateImage(.null,.optionIncludingWindow,id,[.boundsIgnoreFraming]) else { throw InteractionFailure("screen_capture_failed") }
        return image
    }
    #else
    guard let image = CGWindowListCreateImage(.null,.optionIncludingWindow,id,[.boundsIgnoreFraming]) else { throw InteractionFailure("screen_capture_failed") }
    return image
    #endif
}

func axWindow(_ app: NSRunningApplication, bounds: WindowRect, deadline: InteractionDeadline? = nil) -> AXUIElement? {
    let element = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(element,axTimeout)
    let windows = attribute(element,kAXWindowsAttribute,deadline:deadline) as? [AXUIElement] ?? []
    let matching = windows.filter { axRect($0,deadline:deadline) == bounds }
    return matching.count == 1 ? matching[0] : nil
}

func collectControls(_ root: AXUIElement, window: WindowRect, budget: InteractionDeadline) -> (controls:[ObservedControl],truncated:Bool) {
    let traversalEnd = ProcessInfo.processInfo.systemUptime + min(captureTimeout,budget.remaining)
    var truncated = false
    var controls: [ObservedControl] = []
    var visited = 0
    func walk(_ element: AXUIElement, path: [Int]) {
        guard path.count <= maximumDepth, visited < maximumControls, ProcessInfo.processInfo.systemUptime < traversalEnd && budget.remaining > 0 else { truncated = true; return }
        visited += 1
        let role = attribute(element,kAXRoleAttribute,deadline:budget) as? String ?? "unknown"
        let subrole = attribute(element,kAXSubroleAttribute,deadline:budget) as? String ?? ""
        guard subrole != kAXSecureTextFieldSubrole else { return }
        if let bounds = axRect(element,deadline:budget), usable(bounds,within:window) {
            let label = (attribute(element,kAXTitleAttribute,deadline:budget) as? String)
                ?? (attribute(element,kAXDescriptionAttribute,deadline:budget) as? String) ?? ""
            var names: CFArray?
            if budget.remaining > 0 { AXUIElementSetMessagingTimeout(element,min(axTimeout,Float(budget.remaining))); AXUIElementCopyActionNames(element,&names) }
            var states: [String:String] = [:]
            for key in [kAXSelectedAttribute,kAXExpandedAttribute] {
                if let value = attribute(element,key,deadline:budget) as? NSNumber { states[key] = value.stringValue }
            }
            let rawValue = attribute(element,kAXValueAttribute,deadline:budget)
            let value = (rawValue as? String) ?? (rawValue as? NSNumber)?.stringValue
            controls.append(ObservedControl(id:"ax-"+path.map(String.init).joined(separator:"."),source:"accessibility",role:role,
                label:label,value:value,
                enabled:(attribute(element,kAXEnabledAttribute,deadline:budget) as? Bool) ?? true,
                focused:(attribute(element,kAXFocusedAttribute,deadline:budget) as? Bool) ?? false,bounds:bounds,path:path,
                actions:(names as? [String]) ?? [],states:states,identifier:attribute(element,kAXIdentifierAttribute,deadline:budget) as? String))
        }
        let children = attribute(element,kAXChildrenAttribute,deadline:budget) as? [AXUIElement] ?? []
        for (i,child) in children.enumerated() { walk(child,path:path+[i]) }
    }
    walk(root,path:[])
    return (controls,truncated)
}

func imageFingerprint(_ image: CGImage) throws -> String {
    guard image.width <= maximumImagePixels / max(image.height,1) else { throw InteractionFailure("window_image_too_large") }
    var pixels = [UInt8](repeating:0,count:image.width*image.height*4)
    let rendered = pixels.withUnsafeMutableBytes { bytes -> Bool in
        guard let context = CGContext(data:bytes.baseAddress,width:image.width,height:image.height,bitsPerComponent:8,
            bytesPerRow:image.width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
        context.draw(image,in:CGRect(x:0,y:0,width:image.width,height:image.height))
        return true
    }
    guard rendered else { throw InteractionFailure("pixel_normalization_failed") }
    return SHA256.hash(data:Data(pixels)).map { String(format:"%02x",$0) }.joined()
}

func observe(bundle: String, windowID: UInt32?, screenshot: Bool,
             deadline: InteractionDeadline? = nil, includeOCR: Bool = true) throws -> DesktopObservation {
    let budget = deadline ?? InteractionDeadline(seconds:coldRecognitionTimeout + captureTimeout*2 + Double(axTimeout))
    try budget.check()
    guard CGPreflightScreenCaptureAccess() else { throw InteractionFailure("screen_recording_permission_required") }
    let app = try runningApp(bundle)
    let info = try windowInfo(app,id:windowID)
    let bounds = try boundsOf(info)
    let id = (info[kCGWindowNumber as String] as! NSNumber).uint32Value
    let image = includeOCR || screenshot ? try captureWindow(id,deadline:budget) : nil
    let windowElement = AXIsProcessTrusted() ? axWindow(app,bounds:bounds,deadline:budget) : nil
    let tree = windowElement.map { collectControls($0,window:bounds,budget:budget) }
    var controls = tree?.controls ?? []
    try budget.check()
    if includeOCR, let image = image {
        guard image.width <= maximumImagePixels / max(image.height,1) else { throw InteractionFailure("window_image_too_large") }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        let timeout = DispatchWorkItem { request.cancel() }
        let recognitionBudget = min(recognizerWarmed ? recognitionTimeout : coldRecognitionTimeout,budget.remaining)
        DispatchQueue.global().asyncAfter(deadline:.now()+recognitionBudget,execute:timeout)
        defer { timeout.cancel() }
        try VNImageRequestHandler(cgImage:image,options:[:]).perform([request])
        recognizerWarmed = true
        try budget.check()
        for (i,line) in (request.results ?? []).prefix(maximumControls).enumerated() {
            guard let text = line.topCandidates(1).first?.string else { continue }
            let box = line.boundingBox
            let region = WindowRect(x:bounds.x+box.minX*bounds.w,y:bounds.y+(1-box.maxY)*bounds.h,w:box.width*bounds.w,h:box.height*bounds.h)
            let crop = CGRect(x:(region.x-bounds.x)*Double(image.width)/bounds.w,y:(region.y-bounds.y)*Double(image.height)/bounds.h,
                width:region.w*Double(image.width)/bounds.w,height:region.h*Double(image.height)/bounds.h).integral
            let visual = try image.cropping(to:crop).map(imageFingerprint)
            controls.append(ObservedControl(id:"ocr-\(i)",source:"ocr",role:"text",label:text,value:nil,enabled:true,focused:false,
                bounds:region,path:nil,actions:[],confidence:line.confidence,visual_fingerprint:visual))
        }
    }
    // Whole-window hashes are diagnostic, not a readiness gate for semantic AX targets.
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    var digest = SHA256()
    if let image = image { digest.update(data:Data(try imageFingerprint(image).utf8)) }
    digest.update(data:try encoder.encode(controls))
    let currentInfo = try windowInfo(app,id:id)
    let currentBounds = try boundsOf(currentInfo)
    guard currentBounds == bounds else { throw InteractionFailure("window_changed_during_observation") }
    try budget.check()
    let png = screenshot ? image.flatMap { NSBitmapImageRep(cgImage:$0).representation(using:.png,properties:[:])?.base64EncodedString() } : nil
    return DesktopObservation(bundle_id:bundle,pid:app.processIdentifier,window_id:id,
        title:info[kCGWindowName as String] as? String ?? "",bounds:bounds,
        fingerprint:digest.finalize().map { String(format:"%02x",$0) }.joined(),controls:controls,
        captured_at_ms:Int64(Date().timeIntervalSince1970*1000),screenshot_base64:png,
        accessibility_available:windowElement != nil,accessibility_truncated:tree?.truncated ?? false,coverage:"visible_window_only")
}

func resolveElement(_ root: AXUIElement, path: [Int], deadline: InteractionDeadline) -> AXUIElement? {
    var current = root
    for index in path {
        let children = attribute(current,kAXChildrenAttribute,deadline:deadline) as? [AXUIElement] ?? []
        guard children.indices.contains(index) else { return nil }
        current = children[index]
    }
    return current
}

func rect(_ b: WindowRect) -> CGRect { CGRect(x:b.x,y:b.y,width:b.w,height:b.h) }

private let defaultActionTimeout: TimeInterval = 30

/// Only the package's known click-through companion is exempt, never an arbitrary floating window.
func isPassiveCompanion(_ pid: Int32) -> Bool {
    guard let executable = NSRunningApplication(processIdentifier:pid)?.executableURL?.resolvingSymlinksInPath() else { return false }
    let own = URL(fileURLWithPath:CommandLine.arguments[0]).resolvingSymlinksInPath()
    return executable == own.deletingLastPathComponent().appendingPathComponent("wechat_overlay")
}

func frontWindowID(_ pid: Int32, windows: [[String:Any]]) -> UInt32? {
    let front = windows.first { ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
    return (front?[kCGWindowNumber as String] as? NSNumber)?.uint32Value
}

func act(_ command: DesktopCommand) throws -> [String:Any] {
    let budget = InteractionDeadline(seconds:command.timeout_seconds ?? defaultActionTimeout)
    guard AXIsProcessTrusted() else { throw InteractionFailure("accessibility_permission_required") }
    guard let saved = command.observation, let action = command.action, let targetID = command.target_id,
          let permit = command.permit_path,
          let original = saved.controls.first(where: { $0.id == targetID }) else { throw InteractionFailure("invalid_action_request") }
    let app = try runningApp(saved.bundle_id)
    let needsOCR = original.source == "ocr" || command.destination_id.map { id in saved.controls.contains { $0.id == id && $0.source == "ocr" } } == true
    let current = try observe(bundle:saved.bundle_id,windowID:saved.window_id,screenshot:false,deadline:budget,includeOCR:needsOCR)
    let target = try validateBinding(saved:saved,current:current,target:targetID)
    let destination = try command.destination_id.map { try validateBinding(saved:saved,current:current,target:$0) }
    let regions = action == "key" ? [] : [target.bounds] + (destination.map { [$0.bounds] } ?? [])
    func checkCancellation() throws {
        try budget.check()
        guard FileManager.default.fileExists(atPath:permit), command.parent_pid == getppid() else { throw ActionError.cancelled }
        guard !app.isTerminated, app.processIdentifier == saved.pid else { throw InteractionFailure("application_changed") }
    }
    func isFront() -> Bool {
        let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
        return NSWorkspace.shared.frontmostApplication?.processIdentifier == saved.pid && frontWindowID(saved.pid,windows:windows) == saved.window_id
    }
    func allowed() throws {
        try checkCancellation()
        if action != "focus" {
            guard NSWorkspace.shared.frontmostApplication?.processIdentifier == saved.pid else { throw InteractionFailure("application_not_frontmost") }
            let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
            guard frontWindowID(saved.pid,windows:windows) == saved.window_id else { throw InteractionFailure("target_window_not_frontmost") }
            guard let index = windows.firstIndex(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == saved.window_id }) else { throw InteractionFailure("window_missing") }
            for above in windows[..<index] {
                guard let covered = try? boundsOf(above), let owner = (above[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value else { continue }
                if blocksAction(OcclusionCandidate(bounds:covered,alpha:above[kCGWindowAlpha as String] as? Double ?? 1,passive:isPassiveCompanion(owner)),regions:regions) {
                    throw InteractionFailure("window_occluded")
                }
            }
        }
        guard let info = try? windowInfo(app,id:saved.window_id), try boundsOf(info) == saved.bounds else { throw InteractionFailure("window_changed") }
    }
    let actions = Actions(checkAllowed:allowed)
    var inputReadback = "not_applicable"
    var activation = "not_applicable"
    try allowed()
    switch action {
    case "focus":
        _ = app.activate(options:[])
        if let window = axWindow(app,bounds:saved.bounds,deadline:budget) { AXUIElementPerformAction(window,kAXRaiseAction as CFString) }
        do {
            try awaitActivation(deadline:budget,check:{ try checkCancellation(); return isFront() },wait:{
                RunLoop.current.run(until:Date().addingTimeInterval(min(ActionProfile.current().dwell.lowerBound,budget.remaining)))
            })
        } catch { throw InteractionFailure("activation_not_confirmed: \(error)") }
        activation = "confirmed"
    case "click": try actions.click(rect(target.bounds),within:rect(saved.bounds))
    case "right_click": try actions.click(rect(target.bounds),button:.right,within:rect(saved.bounds))
    case "double_click":
        let interval = min(NSEvent.doubleClickInterval/3,Double.random(in:ActionProfile.current().dwell))
        try actions.doubleClick(rect(target.bounds),interval:interval,within:rect(saved.bounds))
    case "drag":
        guard let destination = destination else { throw InteractionFailure("destination_required") }
        try actions.drag(from:rect(target.bounds),to:rect(destination.bounds),within:rect(saved.bounds))
    case "scroll": try actions.scroll(in:rect(target.bounds),lines:command.lines ?? 0,ticks:command.ticks ?? 0)
    case "key":
        guard target.focused || target.role == kAXWindowRole, let keys = command.keys else { throw InteractionFailure("focused_target_required") }
        try actions.key(keys)
    case "type_text":
        guard target.source == "accessibility", target.role == kAXTextFieldRole || target.role == kAXTextAreaRole || target.role == kAXComboBoxRole,
              let path = target.path, let root = axWindow(app,bounds:saved.bounds,deadline:budget), let element = resolveElement(root,path:path,deadline:budget),
              isEditableValue(element), let text = command.text else { throw InteractionFailure("editable_target_required") }
        try actions.click(rect(target.bounds),within:rect(saved.bounds))
        guard (attribute(element,kAXFocusedAttribute,deadline:budget) as? Bool) == true else { throw InteractionFailure("input_not_focused") }
        let pasteboard = NSPasteboard.general
        let savedItems: [NSPasteboardItem] = (pasteboard.pasteboardItems ?? []).map { item in
            let copy = NSPasteboardItem()
            for type in item.types { if let data = item.data(forType:type) { copy.setData(data,forType:type) } }
            return copy
        }
        defer { pasteboard.clearContents(); pasteboard.writeObjects(savedItems) }
        pasteboard.clearContents(); pasteboard.setString(text,forType:.string)
        try actions.key("cmd+a"); try actions.key("cmd+v")
        inputReadback = "timeout"
        while budget.remaining > 0 {
            if (attribute(element,kAXValueAttribute,deadline:budget) as? String) == text { inputReadback = "passed"; break }
            try checkCancellation()
            Thread.sleep(forTimeInterval:min(ActionProfile.current().dwell.lowerBound,budget.remaining))
        }
    default: throw InteractionFailure("unsupported_action")
    }
    var after: DesktopObservation?
    var observationError: String?
    var verification = command.expected_text == nil ? "not_requested" : "timeout"
    repeat {
        guard budget.remaining > 0 else { observationError = "deadline_exceeded"; break }
        do { after = try observe(bundle:saved.bundle_id,windowID:saved.window_id,screenshot:false,deadline:budget) }
        catch { observationError = String(describing:error); break }
        guard let expected = command.expected_text else { break }
        if after?.controls.contains(where: { $0.label.contains(expected) || ($0.value?.contains(expected) ?? false) }) == true { verification = "passed"; break }
        try checkCancellation()
        Thread.sleep(forTimeInterval:min(ActionProfile.current().dwell.lowerBound,budget.remaining))
    } while budget.remaining > 0
    return try actionResult(observation:after,verification:verification,inputReadback:inputReadback,activation:activation,observationError:observationError)
}

func isEditableValue(_ element: AXUIElement) -> Bool {
    var settable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element,kAXValueAttribute as CFString,&settable) == .success && settable.boolValue
}
