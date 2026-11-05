import AppKit
import ApplicationServices
import PDFKit
import CryptoKit

func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value:CFTypeRef?
    guard AXUIElementCopyAttributeValue(element,key as CFString,&value) == .success else { return nil }
    return value
}

func elements(_ root: AXUIElement, depth: Int=0) -> [AXUIElement] {
    var seen:[AXUIElement]=[]
    func visit(_ element: AXUIElement,_ depth: Int) {
        guard depth<=8,seen.count<800,!seen.contains(where:{ CFEqual($0,element) }) else { return }
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

func saveAttachment(_ item: [String:Any], window:[String:Any], destination:URL, checkSize: Bool = true) throws {
    // Images carry no size text; their saved size is checked after the download instead.
    if checkSize {
        let body=item["text"] as? String ?? ""
        let expression=try NSRegularExpression(pattern:#"(\d+(?:\.\d+)?)\s*(KB|MB|GB|字节|B)"#,options:.caseInsensitive)
        let range=NSRange(body.startIndex..<body.endIndex,in:body)
        // OCR may miss the size line; the saved file's size is checked by the caller either way.
        if let match=expression.matches(in:body,range:range).last,
           let number=Range(match.range(at:1),in:body),let unitRange=Range(match.range(at:2),in:body),
           let amount=Double(body[number]) {
            let unit=body[unitRange].uppercased()
            let multiplier:Double = unit=="GB" ? 1073741824 : unit=="MB" ? 1048576 : unit=="KB" ? 1024 : 1
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
    down.post(tap:.cghidEventTap);up.post(tap:.cghidEventTap);pauseUI(0.25)
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
    guard save.count==1 else {
        try? key(53)
        throw Failure.invalid("save_as_menu_missing:"+menus.compactMap { attribute($0,"AXTitle") as? String }.joined(separator:"|"))
    }
    try press(save[0]);pauseUI(0.3)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier==app.processIdentifier else { throw Failure.invalid("unexpected_save_dialog_owner") }
    try key(5,flags:[.maskCommand,.maskShift])
    try paste(destination.deletingLastPathComponent().path)
    try key(36);pauseUI(0.2)
    let fields=elements(root).filter { attribute($0,"AXRole") as? String == "AXTextField" }
    let exact=fields.filter { (attribute($0,"AXIdentifier") as? String ?? "").lowercased().contains("saveas") }
    let names=exact.isEmpty ? fields.filter {
        let value=attribute($0,"AXValue") as? String ?? ""
        return !value.contains("/") && value.lowercased().hasSuffix("."+destination.pathExtension)
    } : exact
    guard names.count==1,AXUIElementSetAttributeValue(names[0],kAXValueAttribute as CFString,destination.lastPathComponent as CFTypeRef) == .success else {
        try? key(53);throw Failure.invalid("save_filename_field_not_unique")
    }
    let buttons=elements(root).filter { attribute($0,"AXRole") as? String == "AXButton" && ["存储","保存","Save"].contains(attribute($0,"AXTitle") as? String ?? "") }
    guard buttons.count==1 else { try? key(53);throw Failure.invalid("save_button_not_unique") }
    try press(buttons[0])
    let deadline=Date().addingTimeInterval(8)
    while Date()<deadline {
        if FileManager.default.fileExists(atPath:destination.path) {
            let first=try FileManager.default.attributesOfItem(atPath:destination.path)[.size] as? NSNumber
            pauseUI(0.3)
            let second=try FileManager.default.attributesOfItem(atPath:destination.path)[.size] as? NSNumber
            if first==second && (second?.intValue ?? 0)>0 { return }
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
