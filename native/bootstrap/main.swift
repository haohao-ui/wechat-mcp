import AppKit
import ApplicationServices
import CoreGraphics

func encodedProbe(_ probe: Probe) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    return try encoder.encode(probe)
}

if CommandLine.arguments.dropFirst() == ["--version"] {
    print(hostAppVersion())
    exit(0)
}

if CommandLine.arguments.dropFirst() == ["--json"] {
    do {
        print(String(decoding: try encodedProbe(readDesktopProbe()), as: UTF8.self))
        exit(0)
    } catch {
        fputs("desktop_probe_encode_failed\n", stderr)
        exit(1)
    }
}

if CommandLine.arguments.dropFirst() == ["--transport-self-test"] {
    do {
        let count = try SocketWorker().testTransport()
        print("{\"transport_self_test\":\"passed\",\"desktop_accessed\":false,\"tool_count\":\(count)}")
        exit(0)
    } catch {
        fputs("desktop_transport_self_test_failed: \(error.localizedDescription)\n", stderr)
        exit(1)
    }
}

final class HostAppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private let worker = SocketWorker()
    private let coreSupervisor = CoreSupervisor()
    private var panelController: StatusPanelController?
    private var terminationSignal: DispatchSourceSignal?

    func applicationDidFinishLaunching(_ notification: Notification) {
        HostLogger.shared.log("微信客服助手启动，版本: \(hostAppVersion())")
        // launchd 停止/重启宿主时发 SIGTERM。AppKit 默认直接退出、不走 applicationWillTerminate，
        // 核心与桌面服务子进程会成为无人管理的孤儿（2026-11-08 实机）。改为走正常退出流程。
        signal(SIGTERM, SIG_IGN)
        let termSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        termSource.setEventHandler {
            HostLogger.shared.log("收到 SIGTERM，正常退出并停止核心")
            NSApplication.shared.terminate(nil)
        }
        termSource.resume()
        terminationSignal = termSource
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 860, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "微信客服助手"
        window.isReleasedWhenClosed = false

        let controller = StatusPanelController(worker: worker, coreSupervisor: coreSupervisor)
        self.panelController = controller
        // 旧面板不再显示，但它的 viewDidLoad 负责读取配置并启动 5 秒定时器
        // （定时器里 CoreSupervisor.checkAndReconcile 负责启动核心、崩溃后重启）：必须加载。
        _ = controller.view  // 触发 viewDidLoad（loadViewIfNeeded 需要 macOS 14，设备目标是 12）

        let shell = ShellViewController()
        shell.register(page: OverviewPageController(context: controller), id: "overview", title: "总览")
        shell.register(page: SessionsPageController(context: controller), id: "sessions", title: "会话")
        shell.register(page: ContactsPageController(context: controller), id: "contacts", title: "联系人")
        shell.register(page: BrainPageController(context: controller), id: "brain", title: "大脑")
        shell.register(page: McpPageController(context: controller), id: "mcp", title: "MCP")
        shell.register(page: SettingsPageController(context: controller), id: "settings", title: "设置")
        shell.register(page: LogsPageController(context: controller), id: "logs", title: "日志")

        window.contentViewController = shell

        let menu = NSMenu()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "退出微信客服助手", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        let item = NSMenuItem()
        item.submenu = appMenu
        menu.addItem(item)
        NSApplication.shared.mainMenu = menu

        self.window = window
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)

        // 尝试启动 worker 服务（若权限具备）
        let probe = readDesktopProbe()
        if probe.screen_recording_authorized && probe.accessibility_trusted {
            try? worker.start()
        }
    }

    // Closing the window must not stop the desktop service: a clean exit is not restarted by the LaunchAgent.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { window?.makeKeyAndOrderFront(nil) }
        return true
    }
    func applicationWillTerminate(_ notification: Notification) {
        HostLogger.shared.log("微信客服助手退出")
        worker.stop()
        coreSupervisor.stopProcess()
    }
}

let delegate = HostAppDelegate()
let application = NSApplication.shared
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
