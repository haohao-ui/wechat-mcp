import Foundation

func run() -> Int32 {
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        fputs("usage: host_selftest <command> <json>\n", stderr)
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
        } else if command == "child_exit" {
            var handler = ChildExitHandler()
            handler.markReadyToAccept()

            let events = dict["events"] as? [[String: Any]] ?? []
            for event in events {
                if let op = event["op"] as? String, op == "reset" {
                    handler.reset()
                    continue
                }
                let status = Int32(event["status"] as? Int ?? 0)
                let reasonStr = event["reason"] as? String ?? "exit"
                let reason: Process.TerminationReason = (reasonStr == "uncaughtSignal") ? .uncaughtSignal : .exit
                let epoch = event["epoch"] as? Double ?? Date().timeIntervalSince1970
                let date = Date(timeIntervalSince1970: epoch)
                _ = handler.handleExit(status: status, reason: reason, at: date)
            }

            var statusDict: [String: Any] = [:]
            switch handler.status {
            case .running(let pid):
                statusDict = ["state": "running", "pid": pid]
            case .restarting(let at):
                statusDict = ["state": "restarting", "at": at.timeIntervalSince1970]
            case .stopped(let reason):
                statusDict = ["state": "stopped", "reason": reason]
            }

            let response: [String: Any] = [
                "is_given_up": handler.isGivenUp,
                "accepting_connections": handler.isAcceptingConnections,
                "status": statusDict
            ]
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
