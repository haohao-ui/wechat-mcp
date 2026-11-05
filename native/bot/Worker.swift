import AppKit
import CoreGraphics
import ImageIO
import CryptoKit
import Darwin
import ScreenCaptureKit

let base = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa")

func checkPrivate(_ path: URL, type: FileAttributeType, mode: Int) throws {
    let info = try FileManager.default.attributesOfItem(atPath:path.path)
    guard info[.type] as? FileAttributeType == type,
          (info[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
          (info[.posixPermissions] as? NSNumber)?.intValue == mode else { throw Failure.invalid("unsafe_private_path") }
}

/// Captures one WeChat window at its native pixel scale (no shadow).
/// macOS 14+: ScreenCaptureKit (CGWindowListCreateImage is unavailable on macOS 15+); older: CGWindowList.
func windowImage(_ window: [String:Any]) throws -> CGImage {
    let id = window["window_id"] as! UInt32
    #if compiler(>=5.9)  // SCScreenshotManager needs the macOS 14 SDK (Xcode 15+); older toolchains build CG only.
    if #available(macOS 14, *) {
        let done = DispatchSemaphore(value:0)
        var image: CGImage?
        var reason = "window_not_shareable"
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly:true) { content, error in
            guard let target = content?.windows.first(where: { $0.windowID == id }) else {
                reason = error.map { "shareable_content: \($0.localizedDescription)" } ?? reason
                done.signal(); return
            }
            let filter = SCContentFilter(desktopIndependentWindow:target)
            let config = SCStreamConfiguration()
            let scale = Double(filter.pointPixelScale)
            config.width = Int((target.frame.width*scale).rounded())
            config.height = Int((target.frame.height*scale).rounded())
            config.showsCursor = false
            config.ignoreShadowsSingleWindow = true
            SCScreenshotManager.captureImage(contentFilter:filter, configuration:config) { captured, error in
                image = captured
                if let error = error { reason = "capture: \(error.localizedDescription)" }
                done.signal()
            }
        }
        guard done.wait(timeout:.now()+5) == .success, let image = image else {
            // Typically missing Screen Recording permission; logged for the operator, retried by the bot.
            fputs("window capture failed: \(reason)\n", stderr)
            throw Failure.invalid("window_capture_failed")
        }
        return image
    }
    #endif
    guard let image = CGWindowListCreateImage(.null,.optionIncludingWindow,id,[.boundsIgnoreFraming,.bestResolution]) else {
        throw Failure.invalid("window_capture_failed")
    }
    return image
}

func selectedWindow() throws -> [String:Any] {
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
    let choices: [[String:Any]] = rows.compactMap { row in
        guard let owner = row[kCGWindowOwnerName as String] as? String, ["WeChat","微信"].contains(owner),
              row[kCGWindowLayer as String] as? Int == 0,
              let id = row[kCGWindowNumber as String] as? UInt32,
              let pid = row[kCGWindowOwnerPID as String] as? Int32,
              let bounds = row[kCGWindowBounds as String] as? [String:Any],
              let rect = CGRect(dictionaryRepresentation:bounds as CFDictionary),rect.width > 0,rect.height > 0 else { return nil }
        return ["window_id":id,"pid":Int(pid),"x":Double(rect.minX),"y":Double(rect.minY),"width":Double(rect.width),"height":Double(rect.height),"on_screen":true]
    }
    guard let result = choices.sorted(by:{
        let a=($0["width"] as! Double)*($0["height"] as! Double),b=($1["width"] as! Double)*($1["height"] as! Double)
        return a == b ? ($0["window_id"] as! UInt32) < ($1["window_id"] as! UInt32) : a > b
    }).first else { throw Failure.invalid("wechat_not_visible") }
    return result
}

func axValue<T>(_ element: AXUIElement, _ attribute: String, _ type: AXValueType, _ empty: T) -> T? {
    var raw: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute as CFString, &raw) == .success, let raw = raw,
          CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var value = empty
    return AXValueGetValue(raw as! AXValue, type, &value) ? value : nil
}

func axElement(_ owner: AXUIElement, _ attribute: String) -> AXUIElement? {
    var raw: CFTypeRef?
    guard AXUIElementCopyAttributeValue(owner, attribute as CFString, &raw) == .success, let value = raw,
          CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
    return (value as! AXUIElement)
}

func axFlag(_ element: AXUIElement, _ attribute: String) -> Bool {
    var raw: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, attribute as CFString, &raw) == .success && (raw as? Bool) == true
}

/// Un-minimizes WeChat's main window (dedicated-machine mode). Throws when there is nothing to restore.
func restoreMinimizedWeChat() throws {
    func unavailable(_ why: String) -> Failure {
        fputs("restore minimized WeChat: \(why)\n", stderr)  // operator diagnostics in bot-worker.log
        return Failure.invalid("wechat_not_visible")
    }
    guard AXIsProcessTrusted() else { throw unavailable("accessibility not trusted") }
    guard let app = NSRunningApplication.runningApplications(withBundleIdentifier:"com.tencent.xinWeChat").first else {
        throw unavailable("WeChat not running")
    }
    let ax = AXUIElementCreateApplication(app.processIdentifier)
    var raw: CFTypeRef?
    let windows = AXUIElementCopyAttributeValue(ax, kAXWindowsAttribute as CFString, &raw) == .success ? (raw as? [AXUIElement] ?? []) : []
    let minimized = windows.filter { axFlag($0, kAXMinimizedAttribute as String) }
    fputs("restore minimized WeChat: \(windows.count) AX windows, \(minimized.count) minimized\n", stderr)
    // Several minimized windows (e.g. an image viewer too): restore only the main one, else wait.
    guard let target = axElement(ax, kAXMainWindowAttribute as String).flatMap({ main in minimized.first { CFEqual($0, main) } })
            ?? (windows.count == 1 && minimized.count == 1 ? minimized[0] : nil),
          AXUIElementSetAttributeValue(target, kAXMinimizedAttribute as CFString, kCFBooleanFalse) == .success else {
        throw unavailable("no unique minimized main window or un-minimize refused")
    }
    Thread.sleep(forTimeInterval:0.6)
}

/// Returns the WeChat main window at the calibrated 880x640 points.
/// manage == true (dedicated machine): restores a minimized main window and resizes it, moving it
/// back inside its display when 880x640 would not fit. Only the AX main window is ever touched,
/// never in full screen; any doubt (several candidates, no permission, change not taking effect)
/// is "window unavailable" and retried next sweep. manage == false: check only.
func readyWindow(manage: Bool) throws -> (window: [String:Any], resized: Bool) {
    var window: [String:Any]
    do { window = try selectedWindow() } catch Failure.invalid("wechat_not_visible") where manage {
        try restoreMinimizedWeChat()
        window = try selectedWindow()
    }
    let x = window["x"] as! Double, y = window["y"] as! Double
    let width = window["width"] as! Double, height = window["height"] as! Double
    if width.rounded() == 880 && height.rounded() == 640 { return (window, false) }
    guard manage, AXIsProcessTrusted() else { throw Failure.invalid("unsupported_image_geometry") }
    let ax = AXUIElementCreateApplication(pid_t(window["pid"] as! Int))
    var listed: CFTypeRef?
    let windows = AXUIElementCopyAttributeValue(ax, kAXWindowsAttribute as CFString, &listed) == .success ? (listed as? [AXUIElement] ?? []) : []
    // An image viewer or preview in front is itself the AX main window: with more than one visible
    // WeChat window we cannot tell which is the chat window, so touch nothing and wait.
    guard windows.filter({ !axFlag($0, kAXMinimizedAttribute as String) }).count == 1,
          let element = axElement(ax, kAXMainWindowAttribute as String),
          !axFlag(element, kAXMinimizedAttribute as String), !axFlag(element, "AXFullScreen"),
          let origin = axValue(element, kAXPositionAttribute as String, .cgPoint, CGPoint.zero),
          let size = axValue(element, kAXSizeAttribute as String, .cgSize, CGSize.zero),
          abs(origin.x - x) < 1, abs(origin.y - y) < 1, abs(size.width - width) < 1, abs(size.height - height) < 1 else {
        // The largest WeChat window is not the main window (e.g. an image viewer): do not touch it.
        throw Failure.invalid("wechat_not_visible")
    }
    var display = CGDirectDisplayID(0), count = UInt32(0)
    guard CGGetDisplaysWithPoint(CGPoint(x:x + width/2, y:y + height/2), 1, &display, &count) == .success, count == 1 else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    let bounds = CGDisplayBounds(display)
    var target = CGPoint(x:max(bounds.minX, min(x, bounds.maxX - 880)), y:max(bounds.minY + 25, min(y, bounds.maxY - 640)))
    if target != origin {
        guard let position = AXValueCreate(.cgPoint, &target),
              AXUIElementSetAttributeValue(element, kAXPositionAttribute as CFString, position) == .success else {
            throw Failure.invalid("unsupported_image_geometry")
        }
    }
    var wanted = CGSize(width:880, height:640)
    guard let value = AXValueCreate(.cgSize, &wanted),
          AXUIElementSetAttributeValue(element, kAXSizeAttribute as CFString, value) == .success else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    Thread.sleep(forTimeInterval:0.3)
    let after = try selectedWindow()
    guard after["window_id"] as? UInt32 == window["window_id"] as? UInt32,
          (after["width"] as! Double).rounded() == 880, (after["height"] as! Double).rounded() == 640 else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    return (after, true)
}

func canonicalBytes(_ image: CGImage) throws -> Data {
    var result = Data(count:image.width*image.height*4)
    let ok = result.withUnsafeMutableBytes { pointer -> Bool in
        guard let context = CGContext(data:pointer.baseAddress,width:image.width,height:image.height,
            bitsPerComponent:8,bytesPerRow:image.width*4,space:CGColorSpaceCreateDeviceRGB(),
            bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
        context.draw(image,in:CGRect(x:0,y:0,width:image.width,height:image.height)); return true
    }
    guard ok else { throw Failure.invalid("bitmap_failed") }
    return result
}

func writePNG(_ image: CGImage, path: URL) throws {
    guard let destination = CGImageDestinationCreateWithURL(path as CFURL,"public.png" as CFString,1,nil) else { throw Failure.invalid("png_failed") }
    CGImageDestinationAddImage(destination,image,nil)
    guard CGImageDestinationFinalize(destination) else { throw Failure.invalid("png_failed") }
    try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path.path)
}

final class CaptureWorker {
    var cachedID = ""
    var cached: [String:Any] = [:]
    var cachedWindow: [String:Any] = [:]
    var files: [String:URL] = [:]
    var fileAttempts=Set<String>()
    var payloads: [String:(Data,String)] = [:]
    let media = base.appendingPathComponent("bot-media")
    var indicator: Process?
    var indicatorInput: FileHandle?
    let sessionProbe = SessionProbe()

    func showIndicator(_ window: [String:Any]) throws {
        if indicator == nil {
            let process=Process(), pipe=Pipe()
            process.executableURL=base.appendingPathComponent("bin/wechat_overlay")
            process.arguments=["--status-only"]
            process.standardInput=pipe; process.standardOutput=FileHandle.nullDevice
            process.standardError=FileHandle.nullDevice
            try process.run()
            indicator=process; indicatorInput=pipe.fileHandleForWriting
        }
        guard indicator?.isRunning == true else { throw Failure.invalid("status_indicator_exited") }
        var event=window; event["event"]="window"
        var data=try JSONSerialization.data(withJSONObject:event); data.append(10)
        indicatorInput?.write(data)
    }

    deinit {
        try? indicatorInput?.close()
        if indicator?.isRunning == true { indicator?.terminate() }
    }

    init() throws {
        try FileManager.default.createDirectory(at:media,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        try checkPrivate(media,type:.typeDirectory,mode:0o700)
    }

    func titleAccepted(_ accepted: Set<String>) -> Bool {
        guard let titles=cached["title_lines"] as? [String], titles.count==1 else { return false }
        return titleMatches(titles[0], accepted)
    }

    func capture(_ recipient: String, accepted: Set<String>, manage: Bool) throws -> [String:Any] {
        let began=ProcessInfo.processInfo.systemUptime
        let (window, resized) = try readyWindow(manage:manage)
        let id = window["window_id"] as! UInt32
        let image = try windowImage(window)
        guard let panel = try normalizedFrame(image,window:window).image.cropping(to:CGRect(x:300,y:0,width:570,height:496)) else { throw Failure.invalid("window_capture_failed") }
        var hashInput = try canonicalBytes(panel)
        hashInput.append(try JSONSerialization.data(withJSONObject:window,options:.sortedKeys))
        let snapshot = SHA256.hash(data:hashInput).map { String(format:"%02x",$0) }.joined()
        let reused = snapshot == cachedID
        if !reused {
            cached = try analyzeFrame(image,window:window,mediaRoot:media,recipient:nil,group:groupTitles)
            var bubbles=cached["bubbles"] as? [[String:Any]] ?? []
            for i in bubbles.indices where bubbles[i]["kind"] as? String == "file" {
                let original=bubbles[i]["media_id"] as! String
                bubbles[i]["media_id"]=SHA256.hash(data:Data((snapshot+original+String(describing:bubbles[i]["y"])).utf8)).map { String(format:"%02x",$0) }.joined()
            }
            cached["bubbles"]=bubbles
            cachedID=snapshot; cachedWindow=window
        }
        guard titleAccepted(accepted) else { throw Failure.invalid("allowlisted_title_mismatch") }
        try showIndicator(window)
        var result=cached
        result["snapshot_id"]=snapshot; result["window_id"]=id; result["ocr_cached"]=reused
        result["observed_title"]=(cached["title_lines"] as? [String])?.first ?? ""
        result["captured_at_ms"]=Int64(Date().timeIntervalSince1970*1000)
        result["capture_seconds"]=ProcessInfo.processInfo.systemUptime-began
        if resized { result["window_resized"]=true }
        return result
    }

    func request(_ request: [String:Any]) throws -> [String:Any] {
        guard request["protocol_version"] as? Int == 1,
              let operation=request["operation"] as? String,
              let params=request["params"] as? [String:Any],
              let recipient=params["recipient"] as? String,!recipient.isEmpty else { throw Failure.invalid("invalid_request") }
        // Exact match against the contact's configured name and aliases (OCR may drop emoji/symbols).
        let names=(params["accepted_names"] as? [String]) ?? [recipient]
        guard (1...20).contains(names.count), names.allSatisfy({ !$0.isEmpty && $0.count<=64 }) else { throw Failure.invalid("invalid_accepted_names") }
        let accepted=Set(names.map(cleanTitle))
        let manage=params["manage_window"] as? Bool ?? false  // Absent (older client): check only.
        groupTitles = params["group_title"] as? Bool ?? false
        if operation == "capture" { return try capture(recipient,accepted:accepted,manage:manage) }
        if operation == "scroll_chat" {
            guard let direction=params["direction"] as? String, direction=="up" || direction=="down",
                  let pages=params["pages"] as? Int, (1...40).contains(pages) else { throw Failure.invalid("invalid_scroll_request") }
            let window=try readyWindow(manage:manage).window
            guard titleAccepted(accepted) else { throw Failure.invalid("allowlisted_title_mismatch") }
            guard let wx=window["x"] as? Double,let wy=window["y"] as? Double else { throw Failure.invalid("invalid_window_bounds") }
            let layout = try currentLayout()
            let chatRegion = try layoutRegion(layout, "chat")
            let chat=CGRect(x:wx+chatRegion.minX,y:wy+chatRegion.minY,width:chatRegion.width,height:chatRegion.height)
            let linesPerTick = Int32(layout.scroll.linesPerTick)
            try act { try botActions.scroll(in:chat,lines:direction=="up" ? linesPerTick : -linesPerTick,ticks:pages) }
            pauseUI(0.1)
            pauseUI(0.3)
            return try capture(recipient,accepted:accepted,manage:manage)
        }
        if operation == "open_session" {
            guard let target=params["target"] as? String, !target.isEmpty, target.count<=64 else { throw Failure.invalid("invalid_session_target") }
            let targets=(params["target_names"] as? [String]) ?? [target]
            guard (1...20).contains(targets.count), targets.allSatisfy({ !$0.isEmpty && $0.count<=64 }) else { throw Failure.invalid("invalid_session_target") }
            return try openSessionRow(targets,probe:sessionProbe) {
                let window=try readyWindow(manage:manage).window
                return (window,try normalizedFrame(try windowImage(window),window:window).image)
            }
        }
        if operation == "sessions" {
            guard let names=params["names"] as? [String], !names.isEmpty, names.count<=50,
                  names.allSatisfy({ !$0.isEmpty && $0.count<=64 }) else { throw Failure.invalid("invalid_session_names") }
            let began=ProcessInfo.processInfo.systemUptime
            let (window,resized)=try readyWindow(manage:manage)
            let image=try normalizedFrame(try windowImage(window),window:window).image
            var result=try sessionProbe.probe(image,names:names,debug:params["debug"] as? Bool == true)
            let layout=try currentLayout()
            let sessionListRect=try layoutRegion(layout, "session_list")
            if params["debug"] as? Bool == true, let list=image.cropping(to:sessionListRect) {
                let id=SHA256.hash(data:try canonicalBytes(list)).map { String(format:"%02x",$0) }.joined()
                try writePNG(list,path:media.appendingPathComponent(id+".png"))
                result["debug_media_id"]=id
            }
            result["capture_seconds"]=ProcessInfo.processInfo.systemUptime-began
            if resized { result["window_resized"]=true }
            return result
        }
        // Dev-only: capture each save step under ~/.wechat-rpa/debug-save/ only when this request asks for it.
        saveDebugDirectory = nil
        if params["debug_save"] as? Bool == true {
            saveDebugDirectory = base.appendingPathComponent("debug-save")
            if let directory=saveDebugDirectory {
                try? FileManager.default.removeItem(at:directory)
                try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
            }
        }
        if operation == "prepare_file" {
            guard let id=params["media_id"] as? String,let snapshot=params["snapshot_id"] as? String else { throw Failure.invalid("missing_file_identity") }
            let frame=try capture(recipient,accepted:accepted,manage:manage)
            guard frame["snapshot_id"] as? String==snapshot,
                  let item=(frame["bubbles"] as? [[String:Any]])?.first(where:{ $0["media_id"] as? String==id && $0["kind"] as? String=="file" }),
                  let name=item["filename"] as? String else { throw Failure.invalid("file_snapshot_changed") }
            let ext=URL(fileURLWithPath:name).pathExtension.lowercased()
            guard ["pdf","txt","md","csv","json","docx","xlsx","png","jpg","jpeg"].contains(ext) else { throw Failure.invalid("unsupported_file_type") }
            let directory=media.appendingPathComponent(id)
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
            try checkPrivate(directory,type:.typeDirectory,mode:0o700)
            let destination=directory.appendingPathComponent("incoming."+ext)
            if !FileManager.default.fileExists(atPath:destination.path) {
                guard !fileAttempts.contains(id) else { throw Failure.invalid("previous_download_outcome_unknown") }
                fileAttempts.insert(id)
                try closingDialogsOnFailure { try saveAttachment(item,window:cachedWindow,destination:destination) }
            }
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:destination.path)
            try checkPrivate(destination,type:.typeRegular,mode:0o600)
            let size=(try FileManager.default.attributesOfItem(atPath:destination.path)[.size] as? NSNumber)?.intValue ?? 0
            guard size>0 && size<=10*1024*1024 else { throw Failure.invalid("file_size_limit") }
            files[id]=destination
            var result:[String:Any]=["media_id":id,"filename":name,"extension":ext,"size":size,"original_bytes":true]
            if ext=="pdf" { result["extraction"]=try pdfContents(destination) }
            return result
        }
        if operation == "prepare_image" {
            guard let id=params["media_id"] as? String,let snapshot=params["snapshot_id"] as? String else { throw Failure.invalid("missing_image_identity") }
            let frame=try capture(recipient,accepted:accepted,manage:manage)
            guard frame["snapshot_id"] as? String==snapshot,
                  let item=(frame["bubbles"] as? [[String:Any]])?.first(where:{ $0["media_id"] as? String==id && $0["kind"] as? String=="image" })
            else { throw Failure.invalid("image_snapshot_changed") }
            let directory=media.appendingPathComponent(id)
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
            try checkPrivate(directory,type:.typeDirectory,mode:0o700)
            // WeChat suggests a .jpg name for chat images; the real type is read from the bytes below.
            let saved=directory.appendingPathComponent("original.jpg")
            if !FileManager.default.fileExists(atPath:saved.path) {
                guard !fileAttempts.contains(id) else { throw Failure.invalid("previous_download_outcome_unknown") }
                fileAttempts.insert(id)
                try closingDialogsOnFailure { try saveAttachment(item,window:cachedWindow,destination:saved,checkSize:false) }
            }
            let head=try Data(contentsOf:saved).prefix(8)
            let ext = head.starts(with:[0x89,0x50,0x4E,0x47]) ? "png" : head.starts(with:[0xFF,0xD8,0xFF]) ? "jpg" : ""
            guard !ext.isEmpty else { throw Failure.invalid("unsupported_image_bytes") }
            var final=saved
            if ext=="png" {
                final=directory.appendingPathComponent("original.png")
                if !FileManager.default.fileExists(atPath:final.path) { try FileManager.default.moveItem(at:saved,to:final) }
            }
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:final.path)
            try checkPrivate(final,type:.typeRegular,mode:0o600)
            let size=(try FileManager.default.attributesOfItem(atPath:final.path)[.size] as? NSNumber)?.intValue ?? 0
            guard size>0 && size<=10*1024*1024 else { throw Failure.invalid("media_too_large") }
            files[id]=final
            payloads.removeValue(forKey:id)  // a cached thumbnail must not be served as the original
            return ["media_id":id,"extension":ext,"size":size,"original_bytes":true]
        }
        if operation == "read_media" {
            guard let id=params["media_id"] as? String,id.count==64,id.allSatisfy({ $0.isHexDigit }),
                  let offset=params["offset"] as? Int, offset>=0 else { throw Failure.invalid("invalid_media_request") }
            // The same current snapshot must still belong to the allowlisted contact.
            guard titleAccepted(accepted) else { throw Failure.invalid("media_contact_mismatch") }
            let file=files[id] ?? media.appendingPathComponent(id+".png")
            try checkPrivate(file,type:.typeRegular,mode:0o600)
            if payloads[id]==nil {
                let bytes=try Data(contentsOf:file)
                guard bytes.count<=10*1024*1024 else { throw Failure.invalid("media_too_large") }
                if payloads.count>=8,let oldest=payloads.keys.first { payloads.removeValue(forKey:oldest) }
                payloads[id]=(bytes,SHA256.hash(data:bytes).map { String(format:"%02x",$0) }.joined())
            }
            let (data,digest)=payloads[id]!
            guard data.count<=10*1024*1024,offset<=data.count else { throw Failure.invalid("media_too_large_or_offset_invalid") }
            let chunk=data.subdata(in:offset..<min(offset+65536,data.count))
            return ["bytes":chunk.base64EncodedString(),"offset":offset,"size":data.count,"extension":file.pathExtension,"eof":offset+chunk.count==data.count,
                    "sha256":digest]
        }
        throw Failure.invalid("unsupported_operation")
    }
}

/// A failed save must not leave a menu, save panel or "replace?" alert covering the chat.
func closingDialogsOnFailure(_ body: () throws -> Void) throws {
    do { try body() } catch {
        try? key(53);pauseUI(0.2);try? key(53);pauseUI(0.2)
        throw error
    }
}

func address(_ path: String) throws -> sockaddr_un {
    var value=sockaddr_un(); value.sun_family=sa_family_t(AF_UNIX);value.sun_len=UInt8(MemoryLayout<sockaddr_un>.size)
    let bytes=Array(path.utf8)+[0]
    guard bytes.count<=MemoryLayout.size(ofValue:value.sun_path) else { throw Failure.invalid("socket_path_too_long") }
    withUnsafeMutableBytes(of:&value.sun_path) { $0.copyBytes(from:bytes) }
    return value
}

func withDesktop<T>(_ operation: () throws -> T) throws -> T {
    let directory=FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/message-gateway")
    try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    let fd=open(directory.appendingPathComponent("desktop.lock").path,O_CREAT|O_RDWR,0o600)
    guard fd>=0 else { throw Failure.invalid("desktop_lock_unavailable") }
    defer { close(fd) }
    let end=Date().addingTimeInterval(15)
    while flock(fd,LOCK_EX|LOCK_NB) != 0 {
        guard errno==EWOULDBLOCK && Date()<end else { throw Failure.invalid("desktop_busy") }
        Thread.sleep(forTimeInterval:0.05)
    }
    defer { flock(fd,LOCK_UN) }
    return try operation()
}

@main
struct BotWorkerMain {
    static func main() {
        do {
            try checkPrivate(base,type:.typeDirectory,mode:0o700)
            try checkPrivate(base.appendingPathComponent("bot-worker.enabled"),type:.typeRegular,mode:0o600)
            let ownerLock=open(base.appendingPathComponent("bot-worker.lock").path,O_CREAT|O_RDWR,0o600)
            guard ownerLock>=0 else { throw Failure.invalid("worker_lock_failed") }
            defer { close(ownerLock) }
            if flock(ownerLock,LOCK_EX|LOCK_NB) != 0 {
                if errno==EWOULDBLOCK { return }
                throw Failure.invalid("worker_lock_failed")
            }
            warmUpRecognizer()  // Before listening, so the first capture is not a cold model load.
            let path=base.appendingPathComponent("bot-worker.sock").path
            var addr=try address(path)
            if FileManager.default.fileExists(atPath:path) {
                try checkPrivate(URL(fileURLWithPath:path),type:.typeSocket,mode:0o600)
                let probe=socket(AF_UNIX,SOCK_STREAM,0)
                let live=withUnsafePointer(to:&addr) { $0.withMemoryRebound(to:sockaddr.self,capacity:1) { connect(probe,$0,socklen_t(MemoryLayout<sockaddr_un>.size)) } }
                let reason=errno; close(probe)
                if live==0 { return }
                guard reason==ECONNREFUSED || reason==ENOENT else { throw Failure.invalid("existing_socket_unavailable") }
                try FileManager.default.removeItem(atPath:path)
            }
            let fd=socket(AF_UNIX,SOCK_STREAM,0)
            guard fd>=0 else { throw Failure.invalid("socket_failed") }
            defer { close(fd); try? FileManager.default.removeItem(atPath:path) }
            let bound=withUnsafePointer(to:&addr) { $0.withMemoryRebound(to:sockaddr.self,capacity:1) { bind(fd,$0,socklen_t(MemoryLayout<sockaddr_un>.size)) } }
            guard bound==0 else { throw Failure.invalid("socket_bind_failed") }
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path)
            guard listen(fd,1)==0 else { throw Failure.invalid("socket_listen_failed") }
            let worker=try CaptureWorker()
            // One control connection; inactivity bounds an orphan worker's lifetime.
            var waiting=pollfd(fd:fd,events:Int16(POLLIN),revents:0)
            guard poll(&waiting,1,30000)>0 else { return }
            let client=accept(fd,nil,nil)
            guard client>=0 else { throw Failure.invalid("socket_accept_failed") }
            defer { close(client) }
            var uid:uid_t=0,gid:gid_t=0
            guard getpeereid(client,&uid,&gid)==0,uid==getuid() else { throw Failure.invalid("peer_uid_mismatch") }
            let handle=FileHandle(fileDescriptor:client,closeOnDealloc:false)
            var buffer=Data()
            while true {
                var descriptor=pollfd(fd:client,events:Int16(POLLIN),revents:0)
                guard poll(&descriptor,1,600000)>0 else { return }
                let chunk=handle.availableData
                if chunk.isEmpty { return }
                buffer.append(chunk)
                guard buffer.count<=65536 else { throw Failure.invalid("request_too_large") }
                while let newline=buffer.firstIndex(of:10) {
                    let line=buffer.prefix(upTo:newline);buffer.removeSubrange(...newline)
                    var id:Any=0
                    var response:[String:Any]
                    do {
                        guard let request=try JSONSerialization.jsonObject(with:Data(line)) as? [String:Any] else { throw Failure.invalid("invalid_json") }
                        id=request["id"] ?? 0
                        resetOCRReport()
                        let result=try withDesktop { try worker.request(request) }
                        response=["id":id,"protocol_version":1,"ok":true,"result":result,"ocr":ocrReport]
                    } catch { response=["id":id,"protocol_version":1,"ok":false,"error":String(describing:error),"ocr":ocrReport] }
                    var data=try JSONSerialization.data(withJSONObject:response,options:.sortedKeys);data.append(10)
                    handle.write(data)
                }
            }
        } catch { fputs("wechat_bot_worker: \(error)\n",stderr); exit(1) }
    }
}
