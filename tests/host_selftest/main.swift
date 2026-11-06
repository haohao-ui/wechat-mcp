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
#if HOST_APP_T3
        if command == "status" {
            let decoder = JSONDecoder()
            let statusInput = try decoder.decode(StatusInput.self, from: jsonData)
            let items = evaluate(statusInput)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let outputData = try encoder.encode(items)
            if let outputStr = String(data: outputData, encoding: .utf8) {
                print(outputStr)
                return 0
            }
        } else if command == "config" {
            let op = dict["op"] as? String ?? ""
            if op == "mask_config" {
                let rawConfig = dict["config"] as? [String: Any] ?? [:]
                let masked = ConfigFormLogic.maskConfig(rawConfig)
                let outputData = try JSONSerialization.data(withJSONObject: masked, options: [.sortedKeys])
                if let outputStr = String(data: outputData, encoding: .utf8) {
                    print(outputStr)
                    return 0
                }
            } else if op == "save_config" {
                let targetPathStr = dict["target_path"] as? String ?? ""
                let targetURL = URL(fileURLWithPath: targetPathStr)
                let newConfig = dict["new_config"] as? [String: Any] ?? [:]
                let newConfigData = try JSONSerialization.data(withJSONObject: newConfig, options: [.prettyPrinted])

                let simCheck = dict["simulated_check_result"] as? [String: Any] ?? [:]
                let simOk = simCheck["ok"] as? Bool ?? true
                let simErr = simCheck["error"] as? String
                let simBackupFail = dict["simulated_backup_failure"] as? Bool ?? false

                var didReload = false
                do {
                    try ConfigFormLogic.saveConfig(
                        targetPath: targetURL,
                        newConfigData: newConfigData,
                        checker: { _ in
                            return (simOk, simErr)
                        },
                        backupHandler: simBackupFail ? { _, _ in
                            throw NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "模拟备份失败"])
                        } : nil,
                        reloader: {
                            didReload = true
                        }
                    )
                    let resp: [String: Any] = ["ok": true, "reloaded": didReload]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                } catch {
                    let resp: [String: Any] = ["ok": false, "error": error.localizedDescription]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                }
            } else if op == "update_and_save_form" {
                let targetPathStr = dict["target_path"] as? String ?? ""
                let targetURL = URL(fileURLWithPath: targetPathStr)
                let originalConfig = dict["original_config"] as? [String: Any] ?? [:]
                let formInputs = dict["form_inputs"] as? [String: Any] ?? [:]

                let simCheck = dict["simulated_check_result"] as? [String: Any] ?? [:]
                let simOk = simCheck["ok"] as? Bool ?? true
                let simErr = simCheck["error"] as? String

                do {
                    let updated = try ConfigFormLogic.buildUpdatedConfig(original: originalConfig, formInputs: formInputs)
                    let updatedData = try JSONSerialization.data(withJSONObject: updated, options: [.prettyPrinted])
                    try ConfigFormLogic.saveConfig(
                        targetPath: targetURL,
                        newConfigData: updatedData,
                        checker: { _ in
                            return (simOk, simErr)
                        }
                    )
                    let resp: [String: Any] = ["ok": true]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                } catch {
                    let resp: [String: Any] = ["ok": false, "error": error.localizedDescription]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                }
            } else if op == "check_temp_security" {
                do {
                    let dir = try ConfigFormLogic.getSecureTempDirectory()
                    let attrs = try FileManager.default.attributesOfItem(atPath: dir.path)
                    let dirPerm = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? 0

                    let testFile = dir.appendingPathComponent("test-sec-\(UUID().uuidString).json")
                    let testData = (dict["test_data"] as? String ?? "{}").data(using: .utf8)!
                    try testData.write(to: testFile, options: .atomic)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: testFile.path)
                    let fileAttrs = try FileManager.default.attributesOfItem(atPath: testFile.path)
                    let filePerm = (fileAttrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
                    try? FileManager.default.removeItem(at: testFile)

                    let inSharedTmp = dir.path == "/tmp" || dir.path == "/var/tmp"

                    let resp: [String: Any] = [
                        "ok": true,
                        "dir_permissions": dirPerm,
                        "file_permissions": filePerm,
                        "in_shared_tmp_root": inSharedTmp
                    ]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                } catch {
                    let resp: [String: Any] = ["ok": false, "error": error.localizedDescription]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                }
            } else if op == "decide_window_adjustment" {
                let winsDicts = dict["windows"] as? [[String: Any]] ?? []
                let mainDict = dict["main_window"] as? [String: Any]
                let dispDict = dict["display"] as? [String: Any] ?? [:]

                func parseCandidate(_ d: [String: Any]) -> WindowCandidate {
                    let isMin = d["is_minimized"] as? Bool ?? false
                    let isFull = d["is_fullscreen"] as? Bool ?? false
                    let x = d["x"] as? Double ?? 0
                    let y = d["y"] as? Double ?? 0
                    let w = d["width"] as? Double ?? 880
                    let h = d["height"] as? Double ?? 640
                    return WindowCandidate(isMinimized: isMin, isFullScreen: isFull, frame: CGRect(x: x, y: y, width: w, height: h))
                }

                let candidates = winsDicts.map { parseCandidate($0) }
                let mainCandidate = mainDict != nil ? parseCandidate(mainDict!) : nil

                let minX = dispDict["minX"] as? Double ?? 0
                let minY = dispDict["minY"] as? Double ?? 0
                let maxX = dispDict["maxX"] as? Double ?? 1920
                let maxY = dispDict["maxY"] as? Double ?? 1080
                let dispBounds = CGRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY)

                let decision = decideWindowAdjustment(windows: candidates, mainWindow: mainCandidate, displayBounds: dispBounds)
                switch decision {
                case .rejected(let msg):
                    let resp: [String: Any] = ["status": "rejected", "message": msg]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                case .adjust(let origin, let size):
                    let resp: [String: Any] = [
                        "status": "adjust",
                        "target_origin": ["x": origin.x, "y": origin.y],
                        "target_size": ["width": size.width, "height": size.height]
                    ]
                    let out = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                    if let s = String(data: out, encoding: .utf8) {
                        print(s)
                        return 0
                    }
                }
            }
        } else if command == "diagnostics" {
            let decoder = JSONDecoder()
            let statusInputData = try JSONSerialization.data(withJSONObject: dict["status_input"] as? [String: Any] ?? [:], options: [])
            let statusInput = try decoder.decode(StatusInput.self, from: statusInputData)
            let version = dict["app_version"] as? String ?? "1.0.0"
            let items = evaluate(statusInput)
            let text = generateDiagnostics(items: items, appVersion: version)
            let resp: [String: Any] = ["text": text]
            let out = try JSONSerialization.data(withJSONObject: resp, options: [])
            if let s = String(data: out, encoding: .utf8) {
                print(s)
                return 0
            }
        }
#endif

        fputs("error: unknown command \(command)\n", stderr)
        return 1
    } catch {
        fputs("error: \(error.localizedDescription)\n", stderr)
        return 1
    }
}

exit(run())
