import AppKit
import QuartzCore

struct OverlayEvent: Decodable {
    let event: String
    let x: Double?
    let y: Double?
    let width: Double?
    let height: Double?

    func validate() throws {
        guard ["window", "move", "click", "hide", "resume"].contains(event) else { throw Failure.invalid }
        if event != "hide" && event != "resume" {
            guard let x = x, let y = y, x.isFinite, y.isFinite else { throw Failure.invalid }
        }
        if event == "window" {
            guard let w = width, let h = height, w.isFinite, h.isFinite, w > 0, h > 0 else { throw Failure.invalid }
        }
    }
}
enum Failure: Error { case invalid }

// CoreGraphics has a top-left origin on the primary display, AppKit a bottom-left origin.
func appKitPoint(_ point: CGPoint, primaryHeight: CGFloat) -> CGPoint {
    CGPoint(x: point.x, y: primaryHeight - point.y)
}

final class PassivePanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    init(size: NSSize) {
        super.init(contentRect: NSRect(origin: .zero, size: size),
                   styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        sharingType = .none
        isReleasedWhenClosed = false
    }
}

final class PointerView: NSView {
    var pulse: CGFloat = 0
    override func draw(_ dirtyRect: NSRect) {
        NSColor.systemTeal.withAlphaComponent(0.12).setFill()
        NSBezierPath(ovalIn: NSRect(x: 8, y: 8, width: 40, height: 40)).fill()
        NSColor.systemTeal.withAlphaComponent(0.85).setStroke()
        let halo = NSBezierPath(ovalIn: NSRect(x: 8, y: 8, width: 40, height: 40))
        halo.lineWidth = 1.5; halo.stroke()
        if pulse > 0 {
            NSColor.systemOrange.withAlphaComponent(1 - pulse).setStroke()
            let radius = 12 + pulse * 15
            let ring = NSBezierPath(ovalIn: NSRect(x: 28-radius, y: 28-radius, width: radius*2, height: radius*2))
            ring.lineWidth = 3; ring.stroke()
        }
        // Cursor hotspot is the centre of the halo, aligned to the actual CG pointer.
        let cursor = NSBezierPath()
        cursor.move(to: NSPoint(x: 28, y: 28)); cursor.line(to: NSPoint(x: 29, y: 11))
        cursor.line(to: NSPoint(x: 34, y: 16)); cursor.line(to: NSPoint(x: 39, y: 8))
        cursor.line(to: NSPoint(x: 43, y: 10)); cursor.line(to: NSPoint(x: 38, y: 18))
        cursor.line(to: NSPoint(x: 45, y: 19)); cursor.close()
        NSColor.systemTeal.setFill(); cursor.fill()
        NSColor.white.setStroke(); cursor.lineWidth = 1.5; cursor.stroke()
    }
}

final class Overlay {
    let badge = PassivePanel(size: NSSize(width: 188, height: 30))
    let pointer = PassivePanel(size: NSSize(width: 56, height: 56))
    let pointerView = PointerView(frame: NSRect(x: 0, y: 0, width: 56, height: 56))
    let label = NSTextField(labelWithString: "●  微信 RPA · 已定位")
    var expires: TimeInterval = 0
    var clicked: TimeInterval = 0
    var active = false
    var positioned = false
    var timer: Timer?
    let renderEvidence: URL?
    let statusOnly: Bool

    init(renderEvidence: URL? = nil, statusOnly: Bool = false) {
        self.statusOnly = statusOnly
        self.renderEvidence = renderEvidence
        let background = NSView(frame: NSRect(x: 0, y: 0, width: 188, height: 30))
        background.wantsLayer = true
        background.layer?.backgroundColor = NSColor(calibratedRed: 0.06, green: 0.13, blue: 0.17, alpha: 0.95).cgColor
        background.layer?.cornerRadius = 10
        background.layer?.borderWidth = 1
        background.layer?.borderColor = NSColor.systemTeal.withAlphaComponent(0.7).cgColor
        label.frame = NSRect(x: 12, y: 7, width: 168, height: 18)
        label.textColor = .white; label.font = .systemFont(ofSize: 12, weight: .medium)
        background.addSubview(label)
        badge.contentView = background; pointer.contentView = pointerView
        timer = Timer.scheduledTimer(withTimeInterval: statusOnly ? 1 : 1.0/60, repeats: true) { [weak self] _ in self?.tick() }
    }

    var primaryHeight: CGFloat { NSScreen.screens.first?.frame.height ?? 0 }

    func hide() {
        active = false; badge.orderOut(nil); pointer.orderOut(nil); CATransaction.flush()
    }

    func apply(_ event: OverlayEvent) throws {
        try event.validate()
        if !statusOnly && FileManager.default.fileExists(atPath: NSHomeDirectory()+"/.wechat-rpa/bot-worker.enabled") {
            hide(); return
        }
        if statusOnly {
            guard event.event == "window" else { return }
            let topLeft = appKitPoint(CGPoint(x:event.x!, y:event.y!),primaryHeight:primaryHeight)
            let screen = NSScreen.screens.first(where: { $0.frame.contains(topLeft) }) ?? NSScreen.screens.first
            let visible = screen?.visibleFrame ?? NSRect(x:0,y:0,width:1920,height:1080)
            badge.setFrameOrigin(NSPoint(x:min(max(topLeft.x+8,visible.minX),visible.maxX-188),
                                        y:min(max(topLeft.y+4,visible.minY),visible.maxY-30)))
            label.stringValue = "●  微信 Bot · 运行中"
            label.textColor = .systemGreen
            active = true; expires = ProcessInfo.processInfo.systemUptime+240
            if !badge.isVisible { badge.orderFrontRegardless() }
            return
        }
        if event.event == "hide" { hide(); return }
        if event.event == "resume" && !positioned { return }
        let now = ProcessInfo.processInfo.systemUptime
        if event.event == "window" {
            positioned = true
            let topLeft = appKitPoint(CGPoint(x: event.x!, y: event.y!), primaryHeight: primaryHeight)
            let windowFrame = NSRect(x: topLeft.x, y: topLeft.y - event.height!, width: event.width!, height: event.height!)
            let screen = NSScreen.screens.max { left, right in
                let l = left.frame.intersection(windowFrame), r = right.frame.intersection(windowFrame)
                return (l.isNull ? 0 : l.width*l.height) < (r.isNull ? 0 : r.width*r.height)
            }
            let visible = screen?.visibleFrame ?? windowFrame
            // Prefer immediately above the window; clamp when it meets a display edge.
            badge.setFrameOrigin(NSPoint(x: min(max(topLeft.x+8, visible.minX), visible.maxX-188),
                                         y: min(max(topLeft.y+4, visible.minY), visible.maxY-30)))
            label.stringValue = "●  微信 RPA · 已定位"
        } else if event.event != "resume" {
            label.stringValue = event.event == "click" ? "●  微信 RPA · 点击" : "●  微信 RPA · 移动"
            if event.event == "click" {
                clicked = now
                if renderEvidence != nil {
                    DispatchQueue.main.asyncAfter(deadline: .now()+0.12) { [weak self] in
                        self?.saveRenderEvidence(clickTime: now)
                    }
                }
            }
        }
        active = true; expires = now + 2
        badge.orderFrontRegardless(); tick()
    }

    // Window sharing is disabled, so explicit diagnostics render only our own views.
    func saveRenderEvidence(clickTime: TimeInterval) {
        guard let directory = renderEvidence, active, clicked == clickTime, pointerView.pulse > 0 else { return }
        do {
            for (name, view) in [("pointer-click.png", pointerView as NSView), ("badge-click.png", badge.contentView!)] {
                guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { throw Failure.invalid }
                view.cacheDisplay(in: view.bounds, to: bitmap)
                guard let data = bitmap.representation(using: .png, properties: [:]) else { throw Failure.invalid }
                let path = directory.appendingPathComponent(name)
                try data.write(to: path, options: .atomic)
                try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path.path)
            }
            let report: [String:Any] = ["click_pulse":Double(pointerView.pulse), "badge_visible":badge.isVisible,
                "pointer_visible":pointer.isVisible, "badge_ignores_mouse":badge.ignoresMouseEvents,
                "pointer_ignores_mouse":pointer.ignoresMouseEvents,"can_become_key":pointer.canBecomeKey,
                "label":label.stringValue]
            let path = directory.appendingPathComponent("render.json")
            try JSONSerialization.data(withJSONObject:report).write(to:path,options:.atomic)
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path.path)
        } catch { fputs("wechat_overlay: render_evidence_failed\n",stderr) }
    }

    func tick() {
        guard active else { return }
        let now = ProcessInfo.processInfo.systemUptime
        guard now < expires else { hide(); return }
        if statusOnly { return }
        if let point = CGEvent(source: nil)?.location {
            let p = appKitPoint(point, primaryHeight: primaryHeight)
            pointer.setFrameOrigin(NSPoint(x: p.x-28, y: p.y-28))
            pointerView.pulse = clicked > 0 && now-clicked < 0.45 ? CGFloat((now-clicked)/0.45) : 0
            pointerView.needsDisplay = true
            pointer.orderFrontRegardless()
        }
    }
}

if CommandLine.arguments.dropFirst() == ["--self-test"] {
    do {
        guard appKitPoint(CGPoint(x: -500, y: -200), primaryHeight: 1024) == CGPoint(x: -500, y: 1224),
              appKitPoint(CGPoint(x: 100, y: 900), primaryHeight: 1024) == CGPoint(x: 100, y: 124) else { throw Failure.invalid }
        for json in ["{\"event\":\"window\",\"x\":0,\"y\":0,\"width\":0,\"height\":400}", "{\"event\":\"send\"}"] {
            do { try JSONDecoder().decode(OverlayEvent.self, from: Data(json.utf8)).validate(); exit(1) }
            catch Failure.invalid {}
        }
        print("{\"self_test\":\"passed\",\"desktop_accessed\":false}")
        exit(0)
    } catch { exit(1) }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
var renderEvidence: URL?
var arguments = Array(CommandLine.arguments.dropFirst())
let statusOnly = arguments.contains("--status-only")
arguments.removeAll { $0 == "--status-only" }
if !arguments.isEmpty {
    guard arguments.count == 2, arguments[0] == "--render-evidence" else { exit(1) }
    let directory = URL(fileURLWithPath:arguments[1])
    guard let info = try? FileManager.default.attributesOfItem(atPath:directory.path),
          info[.type] as? FileAttributeType == .typeDirectory,
          (info[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
          (info[.posixPermissions] as? NSNumber)?.intValue == 0o700 else { exit(1) }
    renderEvidence = directory
}
let overlay = Overlay(renderEvidence:renderEvidence, statusOnly:statusOnly)
DispatchQueue.global(qos: .userInitiated).async {
    while let line = readLine() {
        guard line.utf8.count <= 4096, let data = line.data(using: .utf8),
              let event = try? JSONDecoder().decode(OverlayEvent.self, from: data) else { exit(1) }
        let completed = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            do {
                try overlay.apply(event)
                print("{\"ok\":true}"); fflush(stdout)
            } catch { exit(1) }
            completed.signal()
        }
        completed.wait()
    }
    DispatchQueue.main.async { overlay.hide(); app.terminate(nil) }
}
app.run()
