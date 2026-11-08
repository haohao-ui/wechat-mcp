import Foundation

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        fputs("usage: host_task6_selftest <command> <json>\n", stderr)
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
        guard let dict = jsonObj as? [String: Any] else {
            fputs("error: payload must be a json object\n", stderr)
            return 1
        }

        if command == "run_control" {
            let exe = dict["executable"] as? String ?? "/bin/sh"
            let cmdArgs = dict["args"] as? [String] ?? []
            let timeout = dict["timeout"] as? Double ?? 10
            let began = Date()
            let r = runControlCommand(executable: exe, arguments: cmdArgs, timeout: timeout)
            let out: [String: Any] = ["status": Int(r.status), "bytes": r.output.count, "timed_out": r.timedOut,
                                      "seconds": Date().timeIntervalSince(began)]
            let data = try JSONSerialization.data(withJSONObject: out)
            print(String(data: data, encoding: .utf8)!)
            return 0
        }
        if command == "apply_handling" {
            let config = dict["config"] as? [String: Any] ?? [:]
            let name = dict["name"] as? String ?? ""
            let kind = dict["kind"] as? String ?? "private"
            let category = dict["category"] as? String ?? "personal"
            let handlingStr = dict["handling"] as? String ?? "record_only"

            let handling: Handling
            switch handlingStr {
            case "auto_reply": handling = .autoReply
            case "ignored": handling = .ignored
            case "unignore": handling = .unignore
            default: handling = .recordOnly
            }

            let res = ContactsPageLogic.applyHandling(
                config: config,
                name: name,
                kind: kind,
                category: category,
                handling: handling
            )

            switch res {
            case .success(let updated):
                let resp: [String: Any] = ["ok": true, "updated": updated]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            case .failure(let err):
                let resp: [String: Any] = ["ok": false, "error": err.message]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            }
        } else if command == "parse_contacts_list" {
            let jsonStr = dict["json_str"] as? String ?? ""
            let (payload, counts) = try ContactsPageLogic.parseContactsPayload(json: jsonStr)
            let countsDict: [String: Any] = [
                "personal": counts.personal,
                "work": counts.work,
                "group": counts.group,
                "official": counts.official,
                "ignored": counts.ignored
            ]
            let resp: [String: Any] = [
                "ok": true,
                "counts": countsDict,
                "contact_count": payload.contacts.count,
                "list_wechat": payload.contacts.map { ContactsPageLogic.listWechatText($0) },
                "detail_wechat": payload.contacts.map { ContactsPageLogic.detailWechatText($0) },
                "titles": ["personal", "work", "group", "official"].map { ContactsPageLogic.categoryTitle($0) }
            ]
            let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
            print(String(data: outData, encoding: .utf8)!)
            return 0
        } else if command == "mcp_brain_settings" {
            let config = dict["config"] as? [String: Any] ?? [:]
            let timeoutSeconds = dict["timeout_seconds"] as? Int ?? 120
            let timeoutAction = dict["timeout_action"] as? String ?? "skip"
            let queueMinutes = dict["queue_minutes"] as? Int ?? 30

            let res = McpPageLogic.updateBrainSettings(
                config: config,
                timeoutSeconds: timeoutSeconds,
                timeoutAction: timeoutAction,
                queueMinutes: queueMinutes
            )

            switch res {
            case .success(let updated):
                let resp: [String: Any] = ["ok": true, "updated": updated]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            case .failure(let err):
                let resp: [String: Any] = ["ok": false, "error": err.message]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            }
        }

        fputs("unknown command: \(command)\n", stderr)
        return 1
    } catch {
        fputs("error: \(error.localizedDescription)\n", stderr)
        return 1
    }
}

exit(run())
