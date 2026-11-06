// Conservative visual candidates for a calibrated dark-theme WeChat test window.
import Foundation
import CoreGraphics
import ImageIO
import Vision
import CryptoKit
import AppKit

enum Failure: Error { case invalid(String) }
struct Box { let x: Int; let y: Int; let width: Int; let height: Int }
struct Text { let body: String; let x: Double; let y: Double }

private var layoutCache: (path: String, modified: Date?, layout: WechatLayout)?

var versionMemo = VersionMemo()

func wechatAppVersion() -> String? {
    return versionMemo.resolve(lookup: {
        for id in ["com.tencent.xinWeChat", "com.tencent.WeChat"] {
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: id).first else { continue }
            var version: String? = nil
            if let url = app.bundleURL,
               let info = NSDictionary(contentsOf: url.appendingPathComponent("Contents/Info.plist")),
               let read = info["CFBundleShortVersionString"] as? String, !read.isEmpty {
                version = read
            } else {
                FileHandle.standardError.write(Data("wechat version unreadable: pid \(app.processIdentifier) url \(app.bundleURL?.path ?? "nil")\n".utf8))
            }
            return (app.processIdentifier, version)
        }
        FileHandle.standardError.write(Data("wechat not among running applications\n".utf8))
        return nil
    }, alive: { pid in kill(pid, 0) == 0 })
}

func currentLayout() throws -> WechatLayout {
    let env = ProcessInfo.processInfo.environment
    let path: String
    if let explicit = env["WECHAT_LAYOUT"]?.trimmingCharacters(in: .whitespaces), !explicit.isEmpty {
        path = explicit
    } else {
        guard let ver = wechatAppVersion() else {
            throw Failure.invalid("layout_missing")
        }
        let layoutDir = env["WECHAT_LAYOUT_DIR"] ?? (NSHomeDirectory() + "/.wechat-rpa/layout")
        guard let found = WechatLayout.path(forVersion: ver, directory: layoutDir) else {
            throw Failure.invalid("layout_missing")
        }
        path = found
    }

    guard FileManager.default.fileExists(atPath: path) else {
        throw Failure.invalid("layout_missing")
    }

    let attrs = try? FileManager.default.attributesOfItem(atPath: path)
    let modified = attrs?[.modificationDate] as? Date

    if let cached = layoutCache, cached.path == path, cached.modified == modified {
        return cached.layout
    }

    let loaded: WechatLayout
    do {
        loaded = try WechatLayout.load(path: path)
    } catch let err as LayoutError {
        throw Failure.invalid(err.description)
    } catch {
        throw Failure.invalid("layout_unreadable")
    }

    layoutCache = (path, modified, loaded)
    return loaded
}

func layoutRegion(_ layout: WechatLayout, _ name: String) throws -> CGRect {
    do {
        let r = try layout.region(name)
        return CGRect(x:r.x, y:r.y, width:r.w, height:r.h)
    } catch let err as LayoutError {
        throw Failure.invalid(err.description)
    }
}

func recognize(_ image: CGImage, timeout: Double = 12) throws -> [Text] {
    let completed = DispatchSemaphore(value:0)
    var result: [Text] = []
    var failure: Error?
    // RapidOCR may use its own 12s and then fall back to Vision, so allow both in a row.
    let limit = (try? currentOCREngine()) == "rapidocr" ? timeout*2 : timeout
    DispatchQueue.global(qos:.userInitiated).async {
        do { result = try recognizeNow(image) } catch { failure = error }
        completed.signal()
    }
    guard completed.wait(timeout:.now()+limit) == .success else { throw Failure.invalid("vision_timeout") }
    if let failure = failure { throw failure }
    return result
}

/// Loads the text model once (Vision cold start is ~30s on macOS 26+; RapidOCR loads in a few seconds).
func warmUpRecognizer() {
    if (try? currentOCREngine()) == "rapidocr" {
        do { try rapidOCR.ensureStarted() } catch { fputs("rapidocr warm-up failed: \(error)\n", stderr) }
        return
    }
    guard let context = CGContext(data:nil,width:200,height:60,bitsPerComponent:8,bytesPerRow:0,
        space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    context.setFillColor(CGColor(red:1,green:1,blue:1,alpha:1)); context.fill(CGRect(x:0,y:0,width:200,height:60))
    guard let image = context.makeImage() else { return }
    do { _ = try recognize(image, timeout:60) } catch { fputs("vision warm-up failed: \(error)\n", stderr) }
}

// MARK: OCR engine switch (vision | rapidocr)

private var switchCache: (path: String, modified: Date?, engine: String)?

/// WECHAT_OCR_ENGINE wins; otherwise ~/.wechat-rpa/ocr.json (or WECHAT_OCR_CONFIG); no file = vision.
/// The file is re-read when its modification time changes, so a switch needs no restart.
func currentOCREngine() throws -> String {
    let env = ProcessInfo.processInfo.environment
    if let forced = env["WECHAT_OCR_ENGINE"]?.trimmingCharacters(in:.whitespaces).lowercased(), !forced.isEmpty {
        guard ["vision","rapidocr"].contains(forced) else { throw Failure.invalid("invalid_ocr_engine") }
        return forced
    }
    let path = env["WECHAT_OCR_CONFIG"] ?? NSHomeDirectory()+"/.wechat-rpa/ocr.json"
    let modified = (try? FileManager.default.attributesOfItem(atPath:path))?[.modificationDate] as? Date
    if let cached = switchCache, cached.path == path, cached.modified == modified { return cached.engine }
    var engine = "vision"
    if modified != nil {
        guard let data = FileManager.default.contents(atPath:path),
              let object = try? JSONSerialization.jsonObject(with:data) as? [String:Any],
              let value = (object["engine"] as? String)?.lowercased(),
              ["vision","rapidocr"].contains(value) else { throw Failure.invalid("invalid_ocr_engine") }
        engine = value
    }
    switchCache = (path, modified, engine)
    return engine
}

/// What the last request's recognitions actually used; `fallback` only when RapidOCR was skipped.
var ocrReport: [String:Any] = ["engine":"vision","engine_used":"vision"]

func resetOCRReport() {
    let engine = (try? currentOCREngine()) ?? "vision"
    ocrReport = ["engine":engine,"engine_used":engine]
}

/// RapidOCR four-point boxes to the same centre-point Text that Vision yields (pixel coordinates).
func rapidTexts(_ lines: [[String:Any]]) -> [Text] {
    lines.compactMap { line -> Text? in
        guard let raw = line["text"] as? String, let box = line["box"] as? [[Double]], box.count == 4,
              box.allSatisfy({ $0.count == 2 }) else { return nil }
        let body = raw.replacingOccurrences(of:"\n",with:" ").replacingOccurrences(of:"\t",with:" ")
        guard !body.trimmingCharacters(in:.whitespaces).isEmpty else { return nil }
        return Text(body:body, x:box.map { $0[0] }.reduce(0,+)/4, y:box.map { $0[1] }.reduce(0,+)/4)
    }.sorted { $0.y < $1.y }
}

/// One long-lived helper process (native/rapidocr_server.py); restarted after a failure, but not
/// sooner than `cooldown` so a broken install does not cost a cold start on every frame.
final class RapidOCRClient {
    private var process: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var buffer = Data()
    private var retryAfter = Date.distantPast
    private(set) var lastFailure = "rapidocr_unavailable"
    let requestTimeout: Double = 12
    let startTimeout: Double = 30
    let cooldown: Double = 60

    init() { signal(SIGPIPE, SIG_IGN) }

    func ensureStarted() throws {
        if process?.isRunning == true { return }
        guard Date() >= retryAfter else { throw Failure.invalid(lastFailure) }
        do { try start() } catch { fail(error); throw error }
    }

    func recognize(_ image: CGImage) throws -> [Text] {
        try ensureStarted()
        let env = ProcessInfo.processInfo.environment
        let folder = env["WECHAT_OCR_TMP"] ?? NSHomeDirectory()+"/.wechat-rpa/ocr-tmp"
        do {
            try FileManager.default.createDirectory(atPath:folder, withIntermediateDirectories:true, attributes:[.posixPermissions:0o700])
            let path = folder+"/"+UUID().uuidString+".png"
            defer { try? FileManager.default.removeItem(atPath:path) }
            guard let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath:path) as CFURL, "public.png" as CFString, 1, nil) else {
                throw Failure.invalid("rapidocr_image_write_failed")
            }
            CGImageDestinationAddImage(destination, image, nil)
            guard CGImageDestinationFinalize(destination) else { throw Failure.invalid("rapidocr_image_write_failed") }
            var request = try JSONSerialization.data(withJSONObject:["image":path]); request.append(10)
            input?.write(request)
            let reply = try readJSON(timeout:requestTimeout)
            if let error = reply["error"] as? String { throw Failure.invalid("rapidocr_error: "+error) }
            guard let lines = reply["lines"] as? [[String:Any]] else { throw Failure.invalid("rapidocr_bad_reply") }
            return rapidTexts(lines)
        } catch { fail(error); throw error }
    }

    private func start() throws {
        let env = ProcessInfo.processInfo.environment
        let python = env["WECHAT_RAPIDOCR_PYTHON"] ?? "/usr/bin/python3"
        let script = env["WECHAT_RAPIDOCR_SCRIPT"] ?? NSHomeDirectory()+"/.wechat-rpa/bin/rapidocr_server.py"
        guard FileManager.default.isExecutableFile(atPath:python), FileManager.default.fileExists(atPath:script) else {
            throw Failure.invalid("rapidocr_unavailable")
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath:python)
        process.arguments = [script]
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin; process.standardOutput = stdout
        process.standardError = FileHandle.nullDevice
        try process.run()
        self.process = process; input = stdin.fileHandleForWriting; output = stdout.fileHandleForReading; buffer = Data()
        let ready = try readJSON(timeout:startTimeout)
        if let error = ready["error"] as? String { throw Failure.invalid("rapidocr_unavailable: "+error) }
        guard ready["ready"] as? Bool == true else { throw Failure.invalid("rapidocr_bad_reply") }
    }

    /// Next JSON object line; other lines (library noise) are skipped.
    private func readJSON(timeout: Double) throws -> [String:Any] {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            while let newline = buffer.firstIndex(of:10) {
                let line = buffer.prefix(upTo:newline); buffer.removeSubrange(...newline)
                if let object = try? JSONSerialization.jsonObject(with:Data(line)) as? [String:Any] { return object }
            }
            guard let output = output else { throw Failure.invalid("rapidocr_exited") }
            let remaining = deadline.timeIntervalSinceNow
            guard remaining > 0 else { throw Failure.invalid("rapidocr_timeout") }
            let arrived = DispatchSemaphore(value:0)
            final class Box { var data = Data() }
            let box = Box()
            DispatchQueue.global(qos:.userInitiated).async { box.data = output.availableData; arrived.signal() }
            guard arrived.wait(timeout:.now()+remaining) == .success else { throw Failure.invalid("rapidocr_timeout") }
            if box.data.isEmpty { throw Failure.invalid("rapidocr_exited") }
            buffer.append(box.data)
        }
    }

    private func fail(_ error: Error) {
        if case Failure.invalid(let reason) = error { lastFailure = reason } else { lastFailure = "rapidocr_error: \(error)" }
        if process?.isRunning == true { process?.terminate() }
        process = nil; input = nil; output = nil; buffer = Data()
        retryAfter = Date().addingTimeInterval(cooldown)
    }
}

let rapidOCR = RapidOCRClient()

func recognizeNow(_ image: CGImage) throws -> [Text] {
    if try currentOCREngine() == "rapidocr" {
        do {
            return try rapidOCR.recognize(image)
        } catch {
            ocrReport["engine_used"] = "vision"
            if ocrReport["fallback"] == nil { ocrReport["fallback"] = rapidOCR.lastFailure }
        }
    }
    return try recognizeWithVision(image)
}

func recognizeWithVision(_ image: CGImage) throws -> [Text] {
    let request = VNRecognizeTextRequest()
    request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    try VNImageRequestHandler(cgImage:image, options:[:]).perform([request])
    return (request.results ?? []).compactMap { item in
        guard let text = item.topCandidates(1).first else { return nil }
        return Text(body:text.string, x:item.boundingBox.midX*Double(image.width),
                    y:(1-item.boundingBox.midY)*Double(image.height))
    }.sorted { $0.y < $1.y }
}

/// Calibrated colour rules for one WeChat 4.1.15 theme, applied in 880x640 1x coordinates.
struct ThemeProfile {
    let name: String
    let isBackground: (Int,Int,Int) -> Bool
    /// 0 = none, 1 = incoming (left) bubble fill, 2 = own (right, green) bubble fill.
    let kind: (Int,Int,Int) -> Int
    /// Anything that is not chat background; used to find media and file cards.
    let isForeground: (Int,Int,Int) -> Bool
}

func isOwnGreen(_ r: Int, _ g: Int, _ b: Int) -> Bool { g > 110 && g > r+45 && g > b+30 }

func themeProfiles(_ layout: WechatLayout) -> [ThemeProfile] {
    var profiles: [ThemeProfile] = []
    if let dark = layout.themes["wechat-4-dark"],
       let bgMax = dark.backgroundMax,
       let incoming = dark.incoming, incoming.count == 2 {
        let low = incoming[0], high = incoming[1]
        profiles.append(ThemeProfile(
            name: "wechat-4-dark",
            isBackground: { r, g, b in r < bgMax && g < bgMax && b < bgMax },
            kind: { r, g, b in
                if isOwnGreen(r, g, b) { return 2 }
                if r >= low && r <= high && abs(r - g) <= 8 && abs(r - b) <= 8 { return 1 }
                return 0
            },
            isForeground: { r, g, b in max(r, max(g, b)) > bgMax }
        ))
    }
    if let light = layout.themes["wechat-4-light"],
       let bgMin = light.backgroundMin,
       let incoming = light.incoming, incoming.count == 2 {
        let low = incoming[0], high = incoming[1]
        profiles.append(ThemeProfile(
            name: "wechat-4-light",
            isBackground: { r, g, b in r >= bgMin && g >= bgMin && b >= bgMin },
            kind: { r, g, b in
                if isOwnGreen(r, g, b) { return 2 }
                if r >= low && r <= high && abs(r - g) <= 8 && abs(r - b) <= 8 { return 1 }
                return 0
            },
            isForeground: { r, g, b in min(r, min(g, b)) < bgMin }
        ))
    }
    return profiles
}

/// Picks the theme whose background fills most of the chat area; refuses light/unknown frames
/// rather than interpreting arbitrary rectangles as messages.
func detectTheme(_ pixels: [UInt8], count: Int, profiles: [ThemeProfile]) throws -> ThemeProfile {
    let samples = stride(from:0,to:count,by:101).map { (Int(pixels[$0*4]),Int(pixels[$0*4+1]),Int(pixels[$0*4+2])) }
    for profile in profiles where samples.filter({ profile.isBackground($0.0,$0.1,$0.2) }).count > samples.count/2 {
        return profile
    }
    throw Failure.invalid("uncalibrated_theme")
}

func detectTheme(_ pixels: [UInt8], count: Int) throws -> ThemeProfile {
    let layout = try currentLayout()
    return try detectTheme(pixels, count: count, profiles: themeProfiles(layout))
}

/// Maps a 1x or Retina (2x) capture of the window onto the calibrated 1x pixel grid.
/// All layout constants and colour rules are defined on that grid; the original stays available
/// for full-resolution media crops.
func normalizedFrame(_ captured: CGImage, window: [String:Any]) throws -> (image: CGImage, scale: Int) {
    let layout = try currentLayout()
    let targetWidth = layout.window.width
    let targetHeight = layout.window.height
    let twInt = Int(targetWidth)
    let thInt = Int(targetHeight)
    guard let pointWidth = window["width"] as? Double, let pointHeight = window["height"] as? Double,
          window["on_screen"] as? Bool == true,
          pointWidth.rounded() == targetWidth, pointHeight.rounded() == targetHeight,
          let scale = [1,2].first(where: { captured.width == twInt*$0 && captured.height == thInt*$0 }) else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    if scale == 1 { return (captured, 1) }
    guard let context = CGContext(data:nil,width:twInt,height:thInt,bitsPerComponent:8,bytesPerRow:0,
            space:CGColorSpaceCreateDeviceRGB(),
            bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else {
        throw Failure.invalid("bitmap_failed")
    }
    context.interpolationQuality = .high
    context.draw(captured,in:CGRect(x:0,y:0,width:targetWidth,height:targetHeight))
    guard let image = context.makeImage() else { throw Failure.invalid("bitmap_failed") }
    return (image, scale)
}

/// Extensions a WeChat file card may show. Recognising a card is separate from reading it:
/// `prepare_file` still refuses types the bot cannot read (the reply then says so).
let fileCardExtensions: Set<String> = ["pdf","txt","md","csv","json","rtf","doc","docx","xls","xlsx","ppt","pptx",
    "pages","numbers","key","zip","rar","7z","png","jpg","jpeg","gif","heic","mp3","wav","m4a","mp4","mov","apk","dmg"]

/// The card line as a filename, or nil. OCR confuses l with I/1/| in extensions (".xIs" for ".xls").
func fileNameLine(_ line: String) -> String? {
    let trimmed=line.trimmingCharacters(in:.whitespaces)
    guard let dot=trimmed.lastIndex(of:"."), dot != trimmed.startIndex else { return nil }
    let raw=String(trimmed[trimmed.index(after:dot)...])
    guard (1...7).contains(raw.count) else { return nil }
    var ext=raw.lowercased()
    if !fileCardExtensions.contains(ext) {
        ext=String(ext.map { "i1|".contains($0) ? "l" : $0 })
    }
    guard fileCardExtensions.contains(ext) else { return nil }
    return String(trimmed[..<dot])+"."+ext
}

/// Stage C screen check on the raw capture at its own scale; throws instead of letting anything read
/// or click a screen that does not match the map. An unknown appearance keeps its own code (a known
/// wait, e.g. a theme switch); everything else is `layout_mismatch:<failed anchors>`.
func checkAnchors(_ captured: CGImage, scale: Int, layout: WechatLayout) throws -> AnchorReport {
    let report = verifyAnchors(captured, scale: scale, layout: layout)
    if report.failures.contains("theme") { throw Failure.invalid("uncalibrated_theme") }
    guard report.passed else { throw Failure.invalid("layout_mismatch:"+report.failures.joined(separator:",")) }
    return report
}

func analyzeFrame(_ captured: CGImage, window: [String:Any], mediaRoot: URL? = nil, recipient: String? = nil, group: Bool = false) throws -> [String:Any] {
    let layout = try currentLayout()
    let titleRegion: WindowRect
    let chatContentRegion: WindowRect
    do {
        titleRegion = try layout.region("title")
        chatContentRegion = try layout.region("chat_content")
    } catch let err as LayoutError {
        throw Failure.invalid(err.description)
    }
    let (full, scale) = try normalizedFrame(captured, window:window)
    // Stage C: check the screen against the map before reading anything (anchors work on the raw
    // capture at its own scale). A mismatch is a wait, never a guess.
    let anchorReport = try checkAnchors(captured, scale: scale, layout: layout)
    guard let title = full.cropping(to:CGRect(x:titleRegion.x,y:titleRegion.y,width:titleRegion.w,height:titleRegion.h)),
          let chat = full.cropping(to:CGRect(x:chatContentRegion.x,y:chatContentRegion.y,width:chatContentRegion.w,height:chatContentRegion.h)) else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    let titleNames = try recognize(title).filter { $0.x >= 50 && $0.x < 220 }.map(\.body)
    if let expected = recipient, titleNames != [expected] { throw Failure.invalid("allowlisted_title_mismatch") }
    let width = chat.width, height = chat.height
    var pixels = [UInt8](repeating:0,count:width*height*4)
    let rendered = pixels.withUnsafeMutableBytes { buffer -> Bool in
        guard let context = CGContext(data:buffer.baseAddress,width:width,height:height,
            bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),
            bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
        context.draw(chat,in:CGRect(x:0,y:0,width:width,height:height)); return true
    }
    guard rendered else { throw Failure.invalid("bitmap_failed") }
    let theme = try detectTheme(pixels, count:width*height, profiles:themeProfiles(layout))
    func color(_ index: Int) -> Int {
        theme.kind(Int(pixels[index*4]),Int(pixels[index*4+1]),Int(pixels[index*4+2]))
    }
    var visited = [Bool](repeating:false,count:width*height)
    var regions: [(Box,Int)] = []
    var clippedBubble = false
    for start in 0..<(width*height) where !visited[start] {
        visited[start] = true
        let kind = color(start)
        if kind == 0 { continue }
        var queue = [start], head = 0
        var left=start%width,right=left,top=start/width,bottom=top
        while head < queue.count {
            let index=queue[head]; head += 1
            let x=index%width,y=index/width
            left=min(left,x); right=max(right,x); top=min(top,y); bottom=max(bottom,y)
            for (nx,ny) in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)] where nx >= 0 && nx < width && ny >= 0 && ny < height {
                let next=ny*width+nx
                if !visited[next] && color(next) == kind { visited[next]=true; queue.append(next) }
            }
        }
        let box=Box(x:left,y:top,width:right-left+1,height:bottom-top+1)
        guard queue.count >= 500, box.width >= 30, box.height >= 25, box.height <= 200 else { continue }
        let anchored = (kind == 1 && (42...72).contains(box.x)) ||
                       (kind == 2 && (45...80).contains(width-box.x-box.width))
        if anchored && box.y+box.height >= height-2 { clippedBubble = true }
        guard box.y >= 2, box.y+box.height < height-2 else { continue }
        if kind == 1 && (42...72).contains(box.x) { regions.append((box,kind)) }
        if kind == 2 && (45...80).contains(width-box.x-box.width) { regions.append((box,kind)) }
    }
    let texts = try recognize(chat)
    var bubbles: [[String:Any]] = []
    for (box,kind) in regions.sorted(by:{ $0.0.y < $1.0.y }) {
        let body=texts.filter { $0.x >= Double(box.x) && $0.x < Double(box.x+box.width) &&
                              $0.y >= Double(box.y) && $0.y < Double(box.y+box.height) }.map(\.body).joined(separator:"\n")
        if body.isEmpty { continue }
        var bytes = Data()
        for y in box.y..<(box.y+box.height) {
            bytes.append(contentsOf:pixels[(y*width+box.x)*4..<(y*width+box.x+box.width)*4])
        }
        let digest=SHA256.hash(data:bytes).map { String(format:"%02x",$0) }.joined()
        bubbles.append(["text":body,"direction":kind == 1 ? "inbound_visual" : "outbound_visual",
                        "digest":digest,"kind":"text","x":box.x,"y":box.y,"width":box.width,"height":box.height])
    }
    // Media rectangles have the same left anchor, but do not have a uniform text-bubble fill.
    var broadVisited=[Bool](repeating:false,count:width*height)
    func foreground(_ index: Int) -> Bool {
        return theme.isForeground(Int(pixels[index*4]),Int(pixels[index*4+1]),Int(pixels[index*4+2]))
    }
    for start in 0..<(width*height) where !broadVisited[start] {
        broadVisited[start]=true
        if !foreground(start) { continue }
        var queue=[start],head=0,left=start%width,right=left,top=start/width,bottom=top
        while head<queue.count {
            let index=queue[head];head+=1
            let x=index%width,y=index/width
            left=min(left,x);right=max(right,x);top=min(top,y);bottom=max(bottom,y)
            for (nx,ny) in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)] where nx>=0 && nx<width && ny>=0 && ny<height {
                let next=ny*width+nx
                if !broadVisited[next] && foreground(next) { broadVisited[next]=true;queue.append(next) }
            }
        }
        let box=Box(x:left,y:top,width:right-left+1,height:bottom-top+1)
        guard (42...72).contains(box.x),box.width>=80,box.height>=55,queue.count>=2000 else { continue }
        if box.y+box.height>=height-2 { clippedBubble=true;continue }
        guard box.y>=2 else { continue }
        let body=texts.filter { $0.x>=Double(box.x) && $0.x<Double(box.x+box.width) && $0.y>=Double(box.y) && $0.y<Double(box.y+box.height) }.map(\.body).joined(separator:"\n")
        let names=body.components(separatedBy:"\n").compactMap(fileNameLine)
        // WeChat 4 writes sizes as "220.3K" as well as "12KB"; the transfer status shares that line.
        let sizePattern=#"\d+(\.\d+)?\s*(KB|MB|GB|K|M|G|字节|B)"#
        let sizeRange=body.range(of:sizePattern,options:[.regularExpression,.caseInsensitive])
        let hasSize=sizeRange != nil
        // OCR sometimes misses the size line; a card holding one filename and little else is still a file.
        let lines=body.components(separatedBy:"\n").filter { !$0.trimmingCharacters(in:.whitespaces).isEmpty }
        let isFile=names.count==1 && (hasSize || lines.count<=3)
        let neutral=queue.filter { color($0)==1 }.count
        let isImage = !isFile && (box.height>200 || Double(neutral)/Double(box.width*box.height)<0.60)
        guard isFile || isImage else { continue }
        var bytes=Data()
        for y in box.y..<(box.y+box.height) { bytes.append(contentsOf:pixels[(y*width+box.x)*4..<(y*width+box.x+box.width)*4]) }
        let digest=SHA256.hash(data:bytes).map { String(format:"%02x",$0) }.joined()
        bubbles.removeAll { old in
            guard let x=old["x"] as? Int,let y=old["y"] as? Int else { return false }
            return abs(x-box.x)<10 && abs(y-box.y)<10
        }
        var item:[String:Any] = ["text":isImage ? "[图片]" : body,"direction":"inbound_visual","digest":digest,
            "kind":isImage ? "image" : "file","media_id":digest,"x":box.x,"y":box.y,"width":box.width,"height":box.height]
        if isImage {
            // Pixels change while WeChat loads an image; continuity uses its size plus a perceptual hash.
            item["digest"]=SHA256.hash(data:Data("image:\(box.width/4):\(box.height/4)".utf8)).map { String(format:"%02x",$0) }.joined()
            let chatX = Int(chatContentRegion.x)
            let chatY = Int(chatContentRegion.y)
            let rect=CGRect(x:(chatX+box.x)*scale,y:(chatY+box.y)*scale,width:box.width*scale,height:box.height*scale)
            guard let region=captured.cropping(to:rect) else { throw Failure.invalid("media_crop_failed") }
            item["phash"]=try differenceHash(region)
        }
        if isFile {
            item["filename"]=names[0]
            // Identity is filename + size token only: the status text (上传中 -> done) changes in place.
            let size=sizeRange.map { String(body[$0]).replacingOccurrences(of:" ",with:"") } ?? ""
            let identity="file:"+names[0]+":"+size
            item["transfer_pending"]=["上传中","下载中","接收中","发送中"].contains { body.contains($0) }
            item["digest"]=SHA256.hash(data:Data(identity.utf8)).map { String(format:"%02x",$0) }.joined()
        }
        if isImage,let directory=mediaRoot {
            let path=directory.appendingPathComponent(digest+".png")
            if !FileManager.default.fileExists(atPath:path.path) {
                // Crop media from the original capture so Retina images keep their full resolution.
                let chatX = Int(chatContentRegion.x)
                let chatY = Int(chatContentRegion.y)
                guard let cropped=captured.cropping(to:CGRect(x:(chatX+box.x)*scale,y:(chatY+box.y)*scale,
                                                              width:box.width*scale,height:box.height*scale)),
                      let destination=CGImageDestinationCreateWithURL(path as CFURL,"public.png" as CFString,1,nil) else { throw Failure.invalid("media_crop_failed") }
                CGImageDestinationAddImage(destination,cropped,nil)
                guard CGImageDestinationFinalize(destination) else { throw Failure.invalid("media_write_failed") }
                try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path.path)
            }
            item["image_source"]="chat_thumbnail"
        }
        bubbles.append(item)
    }
    bubbles.sort { ($0["y"] as! Int)<($1["y"] as! Int) }
    if group {
        // Group chats print the member's name just above each left bubble, left-aligned with it.
        let boxes=bubbles.compactMap { b -> CGRect? in
            guard let x=b["x"] as? Int,let y=b["y"] as? Int,let w=b["width"] as? Int,let h=b["height"] as? Int else { return nil }
            return CGRect(x:x,y:y,width:w,height:h)
        }
        let outside=texts.filter { t in !boxes.contains { $0.contains(CGPoint(x:t.x,y:t.y)) } }
        for index in bubbles.indices where bubbles[index]["direction"] as? String == "inbound_visual" {
            guard let x=bubbles[index]["x"] as? Int,let y=bubbles[index]["y"] as? Int else { continue }
            let label=outside.filter { Double(y)-$0.y > 2 && Double(y)-$0.y < 30 && $0.x > Double(x) && $0.x < Double(x+260)
                && $0.body.range(of:#"^\d{1,2}:\d{2}$"#,options:.regularExpression) == nil }.min { abs($0.x-Double(x)) < abs($1.x-Double(x)) }
            if let label=label { bubbles[index]["sender"]=label.body }
        }
    }
    var result: [String:Any] = ["profile":theme.name,"scale":scale,"title_lines":titleNames,
        "bubbles":bubbles,"platform_message_verified":false,"sender_verified":false,
        "width":full.width,"height":full.height,"captured_width":captured.width,"clipped_bubble":clippedBubble]
    result["layout_check"]=["theme":anchorReport.theme ?? NSNull(),"measured":anchorReport.measured] as [String:Any]
    return result
}

/// 64-bit dHash: 9x8 grayscale, each bit = left pixel brighter than its right neighbour (row-major hex).
func differenceHash(_ image: CGImage) throws -> String {
    var gray=[UInt8](repeating:0,count:9*8)
    let drawn=gray.withUnsafeMutableBytes { buffer -> Bool in
        guard let context=CGContext(data:buffer.baseAddress,width:9,height:8,bitsPerComponent:8,bytesPerRow:9,
                                    space:CGColorSpaceCreateDeviceGray(),bitmapInfo:CGImageAlphaInfo.none.rawValue) else { return false }
        context.interpolationQuality = .high
        context.draw(image,in:CGRect(x:0,y:0,width:9,height:8))
        return true
    }
    guard drawn else { throw Failure.invalid("phash_failed") }
    var bits:UInt64=0
    for y in 0..<8 { for x in 0..<8 { bits=(bits<<1)|(gray[y*9+x]>gray[y*9+x+1] ? 1 : 0) } }
    return String(format:"%016llx",bits)
}

#if !BOT_WORKER
@main
struct BubbleCLI {
    static func main() {
        do {
            guard CommandLine.arguments.count == 3,
                  let windowData = CommandLine.arguments[2].data(using:.utf8),
                  let window = try JSONSerialization.jsonObject(with:windowData) as? [String:Any],
                  let source = CGImageSourceCreateWithURL(URL(fileURLWithPath:CommandLine.arguments[1]) as CFURL,nil),
                  let full = CGImageSourceCreateImageAtIndex(source,0,nil) else { throw Failure.invalid("invalid_input") }
            warmUpRecognizer()
            resetOCRReport()
            var result = try analyzeFrame(full,window:window)
            result["ocr"] = ocrReport
            let data = try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
            print(String(decoding:data,as:UTF8.self))
        } catch { fputs("wechat_bubbles: \(error)\n",stderr); exit(1) }
    }
}
#endif
