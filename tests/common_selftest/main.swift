// Self-test driver for native/common Geometry: prints JSON. Posts no events.
import Foundation
import CoreGraphics

func rectJSON(_ r: CGRect?) -> Any {
    guard let r = r else { return NSNull() }
    return ["x": Double(r.minX), "y": Double(r.minY), "w": Double(r.width), "h": Double(r.height)]
}

func runGeometryTest(_ caseName: String) -> Any {
    switch caseName {
    case "window_screen_round_trip":
        guard let frame = WindowFrame(origin: ScreenPoint(x: 100, y: 50), width: 880, height: 640, scale: 1) else {
            return ["error": "failed to create frame"]
        }
        let pt = WindowPoint(x: 315, y: 60)
        let screenPt = frame.toScreen(pt)
        let backPt = frame.toWindow(screenPt)
        return [
            "screen": ["x": screenPt.x, "y": screenPt.y],
            "round_trip": ["x": backPt.x, "y": backPt.y]
        ]

    case "negative_display_origin":
        guard let frame = WindowFrame(origin: ScreenPoint(x: -1800, y: -120), width: 880, height: 640, scale: 1) else {
            return ["error": "failed to create frame"]
        }
        let pt = WindowPoint(x: 100, y: 20)
        let screenPt = frame.toScreen(pt)
        let backPt = frame.toWindow(screenPt)
        let screens = [
            CGRect(x: 0, y: 0, width: 1440, height: 900),
            CGRect(x: -1920, y: -180, width: 1920, height: 1080)
        ]
        let target = ScreenPoint(x: -1700, y: -100)
        let matched = display(containing: target, in: screens)
        return [
            "screen_point": ["x": screenPt.x, "y": screenPt.y],
            "round_trip": ["x": backPt.x, "y": backPt.y],
            "matched_display": rectJSON(matched)
        ]

    case "retina_crop_matches_points":
        let chat = WindowRect(x: 315, y: 60, w: 565, h: 436)
        guard let frame2 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 2),
              let frame1 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 1) else {
            return ["error": "failed to create frame"]
        }
        let crop2 = frame2.toImage(chat)
        let crop1 = frame1.toImage(chat)
        let screenRect = frame2.toScreen(chat)
        return [
            "scale2_crop": ["x": crop2.minX, "y": crop2.minY, "w": crop2.width, "h": crop2.height],
            "scale1_crop": ["x": crop1.minX, "y": crop1.minY, "w": crop1.width, "h": crop1.height],
            "screen_rect": ["x": screenRect.minX, "y": screenRect.minY, "w": screenRect.width, "h": screenRect.height]
        ]

    case "rejects_bad_scale":
        let s0 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 0)
        let s3 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 3)
        let sNeg = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: -1)
        let s1 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 1)
        let s2 = WindowFrame(origin: ScreenPoint(x: 0, y: 0), width: 880, height: 640, scale: 2)
        return [
            "scale0_is_nil": s0 == nil,
            "scale3_is_nil": s3 == nil,
            "scale_neg_is_nil": sNeg == nil,
            "scale1_is_not_nil": s1 != nil,
            "scale2_is_not_nil": s2 != nil
        ]

    case "rect_contains":
        let rect = WindowRect(x: 10, y: 20, w: 100, h: 50)
        let inside = rect.contains(WindowPoint(x: 50, y: 30))
        let onEdge = rect.contains(WindowPoint(x: 10, y: 20))
        let outside = rect.contains(WindowPoint(x: 111, y: 30))
        return [
            "inside": inside,
            "on_edge": onEdge,
            "outside": outside
        ]

    case "displays":
        let activeDisplays = displays()
        return [
            "count": activeDisplays.count,
            "displays": activeDisplays.map { rectJSON($0) }
        ]

    default:
        return ["error": "unknown case: \(caseName)"]
    }
}

let args = CommandLine.arguments
if args.count < 3 || args[1] != "geometry" {
    FileHandle.standardError.write("usage: common_selftest geometry <case>\n".data(using: .utf8)!)
    exit(2)
}

let result = runGeometryTest(args[2])
let data = try! JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
print(String(decoding: data, as: UTF8.self))
