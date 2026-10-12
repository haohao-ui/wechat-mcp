import Foundation
import CoreGraphics

struct WindowRect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    init(_ bounds: CGRect) {
        x = bounds.origin.x
        y = bounds.origin.y
        width = bounds.width
        height = bounds.height
    }
}

struct WechatWindow: Codable {
    let window_id: UInt32
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let on_screen: Bool
}

func output<T: Encodable>(_ value: T) throws {
    let data = try JSONEncoder().encode(value)
    print(String(decoding: data, as: UTF8.self))
}

do {
    if CommandLine.arguments.dropFirst() == ["--screens"] {
        var count: UInt32 = 0
        guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else {
            throw NSError(domain: "无法获取屏幕列表", code: 1)
        }
        var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
        guard CGGetActiveDisplayList(count, &displays, &count) == .success else {
            throw NSError(domain: "无法获取屏幕范围", code: 1)
        }
        try output(displays.prefix(Int(count)).map { WindowRect(CGDisplayBounds($0)) })
    } else {
        guard CommandLine.arguments.count == 1 else {
            throw NSError(domain: "用法: wechat_window [--screens]", code: 1)
        }
        guard let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
            throw NSError(domain: "无法查询微信窗口，请检查系统设置中的屏幕录制权限", code: 1)
        }
        let candidates: [WechatWindow] = windows.compactMap { info in
            guard let owner = info[kCGWindowOwnerName as String] as? String,
                  owner == "WeChat" || owner == "微信",
                  let layer = info[kCGWindowLayer as String] as? Int, layer == 0,
                  let id = info[kCGWindowNumber as String] as? UInt32,
                  let boundsInfo = info[kCGWindowBounds as String] as? [String: Any],
                  let bounds = CGRect(dictionaryRepresentation: boundsInfo as CFDictionary),
                  bounds.width > 0, bounds.height > 0 else { return nil }
            return WechatWindow(window_id: id, x: bounds.origin.x, y: bounds.origin.y,
                                width: bounds.width, height: bounds.height,
                                on_screen: info[kCGWindowIsOnscreen as String] as? Bool ?? false)
        }
        guard let main = candidates.max(by: { $0.width * $0.height < $1.width * $1.height }) else {
            throw NSError(domain: "未找到微信主窗口，请先启动并登录微信；截图需要屏幕录制权限", code: 1)
        }
        try output(main)
    }
} catch {
    fputs("微信窗口查询失败: \(error)\n", stderr)
    exit(1)
}
