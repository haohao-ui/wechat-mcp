import Foundation
import CoreGraphics

/// All saved rectangles are window-relative; capture pixels are derived per frame.
final class WorkspaceLayout {
    private let overrides: [String: RegionRect]

    init(path: String?) throws {
        guard let path, !path.isEmpty else { overrides = [:]; return }
        struct Profile: Decodable {
            let schema_version: Int
            let regions: [String: RegionRect]
        }
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        guard data.count <= 64 * 1024 else { throw RuntimeFailure("layout_profile_too_large") }
        let profile = try JSONDecoder().decode(Profile.self, from: data)
        guard profile.schema_version == 1,
              Set(profile.regions.keys).isSubset(of: ["sessions", "chat", "title", "search"]) else {
            throw RuntimeFailure("unsupported_layout_profile")
        }
        for region in profile.regions.values { try region.validate() }
        overrides = profile.regions
    }

    func region(_ name: String, window: RuntimeWindow) throws -> RegionRect {
        if let explicit = overrides[name] { return explicit }
        let w = window.width
        let h = window.height
        let rect: RegionRect
        switch name {
        case "sessions": rect = RegionRect(x: 65 / w, y: 60 / h, width: min(250, w - 65) / w, height: (h - 60) / h)
        case "chat": rect = RegionRect(x: 315 / w, y: 60 / h, width: (w - 325) / w, height: (h - 220) / h)
        case "title": rect = RegionRect(x: 250 / w, y: 0, width: (w - 260) / w, height: 50 / h)
        case "search": rect = RegionRect(x: 65 / w, y: 0, width: min(250, w - 65) / w, height: 55 / h)
        default: throw RuntimeFailure("unknown_capture_region")
        }
        try rect.validate()
        return rect
    }
}

func pixelCrop(_ region: RegionRect, width: Int, height: Int) throws -> CGRect {
    try region.validate()
    guard width > 0, height > 0 else { throw RuntimeFailure("invalid_frame_size") }
    let left = floor(region.x * Double(width))
    let top = floor(region.y * Double(height))
    let right = min(Double(width), ceil((region.x + region.width) * Double(width)))
    let bottom = min(Double(height), ceil((region.y + region.height) * Double(height)))
    guard right > left, bottom > top else { throw RuntimeFailure("empty_capture_region") }
    return CGRect(x: left, y: top, width: right - left, height: bottom - top)
}
