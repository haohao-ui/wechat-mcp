import AppKit
import ApplicationServices
import PDFKit
import CryptoKit

func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value:CFTypeRef?
    guard AXUIElementCopyAttributeValue(element,key as CFString,&value) == .success else { return nil }
    return value
}

func elements(_ root: AXUIElement, depth: Int=0, limit: Int=800) -> [AXUIElement] {
    var seen:[AXUIElement]=[]
    func visit(_ element: AXUIElement,_ depth: Int) {
        guard depth<=12,seen.count<limit,!seen.contains(where:{ CFEqual($0,element) }) else { return }
        seen.append(element)
        let children=(attribute(element,"AXChildren") as? [AXUIElement] ?? []) + (attribute(element,"AXWindows") as? [AXUIElement] ?? [])
        for child in children.prefix(100) { visit(child,depth+1) }
    }
    visit(root,depth)
    return seen
}

func pauseUI(_ seconds: Double) { RunLoop.current.run(until:Date().addingTimeInterval(seconds)) }

/// Polls `condition` (keeping the run loop going) until it holds or `seconds` pass.
func waitUntil(seconds: Double, _ condition: () -> Bool) -> Bool {
    let deadline=Date().addingTimeInterval(seconds)
    while Date()<deadline {
        if condition() { return true }
        pauseUI(0.1)
    }
    return condition()
}

/// The save panel's name field, when it has keyboard focus.
func focusedNameField(_ root: AXUIElement) -> AXUIElement? {
    guard let ref=attribute(root,"AXFocusedUIElement"), CFGetTypeID(ref)==AXUIElementGetTypeID() else { return nil }
    let element=unsafeBitCast(ref,to:AXUIElement.self)
    return attribute(element,"AXIdentifier") as? String=="saveAsNameTextField" ? element : nil
}

/// The worker's one "hand" (native/action): human-like paths, presses only inside the control's effective
/// region, stops when a person moves the mouse. One instance keeps pointer tracking continuous.
let botActions = Actions()

/// Runs an action and reports its refusals as worker codes (invalid("…")) the bot already classifies.
func act(_ body: () throws -> Void) throws {
    do { try body() } catch let error as ActionError { throw Failure.invalid(error.description) }
}

func key(_ code: CGKeyCode, flags: CGEventFlags=[]) throws {
    guard let down=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:true),
          let up=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:false) else { throw Failure.invalid("key_event_failed") }
    down.flags=flags;up.flags=flags;down.post(tap:.cghidEventTap);up.post(tap:.cghidEventTap)
    pauseUI(0.12)
}

func paste(_ text: String) throws {
    let board=NSPasteboard.general
    let saved=(board.pasteboardItems ?? []).map { item -> [NSPasteboard.PasteboardType:Data] in
        var values:[NSPasteboard.PasteboardType:Data]=[:]
        for type in item.types { if let data=item.data(forType:type) { values[type]=data } }
        return values
    }
    defer {
        board.clearContents()
        board.writeObjects(saved.map { values -> NSPasteboardItem in
            let item=NSPasteboardItem(); for (type,data) in values { item.setData(data,forType:type) };return item
        })
    }
    board.clearContents();board.setString(text,forType:.string)
    try key(9,flags:.maskCommand)
}

func press(_ element: AXUIElement) throws {
    guard AXUIElementPerformAction(element,kAXPressAction as CFString) == .success else { throw Failure.invalid("accessibility_press_failed") }
}

/// Clicks the first of `titles` found by OCR in a small WeChat popup window (context menu) near `point`.
/// Right-clicks `rect` (screen points) and chooses the context-menu row labelled `menu` in the layout
/// map: the accessibility tree first, then the menu read off the screen (WeChat 4 draws it outside the
/// tree). Shared by every context-menu action (save as, 转文字); nothing found closes it with Esc.
func chooseContextMenu(_ rect: CGRect, menu: String, app: NSRunningApplication) throws {
    let root=AXUIElementCreateApplication(app.processIdentifier)
    // Labels come only from the layout map; a map without them is a calibration error, not a guess.
    guard let titles = try currentLayout().menuLabels[menu], !titles.isEmpty else { throw Failure.invalid("layout_missing_menu:\(menu)") }
    // The context menu opens where the press landed; look for it there.
    var point=CGPoint(x:rect.midX,y:rect.midY)
    try act { point=try botActions.rightClick(rect) }
    pauseUI(0.4)
    saveDebugStep("1-after-right-click",root)
    var roots=[root]
    if let focused=attribute(AXUIElementCreateSystemWide(),"AXFocusedUIElement"), CFGetTypeID(focused)==AXUIElementGetTypeID() {
        roots.append(unsafeBitCast(focused,to:AXUIElement.self))
    }
    let menus=roots.flatMap { elements($0) }.filter { attribute($0,"AXRole") as? String == "AXMenuItem" }
    var matches:[AXUIElement]=[]
    for item in menus where titles.contains(attribute(item,"AXTitle") as? String ?? "") {
        if !matches.contains(where:{ CFEqual($0,item) }) { matches.append(item) }
    }
    if matches.count==1 {
        try press(matches[0])
    } else if try !clickMenuTextOnScreen(titles,owner:app.processIdentifier,near:point) {
        try? key(53)
        throw Failure.invalid("\(menu)_menu_missing")
    }
}

func clickMenuTextOnScreen(_ titles: [String], owner: pid_t, near point: CGPoint) throws -> Bool {
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
    let popups: [(UInt32,CGRect)] = rows.compactMap { row in
        guard row[kCGWindowOwnerPID as String] as? Int32 == owner,
              (row[kCGWindowLayer as String] as? Int ?? 0) != 0,
              let id = row[kCGWindowNumber as String] as? UInt32,
              let bounds = row[kCGWindowBounds as String] as? [String:Any],
              let rect = CGRect(dictionaryRepresentation:bounds as CFDictionary),
              rect.width > 20, rect.height > 20, rect.width < 600, rect.height < 900,
              rect.insetBy(dx:-40,dy:-40).contains(point) else { return nil }
        return (id,rect)
    }
    for (id,rect) in popups.sorted(by:{ $0.1.width*$0.1.height < $1.1.width*$1.1.height }) {
        let image = try windowImage(["window_id":id])
        guard let hit = try recognize(image).first(where:{ text in titles.contains { text.body.contains($0) } }) else { continue }
        let target = CGPoint(x:rect.minX+hit.x*rect.width/Double(image.width), y:rect.minY+hit.y*rect.height/Double(image.height))
        // The menu row: the popup's width (minus its padding) and 22pt around the recognised text.
        try act { try botActions.click(CGRect(x:rect.minX+8, y:target.y-11, width:rect.width-16, height:22)) }
        return true
    }
    return false
}

/// Dev-only evidence for the save flow: full-screen PNG + accessibility summary per step.
var saveDebugDirectory: URL? = nil
func saveDebugStep(_ tag: String, _ app: AXUIElement) {
    guard let directory=saveDebugDirectory else { return }
    if let screen=CGWindowListCreateImage(.infinite,.optionOnScreenOnly,kCGNullWindowID,[.bestResolution]) {
        try? writePNG(screen,path:directory.appendingPathComponent(tag+".png"))
    }
    var lines:[String]=["front="+(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "-")]
    if let focused=attribute(app,"AXFocusedWindow"), CFGetTypeID(focused)==AXUIElementGetTypeID() {
        let window=unsafeBitCast(focused,to:AXUIElement.self)
        lines.append("focusedWindow="+(attribute(window,"AXTitle") as? String ?? "-")+" role="+(attribute(window,"AXRole") as? String ?? "-")+" subrole="+(attribute(window,"AXSubrole") as? String ?? "-"))
    }
    if let element=attribute(app,"AXFocusedUIElement"), CFGetTypeID(element)==AXUIElementGetTypeID() {
        let focus=unsafeBitCast(element,to:AXUIElement.self)
        lines.append("focusedElement role="+(attribute(focus,"AXRole") as? String ?? "-")+" id="+(attribute(focus,"AXIdentifier") as? String ?? "-")+" value="+String((attribute(focus,"AXValue") as? String ?? "").suffix(40)))
    }
    let windows=attribute(app,"AXWindows") as? [AXUIElement] ?? []
    for window in windows {
        let all=elements(window,limit:4000)
        let fields=all.filter { ["AXTextField","AXComboBox"].contains(attribute($0,"AXRole") as? String ?? "") }
            .map { (attribute($0,"AXIdentifier") as? String ?? "-")+"="+String((attribute($0,"AXValue") as? String ?? "").suffix(30)) }
        let buttons=all.filter { attribute($0,"AXRole") as? String == "AXButton" }.compactMap { attribute($0,"AXTitle") as? String }.filter { !$0.isEmpty }
        lines.append("window '"+(attribute(window,"AXTitle") as? String ?? "-")+"' sub="+(attribute(window,"AXSubrole") as? String ?? "-")+" n=\(all.count) fields="+fields.joined(separator:"|")+" buttons="+buttons.prefix(25).joined(separator:"|"))
    }
    try? lines.joined(separator:"\n").write(to:directory.appendingPathComponent(tag+".txt"),atomically:true,encoding:.utf8)
}

func saveAttachment(_ item: [String:Any], window:[String:Any], destination:URL, checkSize: Bool = true, maxBytes: Int = 50*1024*1024) throws {
    // Images carry no size text; their saved size is checked after the download instead.
    if checkSize {
        let body=item["text"] as? String ?? ""
        let expression=try NSRegularExpression(pattern:#"(\d+(?:\.\d+)?)\s*(KB|MB|GB|K|M|G|字节|B)"#,options:.caseInsensitive)
        let range=NSRange(body.startIndex..<body.endIndex,in:body)
        // OCR may miss the size line; the saved file's size is checked by the caller either way.
        if let match=expression.matches(in:body,range:range).last,
           let number=Range(match.range(at:1),in:body),let unitRange=Range(match.range(at:2),in:body),
           let amount=Double(body[number]) {
            let unit=body[unitRange].uppercased()
            let multiplier:Double = unit.hasPrefix("G") ? 1073741824 : unit.hasPrefix("M") ? 1048576 : unit.hasPrefix("K") ? 1024 : 1
            guard amount*multiplier<=Double(maxBytes) else { throw Failure.invalid("file_size_limit") }
        }
    }
    // Before touching WeChat: the folders the save panel may drop the file into are privacy-protected.
    // An unanswered permission prompt blocks file calls in the kernel while we hold the desktop lock
    // (2026-11-08: a worker hung for 90+ minutes). Probe them with a time limit and fail clearly instead.
    let home=FileManager.default.homeDirectoryForCurrentUser
    let staging=home.appendingPathComponent("Documents/微信客服附件")
    try bounded(seconds:3,label:"Documents") {
        try FileManager.default.createDirectory(at:staging,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    }
    let probed=try preflightFolders([staging,destination.deletingLastPathComponent()]+["Desktop","Downloads","Documents"].map { home.appendingPathComponent($0) })
    let candidates=probed.usable
    // Right after a previous save the system save-panel service can still be reported as frontmost.
    let wechatIDs=["com.tencent.xinWeChat","com.tencent.WeChat"]
    _ = waitUntil(seconds:1.5) { wechatIDs.contains(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "") }
    guard let app=NSWorkspace.shared.frontmostApplication,
          ["com.tencent.xinWeChat","com.tencent.WeChat"].contains(app.bundleIdentifier ?? ""),
          let x=item["x"] as? Int,let y=item["y"] as? Int,
          let w=item["width"] as? Int,let h=item["height"] as? Int else { throw Failure.invalid("wechat_not_foreground_or_invalid_media") }
    let root=AXUIElementCreateApplication(app.processIdentifier)
    let chatRegion = try layoutRegion(try currentLayout(), "chat")
    let mediaRect=CGRect(x:(window["x"] as! Double)+chatRegion.minX+Double(x),y:(window["y"] as! Double)+chatRegion.minY+Double(y),width:Double(w),height:Double(h))
    try chooseContextMenu(mediaRect,menu:"save_as",app:app)
    pauseUI(0.3)
    saveDebugStep("2-after-menu-click",root)
    pauseUI(0.7)
    saveDebugStep("3-panel-settled",root)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier==app.processIdentifier else { throw Failure.invalid("unexpected_save_dialog_owner") }
    // Cmd+Shift+G does not reach WeChat's out-of-process save panel (the path landed in the name field):
    // save in whatever folder the panel shows, under a unique name, and move the file right after.
    // WeChat is sandboxed: its save panel is drawn by a separate system process whose controls are not
    // reliably in WeChat's accessibility tree. The name field has focus, so type the name and press Return.
    // A unique name never triggers "replace?" and can be found if the panel stayed in another folder.
    let staged="wxbot-"+String(UUID().uuidString.prefix(8)).lowercased()+"."+destination.pathExtension
    // The panel can take well over a second to take focus (acceptance 2026-11-05: a fixed 1s wait failed
    // whenever the step screenshots were off). Wait for the name field itself, then for our name in it.
    guard waitUntil(seconds:5, { focusedNameField(root) != nil }) else { throw Failure.invalid("save_name_field_not_focused") }
    try key(0,flags:[.maskCommand]);pauseUI(0.1)
    try paste(staged)
    let named = waitUntil(seconds:2) { focusedNameField(root).map { attribute($0,"AXValue") as? String==staged } ?? false }
    saveDebugStep("4-after-name",root)
    // Return only when the panel's own name field holds exactly our name (verified 2026-11-05).
    guard named else { throw Failure.invalid("save_name_not_set") }
    try key(36);pauseUI(0.5)
    saveDebugStep("5-after-return",root)
    let deadline=Date().addingTimeInterval(10)
    while Date()<deadline {
        for folder in candidates {
            let found=folder.appendingPathComponent(staged)
            let label=folder.lastPathComponent
            guard try bounded(seconds:5,label:label, { FileManager.default.fileExists(atPath:found.path) }) else { continue }
            let first=try bounded(seconds:5,label:label, { try FileManager.default.attributesOfItem(atPath:found.path)[.size] as? NSNumber })
            pauseUI(0.3)
            let second=try bounded(seconds:5,label:label, { try FileManager.default.attributesOfItem(atPath:found.path)[.size] as? NSNumber })
            guard first==second && (second?.intValue ?? 0)>0 else { continue }
            try bounded(seconds:5,label:destination.deletingLastPathComponent().lastPathComponent) {
                if FileManager.default.fileExists(atPath:destination.path) { try FileManager.default.removeItem(at:destination) }
                try FileManager.default.moveItem(at:found,to:destination)
            }
            return
        }
        pauseUI(0.1)
    }
    // A folder whose access was refused may be exactly where the panel saved: say so instead of "unknown".
    if !probed.denied.isEmpty { throw Failure.invalid("download_completion_unknown:denied="+probed.denied.joined(separator:",")) }
    throw Failure.invalid("download_completion_unknown")
}

func pdfContents(_ file: URL) throws -> [String:Any] {
    guard let document=PDFDocument(url:file),!document.isLocked else { throw Failure.invalid("pdf_unreadable_or_encrypted") }
    var text=""
    for index in 0..<min(document.pageCount,20) {
        if let page=document.page(at:index) { text+=(page.string ?? "")+"\n" }
        if text.count>16000 { break }
    }
    let empty=text.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty
    var previews:[String]=[]
    if empty {
        for index in 0..<min(document.pageCount,2) {
            guard let page=document.page(at:index),
                  let image=page.thumbnail(of:NSSize(width:1024,height:1400),for:.mediaBox).cgImage(forProposedRect:nil,context:nil,hints:nil) else { throw Failure.invalid("pdf_preview_failed") }
            let digest=SHA256.hash(data:try canonicalBytes(image)).map { String(format:"%02x",$0) }.joined()
            try writePNG(image,path:base.appendingPathComponent("bot-media").appendingPathComponent(digest+".png"))
            previews.append(digest)
        }
    }
    return ["text":String(text.prefix(16000)),"page_count":document.pageCount,"previews":previews,
            "truncated":document.pageCount>(empty ? 2 : 20) || text.count>16000,"scanned_or_empty":empty,
            "embedded_images_read":empty]
}
