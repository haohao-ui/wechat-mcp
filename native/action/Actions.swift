// Shared action module: the public "hand". Callers pass the control's rectangle (screen points);
// the press lands anywhere inside its effective region, reached along a human-like path.
import Foundation
import CoreGraphics
#if canImport(AppKit)
import AppKit
#endif

final class Actions {
    let profile: ActionProfile
    private let executor: Executor
    private var rng: ActionRandom
    private let screens: [CGRect]?

    /// `screens` overrides the display layout (tests); production reads NSScreen.
    init(sink: EventSink = SystemEventSink(), profile: ActionProfile = .current(), seed: UInt64? = nil, screens: [CGRect]? = nil, checkAllowed: @escaping () throws -> Void = {}) {
        self.profile = profile
        executor = Executor(sink:sink, checkAllowed:checkAllowed)
        rng = ActionRandom(seeded:seed.map { SeededRandom(seed:$0) })
        self.screens = screens
    }

    /// Display frames in global CG coordinates (top-left origin of the main display; others may be negative).
    func displayFrames() -> [CGRect] {
        if let screens = screens { return screens }
        #if canImport(AppKit)
        let frames = NSScreen.screens.map { $0.frame }
        guard let main = frames.first else { return [] }
        return frames.map { CGRect(x:$0.minX, y:main.maxY - $0.maxY, width:$0.width, height:$0.height) }
        #else
        return []
        #endif
    }

    /// The display that holds the target centre, else the start point, else the first display.
    func screen(for rect: CGRect, from start: CGPoint) throws -> CGRect {
        let frames = displayFrames()
        let centre = CGPoint(x:rect.midX, y:rect.midY)
        guard let frame = frames.first(where:{ regionContains($0, centre) }) ?? frames.first(where:{ regionContains($0, start) }) ?? frames.first else { throw ActionError.targetNotReached }
        return frame
    }

    /// `within` also bounds the press (e.g. the session list inside the WeChat window): the effective
    /// region is clipped to it as it is to the screen, so a press never lands outside the container.
    private func plan(_ rect: CGRect, within container: CGRect? = nil) throws -> ClickPlan {
        let start = executor.start()
        return planClick(from:start, rect:rect, screen:try screen(for:rect, from:start), within:container, profile:profile, rng:&rng)
    }

    /// Moves into the control's effective region and returns where the pointer stopped.
    @discardableResult
    func move(to rect: CGRect) throws -> CGPoint {
        let plan = try plan(rect)
        guard let point = plan.press else { throw ActionError.targetNotReached }
        try executor.run(plan.moves)
        return point
    }

    /// Returns where it pressed (a context menu opens there, not at the control's centre).
    @discardableResult
    func click(_ rect: CGRect, button: CGMouseButton = .left, within container: CGRect? = nil) throws -> CGPoint {
        let plan = try plan(rect, within:container)
        guard let point = plan.press else { throw ActionError.targetNotReached }
        try executor.run(plan.moves)
        try executor.press(at:point, button:button, dwell:plan.dwell, hold:plan.hold)
        return point
    }

    @discardableResult
    func rightClick(_ rect: CGRect) throws -> CGPoint { try click(rect, button:.right) }

    /// Key combinations the hand may press. Deliberately small: editing the input box and sending.
    static let keyCombos: [String: (code: CGKeyCode, command: Bool)] = [
        "cmd+v": (9, true), "cmd+a": (0, true), "cmd+c": (8, true), "return": (36, false),
        "delete": (51, false), "escape": (53, false), "cmd+1": (18, true)
    ]

    /// Reuses the same approach and landing point; click counts are native double-click events.
    func doubleClick(_ rect: CGRect, interval: Double, within container: CGRect? = nil) throws {
        guard interval > 0, interval.isFinite else { throw ActionError.eventFailed }
        let plan = try plan(rect, within:container)
        guard let point = plan.press else { throw ActionError.targetNotReached }
        try executor.run(plan.moves)
        try executor.press(at:point, button:.left, dwell:plan.dwell, hold:min(plan.hold,interval))
        executor.sink.sleep(interval)
        try executor.press(at:point, button:.left, dwell:0, hold:plan.hold, clickCount:2)
    }

    /// Dragging uses the existing Fitts/minimum-jerk trajectory under a held mouse button.
    func drag(from source: CGRect, to destination: CGRect, within container: CGRect? = nil) throws {
        let start = try move(to:source)
        let display = try screen(for:destination, from:start)
        let dragBounds = container.map { display.intersection($0) } ?? display
        guard !dragBounds.isNull else { throw ActionError.targetNotReached }
        let plan = planClick(from:start, rect:destination, screen:dragBounds, within:container, profile:profile, rng:&rng)
        guard plan.press != nil else { throw ActionError.targetNotReached }
        try executor.drag(plan.moves, from:start, hold:plan.hold)
    }

    static let shortcutKeys: [String: CGKeyCode] = [
        "a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,"b":11,
        "q":12,"w":13,"e":14,"r":15,"y":16,"t":17,"1":18,"2":19,"3":20,"4":21,
        "6":22,"5":23,"9":25,"7":26,"8":28,"0":29,"o":31,"u":32,"i":34,"p":35,
        "l":37,"j":38,"k":40,"n":45,"m":46,"return":36,"tab":48,"space":49,"delete":51,
        "escape":53,"left":123,"right":124,"down":125,"up":126,"home":115,"end":119,
        "pageup":116,"pagedown":121,"forwarddelete":117
    ]

    static func shortcut(_ combo: String) throws -> (CGKeyCode, [(CGKeyCode, CGEventFlags)]) {
        let parts = combo.lowercased().split(separator:"+", omittingEmptySubsequences:false).map(String.init)
        guard let last = parts.last, let key = shortcutKeys[last] else { throw ActionError.eventFailed }
        let known: [String:(CGKeyCode,CGEventFlags)] = ["cmd":(55,.maskCommand),"shift":(56,.maskShift),"alt":(58,.maskAlternate),"ctrl":(59,.maskControl)]
        var modifiers: [(CGKeyCode,CGEventFlags)] = []
        var seen = Set<String>()
        for part in parts.dropLast() {
            guard let spec = known[part], seen.insert(part).inserted else { throw ActionError.eventFailed }
            modifiers.append(spec)
        }
        return (key, modifiers)
    }

    func key(_ combo: String) throws {
        let (code, modifiers) = try Actions.shortcut(combo)
        let sink = executor.sink
        var held: [CGKeyCode] = []
        var flags = CGEventFlags()
        defer {
            for key in held.reversed() { try? sink.key(key, down:false, modifiers:[]) }
        }
        for (key, flag) in modifiers {
            try executor.checkAllowed()
            flags.insert(flag)
            held.append(key)
            try sink.key(key, down:true, modifiers:flags)
            sink.sleep(Double.random(in:profile.dwell, using:&rng)/2)
        }
        try executor.checkAllowed()
        held.append(code)
        try sink.key(code, down:true, modifiers:flags)
        sink.sleep(Double.random(in:profile.press, using:&rng))
        try sink.key(code, down:false, modifiers:flags)
        held.removeLast()
        for (key, flag) in modifiers.reversed() {
            flags.remove(flag)
            try sink.key(key, down:false, modifiers:flags)
            held.removeLast()
        }
        try executor.checkAllowed()
    }

    /// Wheel ticks over the control. Tick size is the caller's (scroll calibration depends on it);
    /// only the pointer path and the pause between ticks (80–200 ms) are human-like.
    func scroll(in rect: CGRect, lines: Int32, ticks: Int) throws {
        let point = try move(to:rect)
        for index in 0..<max(ticks, 0) {
            if index > 0 { executor.sink.sleep(Double.random(in:0.08...0.2, using:&rng)) }
            try executor.wheel(lines:lines, at:point)
        }
    }
}
