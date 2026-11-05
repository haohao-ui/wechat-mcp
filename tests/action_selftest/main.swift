// Self-test driver for native/action: prints JSON statistics for seeded samples. Posts no events.
import Foundation
import CoreGraphics

func rectJSON(_ r: CGRect?) -> Any {
    guard let r = r else { return NSNull() }
    return ["x":Double(r.minX), "y":Double(r.minY), "w":Double(r.width), "h":Double(r.height)]
}

func clickSamples(_ n: Int, _ seed: UInt64) -> [[String:Any]] {
    var rng = SeededRandom(seed:seed)
    let screens = [CGRect(x:0, y:0, width:1440, height:900), CGRect(x:-1920, y:-180, width:1920, height:1080)]
    let kinds = ["normal", "tiny", "half_off_screen", "start_inside", "off_screen"]
    var out: [[String:Any]] = []
    for i in 0..<n {
        let screen = screens[Int.random(in:0..<2, using:&rng)]
        let kind = kinds[i % kinds.count]
        let w = kind == "tiny" ? 4.0 : Double.random(in:20...300, using:&rng)
        let h = kind == "tiny" ? 4.0 : Double.random(in:12...80, using:&rng)
        var x = Double.random(in:Double(screen.minX)...(Double(screen.maxX)-w-1), using:&rng)
        let y = Double.random(in:Double(screen.minY)...(Double(screen.maxY)-h-1), using:&rng)
        if kind == "half_off_screen" { x = Double(screen.maxX) - w/2 }
        if kind == "off_screen" { x = Double(screen.maxX) + 50 }
        let rect = CGRect(x:x, y:y, width:w, height:h)
        var start = CGPoint(x:Double.random(in:Double(screen.minX)..<Double(screen.maxX), using:&rng),
                            y:Double.random(in:Double(screen.minY)..<Double(screen.maxY), using:&rng))
        if kind == "start_inside" {
            let r = effectiveRegion(rect)
            start = CGPoint(x:Double.random(in:Double(r.minX)...Double(r.maxX), using:&rng), y:Double.random(in:Double(r.minY)...Double(r.maxY), using:&rng))
        }
        let profile = i % 7 == 3 ? ActionProfile.precise : ActionProfile.standard
        var noisyRNG = rng, idealRNG = rng
        let plan = planClick(from:start, rect:rect, screen:screen, profile:profile, rng:&noisyRNG)
        let ideal = planClick(from:start, rect:rect, screen:screen, profile:profile.withoutNoise(), rng:&idealRNG)
        rng = noisyRNG
        var sample: [String:Any] = ["kind":kind, "rect":rectJSON(rect), "screen":rectJSON(screen), "region":rectJSON(plan.region),
            "press":plan.press.map { [Double($0.x), Double($0.y)] } ?? NSNull(), "corrections":max(plan.moves.count-1, 0),
            "target_width":w, "target_height":h, "tremor":profile.tremorAmplitude, "drift":profile.driftAmplitude]
        let all = plan.moves.flatMap { $0 }
        sample["finite"] = all.allSatisfy { $0.point.x.isFinite && $0.point.y.isFinite && $0.t.isFinite }
        sample["on_screen"] = all.allSatisfy { $0.point.x >= screen.minX && $0.point.x <= screen.maxX-1 && $0.point.y >= screen.minY && $0.point.y <= screen.maxY-1 }
        if let main = plan.moves.first, let end = main.last {
            let duration = end.t
            sample["duration"] = duration
            sample["distance"] = distance(start, end.point)
            sample["fitts"] = fittsTime(distance(clamp(start, to:screen), end.point), plan.targetWidth, profile)
            var peak = 0.0, peakT = 0.0
            var previous = clamp(start, to:screen), previousT = 0.0
            for p in main {
                let dt = p.t - previousT
                if dt > 0 { let v = distance(previous, p.point)/dt; if v > peak { peak = v; peakT = p.t } }
                previous = p.point; previousT = p.t
            }
            sample["peak_ratio"] = duration > 0 ? peakT/duration : 0
            var maxDev = 0.0, tailDev = 0.0
            for (moveIndex, move) in plan.moves.enumerated() {
                let reference = ideal.moves[moveIndex]
                let total = move.last?.t ?? 0
                for (k, p) in move.enumerated() {
                    let dev = distance(p.point, reference[k].point)
                    maxDev = max(maxDev, dev)
                    if total > 0 && p.t >= 0.9*total { tailDev = max(tailDev, dev) }
                }
            }
            sample["max_deviation"] = maxDev
            sample["tail_deviation"] = tailDev
        } else {
            sample["duration"] = 0; sample["distance"] = 0; sample["fitts"] = 0; sample["peak_ratio"] = 0
            sample["max_deviation"] = 0; sample["tail_deviation"] = 0
        }
        out.append(sample)
    }
    return out
}

/// Simulated pointer and clock; optionally a "person" moves the mouse at a chosen moment.
final class RecordingSink: EventSink {
    var position = CGPoint(x:100, y:100)
    var clock = 0.0
    var events: [[String:Any]] = []
    var interfereAfterMoves: Int? = nil
    var interfereAfterDown = false
    private var moves = 0
    func cursor() -> CGPoint { position }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws {
        let names: [CGEventType:String] = [.mouseMoved:"moved", .leftMouseDown:"left_down", .leftMouseUp:"left_up",
                                           .rightMouseDown:"right_down", .rightMouseUp:"right_up"]
        events.append(["type":names[type] ?? "other", "x":Double(point.x), "y":Double(point.y), "t":clock])
        position = point
        if type == .mouseMoved {
            moves += 1
            if moves == interfereAfterMoves { position.x += 50 }
        }
        if type == .leftMouseDown && interfereAfterDown { position.y += 50 }
    }
    func scroll(lines: Int32, at point: CGPoint) throws {
        events.append(["type":"scroll", "lines":Int(lines), "x":Double(point.x), "y":Double(point.y), "t":clock])
    }
    func sleep(_ seconds: Double) { clock += max(seconds, 0) }
}

func execScenario(_ name: String, _ seed: UInt64) -> [String:Any] {
    let sink = RecordingSink()
    let screens = [CGRect(x:0, y:0, width:1440, height:900)]
    var rect = CGRect(x:500, y:400, width:120, height:40)
    var profile = ActionProfile.standard
    switch name {
    case "unreachable": rect = CGRect(x:2000, y:400, width:50, height:20)
    case "interfere_move": sink.interfereAfterMoves = 3
    case "interfere_down": sink.interfereAfterDown = true
    case "instant": profile = .instant
    case "scroll": rect = CGRect(x:315, y:60, width:565, height:436)
    default: break
    }
    let actions = Actions(sink:sink, profile:profile, seed:seed, screens:screens)
    var failure: Any = NSNull()
    do {
        switch name {
        case "scroll": try actions.scroll(in:rect, lines:10, ticks:3)
        case "right_click": try actions.rightClick(rect)
        default: try actions.click(rect)
        }
    } catch { failure = String(describing:error) }
    return ["events":sink.events, "error":failure, "region":rectJSON(reachableRegion(rect, screen:screens[0]))]
}

let args = CommandLine.arguments
var result: Any = ["error":"usage: action_selftest clicks <n> <seed>"]
if args.count == 4, args[1] == "clicks", let n = Int(args[2]), let seed = UInt64(args[3]) {
    result = clickSamples(n, seed)
} else if args.count == 4, args[1] == "exec", let seed = UInt64(args[3]) {
    result = execScenario(args[2], seed)
} else {
    FileHandle.standardError.write("usage: action_selftest clicks <n> <seed>\n".data(using:.utf8)!)
    exit(2)
}
let data = try! JSONSerialization.data(withJSONObject:result, options:[.sortedKeys])
print(String(decoding:data, as:UTF8.self))
