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

let args = CommandLine.arguments
var result: Any = ["error":"usage: action_selftest clicks <n> <seed>"]
if args.count == 4, args[1] == "clicks", let n = Int(args[2]), let seed = UInt64(args[3]) {
    result = clickSamples(n, seed)
} else {
    FileHandle.standardError.write("usage: action_selftest clicks <n> <seed>\n".data(using:.utf8)!)
    exit(2)
}
let data = try! JSONSerialization.data(withJSONObject:result, options:[.sortedKeys])
print(String(decoding:data, as:UTF8.self))
