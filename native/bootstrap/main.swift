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

final class ProbeApp: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private let output = NSTextView()
    private let status = NSTextField(labelWithString: "")
    private let worker = SocketWorker()

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 560),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable],
                              backing: .buffered, defer: false)
        window.title = "WeChat RPA Test — 环境检查"
        window.isReleasedWhenClosed = false
        let explanation = NSTextField(wrappingLabelWithString:
            "此应用检查环境并托管微信桌面服务。检查本身不读取聊天正文或发送消息；收到已连接客户端的明确操作请求后，桌面服务才执行相应操作。")
        let buttons = NSStackView(views: [
            NSButton(title: "刷新检查", target: self, action: #selector(refresh)),
            NSButton(title: "请求屏幕录制权限", target: self, action: #selector(requestScreen)),
            NSButton(title: "请求辅助功能权限", target: self, action: #selector(requestAccessibility))
        ])
        buttons.orientation = .horizontal
        buttons.spacing = 12
        output.isEditable = false
        output.isSelectable = true
        output.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        let scroll = NSScrollView()
        scroll.documentView = output
        scroll.hasVerticalScroller = true
        let stack = NSStackView(views: [explanation, buttons, status, scroll])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        window.contentView?.addSubview(stack)
        if let content = window.contentView {
            NSLayoutConstraint.activate([
                stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
                stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),
                stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 20),
                stack.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -20),
                explanation.widthAnchor.constraint(equalTo: stack.widthAnchor),
                scroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
                scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 260)
            ])
        }
        let menu = NSMenu()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "退出环境检查", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        let item = NSMenuItem()
        item.submenu = appMenu
        menu.addItem(item)
        NSApplication.shared.mainMenu = menu
        self.window = window
        refresh()
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    @objc private func refresh() {
        do {
            let probe = readDesktopProbe()
            let data = try encodedProbe(probe)
            output.string = String(decoding: data, as: UTF8.self)
            let directory = FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Application Support/WeChatRPATest", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
            let report = directory.appendingPathComponent("desktop-probe.json")
            try data.write(to: report, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: report.path)
            status.stringValue = "屏幕录制：\(probe.screen_recording_authorized ? "已授权" : "未授权")；辅助功能：\(probe.accessibility_trusted ? "已授权" : "未授权")"
            if probe.screen_recording_authorized && probe.accessibility_trusted {
                do { try worker.start(); status.stringValue += "；桌面服务就绪" }
                catch { status.stringValue += "；\(error.localizedDescription)" }
            }
        } catch {
            status.stringValue = "检查或保存报告失败：\(error.localizedDescription)"
        }
    }

    @objc private func requestScreen() {
        _ = CGRequestScreenCaptureAccess()
        refresh()
    }

    @objc private func requestAccessibility() {
        _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary)
        refresh()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationWillTerminate(_ notification: Notification) { worker.stop() }
}

let delegate = ProbeApp()
let application = NSApplication.shared
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
