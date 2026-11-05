// Shared action module: sends planned movements as events, with effective-region and
// "someone else moved the mouse" checks. The sink is replaceable so tests post nothing.
import Foundation
import CoreGraphics

protocol EventSink {
    func cursor() -> CGPoint
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws
    func scroll(lines: Int32, at point: CGPoint) throws
    func sleep(_ seconds: Double)
}

enum ActionError: Error, CustomStringConvertible {
    case targetNotReached, userInputDetected, eventFailed
    var description: String {
        switch self {
        case .targetNotReached: return "target_not_reached"
        case .userInputDetected: return "user_input_detected"
        case .eventFailed: return "mouse_event_failed"
        }
    }
}

/// Real events to the HID tap.
struct SystemEventSink: EventSink {
    func cursor() -> CGPoint { CGEvent(source:nil)?.location ?? .zero }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws {
        guard let event = CGEvent(mouseEventSource:nil, mouseType:type, mouseCursorPosition:point, mouseButton:button) else {
            throw ActionError.eventFailed
        }
        event.post(tap:.cghidEventTap)
    }
    func scroll(lines: Int32, at point: CGPoint) throws {
        guard let wheel = CGEvent(scrollWheelEvent2Source:nil, units:.line, wheelCount:1, wheel1:lines, wheel2:0, wheel3:0) else {
            throw ActionError.eventFailed
        }
        wheel.location = point
        wheel.post(tap:.cghidEventTap)
    }
    func sleep(_ seconds: Double) { if seconds > 0 { Thread.sleep(forTimeInterval:seconds) } }
}

/// Seeded in tests, system randomness in production.
struct ActionRandom: RandomNumberGenerator {
    var seeded: SeededRandom?
    var system = SystemRandomNumberGenerator()
    mutating func next() -> UInt64 {
        if seeded != nil { return seeded!.next() }
        return system.next()
    }
}

/// A pointer more than this far from where we last put it means a person is using the mouse.
let userInputThreshold: Double = 3

final class Executor {
    let sink: EventSink
    private var lastPosted: CGPoint?

    init(sink: EventSink) { self.sink = sink }

    /// Throws userInputDetected when the pointer is not where we left it.
    func checkPointer() throws {
        guard let expected = lastPosted else { return }
        if distance(sink.cursor(), expected) > userInputThreshold { lastPosted = nil; throw ActionError.userInputDetected }
    }

    func run(_ moves: [[TrajectoryPoint]]) throws {
        for move in moves {
            var previous = 0.0
            for p in move {
                sink.sleep(p.t - previous); previous = p.t
                try checkPointer()
                try sink.post(.mouseMoved, at:p.point, button:.left)
                lastPosted = p.point
            }
        }
    }

    /// Press and release at `point`. If someone moves the mouse while the button is down, the button is
    /// still released (never left stuck) and then userInputDetected is thrown.
    func press(at point: CGPoint, button: CGMouseButton, dwell: Double, hold: Double) throws {
        let down: CGEventType = button == .right ? .rightMouseDown : .leftMouseDown
        let up: CGEventType = button == .right ? .rightMouseUp : .leftMouseUp
        sink.sleep(dwell)
        try checkPointer()
        try sink.post(down, at:point, button:button)
        sink.sleep(hold)
        let moved = distance(sink.cursor(), point) > userInputThreshold
        try sink.post(up, at:point, button:button)
        lastPosted = point
        if moved { lastPosted = nil; throw ActionError.userInputDetected }
    }

    func wheel(lines: Int32, at point: CGPoint) throws {
        try checkPointer()
        try sink.scroll(lines:lines, at:point)
    }

    /// Where the pointer starts the next movement: our last position, or the real one at first use.
    func start() -> CGPoint { lastPosted ?? sink.cursor() }
}
