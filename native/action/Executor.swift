// Shared action module: sends planned movements as events, with effective-region and
// "someone else moved the mouse" checks. The sink is replaceable so tests post nothing.
import Foundation
import CoreGraphics

protocol EventSink {
    func cursor() -> CGPoint
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton, clickCount: Int) throws
    func key(_ code: CGKeyCode, down: Bool, modifiers: CGEventFlags) throws
    func scroll(lines: Int32, at point: CGPoint) throws
    /// One key transition; `command` is the Command modifier state carried by the event.
    func key(_ code: CGKeyCode, down: Bool, command: Bool) throws
    func sleep(_ seconds: Double)
}

extension EventSink {
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton, clickCount: Int) throws {
        try post(type, at: point, button: button)
    }
    func key(_ code: CGKeyCode, down: Bool, modifiers: CGEventFlags) throws {
        try key(code, down: down, command: modifiers.contains(.maskCommand))
    }
}

enum ActionError: Error, CustomStringConvertible {
    case targetNotReached, userInputDetected, eventFailed, cancelled
    var description: String {
        switch self {
        case .targetNotReached: return "target_not_reached"
        case .userInputDetected: return "user_input_detected"
        case .eventFailed: return "mouse_event_failed"
        case .cancelled: return "action_cancelled"
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
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton, clickCount: Int) throws {
        guard let event = CGEvent(mouseEventSource:nil, mouseType:type, mouseCursorPosition:point, mouseButton:button) else { throw ActionError.eventFailed }
        event.setIntegerValueField(.mouseEventClickState, value: Int64(clickCount))
        event.post(tap:.cghidEventTap)
    }
    func key(_ code: CGKeyCode, down: Bool, modifiers: CGEventFlags) throws {
        guard let event = CGEvent(keyboardEventSource:nil, virtualKey:code, keyDown:down) else { throw ActionError.eventFailed }
        event.flags = modifiers
        event.post(tap:.cghidEventTap)
    }
    func scroll(lines: Int32, at point: CGPoint) throws {
        guard let wheel = CGEvent(scrollWheelEvent2Source:nil, units:.line, wheelCount:1, wheel1:lines, wheel2:0, wheel3:0) else {
            throw ActionError.eventFailed
        }
        wheel.location = point
        wheel.post(tap:.cghidEventTap)
    }
    func key(_ code: CGKeyCode, down: Bool, command: Bool) throws {
        guard let event = CGEvent(keyboardEventSource:nil, virtualKey:code, keyDown:down) else {
            throw ActionError.eventFailed
        }
        event.flags = command ? .maskCommand : []
        event.post(tap:.cghidEventTap)
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

/// The system reports the pointer a few events late, so "ours" means near the path we just posted.
let recentPathLength = 8

func distanceToSegment(_ p: CGPoint, _ a: CGPoint, _ b: CGPoint) -> Double {
    let dx = Double(b.x-a.x), dy = Double(b.y-a.y)
    let length = dx*dx + dy*dy
    guard length > 1e-12 else { return distance(p, a) }
    let t = min(max((Double(p.x-a.x)*dx + Double(p.y-a.y)*dy)/length, 0), 1)
    return distance(p, CGPoint(x:Double(a.x)+t*dx, y:Double(a.y)+t*dy))
}

final class Executor {
    let sink: EventSink
    let checkAllowed: () throws -> Void
    private var recent: [CGPoint] = []
    private var lastPosted: CGPoint? {
        get { recent.last }
        set { if let point = newValue { remember(point) } else { recent = [] } }
    }

    init(sink: EventSink, checkAllowed: @escaping () throws -> Void = {}) {
        self.sink = sink
        self.checkAllowed = checkAllowed
    }

    private func remember(_ point: CGPoint) {
        recent.append(point)
        if recent.count > recentPathLength { recent.removeFirst(recent.count - recentPathLength) }
    }

    /// Throws userInputDetected when the pointer is off the path we recently posted.
    func checkPointer() throws {
        try checkAllowed()
        guard let last = recent.last else { return }
        let here = sink.cursor()
        var nearest = distance(here, last)
        for index in recent.indices.dropFirst() { nearest = min(nearest, distanceToSegment(here, recent[index-1], recent[index])) }
        if nearest > userInputThreshold { recent = []; throw ActionError.userInputDetected }
    }

    func run(_ moves: [[TrajectoryPoint]], dragging: Bool = false) throws {
        for move in moves {
            var previous = 0.0
            for p in move {
                sink.sleep(p.t - previous); previous = p.t
                try checkPointer()
                try sink.post(dragging ? .leftMouseDragged : .mouseMoved, at:p.point, button:.left)
                lastPosted = p.point
            }
        }
    }

    /// Press and release at `point`. If someone moves the mouse while the button is down, the button is
    /// still released (never left stuck) and then userInputDetected is thrown.
    func press(at point: CGPoint, button: CGMouseButton, dwell: Double, hold: Double, clickCount: Int = 1) throws {
        let down: CGEventType = button == .right ? .rightMouseDown : .leftMouseDown
        let up: CGEventType = button == .right ? .rightMouseUp : .leftMouseUp
        sink.sleep(dwell)
        try checkPointer()
        var released = false
        defer { if !released { try? sink.post(up, at:point, button:button, clickCount:clickCount) } }
        try sink.post(down, at:point, button:button, clickCount:clickCount)
        sink.sleep(hold)
        let moved = distance(sink.cursor(), point) > userInputThreshold
        // Always release before reporting cancellation or focus loss.
        try sink.post(up, at:point, button:button, clickCount:clickCount)
        released = true
        lastPosted = point
        if moved { lastPosted = nil; throw ActionError.userInputDetected }
        try checkAllowed()
    }

    func drag(_ moves: [[TrajectoryPoint]], from point: CGPoint, hold: Double) throws {
        try checkPointer()
        defer { try? sink.post(.leftMouseUp, at:sink.cursor(), button:.left) }
        try sink.post(.leftMouseDown, at:point, button:.left)
        sink.sleep(hold)
        try run(moves, dragging:true)
        try checkPointer()
    }

    func wheel(lines: Int32, at point: CGPoint) throws {
        try checkPointer()
        try sink.scroll(lines:lines, at:point)
    }

    /// Begins an action from wherever the pointer really is. Only movement *during* an action counts as
    /// user input; a pointer moved between actions (by a person or the system) is just the new start.
    func start() -> CGPoint {
        let here = sink.cursor()
        recent = [here]
        return here
    }
}
