import Foundation

let runtimeProtocolVersion = 1
let runtimeFrameLimit = 4 * 1024 * 1024

struct RuntimeFailure: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}

struct RegionRect: Codable, Equatable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    func validate() throws {
        guard [x, y, width, height].allSatisfy({ $0.isFinite }),
              x >= 0, y >= 0, width > 0, height > 0,
              x + width <= 1.000001, y + height <= 1.000001 else {
            throw RuntimeFailure("invalid_region")
        }
    }
}

struct RuntimeText: Codable {
    let text: String
    let bounds: RegionRect
    let confidence: Double
}

struct RuntimeWindow: Codable, Equatable {
    let window_id: UInt32
    let pid: Int32
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let on_screen: Bool
}

struct RuntimeSnapshot: Codable {
    let epoch: String
    let sequence: UInt64
    let window: RuntimeWindow
    let region: RegionRect
    let digest: String
    let captured_at_ms: UInt64
    let ocr_cached: Bool
    let lines: [RuntimeText]
    let title_lines: [RuntimeText]
}

func jsonObject<T: Encodable>(_ value: T) throws -> Any {
    try JSONSerialization.jsonObject(with: JSONEncoder().encode(value))
}

func writeResponse(id: UInt64, result: Any? = nil, error: String? = nil) throws {
    var envelope: [String: Any] = ["protocol_version": runtimeProtocolVersion, "id": id, "ok": error == nil]
    if let result { envelope["result"] = result }
    if let error { envelope["error"] = error }
    var data = try JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])
    guard data.count < runtimeFrameLimit else { throw RuntimeFailure("response_too_large") }
    data.append(10)
    try FileHandle.standardOutput.write(contentsOf: data)
}
