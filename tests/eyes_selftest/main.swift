// Self-test driver for native/eyes/Screens.swift.
import Foundation
import CoreGraphics

func jsonString(_ obj: Any) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys])
    guard let s = String(data: data, encoding: .utf8) else {
        throw NSError(domain: "EyesSelftest", code: 1, userInfo: [NSLocalizedDescriptionKey: "utf8_encoding_failed"])
    }
    return s
}

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        fputs("usage: eyes_selftest identify <json> | convert_rows <json>\n", stderr)
        return 1
    }

    let command = args[1]
    let jsonArg = args[2]

    guard let jsonData = jsonArg.data(using: .utf8) else {
        fputs("error: invalid json argument\n", stderr)
        return 1
    }

    do {
        let jsonObj = try JSONSerialization.jsonObject(with: jsonData, options: [])

        if command == "identify" {
            guard let dict = jsonObj as? [String: Any] else {
                fputs("error: payload must be a json object\n", stderr)
                return 1
            }

            let rawWindows = dict["windows"] as? [[String: Any]] ?? []
            let windows = windowInfos(rawWindows)

            let focusDict = dict["focus"] as? [String: Any] ?? [:]
            let frontmostBundle = focusDict["frontmost_bundle"] as? String
            let focusedIdentifier = focusDict["focused_identifier"] as? String
            let focus = FocusInfo(frontmostBundle: frontmostBundle, focusedIdentifier: focusedIdentifier)

            let wechatPID: Int32
            if let pidNum = dict["wechat_pid"] as? NSNumber {
                wechatPID = pidNum.int32Value
            } else if let pidInt = dict["wechat_pid"] as? Int {
                wechatPID = Int32(pidInt)
            } else {
                wechatPID = 0
            }

            var mainSize = CGSize(width: 880, height: 640)
            if let mainArr = dict["main"] as? [NSNumber], mainArr.count >= 2 {
                mainSize = CGSize(width: mainArr[0].doubleValue, height: mainArr[1].doubleValue)
            } else if let mainDict = dict["main"] as? [String: NSNumber],
                      let w = mainDict["w"] ?? mainDict["width"],
                      let h = mainDict["h"] ?? mainDict["height"] {
                mainSize = CGSize(width: w.doubleValue, height: h.doubleValue)
            }

            let kind = identifyScreen(windows: windows, focus: focus, wechatPID: wechatPID, mainSize: mainSize)
            let out: [String: Any] = ["screen": kind.rawValue]
            print(try jsonString(out))
            return 0

        } else if command == "blocking" {
            guard let dict = jsonObj as? [String: Any] else {
                fputs("error: payload must be a json object\n", stderr)
                return 1
            }

            let rawWindows = dict["windows"] as? [[String: Any]] ?? []
            let windows = windowInfos(rawWindows)

            let focusedIdentifier = dict["focused_identifier"] as? String ?? (dict["focus"] as? [String: Any])?["focused_identifier"] as? String

            let wechatPID: Int32
            if let pidNum = dict["wechat_pid"] as? NSNumber {
                wechatPID = pidNum.int32Value
            } else if let pidInt = dict["wechat_pid"] as? Int {
                wechatPID = Int32(pidInt)
            } else {
                wechatPID = 0
            }

            var mainSize = CGSize(width: 880, height: 640)
            if let mainArr = dict["main"] as? [NSNumber], mainArr.count >= 2 {
                mainSize = CGSize(width: mainArr[0].doubleValue, height: mainArr[1].doubleValue)
            } else if let mainDict = dict["main"] as? [String: NSNumber],
                      let w = mainDict["w"] ?? mainDict["width"],
                      let h = mainDict["h"] ?? mainDict["height"] {
                mainSize = CGSize(width: w.doubleValue, height: h.doubleValue)
            }

            let kind = blockingScreen(windows: windows, wechatPID: wechatPID, mainSize: mainSize, focusedIdentifier: focusedIdentifier)
            var out: [String: Any] = [:]
            if let k = kind {
                out["blocked"] = true
                out["screen"] = k.rawValue
            } else {
                out["blocked"] = false
                out["screen"] = NSNull()
            }
            print(try jsonString(out))
            return 0

        } else if command == "convert_rows" {
            guard let rows = jsonObj as? [[String: Any]] else {
                fputs("error: payload must be an array of objects\n", stderr)
                return 1
            }

            let infos = windowInfos(rows)
            let outList: [[String: Any]] = infos.map { w in
                [
                    "owner": w.owner,
                    "pid": Int(w.pid),
                    "layer": w.layer,
                    "bounds": [
                        "x": Double(w.bounds.origin.x),
                        "y": Double(w.bounds.origin.y),
                        "w": Double(w.bounds.size.width),
                        "h": Double(w.bounds.size.height)
                    ],
                    "on_screen": w.onScreen
                ]
            }
            print(try jsonString(outList))
            return 0

        } else {
            fputs("unknown command: \(command)\n", stderr)
            return 1
        }
    } catch {
        fputs("error: \(error)\n", stderr)
        return 1
    }
}

exit(run())
