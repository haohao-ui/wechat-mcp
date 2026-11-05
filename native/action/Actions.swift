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
    init(sink: EventSink = SystemEventSink(), profile: ActionProfile = .current(), seed: UInt64? = nil, screens: [CGRect]? = nil) {
        self.profile = profile
        executor = Executor(sink:sink)
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
    func screen(for rect: CGRect, from start: CGPoint) -> CGRect {
        let frames = displayFrames()
        let centre = CGPoint(x:rect.midX, y:rect.midY)
        return frames.first(where:{ regionContains($0, centre) }) ?? frames.first(where:{ regionContains($0, start) })
            ?? frames.first ?? CGRect(x:0, y:0, width:100000, height:100000)
    }

    private func plan(_ rect: CGRect) -> ClickPlan {
        let start = executor.start()
        return planClick(from:start, rect:rect, screen:screen(for:rect, from:start), profile:profile, rng:&rng)
    }

    /// Moves into the control's effective region and returns where the pointer stopped.
    @discardableResult
    func move(to rect: CGRect) throws -> CGPoint {
        let plan = plan(rect)
        guard let point = plan.press else { throw ActionError.targetNotReached }
        try executor.run(plan.moves)
        return point
    }

    func click(_ rect: CGRect, button: CGMouseButton = .left) throws {
        let plan = plan(rect)
        guard let point = plan.press else { throw ActionError.targetNotReached }
        try executor.run(plan.moves)
        try executor.press(at:point, button:button, dwell:plan.dwell, hold:plan.hold)
    }

    func rightClick(_ rect: CGRect) throws { try click(rect, button:.right) }

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
