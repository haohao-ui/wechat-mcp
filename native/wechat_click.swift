import Cocoa

guard CommandLine.arguments.count >= 3 else {
    exit(1)
}

guard let x = Double(CommandLine.arguments[1]),
      let y = Double(CommandLine.arguments[2]) else {
    exit(1)
}

let pt = CGPoint(x: x, y: y)
let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: pt, mouseButton: .left)
let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: pt, mouseButton: .left)

down?.post(tap: .cghidEventTap)
Thread.sleep(forTimeInterval: 0.06)
up?.post(tap: .cghidEventTap)
