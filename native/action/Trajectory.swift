// Shared action module: pure trajectory planning. No system access; deterministic for a given RNG.
import Foundation
import CoreGraphics

/// Inclusive containment (CGRect.contains excludes the max edges).
func regionContains(_ rect: CGRect, _ p: CGPoint) -> Bool {
    p.x >= rect.minX - 1e-9 && p.x <= rect.maxX + 1e-9 && p.y >= rect.minY - 1e-9 && p.y <= rect.maxY + 1e-9
}

/// The part of a control that counts as a hit: 20% inset per side, 2pt for small controls,
/// the centre point when the inset leaves nothing.
func effectiveRegion(_ rect: CGRect) -> CGRect {
    let rect = rect.standardized
    let small = rect.width < 16 || rect.height < 16
    let dx = small ? 2 : rect.width*0.2, dy = small ? 2 : rect.height*0.2
    if rect.width - 2*dx <= 0 || rect.height - 2*dy <= 0 {
        return CGRect(x:rect.midX-0.5, y:rect.midY-0.5, width:1, height:1)
    }
    return rect.insetBy(dx:dx, dy:dy)
}

/// Effective region clipped to the screen; nil when nothing of it is on screen.
func reachableRegion(_ rect: CGRect, screen: CGRect) -> CGRect? {
    let region = effectiveRegion(rect)
    let x0 = max(region.minX, screen.minX), x1 = min(region.maxX, screen.maxX - 1)
    let y0 = max(region.minY, screen.minY), y1 = min(region.maxY, screen.maxY - 1)
    guard x0 <= x1, y0 <= y1 else { return nil }
    return CGRect(x:x0, y:y0, width:x1-x0, height:y1-y0)
}

func clamp(_ p: CGPoint, to screen: CGRect) -> CGPoint {
    CGPoint(x:min(max(p.x, screen.minX), screen.maxX - 1), y:min(max(p.y, screen.minY), screen.maxY - 1))
}

func distance(_ a: CGPoint, _ b: CGPoint) -> Double { Double(hypot(b.x-a.x, b.y-a.y)) }

/// Landing point: truncated 2-D Gaussian around the region centre (σ = half-size / 3), shifted a little
/// toward the start (people tend to stop slightly short), always inside the region.
func sampleLanding<R: RandomNumberGenerator>(in region: CGRect, from start: CGPoint, profile: ActionProfile, rng: inout R) -> CGPoint {
    let centre = CGPoint(x:region.midX, y:region.midY)
    let d = distance(start, centre)
    let ux = d > 1e-9 ? Double(start.x-centre.x)/d : 0, uy = d > 1e-9 ? Double(start.y-centre.y)/d : 0
    let sx = Double(region.width)/6, sy = Double(region.height)/6
    let bias = 0.15
    var p = CGPoint(x:Double(centre.x) + (gaussian(&rng) + ux*bias*3)*sx, y:Double(centre.y) + (gaussian(&rng) + uy*bias*3)*sy)
    p.x = min(max(p.x, region.minX), region.maxX)
    p.y = min(max(p.y, region.minY), region.maxY)
    return p
}

struct TrajectoryPoint { let t: Double; let point: CGPoint }

/// Fitts' law time for distance d to a target of width w along the movement.
func fittsTime(_ d: Double, _ w: Double, _ profile: ActionProfile) -> Double {
    profile.fittsA + profile.fittsB*log2(d/max(w, 1) + 1)
}

/// One movement from `from` to `to`: cubic Bézier path bent sideways, minimum-jerk progress with the
/// speed peak moved forward (fast then slow), tremor + drift fading as (1−s)², clamped to the screen.
/// `noiseScale` 0 gives the same path without tremor/drift (same random draws), for tests.
func movement<R: RandomNumberGenerator>(from: CGPoint, to: CGPoint, targetWidth: Double, screen: CGRect,
                                        profile: ActionProfile, rng: inout R, noiseScale: Double = 1) -> [TrajectoryPoint] {
    if profile.instant { return [TrajectoryPoint(t:0, point:clamp(to, to:screen))] }
    let d = distance(from, to)
    let factor = min(max(exp(gaussian(&rng)*profile.durationJitter), 0.6), 1.6)
    let duration = fittsTime(d, targetWidth, profile)*factor
    // Sideways bend: control points offset perpendicular to the line, proportional to distance.
    let nx = d > 1e-9 ? -Double(to.y-from.y)/d : 0, ny = d > 1e-9 ? Double(to.x-from.x)/d : 0
    let bend1 = min(max(gaussian(&rng)*0.08, -0.2), 0.2)*d, bend2 = min(max(gaussian(&rng)*0.08, -0.2), 0.2)*d
    let c1 = CGPoint(x:Double(from.x) + Double(to.x-from.x)*0.3 + nx*bend1, y:Double(from.y) + Double(to.y-from.y)*0.3 + ny*bend1)
    let c2 = CGPoint(x:Double(from.x) + Double(to.x-from.x)*0.7 + nx*bend2, y:Double(from.y) + Double(to.y-from.y)*0.7 + ny*bend2)
    // Tremor: two 8–12 Hz sines per axis, each ≤ amplitude/2; drift: bounded random walk.
    let amp = profile.tremorAmplitude*Double.random(in:0.4...1, using:&rng)
    var waves: [(Double, Double, Double)] = []
    for _ in 0..<4 { waves.append((Double.random(in:8...12, using:&rng), Double.random(in:0..<(2*Double.pi), using:&rng), amp/2)) }
    var drift = (0.0, 0.0)
    var points: [TrajectoryPoint] = []
    var t = 0.0
    while true {
        let hz = Double.random(in:profile.eventHz, using:&rng)
        t = min(duration, t + 1/hz)
        let walk = (gaussian(&rng)*0.3, gaussian(&rng)*0.3)
        drift = (min(max(drift.0 + walk.0, -profile.driftAmplitude), profile.driftAmplitude),
                 min(max(drift.1 + walk.1, -profile.driftAmplitude), profile.driftAmplitude))
        let u = duration > 0 ? pow(t/duration, 0.75) : 1   // time warp: speed peak at ≈40%
        let s = 10*pow(u,3) - 15*pow(u,4) + 6*pow(u,5)       // minimum jerk
        let a = 1-s
        let bx = a*a*a*Double(from.x) + 3*a*a*s*Double(c1.x) + 3*a*s*s*Double(c2.x) + s*s*s*Double(to.x)
        let by = a*a*a*Double(from.y) + 3*a*a*s*Double(c1.y) + 3*a*s*s*Double(c2.y) + s*s*s*Double(to.y)
        let envelope = (1-s)*(1-s)*noiseScale
        let tx = waves[0].2*sin(2*Double.pi*waves[0].0*t + waves[0].1) + waves[1].2*sin(2*Double.pi*waves[1].0*t + waves[1].1)
        let ty = waves[2].2*sin(2*Double.pi*waves[2].0*t + waves[2].1) + waves[3].2*sin(2*Double.pi*waves[3].0*t + waves[3].1)
        let p = CGPoint(x:bx + envelope*(tx + drift.0), y:by + envelope*(ty + drift.1))
        points.append(TrajectoryPoint(t:t, point:clamp(t >= duration ? to : p, to:screen)))
        if t >= duration { break }
    }
    return points
}

struct ClickPlan {
    /// Primary movement followed by corrective movements; the last point of the last move is the press point.
    let moves: [[TrajectoryPoint]]
    /// nil: no part of the target is reachable on this screen (target_not_reached); nothing is pressed.
    let press: CGPoint?
    let dwell: Double
    let hold: Double
    let region: CGRect?
    /// Target width along the primary movement (the W in Fitts' law).
    let targetWidth: Double
}

/// Noisy end-point approach: primary movement lands at the sampled point plus distance-proportional error;
/// outside the effective region → slower corrective movement with 30% of the error; the last allowed
/// correction goes straight to the sampled point, so a reachable target is always pressed inside.
func planClick<R: RandomNumberGenerator>(from start: CGPoint, rect: CGRect, screen: CGRect,
                                         profile: ActionProfile, rng: inout R) -> ClickPlan {
    guard let region = reachableRegion(rect, screen:screen) else {
        return ClickPlan(moves:[], press:nil, dwell:0, hold:0, region:nil, targetWidth:0)
    }
    let start = clamp(start, to:screen)
    let landing = sampleLanding(in:region, from:start, profile:profile, rng:&rng)
    let d = distance(start, landing)
    let along = d > 1e-9 ? (abs(Double(landing.x-start.x))*Double(rect.width) + abs(Double(landing.y-start.y))*Double(rect.height))/d : Double(min(rect.width, rect.height))
    var sigma = profile.instant ? 0 : profile.scatter*d + 0.5
    var moves: [[TrajectoryPoint]] = []
    var current = start
    var attempt = 0
    while true {
        let last = attempt >= profile.maxCorrections
        let error = last ? CGPoint.zero : CGPoint(x:gaussian(&rng)*sigma, y:gaussian(&rng)*sigma)
        let end = clamp(CGPoint(x:landing.x + error.x, y:landing.y + error.y), to:screen)
        // Corrections are slower (smaller effective width), like a careful final adjustment.
        let width = attempt == 0 ? along : max(along/2, 1)
        moves.append(movement(from:current, to:end, targetWidth:width, screen:screen, profile:profile, rng:&rng))
        current = end
        if regionContains(region, end) || last { break }
        // Corrective error shrinks to 30% and never exceeds what this target needs (a third of its short side).
        sigma = min(sigma*0.3, max(Double(min(region.width, region.height))/3, 0.33))
        attempt += 1
    }
    return ClickPlan(moves:moves, press:current,
                     dwell:Double.random(in:profile.dwell, using:&rng), hold:Double.random(in:profile.press, using:&rng),
                     region:region, targetWidth:along)
}
