import AppKit
import ApplicationServices
import CoreGraphics

func encodedProbe(_ probe: Probe) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    return try encoder.encode(probe)
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

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "微信客服助手"
        window.isReleasedWhenClosed = false

        let controller = StatusPanelController(worker: worker, coreSupervisor: coreSupervisor)
        self.panelController = controller
        window.contentViewController = controller

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
        worker.stop()
        coreSupervisor.stopProcess()
    }
}

let delegate = HostAppDelegate()
let application = NSApplication.shared
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
