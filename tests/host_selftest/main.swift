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
            case .idle:
                statusDict = ["state": "idle"]
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
#if HOST_APP_CORE
        if command == "core_decision" {
            let coreEnabled = dict["core_enabled"] as? Bool ?? false
            let pythonLoaded = dict["python_loaded"] as? Bool ?? false
            let procState = dict["process_state"] as? String ?? "not_running"
            let nowEpoch = dict["now"] as? Double ?? Date().timeIntervalSince1970
            let now = Date(timeIntervalSince1970: nowEpoch)

            let status: CoreProcessStatus
            switch procState {
            case "running":
                let pid = Int32(dict["pid"] as? Int ?? 1000)
                status = .running(pid: pid)
            case "stopping":
                let pid = Int32(dict["pid"] as? Int ?? 1000)
                status = .stopping(pid: pid)
            case "restarting":
                let atEpoch = dict["restarting_at"] as? Double ?? (nowEpoch + 10.0)
                status = .restarting(at: Date(timeIntervalSince1970: atEpoch))
            case "gave_up":
                status = .gaveUp
            case "config_error":
                status = .configError
            default:
                status = .notRunning
            }

            let decision = decideCoreAction(coreEnabled: coreEnabled, pythonLoaded: pythonLoaded, status: status, now: now)
            let resp: [String: Any] = ["decision": decision.rawValue]
            let outputData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
            if let outputStr = String(data: outputData, encoding: .utf8) {
                print(outputStr)
                return 0
            }
        } else if command == "core_supervisor_run" {
            let action = dict["action"] as? String ?? "run_once"
            let binPath = dict["binary_path"] as? String ?? ""
            let logPath = dict["log_path"] as? String ?? ""
            let graceSec = dict["grace_seconds"] as? Double ?? 240.0

            let sup = CoreSupervisor(
                coreEnabledURL: URL(fileURLWithPath: "/tmp/fake.enabled"),
                binaryURL: URL(fileURLWithPath: binPath),
                configURL: URL(fileURLWithPath: "/tmp/fake.json"),
                logURL: URL(fileURLWithPath: logPath),
                gracePeriod: graceSec,
                pythonLoadedChecker: { false }
            )

            if action == "run_once" {
                try sup.startProcess(binaryPath: binPath, arguments: [], logPath: logPath)
                sup.waitUntilExit()
                usleep(50_000) // 50ms 等待 terminationHandler 完成
                let st = sup.currentStatus
                let exitCode = sup.lastExitCode ?? 0
                let resp: [String: Any] = [
                    "ok": true,
                    "exit_code": exitCode,
                    "supervisor_status": st.rawStatusString,
                    "gave_up": sup.gaveUp
                ]
                let outputData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                if let outputStr = String(data: outputData, encoding: .utf8) {
                    print(outputStr)
                    return 0
                }
            } else if action == "start_and_stop" {
                let readyPath = dict["ready_file"] as? String
                try sup.startProcess(binaryPath: binPath, arguments: [], logPath: logPath)
                if let rp = readyPath {
                    var waited = 0
                    while waited < 200 { // 最多等 2 秒
                        if FileManager.default.fileExists(atPath: rp) {
                            break
                        }
                        usleep(10_000) // 10ms
                        waited += 1
                    }
                } else {
                    usleep(100_000) // 100ms
                }
                let stoppedBy = sup.stopProcess(gracePeriod: graceSec)
                let resp: [String: Any] = [
                    "ok": true,
                    "stopped_by": stoppedBy
                ]
                let outputData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                if let outputStr = String(data: outputData, encoding: .utf8) {
                    print(outputStr)
                    return 0
                }
            } else if action == "test_async_reconcile_stopping" {
                let readyPath = dict["ready_file"] as? String
                let enabledFile = URL(fileURLWithPath: "/tmp/fake_enable_\(UUID().uuidString)")
                FileManager.default.createFile(atPath: enabledFile.path, contents: Data())

                let testSup = CoreSupervisor(
                    coreEnabledURL: enabledFile,
                    binaryURL: URL(fileURLWithPath: binPath),
                    configURL: URL(fileURLWithPath: "/tmp/fake.json"),
                    logURL: URL(fileURLWithPath: logPath),
                    gracePeriod: graceSec,
                    pythonLoadedChecker: { false }
                )

                // 1. 启动进程
                testSup.checkAndReconcile()

                // 2. 等待 ready_file
                if let rp = readyPath {
                    var waited = 0
                    while waited < 200 {
                        if FileManager.default.fileExists(atPath: rp) { break }
                        usleep(10_000)
                        waited += 1
                    }
                } else {
                    usleep(100_000)
                }

                let initialStatus = testSup.currentStatus.rawStatusString

                // 3. 删除开关文件，触发停止
                try? FileManager.default.removeItem(at: enabledFile)

                // 4. 调用 checkAndReconcile，并精确计时
                let t0 = Date()
                testSup.checkAndReconcile()
                let elapsed = Date().timeIntervalSince(t0)

                let stoppingStatus = testSup.currentStatus.rawStatusString

                // 5. 停止期间再次调用，断言返回 keep 且不重复发信号
                let secondDecision = testSup.checkAndReconcile()

                // 6. 等待后台停止完成（最多等 3 秒）
                var waitStop = 0
                while waitStop < 300 {
                    if testSup.currentStatus == .notRunning {
                        break
                    }
                    usleep(10_000)
                    waitStop += 1
                }
                let finalStatus = testSup.currentStatus.rawStatusString

                // 7. 重新开启开关文件，断言状态回到可再次启动
                FileManager.default.createFile(atPath: enabledFile.path, contents: Data())
                let restartDecision = testSup.checkAndReconcile()
                try? FileManager.default.removeItem(at: enabledFile)

                // 清理可能遗留的子进程
                testSup.stopProcess(gracePeriod: 0.1)

                let resp: [String: Any] = [
                    "ok": true,
                    "elapsed_seconds": elapsed,
                    "initial_status": initialStatus,
                    "stopping_status": stoppingStatus,
                    "second_reconcile_decision": secondDecision.rawValue,
                    "final_status": finalStatus,
                    "can_restart_decision": restartDecision.rawValue
                ]
                let outputData = try JSONSerialization.data(withJSONObject: resp, options: [.sortedKeys])
                if let outputStr = String(data: outputData, encoding: .utf8) {
                    print(outputStr)
                    return 0
                }
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
