import Cocoa
import ApplicationServices

/// Records what would be posted (pointer follows our own moves; no time passes).
final class PlanSink: EventSink {
    var position = CGEvent(source: nil)?.location ?? .zero
    var events: [[String: Any]] = []
    func cursor() -> CGPoint { position }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws {
        let names: [CGEventType: String] = [.mouseMoved: "moved", .leftMouseDown: "left_down", .leftMouseUp: "left_up"]
        events.append(["type": names[type] ?? "other", "x": Double(point.x), "y": Double(point.y)])
        position = point
    }
    func scroll(lines: Int32, at point: CGPoint) throws {}
    func key(_ code: CGKeyCode, down: Bool, command: Bool) throws {
        events.append(["type": down ? "key_down" : "key_up", "code": Int(code), "cmd": command])
    }
    func sleep(_ seconds: Double) {}
}

struct SavedPasteboard {
    struct Item {
        var dataByType: [NSPasteboard.PasteboardType: Data] = [:]
    }
    var items: [Item] = []
}

protocol Pasteboard: AnyObject {
    func save() -> SavedPasteboard
    func writeText(_ text: String)
    func writeSentinel(_ text: String)
    func readText() -> String?
    func restore(_ saved: SavedPasteboard)
    var operations: [[String: Any]] { get }
}

final class SystemPasteboard: Pasteboard {
    var operations: [[String: Any]] { [] }

    func save() -> SavedPasteboard {
        let pb = NSPasteboard.general
        guard let pbItems = pb.pasteboardItems else { return SavedPasteboard() }
        var items: [SavedPasteboard.Item] = []
        for pbItem in pbItems {
            var item = SavedPasteboard.Item()
            for type in pbItem.types {
                if let data = pbItem.data(forType: type) {
                    item.dataByType[type] = data
                }
            }
            items.append(item)
        }
        return SavedPasteboard(items: items)
    }

    func writeText(_ text: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(text, forType: .string)
    }

    func writeSentinel(_ text: String) {
        writeText(text)
    }

    func readText() -> String? {
        return NSPasteboard.general.string(forType: .string)
    }

    func restore(_ saved: SavedPasteboard) {
        let pb = NSPasteboard.general
        pb.clearContents()
        guard !saved.items.isEmpty else { return }
        var newItems: [NSPasteboardItem] = []
        for savedItem in saved.items {
            let item = NSPasteboardItem()
            for (type, data) in savedItem.dataByType {
                item.setData(data, forType: type)
            }
            newItems.append(item)
        }
        pb.writeObjects(newItems)
    }
}

final class PlanPasteboard: Pasteboard {
    var operations: [[String: Any]] = []
    let readbackMode: String
    var bodyText: String = ""
    var sentinelText: String = ""

    init(mode: String = ProcessInfo.processInfo.environment["WECHAT_PLAN_READBACK"] ?? "match") {
        self.readbackMode = mode
    }

    func save() -> SavedPasteboard {
        operations.append(["op": "save"])
        return SavedPasteboard()
    }

    func writeText(_ text: String) {
        operations.append(["op": "write_text"])
        self.bodyText = text
    }

    func writeSentinel(_ text: String) {
        operations.append(["op": "write_sentinel"])
        self.sentinelText = text
    }

    func readText() -> String? {
        operations.append(["op": "read"])
        switch readbackMode {
        case "sentinel":
            return sentinelText
        case "mismatch":
            return "mismatch"
        default:
            return bodyText
        }
    }

    func restore(_ saved: SavedPasteboard) {
        operations.append(["op": "restore"])
    }
}

func isWeChatFrontmost() -> Bool {
    return NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.tencent.xinWeChat"
}

func normalizeNewlines(_ text: String) -> String {
    return text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
}

func readbackMismatchDetail(actual: String, expected: String, sentinel: String) -> String {
    let expectedCount = expected.count
    if actual == sentinel {
        return "复制未取到输入框内容，应为 \(expectedCount) 字"
    }
    let contains = actual.contains(expected) ? "包含正文" : "不含正文"
    return "读回 \(actual.count) 字，应为 \(expectedCount) 字，\(contains)"
}

func runClearInput(planOnly: Bool) {
    let sink = planOnly ? PlanSink() : nil
    let actions = planOnly ? Actions(sink: sink!) : Actions()

    if !planOnly {
        guard AXIsProcessTrusted() else {
            fputs("wechat_click: accessibility_permission_missing_for_executing_process\n", stderr)
            exit(1)
        }
        guard isWeChatFrontmost() else {
            fputs("wechat_click: frontmost_not_wechat\n", stderr)
            exit(3)
        }
    }

    do {
        try actions.key("cmd+a")
        try actions.key("delete")
        if planOnly, let s = sink {
            let data = try JSONSerialization.data(withJSONObject: ["events": s.events])
            print(String(decoding: data, as: UTF8.self))
        }
        exit(0)
    } catch {
        fputs("wechat_click: \(error)\n", stderr)
        exit(1)
    }
}

func runInputText(x: Double, y: Double, planOnly: Bool) {
    let inputData = FileHandle.standardInput.readDataToEndOfFile()
    guard !inputData.isEmpty, inputData.count <= 16384 else {
        exit(1)
    }
    guard let body = String(data: inputData, encoding: .utf8) else {
        exit(1)
    }

    let sink = planOnly ? PlanSink() : nil
    let actions = planOnly ? Actions(sink: sink!) : Actions()
    let pasteboard: Pasteboard = planOnly ? PlanPasteboard() : SystemPasteboard()

    if !planOnly {
        guard AXIsProcessTrusted() else {
            fputs("wechat_click: accessibility_permission_missing_for_executing_process\n", stderr)
            exit(1)
        }
        guard isWeChatFrontmost() else {
            fputs("wechat_click: frontmost_not_wechat\n", stderr)
            exit(3)
        }
    }

    let target = CGRect(x: x - 6, y: y - 6, width: 12, height: 12)

    var saved: SavedPasteboard? = nil
    var restored = false
    func doRestore() {
        if let s = saved, !restored {
            pasteboard.restore(s)
            restored = true
        }
    }

    func printPlanOutput() {
        if planOnly, let s = sink {
            let obj: [String: Any] = [
                "events": s.events,
                "pasteboard": pasteboard.operations
            ]
            if let data = try? JSONSerialization.data(withJSONObject: obj) {
                print(String(decoding: data, as: UTF8.self))
            }
        }
    }

    do {
        try actions.click(target)

        if !planOnly && !isWeChatFrontmost() {
            fputs("wechat_click: frontmost_not_wechat\n", stderr)
            exit(3)
        }

        saved = pasteboard.save()

        try actions.key("cmd+a")

        pasteboard.writeText(body)
        try actions.key("cmd+v")
        if !planOnly {
            Thread.sleep(forTimeInterval: 0.3)
        }

        if !planOnly && !isWeChatFrontmost() {
            doRestore()
            fputs("wechat_click: frontmost_not_wechat\n", stderr)
            exit(3)
        }

        let sentinel = "wechat-readback-\(UUID().uuidString)"
        pasteboard.writeSentinel(sentinel)
        try actions.key("cmd+a")
        try actions.key("cmd+c")
        if !planOnly {
            Thread.sleep(forTimeInterval: 0.15)
        }

        let actual = pasteboard.readText() ?? ""

        doRestore()

        let matched = normalizeNewlines(actual) == normalizeNewlines(body)
        if matched {
            printPlanOutput()
            exit(0)
        } else {
            if !planOnly && !isWeChatFrontmost() {
                fputs("wechat_click: frontmost_not_wechat\n", stderr)
                exit(3)
            }
            try actions.key("cmd+a")
            try actions.key("delete")

            let detail = readbackMismatchDetail(actual: actual, expected: body, sentinel: sentinel)
            fputs("wechat_click: input_readback_mismatch: \(detail)\n", stderr)

            printPlanOutput()
            exit(4)
        }
    } catch {
        doRestore()
        fputs("wechat_click: \(error)\n", stderr)
        exit(1)
    }
}
