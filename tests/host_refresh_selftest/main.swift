import AppKit
import Foundation

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let context = StatusPanelController(worker: SocketWorker())
let other = StatusPanelController(worker: SocketWorker())
let overview = OverviewPageController(context: context)
let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 700), styleMask: [.titled], backing: .buffered, defer: false)
window.isReleasedWhenClosed = false
window.contentViewController = overview
_ = overview.view
func views(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap { views($0) } }
func text(_ root: NSView, _ value: String) -> Bool { views(root).compactMap { $0 as? NSTextField }.contains { $0.stringValue == value } }
func button(_ root: NSView, _ value: String) -> Bool { views(root).compactMap { $0 as? NSButton }.contains { $0.title == value } }
var failures = 0
func check(_ name: String, _ result: Bool) { print("\(result ? "PASS" : "FAIL"): \(name)"); if !result { failures += 1 } }
var input = StatusInput(
    permissions: PermissionsInput(screenRecordingAuthorized: true, accessibilityTrusted: true),
    desktopService: DesktopServiceInput(socketExists: true, status: "running"),
    bot: BotInput(launchdLoaded: false, healthState: "paused", healthUpdatedAt: 100, now: 100),
    sessions: ["验收会话": SessionHealthInput(state: "watching"), "待人工会话": SessionHealthInput(state: "handoff", handoff: true)],
    wechat: WechatInput(isRunning: true), layout: LayoutInput(layoutExists: true), disk: DiskInput(freeBytes: 10_000_000_000)
)
func publish(_ source: StatusPanelController) {
    NotificationCenter.default.post(name: Notification.Name("WeChatHostStatusDidRefresh"), object: source, userInfo: ["input": input, "items": [StatusItem(key: "bot", level: .green, reason: "fixture-live-health")]])
}
publish(context)
check("overview_refreshes_metrics_from_host_snapshot", text(overview.view, "监听会话: 1") && text(overview.view, "待人工: 1"))
check("overview_refreshes_pause_action", button(overview.view, "恢复自动回复"))
check("overview_replaces_old_diagnostics", text(overview.view, "fixture-live-health"))
input.bot.healthState = "watching"
input.sessions = ["新会话": SessionHealthInput(state: "watching"), "第二会话": SessionHealthInput(state: "watching")]
publish(context)
check("overview_updates_again_without_reopening", text(overview.view, "监听会话: 2") && text(overview.view, "待人工: 0") && button(overview.view, "暂停自动回复"))
input.bot.healthState = "paused"
publish(other)
check("overview_ignores_another_host_context", button(overview.view, "暂停自动回复"))
let sessions = SessionsPageController(context: context)
window.contentViewController = sessions
_ = sessions.view
let query = views(sessions.view).compactMap { $0 as? NSTextField }.first { $0.placeholderString?.contains("turn_id") == true }
query?.stringValue = "unsaved-query"
input.sessions = ["实时待人工": SessionHealthInput(state: "handoff", handoff: true)]
publish(context)
check("sessions_refresh_from_host_snapshot", text(sessions.view, "实时待人工") && button(sessions.view, "恢复自动回复"))
check("sessions_keep_query_text_during_refresh", query?.stringValue == "unsaved-query")
print("RESULT: \(failures) failures")
exit(failures == 0 ? 0 : 1)
