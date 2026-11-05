// Shared action module: parameters for human-like pointer movement (see docs/superpowers/specs/2026-11-05-action-module-design.md).
// Swift 5.7 compatible (macOS 12 target): no `some` parameters, no if-expressions.
import Foundation

struct ActionProfile {
    let name: String
    /// Fitts' law T = a + b·log2(D/W + 1), seconds.
    let fittsA: Double
    let fittsB: Double
    /// Log-normal sigma on the movement time; the factor is clamped to 0.6...1.6.
    let durationJitter: Double
    /// Physiological tremor amplitude (points) and slow drift bound (points).
    let tremorAmplitude: Double
    let driftAmplitude: Double
    /// Primary-movement endpoint error, as a fraction of the distance (plus a 0.5pt floor).
    let scatter: Double
    let maxCorrections: Int
    /// Pause before pressing and button hold time, seconds.
    let dwell: ClosedRange<Double>
    let press: ClosedRange<Double>
    /// Event rate, Hz.
    let eventHz: ClosedRange<Double>
    /// No trajectory at all: jump to the landing point (tests, troubleshooting). Region checks still apply.
    let instant: Bool

    static let standard = ActionProfile(name:"standard", fittsA:0.10, fittsB:0.11, durationJitter:0.15,
        tremorAmplitude:0.8, driftAmplitude:1.5, scatter:0.035, maxCorrections:3,
        dwell:0.05...0.15, press:0.06...0.12, eventHz:60...120, instant:false)
    /// Dense small targets (menu items): steadier hand, tighter landing.
    static let precise = ActionProfile(name:"precise", fittsA:0.12, fittsB:0.13, durationJitter:0.10,
        tremorAmplitude:0.4, driftAmplitude:0.6, scatter:0.02, maxCorrections:3,
        dwell:0.06...0.16, press:0.06...0.12, eventHz:80...120, instant:false)
    static let instant = ActionProfile(name:"instant", fittsA:0, fittsB:0, durationJitter:0,
        tremorAmplitude:0, driftAmplitude:0, scatter:0, maxCorrections:0,
        dwell:0.02...0.02, press:0.05...0.05, eventHz:60...60, instant:true)

    /// Same random draws, no tremor or drift: the ideal path, for tests.
    func withoutNoise() -> ActionProfile {
        ActionProfile(name:name, fittsA:fittsA, fittsB:fittsB, durationJitter:durationJitter, tremorAmplitude:0,
                      driftAmplitude:0, scatter:scatter, maxCorrections:maxCorrections, dwell:dwell, press:press,
                      eventHz:eventHz, instant:instant)
    }

    /// WECHAT_ACTION_PROFILE=instant|precise|standard (default standard).
    static func current() -> ActionProfile {
        switch (ProcessInfo.processInfo.environment["WECHAT_ACTION_PROFILE"] ?? "").lowercased() {
        case "instant": return .instant
        case "precise": return .precise
        default: return .standard
        }
    }
}

/// SplitMix64: small, fast, reproducible for seeded tests.
struct SeededRandom: RandomNumberGenerator {
    private var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt64 {
        state &+= 0x9E3779B97F4A7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58476D1CE4E5B9
        z = (z ^ (z >> 27)) &* 0x94D049BB133111EB
        return z ^ (z >> 31)
    }
}

/// Standard normal sample (Box-Muller).
func gaussian<R: RandomNumberGenerator>(_ rng: inout R) -> Double {
    let u1 = Double.random(in: Double.ulpOfOne..<1, using: &rng)
    let u2 = Double.random(in: 0..<1, using: &rng)
    return (-2*log(u1)).squareRoot() * cos(2*Double.pi*u2)
}
