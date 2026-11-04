import Foundation
import CoreGraphics
import Darwin

func startBotWorkerIfRequested() throws {
    let root=FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa")
    let flag=root.appendingPathComponent("bot-worker.enabled")
    guard FileManager.default.fileExists(atPath:flag.path) else { return }
    for path in [root,flag] {
        let info=try FileManager.default.attributesOfItem(atPath:path.path)
        guard (info[.ownerAccountID] as? NSNumber)?.uint32Value==getuid(),
              ((info[.posixPermissions] as? NSNumber)?.intValue ?? 0o777)&0o077==0,
              info[.type] as? FileAttributeType == (path==root ? .typeDirectory : .typeRegular) else {
            throw NSError(domain:"Bot私有配置权限不安全",code:1)
        }
    }
    let fd=open(root.appendingPathComponent("bot-worker.lock").path,O_CREAT|O_RDWR,0o600)
    guard fd>=0 else { throw NSError(domain:"Bot进程锁不可用",code:1) }
    defer { close(fd) }
    if flock(fd,LOCK_EX|LOCK_NB) != 0 {
        guard errno==EWOULDBLOCK else { throw NSError(domain:"Bot进程锁失败",code:1) }
        return
    }
    flock(fd,LOCK_UN)
    let executable=URL(fileURLWithPath:CommandLine.arguments[0]).deletingLastPathComponent().appendingPathComponent("wechat_bot_worker")
    let info=try FileManager.default.attributesOfItem(atPath:executable.path)
    guard info[.type] as? FileAttributeType == .typeRegular,
          (info[.ownerAccountID] as? NSNumber)?.uint32Value==getuid(),
          ((info[.posixPermissions] as? NSNumber)?.intValue ?? 0o777)&0o022==0 else { throw NSError(domain:"Bot程序权限不安全",code:1) }
    let child=Process()
    child.executableURL=executable
    child.standardInput=FileHandle.nullDevice;child.standardOutput=FileHandle.nullDevice
    // A long-lived child must not keep the caller's captured stderr pipe open.
    let log=root.appendingPathComponent("bot-worker.log")
    if !FileManager.default.fileExists(atPath:log.path) {
        FileManager.default.createFile(atPath:log.path,contents:nil,attributes:[.posixPermissions:0o600])
    }
    let logInfo=try FileManager.default.attributesOfItem(atPath:log.path)
    guard logInfo[.type] as? FileAttributeType == .typeRegular,
          (logInfo[.ownerAccountID] as? NSNumber)?.uint32Value==getuid(),
          (logInfo[.posixPermissions] as? NSNumber)?.intValue==0o600 else { throw NSError(domain:"Bot日志权限不安全",code:1) }
    let errors=try FileHandle(forWritingTo:log)
    defer { errors.closeFile() }
    errors.seekToEndOfFile()
    child.standardError=errors
    try child.run()
}

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

func preferredWindow(_ windows: [WechatWindow]) -> WechatWindow? {
    windows.sorted {
        if $0.on_screen != $1.on_screen { return $0.on_screen }
        let left = $0.width * $0.height, right = $1.width * $1.height
        return left == right ? $0.window_id < $1.window_id : left > right
    }.first
}

func output<T: Encodable>(_ value: T) throws {
    let data = try JSONEncoder().encode(value)
    print(String(decoding: data, as: UTF8.self))
}

do {
    if CommandLine.arguments.dropFirst() == ["--self-test"] {
        let visible = WechatWindow(window_id: 10, x: 0, y: 0, width: 600, height: 400, on_screen: true)
        let hidden = WechatWindow(window_id: 20, x: 0, y: 0, width: 880, height: 640, on_screen: false)
        let tied = WechatWindow(window_id: 30, x: 0, y: 0, width: 600, height: 400, on_screen: true)
        guard preferredWindow([hidden, visible])?.window_id == 10,
              preferredWindow([tied, visible, hidden])?.window_id == 10,
              preferredWindow([hidden])?.on_screen == false,
              preferredWindow([]) == nil else {
            throw NSError(domain: "窗口选择自检失败", code: 1)
        }
        print("{\"self_test\":\"passed\",\"desktop_accessed\":false}")
    } else if CommandLine.arguments.dropFirst() == ["--screens"] {
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
        guard let main = preferredWindow(candidates) else {
            throw NSError(domain: "未找到微信主窗口，请先启动并登录微信；截图需要屏幕录制权限", code: 1)
        }
        try startBotWorkerIfRequested()
        let diagnostics = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/diagnostics")
        if FileManager.default.fileExists(atPath: diagnostics.appendingPathComponent("capture.enabled").path) {
            let attributes = try FileManager.default.attributesOfItem(atPath: diagnostics.path)
            guard (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
                  ((attributes[.posixPermissions] as? NSNumber)?.intValue ?? 0o777) & 0o077 == 0 else {
                throw NSError(domain: "窗口诊断目录权限不安全", code: 1)
            }
            let capture = Process()
            capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            let image = diagnostics.appendingPathComponent("window.png")
            capture.arguments = ["-l", String(main.window_id), "-o", "-x", image.path]
            try capture.run()
            capture.waitUntilExit()
            guard capture.terminationStatus == 0 else { throw NSError(domain: "窗口诊断采集失败", code: 1) }
            try FileManager.default.setAttributes([.posixPermissions:0o600], ofItemAtPath:image.path)
            let visible = diagnostics.appendingPathComponent("visible-window.png")
            let composition = Process()
            composition.executableURL = URL(fileURLWithPath:"/usr/sbin/screencapture")
            composition.arguments = ["-R", "\(Int(main.x)),\(Int(main.y)),\(Int(main.width)),\(Int(main.height))", "-x", visible.path]
            try composition.run()
            composition.waitUntilExit()
            guard composition.terminationStatus == 0 else { throw NSError(domain:"窗口可见区域诊断采集失败",code:1) }
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:visible.path)
            let metadata = diagnostics.appendingPathComponent("window.json")
            try JSONEncoder().encode(main).write(to:metadata,options:.atomic)
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:metadata.path)
        }
        try output(main)
    }
} catch {
    fputs("微信窗口查询失败: \(error)\n", stderr)
    exit(1)
}
