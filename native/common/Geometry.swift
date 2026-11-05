import Foundation
import CoreGraphics

public struct WindowPoint: Equatable, Codable {
    public var x: Double
    public var y: Double

    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

public struct WindowRect: Codable, Equatable {
    public var x: Double
    public var y: Double
    public var w: Double
    public var h: Double

    public init(x: Double, y: Double, w: Double, h: Double) {
        self.x = x
        self.y = y
        self.w = w
        self.h = h
    }

    public func contains(_ p: WindowPoint) -> Bool {
        return p.x >= x && p.x <= x + w && p.y >= y && p.y <= y + h
    }
}

public struct ScreenPoint: Equatable, Codable {
    public var x: Double
    public var y: Double

    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

public struct ImagePixel: Equatable, Codable {
    public var x: Int
    public var y: Int

    public init(x: Int, y: Int) {
        self.x = x
        self.y = y
    }
}

public struct WindowFrame: Equatable {
    public let origin: ScreenPoint
    public let width: Double
    public let height: Double
    public let scale: Int

    public init?(origin: ScreenPoint, width: Double, height: Double, scale: Int) {
        if scale != 1 && scale != 2 {
            return nil
        }
        self.origin = origin
        self.width = width
        self.height = height
        self.scale = scale
    }

    public func toScreen(_ p: WindowPoint) -> ScreenPoint {
        return ScreenPoint(x: origin.x + p.x, y: origin.y + p.y)
    }

    public func toScreen(_ r: WindowRect) -> CGRect {
        return CGRect(x: origin.x + r.x, y: origin.y + r.y, width: r.w, height: r.h)
    }

    public func toWindow(_ p: ScreenPoint) -> WindowPoint {
        return WindowPoint(x: p.x - origin.x, y: p.y - origin.y)
    }

    public func toImage(_ r: WindowRect) -> CGRect {
        let s = Double(scale)
        return CGRect(x: r.x * s, y: r.y * s, width: r.w * s, height: r.h * s)
    }
}

public func displays() -> [CGRect] {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else {
        return []
    }
    var displayIDs = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &displayIDs, &count) == .success else {
        return []
    }
    var rects: [CGRect] = []
    for i in 0..<Int(count) {
        rects.append(CGDisplayBounds(displayIDs[i]))
    }
    return rects
}

public func display(containing p: ScreenPoint, in displays: [CGRect]) -> CGRect? {
    for rect in displays {
        // Half-open, so a point on an edge two displays share belongs to exactly one of them.
        if p.x >= rect.minX && p.x < rect.maxX && p.y >= rect.minY && p.y < rect.maxY {
            return rect
        }
    }
    return nil
}
