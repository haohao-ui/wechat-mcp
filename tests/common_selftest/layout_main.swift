// Self-test driver for native/common/WechatLayout.swift: prints JSON for layout loading and fallback queries.
import Foundation

func jsonString(_ obj: Any) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys])
    guard let s = String(data: data, encoding: .utf8) else {
        throw NSError(domain: "LayoutSelftest", code: 1, userInfo: [NSLocalizedDescriptionKey: "utf8_encoding_failed"])
    }
    return s
}

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 2 else {
        fputs("usage: layout_selftest layout <path> | fallback <dir> <version>\n", stderr)
        return 1
    }

    let command = args[1]
    if command == "memo" {
        // Steps: "<pid>:<version>" (version "-" = unreadable), "none" (no WeChat found, last one alive),
        // "gone" (no WeChat found, last one exited).
        var memo = VersionMemo()
        var resolved: [Any] = []
        for step in args.dropFirst(2) {
            let alive = step != "gone"
            var found: (pid: Int32, version: String?)? = nil
            if step != "none" && step != "gone" {
                let parts = step.split(separator: ":", maxSplits: 1).map(String.init)
                found = (Int32(parts[0]) ?? 0, parts[1] == "-" ? nil : parts[1])
            }
            resolved.append(memo.resolve(lookup: { found }, alive: { _ in alive }) ?? NSNull())
        }
        print((try? jsonString(["resolved": resolved])) ?? "{}")
        return 0
    }
    do {
        if command == "layout" {
            guard args.count >= 3 else {
                fputs("usage: layout_selftest layout <path>\n", stderr)
                return 1
            }
            let path = args[2]
            do {
                let layout = try WechatLayout.load(path: path)
                let chat = try layout.region("chat")
                let sessionList = try layout.region("session_list")
                let title = try layout.region("title")
                let input = try layout.region("input")

                let out: [String: Any] = [
                    "ok": true,
                    "schema": layout.schema,
                    "wechat_version": layout.wechatVersion,
                    "window": [
                        "width": layout.window.width,
                        "height": layout.window.height
                    ],
                    "chat": ["x": chat.x, "y": chat.y, "w": chat.w, "h": chat.h],
                    "session_list": ["x": sessionList.x, "y": sessionList.y, "w": sessionList.w, "h": sessionList.h],
                    "title": ["x": title.x, "y": title.y, "w": title.w, "h": title.h],
                    "input": ["x": input.x, "y": input.y, "w": input.w, "h": input.h],
                    "points_per_tick": layout.scroll.pointsPerTick,
                    "lines_per_tick": layout.scroll.linesPerTick,
                    "anchors_count": layout.anchors.count,
                    "menu_labels": layout.menuLabels
                ]
                print(try jsonString(out))
                return 0
            } catch {
                let out: [String: Any] = [
                    "ok": false,
                    "error": String(describing: error)
                ]
                print(try jsonString(out))
                return 0
            }
        } else if command == "fallback" {
            guard args.count >= 4 else {
                fputs("usage: layout_selftest fallback <dir> <version>\n", stderr)
                return 1
            }
            let dir = args[2]
            let version = args[3]
            let found = WechatLayout.path(forVersion: version, directory: dir)
            let out: [String: Any] = [
                "path": found != nil ? (found as Any) : NSNull()
            ]
            print(try jsonString(out))
            return 0
        } else {
            fputs("unknown command: \(command)\n", stderr)
            return 1
        }
    } catch {
        fputs("unexpected error: \(error)\n", stderr)
        return 1
    }
}

@main
struct LayoutSelftestApp {
    static func main() {
        exit(run())
    }
}
