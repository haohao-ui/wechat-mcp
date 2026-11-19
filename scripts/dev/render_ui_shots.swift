// render_ui_shots.swift: 离屏渲染 9 个页面的浅色截图（开发工具，不进 App 产物）。
//
// 用途
//   真机上点击侧栏切页会被桌面帮助器的遮挡检查挡住（window_occluded），无法逐页截图核对效果图；
//   本脚本在离屏窗口里按 1180×800 浅色渲染全部页面，供 docs/design/current-pages-v1 与
//   planned-pages-v1 的效果图逐页比对。只读页面状态，不改业务逻辑，不连接真实核心。
//
// 依赖
//   - macOS 12+，本机有 Xcode Command Line Tools（swiftc / AppKit）。
//   - 不依赖真实业务数据、数据库、微信或核心服务：合成 Home 目录 + control 桩命令，
//     所有 control 调用固定失败；缺数据的页面保持如实空态。
//
// 用法（在仓库根一条命令）
//   swiftc -O -target "$(uname -m)-apple-macos12" \
//     $(ls native/bootstrap/*.swift | grep -v '/main.swift$') \
//     native/eyes/Screens.swift scripts/dev/render_ui_shots.swift \
//     -o /tmp/render_ui_shots && \
//   /tmp/render_ui_shots target/ui-fidelity-shots/pages
//
//   参数：<out_dir>  输出目录（缺省 target/ui-fidelity-shots/pages），会创建 <out_dir>/<page-id>.png。
//   页面 id：overview / sessions / contacts / brain / model / mcp / settings / logs / update。
//   每次运行都覆盖同名文件；已有其它目录的截图不受影响。
//
// 说明
//   - 合成 fixture：宿主状态（StatusInput / StatusItem）与 6 条会话样例来自 11-14 设计稿示例数据，
//     仅用于视觉核对，不代表真实账号、会话或统计；应用更新页固定渲染「未配置更新来源」空状态。
//   - 渲染方式：bitmapImageRepForCachingDisplay + cacheDisplay 抓取视图缓存，再等比落成
//     1180×800 逻辑像素的 NSBitmapImageRep；窗口不上屏，逐页 selectPage(id:) 单独渲染。
//   - 语言与外观：强制浅色（.aqua），文案取当前 L10n 默认语言（中文）。

import AppKit
import Foundation

@main
struct RenderUIShots {
    static func main() {
        // 1. 隔离运行环境：先定临时 Home，再 setenv，避免读写真实 ~/.WeChatRPABot 或 ~/.wechat-rpa。
        let fileManager = FileManager.default
        let isolatedHome = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("wechat-mcp-ui-shots-\(ProcessInfo.processInfo.processIdentifier)")
        let outputDirectory = URL(
            fileURLWithPath: CommandLine.arguments.dropFirst().first ?? "target/ui-fidelity-shots/pages"
        )
        do {
            try fileManager.createDirectory(at: isolatedHome.appendingPathComponent("bin"), withIntermediateDirectories: true)
            try fileManager.createDirectory(at: outputDirectory, withIntermediateDirectories: true)
        } catch {
            fputs("render_ui_shots: 无法创建目录: \(error.localizedDescription)\n", stderr)
            exit(1)
        }
        defer { try? fileManager.removeItem(at: isolatedHome) }

        // control 桩：固定返回 ok=false，页面按「无数据」如实降级，不会连到真实核心。
        let stubPath = isolatedHome.appendingPathComponent("bin/control-stub").path
        let stub = "#!/bin/sh\nprintf '%s\\n' '{\"ok\":false,\"error\":{\"code\":\"render_only\",\"message\":\"offscreen render: no business data\"}}'\n"
        do {
            try stub.write(toFile: stubPath, atomically: true, encoding: .utf8)
            try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: stubPath)
        } catch {
            fputs("render_ui_shots: 无法写入 control 桩: \(error.localizedDescription)\n", stderr)
            exit(1)
        }
        setenv("CFFIXED_USER_HOME", isolatedHome.path, 1)
        setenv("WECHAT_MCP_BIN", stubPath, 1)

        // 2. 应用与离屏窗口（强制浅色）。
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        app.appearance = NSAppearance(named: .aqua)

        let context = StatusPanelController(worker: SocketWorker(), coreSupervisor: CoreSupervisor())
        // control 固定失败，避免离屏渲染时真的去问核心/更新服务。
        let failingRun: (String, [String], Double) -> ControlRun = { _, _, _ in
            ControlRun(status: 1, output: Data(), timedOut: false)
        }
        let brainClient = BrainClient(
            configPath: isolatedHome.appendingPathComponent("config.json").path,
            executor: failingRun
        )
        let updateClient = UpdateClient(
            configPath: isolatedHome.appendingPathComponent("config.json").path,
            executor: failingRun
        )

        let shell = ShellViewController()
        let settingsPage = SettingsPageController(context: context)
        settingsPage.onNavigateToUpdate = { [weak shell] in shell?.selectPage(id: "update") }
        let updatePage = UpdatePageController(context: context, client: updateClient)
        let pages: [(id: String, controller: NSViewController)] = [
            ("overview", OverviewPageController(context: context)),
            ("sessions", SessionsPageController(context: context)),
            ("contacts", ContactsPageController(context: context)),
            ("brain", BrainPageController(context: context, client: brainClient)),
            ("model", ModelPageController(context: context)),
            ("mcp", McpPageController(context: context)),
            ("settings", settingsPage),
            ("logs", LogsPageController(context: context)),
            ("update", updatePage)
        ]
        for page in pages {
            shell.register(page: page.controller, id: page.id, title: pageTitle(for: page.id))
        }

        let size = NSSize(width: 1180, height: 800)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .aqua)
        window.contentViewController = shell
        window.setContentSize(size)
        shell.view.frame = NSRect(origin: .zero, size: size)

        let fixture = makeStatusFixture()
        var rendered: [String] = []
        for page in pages {
            // 3. 逐页：切页 → 注入合成宿主状态 → 等异步布局稳定 → 离屏快照。
            shell.selectPage(id: page.id)
            NotificationCenter.default.post(
                name: hostStatusDidRefreshNotification,
                object: context,
                userInfo: ["input": fixture.input, "items": fixture.items]
            )
            if page.id == "update" {
                // 合成空状态：未配置更新来源、未检查，不伪造版本或进度。
                updatePage.applyStatusPayload(UpdateStatusPayload(
                    currentRelease: UpdateReleaseInfo(architecture: "unknown"),
                    candidateRelease: nil,
                    transaction: UpdateTransactionInfo(status: .idle),
                    guardInfo: UpdateGuardInfo(highestAcceptedSequence: nil, maintenanceActive: false)
                ))
                updatePage.applyCheckResult(UpdateCheckResult(status: .unconfigured))
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
            shell.view.layoutSubtreeIfNeeded()
            window.display()

            let url = outputDirectory.appendingPathComponent("\(page.id).png")
            guard writeSnapshot(of: shell.view, size: size, to: url) else {
                fputs("render_ui_shots: 渲染失败 \(page.id)\n", stderr)
                exit(1)
            }
            let pixels = readPixelSize(of: url)
            print("rendered \(url.path) (\(pixels))")
            rendered.append(page.id)
        }
        print("done: \(rendered.count) pages -> \(outputDirectory.path)")
    }

    // MARK: - 页面标题（与正式 App 的侧栏一致，为离屏壳提供同一套标题）
    private static func pageTitle(for id: String) -> String {
        switch id {
        case "overview": return L10n.t(.navOverview)
        case "sessions": return L10n.t(.navSessions)
        case "contacts": return L10n.t(.navContacts)
        case "brain": return L10n.t(.termPersona)
        case "model": return L10n.t(.navModel)
        case "mcp": return L10n.t(.navMcp)
        case "settings": return L10n.t(.navSettings)
        case "logs": return L10n.t(.navLogs)
        case "update": return L10n.t(.navUpdate)
        default: return id
        }
    }

    // MARK: - 合成 fixture（11-14 设计稿示例数据；不代表真实业务状态）
    private static func makeStatusFixture() -> (input: StatusInput, items: [StatusItem]) {
        let input = StatusInput(
            permissions: PermissionsInput(screenRecordingAuthorized: false, accessibilityTrusted: true),
            desktopService: DesktopServiceInput(socketExists: true, status: "running", pid: 4312),
            bot: BotInput(launchdLoaded: true, healthState: "watching",
                          now: Date().timeIntervalSince1970),
            sessions: [
                "阿林": SessionHealthInput(state: "watching"),
                "客服示例群": SessionHealthInput(state: "listening"),
                "小陈": SessionHealthInput(state: "handoff", reason: "等待人工处理", handoff: true),
                "小周": SessionHealthInput(state: "needs_recalibration", reason: "等待环境恢复"),
                "张女士": SessionHealthInput(state: "watching"),
                "王先生": SessionHealthInput(state: "handoff", reason: "回复超时", handoff: true)
            ],
            wechat: WechatInput(isRunning: true),
            layout: LayoutInput(layoutExists: true),
            disk: DiskInput(freeBytes: 50_000_000_000)
        )
        let items = [
            StatusItem(key: "permissions", level: .yellow,
                       reason: "未授权，请在系统设置中授权",
                       fix: "授予屏幕录制和辅助功能权限，以便正常读取微信界面和进行自动操作。"),
            StatusItem(key: "desktop_service", level: .green, reason: "正常"),
            StatusItem(key: "bot", level: .green, reason: "正常"),
            StatusItem(key: "sessions", level: .green, reason: "3 个监听会话",
                       fix: "已在监听 3 个微信会话，可正常接收并处理消息。"),
            StatusItem(key: "wechat", level: .yellow, reason: "窗口位置需要调整",
                       fix: "将微信窗口放回标准位置（左上角），以确保界面识别的稳定性。"),
            StatusItem(key: "layout", level: .green, reason: "已识别当前布局"),
            StatusItem(key: "disk", level: .green, reason: "可用空间充足"),
            StatusItem(key: "brain", level: .green, reason: "内置大脑已配置")
        ]
        return (input, items)
    }

    // MARK: - 离屏快照
    private static func writeSnapshot(of view: NSView, size: NSSize, to url: URL) -> Bool {
        // 先用视图自身的缓存显示抓取（Retina 下会是 2x），再等比落成目标 1180×800 逻辑像素。
        guard let cached = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return false }
        view.cacheDisplay(in: view.bounds, to: cached)

        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: Int(size.width),
            pixelsHigh: Int(size.height),
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: 0,
            bitsPerPixel: 0
        ) else { return false }
        rep.size = size
        NSGraphicsContext.saveGraphicsState()
        guard let graphicsContext = NSGraphicsContext(bitmapImageRep: rep) else {
            NSGraphicsContext.restoreGraphicsState()
            return false
        }
        NSGraphicsContext.current = graphicsContext
        NSColor.windowBackgroundColor.setFill()
        NSRect(origin: .zero, size: size).fill()
        cached.draw(in: NSRect(origin: .zero, size: size))
        guard let png = rep.representation(using: .png, properties: [:]) else { return false }
        do {
            try png.write(to: url, options: .atomic)
            return true
        } catch {
            return false
        }
    }

    private static func readPixelSize(of url: URL) -> String {
        guard let rep = NSBitmapImageRep(data: (try? Data(contentsOf: url)) ?? Data()) else {
            return "size unknown"
        }
        return "\(rep.pixelsWide)x\(rep.pixelsHigh)"
    }
}
