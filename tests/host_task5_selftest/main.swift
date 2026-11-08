import Foundation

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        fputs("usage: host_task5_selftest <command> <json>\n", stderr)
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

        if command == "log_tail" {
            let filePath = dict["file_path"] as? String ?? ""
            let maxLines = dict["max_lines"] as? Int ?? 500
            let op = dict["op"] as? String ?? "poll"
            let url = URL(fileURLWithPath: filePath)

            // 使用全局或静态字典保持 LogTail 实例？或者按路径缓存
            // 为简单起见，如果 op == "init_or_poll"，可以通过 token 传或者单测直接做复合指令
            let ops = dict["operations"] as? [[String: Any]] ?? []
            if !ops.isEmpty {
                let tail = LogTail(url: url, maxLines: maxLines)
                var pollResults: [[[String]]] = []
                for step in ops {
                    let stepOp = step["op"] as? String ?? ""
                    if stepOp == "poll" {
                        let lines = tail.poll()
                        pollResults.append([lines])
                    } else if stepOp == "append" {
                        let linesToAppend = step["lines"] as? [String] ?? []
                        let content = linesToAppend.joined(separator: "\n") + "\n"
                        if let handle = try? FileHandle(forWritingTo: url) {
                            handle.seekToEndOfFile()
                            if let d = content.data(using: .utf8) {
                                handle.write(d)
                            }
                            handle.closeFile()
                        } else {
                            try? content.data(using: .utf8)?.write(to: url)
                        }
                    } else if stepOp == "truncate" {
                        let content = (step["lines"] as? [String] ?? []).joined(separator: "\n") + "\n"
                        try? content.data(using: .utf8)?.write(to: url)
                    }
                }
                let resp: [String: Any] = ["ok": true, "polls": pollResults]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            }
        } else if command == "session_copy" {
            let state = dict["state"] as? String ?? ""
            let reason = dict["reason"] as? String
            let code = dict["code"] as? String
            let handoff = dict["handoff"] as? Bool
            let text = sessionStateHumanReadable(state: state, reason: reason, code: code, handoff: handoff)
            let resp: [String: Any] = ["ok": true, "text": text]
            let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
            print(String(data: outData, encoding: .utf8)!)
            return 0
        } else if command == "settings_validate" {
            let original = dict["original_config"] as? [String: Any] ?? ["schema_version": 1, "account_id": "test"]
            let formInputs = dict["form_inputs"] as? [String: Any] ?? [:]
            let res = SettingsFormLogic.validateAndSave(original: original, formInputs: formInputs)
            switch res {
            case .success(let updated):
                let resp: [String: Any] = ["ok": true, "updated": updated]
                let outData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                print(String(data: outData, encoding: .utf8)!)
                return 0
            case .failure(let err):
                let resp: [String: Any] = ["ok": false, "error": err.localizedDescription]
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
