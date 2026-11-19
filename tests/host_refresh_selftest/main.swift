import AppKit
import Foundation

if CommandLine.arguments.dropFirst() == ["--host-version"] {
    print(hostAppVersion())
    exit(0)
}

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
// 指标改成「标题 + 数值」两个 label（对齐效果图），按 identifier 取数值断言。
func metric(_ root: NSView, _ id: String) -> String? {
    views(root).compactMap { $0 as? NSTextField }.first { $0.identifier?.rawValue == id }?.stringValue
}
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
check("overview_refreshes_metrics_from_host_snapshot", metric(overview.view, "overview.metric.listening") == "1" && metric(overview.view, "overview.metric.handoff") == "1")
check("overview_refreshes_pause_action", button(overview.view, "恢复自动回复"))
check("overview_replaces_old_diagnostics", text(overview.view, "fixture-live-health"))
input.bot.healthState = "watching"
input.sessions = ["新会话": SessionHealthInput(state: "watching"), "第二会话": SessionHealthInput(state: "watching")]
publish(context)
check("overview_updates_again_without_reopening", metric(overview.view, "overview.metric.listening") == "2" && metric(overview.view, "overview.metric.handoff") == "0" && button(overview.view, "暂停自动回复"))
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
check("log_display_strips_ansi_but_keeps_chinese", plainLogDisplayText("\u{001B}[2m时间\u{001B}[0m \u{001B}[32mINFO\u{001B}[0m 已暂停") == "时间 INFO 已暂停")
check("log_display_preserves_plain_content", plainLogDisplayText("[INFO] 状态 [paused]") == "[INFO] 状态 [paused]")
print("RESULT: \(failures) failures")
exit(failures == 0 ? 0 : 1)
