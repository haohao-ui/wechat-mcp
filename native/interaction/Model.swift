import Foundation
import CoreGraphics

struct InteractionFailure: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}

struct ObservedControl: Codable {
    let id: String
    let source: String
    let role: String
    let label: String
    let value: String?
    let enabled: Bool
    let focused: Bool
    let bounds: WindowRect
    let path: [Int]?
    let actions: [String]
    var states: [String:String]? = nil
    var confidence: Float? = nil
    var identifier: String? = nil
    var visual_fingerprint: String? = nil
}

struct DesktopObservation: Codable {
    let bundle_id: String
    let pid: Int32
    let window_id: UInt32
    let title: String
    let bounds: WindowRect
    let fingerprint: String
    let controls: [ObservedControl]
    let captured_at_ms: Int64
    var screenshot_base64: String?
    let accessibility_available: Bool
    let accessibility_truncated: Bool
    let coverage: String
}

struct DesktopCommand: Decodable {
    let operation: String
    let bundle_id: String?
    let window_id: UInt32?
    let include_screenshot: Bool?
    let observation: DesktopObservation?
    let action: String?
    let target_id: String?
    let destination_id: String?
    let text: String?
    let keys: String?
    let lines: Int32?
    let ticks: Int?
    let expected_text: String?
    let timeout_seconds: Double?
    let permit_path: String?
    let parent_pid: Int32?
}

func usable(_ bounds: WindowRect, within window: WindowRect) -> Bool {
    let values = [bounds.x,bounds.y,bounds.w,bounds.h,window.x,window.y,window.w,window.h]
    return values.allSatisfy { $0.isFinite } && bounds.w > 0 && bounds.h > 0
        && bounds.x >= window.x && bounds.y >= window.y
        && bounds.x+bounds.w <= window.x+window.w && bounds.y+bounds.h <= window.y+window.h
}

/// Rendering hashes remain diagnostic for AX targets: caret/focus animations are not identity changes.
func validateBinding(saved: DesktopObservation, current: DesktopObservation, target: String) throws -> ObservedControl {
    guard saved.bundle_id == current.bundle_id, saved.pid == current.pid,
          saved.window_id == current.window_id, saved.bounds == current.bounds else { throw InteractionFailure("stale_observation") }
    guard let original = saved.controls.first(where: { $0.id == target }) else { throw InteractionFailure("target_unavailable") }
    let candidates = current.controls.filter { control in
        guard control.source == original.source, control.role == original.role else { return false }
        if let identifier = original.identifier, !identifier.isEmpty { return control.identifier == identifier }
        if original.source == "accessibility" { return control.path == original.path }
        return control.label == original.label && control.bounds == original.bounds
    }
    guard candidates.count == 1 else { throw InteractionFailure("target_ambiguous_or_changed") }
    let control = candidates[0]
    guard control.enabled, usable(control.bounds,within:current.bounds), control.bounds == original.bounds,
          control.label == original.label, control.value == original.value,
          control.states == original.states, control.identifier == original.identifier else { throw InteractionFailure("target_changed") }
    if original.source == "ocr" {
        guard let fingerprint = original.visual_fingerprint, !fingerprint.isEmpty,
              control.visual_fingerprint == fingerprint else { throw InteractionFailure("stale_visual_target") }
    }
    return control
}

struct InteractionDeadline {
    private let end: TimeInterval
    private let clock: () -> TimeInterval
    init(seconds: TimeInterval, clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.clock = clock
        end = clock() + seconds
    }
    var remaining: TimeInterval { max(0,end-clock()) }
    func check() throws { guard remaining > 0 else { throw InteractionFailure("deadline_exceeded") } }
}

struct OcclusionCandidate {
    let bounds: WindowRect
    let alpha: Double
    let passive: Bool
}

func blocksAction(_ window: OcclusionCandidate, regions: [WindowRect]) -> Bool {
    guard !window.passive, window.alpha > 0 else { return false }
    let bounds = CGRect(x:window.bounds.x,y:window.bounds.y,width:window.bounds.w,height:window.bounds.h)
    return regions.contains { bounds.intersects(CGRect(x:$0.x,y:$0.y,width:$0.w,height:$0.h)) }
}

func actionResult(observation: DesktopObservation?, verification: String, inputReadback: String,
                  activation: String, observationError: String?) throws -> [String:Any] {
    var result: [String:Any] = ["status":"executed","verification":verification,
        "input_readback":inputReadback,"activation":activation,"business_success":"unknown"]
    if let observation = observation { result["observation"] = try JSONSerialization.jsonObject(with:JSONEncoder().encode(observation)) }
    if let error = observationError { result["observation_error"] = error }
    return result
}

/// Activation's request return value is not proof that either the app or the window is in front.
func awaitActivation(deadline: InteractionDeadline, check: () throws -> Bool, wait: () -> Void) throws {
    while try !check() {
        try deadline.check()
        wait()
    }
}
