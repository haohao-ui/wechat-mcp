// Explicit, isolated device acceptance; never opens the business StatusPanel view.
import AppKit
import Foundation

let environment = ProcessInfo.processInfo.environment
guard let config = environment["WECHAT_UPDATE_ACCEPTANCE_CONFIG"],
      let configData = FileManager.default.contents(atPath: config),
      let object = try? JSONSerialization.jsonObject(with: configData) as? [String: Any],
      let state = object["state_dir"] as? String else { fatalError("explicit acceptance configuration required") }
let base = URL(fileURLWithPath: config).deletingLastPathComponent().resolvingSymlinksInPath()
let stateURL = URL(fileURLWithPath: state).resolvingSymlinksInPath()
guard stateURL.path.hasPrefix(base.path + "/"), stateURL.path != base.path else { fatalError("acceptance state must be isolated below configuration directory") }
let cacheURL = stateURL.appendingPathComponent("update-downloads")
let expectReuse = environment["WECHAT_UPDATE_EXPECT_REUSE"] == "1"
let initialCandidates = (try? FileManager.default.contentsOfDirectory(at: cacheURL, includingPropertiesForKeys: nil)) ?? []
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let context = StatusPanelController(worker: SocketWorker())
let client = UpdateClient(configPath: config)
let expected = try client.checkUpdate()
let page = UpdatePageController(context: context, client: client)
let window = NSWindow(contentRect: NSRect(x: 0,y: 0,width: 1000,height: 800),styleMask: [.titled],backing: .buffered,defer: false)
window.isReleasedWhenClosed = false
window.appearance = NSAppearance(named: .darkAqua)
window.contentViewController = page
_ = page.view
window.setContentSize(NSSize(width: 1000, height: 800))
page.view.frame = NSRect(x: 0, y: 0, width: 1000, height: 800)
func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap { views($0) } }
func text(_ value: String) -> Bool { views(page.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue == value } }
func spinUntil(_ condition: () -> Bool, seconds: Double) {
    let end = ProcessInfo.processInfo.systemUptime + seconds
    while !condition() && ProcessInfo.processInfo.systemUptime < end { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.02)) }
}
let check = views(page.view).compactMap { $0 as? NSButton }.first { $0.title == "检查更新" }!
let download = views(page.view).compactMap { $0 as? NSButton }.first { $0.title == "下载更新" }!
check.performClick(nil)
spinUntil({ check.isEnabled && download.isEnabled },seconds: 30)
guard download.isEnabled else { fatalError("verified source and updater capability did not enable download") }
if expectReuse {
    guard download.title == "校验本地更新包", text("本机有候选缓存，需重新校验更新包") else { fatalError("cache was not explicitly labelled pending revalidation") }
}
download.performClick(nil)
spinUntil({ download.title == "已下载" || views(page.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue.hasPrefix("下载未完成") } },seconds: 90)
guard download.title == "已下载", !download.isEnabled,
      text("更新包已下载并校验，尚未安装。") else { fatalError("native download did not complete") }
for title in ["安装并重启","延期更新"] {
    guard views(page.view).compactMap({ $0 as? NSButton }).first(where: { $0.title == title })?.isEnabled == false else { fatalError("installation action unexpectedly enabled") }
}
guard !FileManager.default.fileExists(atPath: stateURL.appendingPathComponent("current_release.json").path),
      !FileManager.default.fileExists(atPath: stateURL.appendingPathComponent("core.maintenance").path) else { fatalError("download wrote installation or maintenance state") }
let cache = stateURL.appendingPathComponent("update-downloads")
let candidates = try FileManager.default.contentsOfDirectory(at: cache,includingPropertiesForKeys: nil)
let expectedCount = initialCandidates.count + (expectReuse ? 0 : 1)
guard candidates.count == expectedCount else { fatalError("unexpected cache entry count") }
let selected = candidates.first { candidate in
    guard let data = FileManager.default.contents(atPath: candidate.appendingPathComponent("receipt.json").path),
          let receipt = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
    return receipt["manifest_sha256"] as? String == expected.manifestSha256
}
guard let selectedCandidate = selected,
      let receiptData = FileManager.default.contents(atPath: selectedCandidate.appendingPathComponent("receipt.json").path),
      let receipt = try JSONSerialization.jsonObject(with: receiptData) as? [String: Any],
      receipt["status"] as? String == "downloaded" else { fatalError("real matching download receipt required") }
page.view.layoutSubtreeIfNeeded()
let rect = page.view.bounds
guard abs(rect.width - 1000) < 1, abs(rect.height - 800) < 1 else { fatalError("viewport size mismatch") }
if let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1000, pixelsHigh: 800, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) {
    bitmap.size = NSSize(width: 1000, height: 800)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    window.appearance?.performAsCurrentDrawingAppearance {
        NSColor.windowBackgroundColor.setFill()
        rect.fill()
        page.view.displayIgnoringOpacity(rect, in: NSGraphicsContext.current!)
    }
    NSGraphicsContext.restoreGraphicsState()
    if let data = bitmap.representation(using: .png,properties: [:]) { try data.write(to: base.appendingPathComponent("download-page.png")) }
}
var proof = receipt
proof["native_download_action_completed"] = true
proof["reused_existing_cache"] = expectReuse
proof["install_actions_disabled"] = true
proof["installation_performed"] = false
proof["cache_candidate_count"] = candidates.count
proof["new_cache_candidates"] = candidates.count - initialCandidates.count
proof["viewport_width"] = page.view.bounds.width
proof["viewport_height"] = page.view.bounds.height
let result = try JSONSerialization.data(withJSONObject: proof,options: [.sortedKeys,.prettyPrinted])
try result.write(to: base.appendingPathComponent("native-verification.json"))
print(String(decoding: result,as: UTF8.self))
exit(0)
