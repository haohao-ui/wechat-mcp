// WechatLayout.swift
// Representation, validation, and fallback lookup for WeChat UI layouts.
import Foundation


public enum LayoutError: Error, CustomStringConvertible, Equatable {
    case unreadable
    case tooLarge
    case schemaUnsupported
    case missingRegion(String)
    case regionOutOfWindow(String)
    case invalidValue(String)

    public var description: String {
        switch self {
        case .unreadable:
            return "layout_unreadable"
        case .tooLarge:
            return "layout_too_large"
        case .schemaUnsupported:
            return "layout_schema_unsupported"
        case .missingRegion(let name):
            return "layout_missing_region:\(name)"
        case .regionOutOfWindow(let name):
            return "layout_region_out_of_window:\(name)"
        case .invalidValue(let field):
            return "layout_invalid_value:\(field)"
        }
    }
}

public struct LayoutSize: Codable, Equatable {
    public let width: Double
    public let height: Double

    public init(width: Double, height: Double) {
        self.width = width
        self.height = height
    }
}

public struct SessionRowSpec: Codable, Equatable {
    public let height: Double
    public let nameOffset: Double
    public let timeMinXRatio: Double?

    enum CodingKeys: String, CodingKey {
        case height
        case nameOffset = "name_offset"
        case timeMinXRatio = "time_min_x_ratio"
    }

    public init(height: Double, nameOffset: Double, timeMinXRatio: Double? = nil) {
        self.height = height
        self.nameOffset = nameOffset
        self.timeMinXRatio = timeMinXRatio
    }
}

public struct ThemeSpec: Codable, Equatable {
    public let backgroundMax: Int?
    public let backgroundMin: Int?
    public let background: [Int]?
    public let incoming: [Int]?
    public let own: String?

    enum CodingKeys: String, CodingKey {
        case backgroundMax = "background_max"
        case backgroundMin = "background_min"
        case background
        case incoming
        case own
    }

    public init(backgroundMax: Int? = nil, backgroundMin: Int? = nil, background: [Int]? = nil, incoming: [Int]? = nil, own: String? = nil) {
        self.backgroundMax = backgroundMax
        self.backgroundMin = backgroundMin
        self.background = background
        self.incoming = incoming
        self.own = own
    }
}

public struct ScrollSpec: Codable, Equatable {
    public let linesPerTick: Int
    public let pointsPerTick: Double

    enum CodingKeys: String, CodingKey {
        case linesPerTick = "lines_per_tick"
        case pointsPerTick = "points_per_tick"
    }

    public init(linesPerTick: Int, pointsPerTick: Double) {
        self.linesPerTick = linesPerTick
        self.pointsPerTick = pointsPerTick
    }
}

public struct AnchorSpec: Codable, Equatable {
    public let name: String
    public let region: String
    public let edge: String?
    public let kind: String

    public init(name: String, region: String, edge: String? = nil, kind: String) {
        self.name = name
        self.region = region
        self.edge = edge
        self.kind = kind
    }
}

public struct WechatLayout: Decodable, Equatable {
    public let schema: Int
    public let wechatVersion: String
    public let window: LayoutSize
    public let regions: [String: WindowRect]
    public let sessionRow: SessionRowSpec
    public let themes: [String: ThemeSpec]
    public let menuLabels: [String: [String]]
    public let scroll: ScrollSpec
    public let anchors: [AnchorSpec]

    enum CodingKeys: String, CodingKey {
        case schema
        case wechatVersion = "wechat_version"
        case window
        case regions
        case sessionRow = "session_row"
        case themes
        case menuLabels = "menu_labels"
        case scroll
        case anchors
    }

    public init(
        schema: Int,
        wechatVersion: String,
        window: LayoutSize,
        regions: [String: WindowRect],
        sessionRow: SessionRowSpec,
        themes: [String: ThemeSpec],
        menuLabels: [String: [String]],
        scroll: ScrollSpec,
        anchors: [AnchorSpec]
    ) {
        self.schema = schema
        self.wechatVersion = wechatVersion
        self.window = window
        self.regions = regions
        self.sessionRow = sessionRow
        self.themes = themes
        self.menuLabels = menuLabels
        self.scroll = scroll
        self.anchors = anchors
    }

    public static func load(path: String) throws -> WechatLayout {
        let url = URL(fileURLWithPath: path)
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            throw LayoutError.unreadable
        }

        if data.count > 256 * 1024 {
            throw LayoutError.tooLarge
        }

        guard let rawObj = (try? JSONSerialization.jsonObject(with: data, options: [])) as? [String: Any] else {
            throw LayoutError.unreadable
        }

        if let rawSchema = rawObj["schema"] as? Int {
            if rawSchema != 1 {
                throw LayoutError.schemaUnsupported
            }
        } else {
            throw LayoutError.schemaUnsupported
        }

        let decoder = JSONDecoder()
        let layout: WechatLayout
        do {
            layout = try decoder.decode(WechatLayout.self, from: data)
        } catch {
            throw LayoutError.unreadable
        }

        if layout.schema != 1 {
            throw LayoutError.schemaUnsupported
        }

        let requiredRegions = ["session_list", "title", "chat", "input"]
        for name in requiredRegions {
            if layout.regions[name] == nil {
                throw LayoutError.missingRegion(name)
            }
        }

        for (name, rect) in layout.regions {
            if rect.w <= 0 {
                throw LayoutError.invalidValue("\(name).w")
            }
            if rect.h <= 0 {
                throw LayoutError.invalidValue("\(name).h")
            }
            if rect.x < 0 || rect.y < 0 || (rect.x + rect.w) > layout.window.width || (rect.y + rect.h) > layout.window.height {
                throw LayoutError.regionOutOfWindow(name)
            }
        }

        if layout.scroll.pointsPerTick <= 0 {
            throw LayoutError.invalidValue("scroll.points_per_tick")
        }
        if layout.scroll.linesPerTick <= 0 {
            throw LayoutError.invalidValue("scroll.lines_per_tick")
        }

        return layout
    }

    public func region(_ name: String) throws -> WindowRect {
        guard let r = regions[name] else {
            throw LayoutError.missingRegion(name)
        }
        return r
    }

    public static func path(forVersion v: String, directory: String) -> String? {
        let directFile = "wechat-\(v).json"
        let directPath = (directory as NSString).appendingPathComponent(directFile)
        if FileManager.default.fileExists(atPath: directPath) {
            return directPath
        }

        guard let targetSegments = parseVersion(v) else {
            return nil
        }

        guard let files = try? FileManager.default.contentsOfDirectory(atPath: directory) else {
            return nil
        }

        var bestFilename: String? = nil
        var bestVersion: [Int]? = nil

        for file in files {
            guard file.hasPrefix("wechat-") && file.hasSuffix(".json") else {
                continue
            }
            let verStart = file.index(file.startIndex, offsetBy: "wechat-".count)
            let verEnd = file.index(file.endIndex, offsetBy: -".json".count)
            guard verStart < verEnd else { continue }
            let verStr = String(file[verStart..<verEnd])
            guard let segments = parseVersion(verStr) else {
                continue
            }

            if compareVersions(segments, targetSegments) != .orderedDescending {
                if let curBest = bestVersion {
                    if compareVersions(segments, curBest) == .orderedDescending {
                        bestFilename = file
                        bestVersion = segments
                    }
                } else {
                    bestFilename = file
                    bestVersion = segments
                }
            }
        }

        if let filename = bestFilename {
            return (directory as NSString).appendingPathComponent(filename)
        }
        return nil
    }

    private static func parseVersion(_ str: String) -> [Int]? {
        let parts = str.split(separator: ".")
        if parts.isEmpty { return nil }
        var result: [Int] = []
        for p in parts {
            guard let n = Int(p), n >= 0 else { return nil }
            result.append(n)
        }
        return result
    }

    private static func compareVersions(_ a: [Int], _ b: [Int]) -> ComparisonResult {
        let count = max(a.count, b.count)
        for i in 0..<count {
            let va = i < a.count ? a[i] : 0
            let vb = i < b.count ? b[i] : 0
            if va < vb { return .orderedAscending }
            if va > vb { return .orderedDescending }
        }
        return .orderedSame
    }
}
