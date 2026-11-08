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
    case missingContacts

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
        case .missingContacts:
            return "layout_missing_contacts"
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

public struct ContactsManagerColumnsSpec: Codable, Equatable {
    public let nameX: Double
    public let remarkX: Double
    public let tagsX: Double
    public let permissionX: Double

    enum CodingKeys: String, CodingKey {
        case nameX = "name_x"
        case remarkX = "remark_x"
        case tagsX = "tags_x"
        case permissionX = "permission_x"
    }

    public init(nameX: Double, remarkX: Double, tagsX: Double, permissionX: Double) {
        self.nameX = nameX
        self.remarkX = remarkX
        self.tagsX = tagsX
        self.permissionX = permissionX
    }
}

public struct ContactsManagerFilterSpec: Codable, Equatable {
    public let x: Double
    /// 左栏「标签」标题的 y；具体标签在它下方，位置由识别结果给出
    public let tagY: Double
    /// 左栏「全部」的 y
    public let allY: Double

    enum CodingKeys: String, CodingKey {
        case x
        case tagY = "tag_y"
        case allY = "all_y"
    }

    public init(x: Double, tagY: Double, allY: Double) {
        self.x = x
        self.tagY = tagY
        self.allY = allY
    }
}

public struct ContactsManagerSpec: Codable, Equatable {
    public let firstRowY: Double
    public let rowHeight: Double
    public let rows: Int
    public let columns: ContactsManagerColumnsSpec
    public let filter: ContactsManagerFilterSpec

    enum CodingKeys: String, CodingKey {
        case firstRowY = "first_row_y"
        case rowHeight = "row_height"
        case rows
        case columns
        case filter
    }

    public init(firstRowY: Double, rowHeight: Double, rows: Int, columns: ContactsManagerColumnsSpec, filter: ContactsManagerFilterSpec) {
        self.firstRowY = firstRowY
        self.rowHeight = rowHeight
        self.rows = rows
        self.columns = columns
        self.filter = filter
    }
}

public struct ContactsLayoutSpec: Codable, Equatable {
    public let navContacts: WindowPoint
    public let navChats: WindowPoint
    public let list: WindowRect
    public let managerButton: WindowPoint
    public let profile: WindowRect
    public let profileForbidden: WindowRect
    public let rowHeight: Double
    public let manager: ContactsManagerSpec

    enum CodingKeys: String, CodingKey {
        case navContacts = "nav_contacts"
        case navChats = "nav_chats"
        case list
        case managerButton = "manager_button"
        case profile
        case profileForbidden = "profile_forbidden"
        case rowHeight = "row_height"
        case manager
    }

    public init(
        navContacts: WindowPoint,
        navChats: WindowPoint,
        list: WindowRect,
        managerButton: WindowPoint,
        profile: WindowRect,
        profileForbidden: WindowRect,
        rowHeight: Double,
        manager: ContactsManagerSpec
    ) {
        self.navContacts = navContacts
        self.navChats = navChats
        self.list = list
        self.managerButton = managerButton
        self.profile = profile
        self.profileForbidden = profileForbidden
        self.rowHeight = rowHeight
        self.manager = manager
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
    public let contacts: ContactsLayoutSpec?

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
        case contacts
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
        anchors: [AnchorSpec],
        contacts: ContactsLayoutSpec? = nil
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
        self.contacts = contacts
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

    public func contactsSpec() throws -> ContactsLayoutSpec {
        guard let spec = contacts else {
            throw LayoutError.missingContacts
        }
        return spec
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

    public func hasRegion(_ name: String) -> Bool {
        return regions[name] != nil
    }
}

/// WeChat's version as last read for the running WeChat process. A long-running worker can fail an
/// individual read (acceptance 2026-11-06: intermittent layout_missing with the map present); the
/// version of a live process cannot change, so the reading is reused until a different or no
/// process is seen. Never invents a version: a new process must be read successfully first.
struct VersionMemo {
    private(set) var pid: Int32?
    private(set) var version: String?

    mutating func resolve(lookup: () -> (pid: Int32, version: String?)?, alive: (Int32) -> Bool) -> String? {
        if let found = lookup() {
            if found.pid != pid { pid = found.pid; version = nil }
            if let read = found.version { version = read }
            return version
        }
        if let known = pid, alive(known) { return version }
        pid = nil; version = nil
        return nil
    }
}

/// The `.app` bundle holding an executable (".../WeChat.app/Contents/MacOS/WeChat" -> ".../WeChat.app").
func appBundlePath(executable: String) -> String? {
    guard let range = executable.range(of: ".app/Contents/MacOS/", options: .backwards) else { return nil }
    return String(executable[..<range.lowerBound]) + ".app"
}

/// CFBundleShortVersionString read straight from the bundle's Info.plist (no LaunchServices cache).
func bundleShortVersion(_ bundlePath: String) -> String? {
    guard let info = NSDictionary(contentsOfFile: bundlePath + "/Contents/Info.plist"),
          let version = info["CFBundleShortVersionString"] as? String, !version.isEmpty else { return nil }
    return version
}

/// A process's executable path via sysctl KERN_PROCARGS2 (layout: Int32 argc, then the exec path).
func executablePath(_ pid: Int32) -> String? {
    var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
    var size = 0
    guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > MemoryLayout<Int32>.size else { return nil }
    var buffer = [UInt8](repeating: 0, count: size)
    guard sysctl(&mib, 3, &buffer, &size, nil, 0) == 0 else { return nil }
    let start = MemoryLayout<Int32>.size
    guard let end = buffer[start..<size].firstIndex(of: 0), end > start else { return nil }
    return String(decoding: buffer[start..<end], as: UTF8.self)
}
