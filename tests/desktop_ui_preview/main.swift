// Offscreen layout evidence. Never loads StatusPanel.view or starts the host services.
import AppKit
import Foundation

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let output = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
let stub = ProcessInfo.processInfo.environment["WECHAT_MCP_BIN"] ?? ""
guard stub.hasPrefix(output.path), FileManager.default.isExecutableFile(atPath: stub) else {
    fatalError("Preview requires an isolated stub inside the evidence directory")
}
let context = StatusPanelController(worker: SocketWorker(), coreSupervisor: CoreSupervisor())
let brainClient = BrainClient(configPath: output.appendingPathComponent("config.json").path, executor: { _, _, _ in
    ControlRun(status: 1, output: Data(), timedOut: false)
})
let updateClient = UpdateClient(configPath: output.appendingPathComponent("config.json").path, executor: { _, _, _ in
    ControlRun(status: 1, output: Data(), timedOut: false)
})
let update = UpdatePageController(context: context, client: updateClient)
let pages: [(String, String, NSViewController)] = [
    ("overview", "总览", OverviewPageController(context: context)),
    ("sessions", "会话", SessionsPageController(context: context)),
    ("contacts", "联系人", ContactsPageController(context: context)),
    ("brain", "大脑", BrainPageController(context: context, client: brainClient)),
    ("mcp", "MCP", McpPageController(context: context)),
    ("settings", "设置", SettingsPageController(context: context)),
    ("logs", "日志", LogsPageController(context: context)),
    ("update", "应用更新", update)
]
let shell = ShellViewController()
for (id, title, controller) in pages { shell.register(page: controller, id: id, title: title) }
let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 800), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
window.isReleasedWhenClosed = false
window.contentViewController = shell
var results: [[String: Any]] = []
for appearanceName in [NSAppearance.Name.aqua, .darkAqua] {
    let mode = appearanceName == .aqua ? "light" : "dark"
    window.appearance = NSAppearance(named: appearanceName)
    for size in [NSSize(width: 900, height: 640), NSSize(width: 1180, height: 800), NSSize(width: 1440, height: 900)] {
        window.setContentSize(size)
        shell.view.frame = NSRect(origin: .zero, size: size)
        for (id, _, _) in pages {
            shell.selectPage(id: id)
            let tabCount = id == "brain" ? 6 : 1
            for tab in 0..<tabCount {
            if let brain = pages[3].2 as? BrainPageController, id == "brain" { brain.selectTab(index: tab) }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
            if id == "update" {
                update.applyStatusPayload(UpdateStatusPayload(currentRelease: UpdateReleaseInfo(releaseId: nil, architecture: "unknown"), candidateRelease: nil, transaction: UpdateTransactionInfo(status: .idle), guardInfo: UpdateGuardInfo(highestAcceptedSequence: nil, maintenanceActive: false)))
                update.applyCheckResult(UpdateCheckResult(status: .unconfigured, reason: nil, candidate: nil, verification: nil))
            }
            shell.view.layoutSubtreeIfNeeded()
            window.display()
            guard abs(shell.view.bounds.width - size.width) < 1, abs(shell.view.bounds.height - size.height) < 1 else {
                fatalError("Layout grew beyond viewport: \(id) tab \(tab), requested \(size), actual \(shell.view.bounds.size)")
            }
            guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width), pixelsHigh: Int(size.height), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { fatalError("bitmap") }
            rep.size = size
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            window.appearance?.performAsCurrentDrawingAppearance {
                NSColor.windowBackgroundColor.setFill()
                NSRect(origin: .zero, size: size).fill()
                shell.view.displayIgnoringOpacity(shell.view.bounds, in: NSGraphicsContext.current!)
            }
            NSGraphicsContext.restoreGraphicsState()
            let pageName = id == "brain" && tab > 0 ? "brain-tab\(tab)" : id
            let name = "\(pageName)-\(Int(size.width))-\(mode).png"
            try rep.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(name))
            results.append(["page": pageName, "width": Int(size.width), "height": Int(size.height), "mode": mode, "file": name, "actual_width": shell.view.bounds.width, "actual_height": shell.view.bounds.height])
            }
        }
    }
}
try JSONSerialization.data(withJSONObject: results, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("screenshots.json"))
print("Rendered \(results.count) isolated page snapshots")

let figures = results.map { item -> String in
    let name = item["file"] as! String
    let page = item["page"] as! String
    return "<figure data-page='\(page)'><a href='\(name)'><img loading='lazy' src='\(name)' alt='\(name)'></a><figcaption>\(name)</figcaption></figure>"
}.joined(separator: "\n")
let options = Set(results.map { $0["page"] as! String }).sorted().map { "<option>\($0)</option>" }.joined()
let html = """
<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>桌面界面实际渲染</title>
<style>body{font:15px system-ui;margin:28px;background:#f4f6f5;color:#18231e}header{position:sticky;top:0;background:#f4f6f5;padding:12px 0}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:24px}figure{margin:0;background:white;border:1px solid #dce3de;border-radius:8px;overflow:hidden}img{width:100%;height:auto}figcaption{padding:10px;color:#47564d}select{padding:8px}p{max-width:900px;line-height:1.6}</style>
<header><h1>桌面界面实际渲染</h1><p>八页及大脑六分区，三种尺寸、浅深色，共 \(results.count) 张。离屏布局证据，隔离控制命令；不是生产数据或业务验收。点击图片查看原尺寸。</p><label>页面 <select id="page"><option value="all">全部</option>\(options)</select></label></header>
<main>\(figures)</main><script>document.querySelector('#page').onchange=e=>{document.querySelectorAll('figure').forEach(f=>f.hidden=e.target.value!=='all'&&f.dataset.page!==e.target.value)};</script></html>
"""
try html.write(to: output.appendingPathComponent("index.html"), atomically: true, encoding: .utf8)
