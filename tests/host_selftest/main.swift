import Foundation

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        fputs("usage: host_selftest restart_policy <json>\n", stderr)
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

        if command == "restart_policy" {
            var policy = RestartPolicy()
            var results: [[String: Any]] = []

            let actions = dict["actions"] as? [[String: Any]] ?? []
            for action in actions {
                let op = action["op"] as? String ?? ""
                if op == "exit" {
                    let epoch = action["epoch"] as? Double ?? Date().timeIntervalSince1970
                    let date = Date(timeIntervalSince1970: epoch)
                    let decision = policy.recordExit(at: date)
                    switch decision {
                    case .restart(let after):
                        results.append(["decision": "restart", "after": after])
                    case .giveUp:
                        results.append(["decision": "give_up"])
                    }
                } else if op == "reset" {
                    policy.reset()
                }
            }

            let response: [String: Any] = ["results": results]
            let outputData = try JSONSerialization.data(withJSONObject: response, options: [.sortedKeys])
            if let outputStr = String(data: outputData, encoding: .utf8) {
                print(outputStr)
                return 0
            }
        }

        fputs("error: unknown command \(command)\n", stderr)
        return 1
    } catch {
        fputs("error: \(error.localizedDescription)\n", stderr)
        return 1
    }
}

exit(run())
