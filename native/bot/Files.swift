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
        guard let down=CGEvent(mouseEventSource:nil,mouseType:.leftMouseDown,mouseCursorPosition:target,mouseButton:.left),
              let up=CGEvent(mouseEventSource:nil,mouseType:.leftMouseUp,mouseCursorPosition:target,mouseButton:.left) else { throw Failure.invalid("mouse_event_failed") }
        down.post(tap:.cghidEventTap);pauseUI(0.05);up.post(tap:.cghidEventTap)
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

func saveAttachment(_ item: [String:Any], window:[String:Any], destination:URL, checkSize: Bool = true) throws {
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
            guard amount*multiplier<=10*1024*1024 else { throw Failure.invalid("file_size_limit") }
        }
    }
    guard let app=NSWorkspace.shared.frontmostApplication,
          ["com.tencent.xinWeChat","com.tencent.WeChat"].contains(app.bundleIdentifier ?? ""),
          let x=item["x"] as? Int,let y=item["y"] as? Int,
          let w=item["width"] as? Int,let h=item["height"] as? Int else { throw Failure.invalid("wechat_not_foreground_or_invalid_media") }
    let root=AXUIElementCreateApplication(app.processIdentifier)
    let point=CGPoint(x:(window["x"] as! Double)+315+Double(x+w/2),y:(window["y"] as! Double)+60+Double(y+h/2))
    guard let down=CGEvent(mouseEventSource:nil,mouseType:.rightMouseDown,mouseCursorPosition:point,mouseButton:.right),
          let up=CGEvent(mouseEventSource:nil,mouseType:.rightMouseUp,mouseCursorPosition:point,mouseButton:.right) else { throw Failure.invalid("mouse_event_failed") }
    down.post(tap:.cghidEventTap);up.post(tap:.cghidEventTap);pauseUI(0.4)
    saveDebugStep("1-after-right-click",root)
    let titles=["另存为…","另存为...","另存为","存储为…","Save As…","Save As..."]
    var roots=[root]
    if let focused=attribute(AXUIElementCreateSystemWide(),"AXFocusedUIElement"), CFGetTypeID(focused)==AXUIElementGetTypeID() {
        roots.append(unsafeBitCast(focused,to:AXUIElement.self))
    }
    let menus=roots.flatMap { elements($0) }.filter { attribute($0,"AXRole") as? String == "AXMenuItem" }
    var save:[AXUIElement]=[]
    for item in menus where titles.contains(attribute(item,"AXTitle") as? String ?? "") {
        if !save.contains(where:{ CFEqual($0,item) }) { save.append(item) }
    }
    if save.count==1 {
        try press(save[0])
    } else if try !clickMenuTextOnScreen(titles,owner:app.processIdentifier,near:point) {
        // WeChat 4 draws its context menu outside the accessibility tree: read it off the screen instead.
        try? key(53)
        throw Failure.invalid("save_as_menu_missing")
    }
    pauseUI(0.3)
    saveDebugStep("2-after-menu-click",root)
    pauseUI(0.7)
    saveDebugStep("3-panel-settled",root)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier==app.processIdentifier else { throw Failure.invalid("unexpected_save_dialog_owner") }
    // Cmd+Shift+G does not reach WeChat's out-of-process save panel (the path landed in the name field):
    // save in whatever folder the panel shows, under a unique name, and move the file right after.
    let staging=FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Documents/微信客服附件")
    try FileManager.default.createDirectory(at:staging,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    // WeChat is sandboxed: its save panel is drawn by a separate system process whose controls are not
    // reliably in WeChat's accessibility tree. The name field has focus, so type the name and press Return.
    // A unique name never triggers "replace?" and can be found if the panel stayed in another folder.
    let staged="wxbot-"+String(UUID().uuidString.prefix(8)).lowercased()+"."+destination.pathExtension
    try key(0,flags:[.maskCommand]);pauseUI(0.1)
    try paste(staged);pauseUI(0.3)
    saveDebugStep("4-after-name",root)
    // Return only when the panel's own name field holds exactly our name (verified 2026-11-05).
    guard let focusedRef=attribute(root,"AXFocusedUIElement"), CFGetTypeID(focusedRef)==AXUIElementGetTypeID() else {
        throw Failure.invalid("save_name_field_not_focused")
    }
    let focused=unsafeBitCast(focusedRef,to:AXUIElement.self)
    guard attribute(focused,"AXIdentifier") as? String=="saveAsNameTextField", attribute(focused,"AXValue") as? String==staged else {
        throw Failure.invalid("save_name_not_set")
    }
    try key(36);pauseUI(0.5)
    saveDebugStep("5-after-return",root)
    let home=FileManager.default.homeDirectoryForCurrentUser
    let candidates=[staging,destination.deletingLastPathComponent()]+["Desktop","Downloads","Documents"].map { home.appendingPathComponent($0) }
    let deadline=Date().addingTimeInterval(10)
    while Date()<deadline {
        for folder in candidates {
            let found=folder.appendingPathComponent(staged)
            guard FileManager.default.fileExists(atPath:found.path) else { continue }
            let first=try FileManager.default.attributesOfItem(atPath:found.path)[.size] as? NSNumber
            pauseUI(0.3)
            let second=try FileManager.default.attributesOfItem(atPath:found.path)[.size] as? NSNumber
            guard first==second && (second?.intValue ?? 0)>0 else { continue }
            if FileManager.default.fileExists(atPath:destination.path) { try FileManager.default.removeItem(at:destination) }
            try FileManager.default.moveItem(at:found,to:destination)
            return
        }
        pauseUI(0.1)
    }
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
