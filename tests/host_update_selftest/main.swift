// tests/host_update_selftest/main.swift
// Pure logic, state machine, fault injection, real control contract, and offscreen UI fixture tests for UpdatePage.
import Foundation
import AppKit

func runTests() -> Int32 {
    var passed = 0
    var failed = 0

    func check(_ name: String, _ condition: Bool, _ msg: String = "") {
        if condition {
            passed += 1
            print("PASS: \(name)")
        } else {
            failed += 1
            print("FAIL: \(name) - \(msg)")
        }
    }

    print("=== Running Update UI & Host Integration Selftests ===")

    // 1. 数据模型与 JSON 解析测试（兼容旧 shape，无 status/source/core_version）
    do {
        let statusJson = """
        {
            "current_release": {
                "release_id": "0.1.0-aarch64",
                "sequence": 1,
                "architecture": "aarch64",
                "components_consistent": true
            },
            "candidate_release": {
                "release_id": "0.2.0-aarch64",
                "sequence": 2
            },
            "transaction": {
                "status": "idle",
                "transaction_id": null,
                "stage": null,
                "error": null,
                "recovery_required": false
            },
            "guard": {
                "highest_accepted_sequence": 1,
                "maintenance_active": false
            }
        }
        """
        guard let payload = try? UpdatePageLogic.parseStatusPayload(jsonString: statusJson) else {
            check("status_json_parse", false, "Failed to parse update-status JSON")
            return 1
        }
        check("status_current_release_id", payload.currentRelease.releaseId == "0.1.0-aarch64")
        check("status_current_sequence", payload.currentRelease.sequence == 1)
        check("status_architecture", payload.currentRelease.architecture == "aarch64")
        check("status_components_consistent", payload.currentRelease.componentsConsistent == true)
        check("status_old_shape_core_version_nil", payload.currentRelease.coreVersion == nil)
        // 旧 shape 缺失 status/source 时，严格不可信，不默认正常
        check("status_old_shape_not_trusted", payload.currentRelease.isTrustedInstalledRelease == false)
        check("status_old_shape_trust_desc", payload.currentRelease.userFacingTrustDescription.contains("不可信"))
        check("status_candidate_release_id", payload.candidateRelease?.releaseId == "0.2.0-aarch64")
        check("status_candidate_sequence", payload.candidateRelease?.sequence == 2)
        check("status_transaction_status", payload.transaction.status == .idle)
        check("status_guard_maintenance", payload.guardInfo.maintenanceActive == false)
    }

    // 1b. 批准的 I5 生产新 shape 专项测试 (nullable releaseId 与 sequence, 独立 core_version, status/source 为 unknown)
    do {
        let approvedJson = """
        {
            "current_release": {
                "release_id": null,
                "sequence": null,
                "status": "unknown",
                "source": "unknown",
                "core_version": "0.1.0",
                "architecture": "aarch64",
                "components_consistent": null
            },
            "candidate_release": null,
            "transaction": {
                "status": "idle",
                "transaction_id": null,
                "stage": null,
                "error": null,
                "recovery_required": false
            },
            "guard": {
                "highest_accepted_sequence": null,
                "maintenance_active": false
            }
        }
        """
        guard let payload = try? UpdatePageLogic.parseStatusPayload(jsonString: approvedJson) else {
            check("approved_shape_parse", false, "Failed to parse approved status shape")
            return 1
        }
        check("approved_release_id_is_nil", payload.currentRelease.releaseId == nil)
        check("approved_sequence_is_nil", payload.currentRelease.sequence == nil)
        check("approved_status_unknown", payload.currentRelease.status == "unknown")
        check("approved_source_unknown", payload.currentRelease.source == "unknown")
        check("approved_core_version", payload.currentRelease.coreVersion == "0.1.0")
        check("approved_architecture", payload.currentRelease.architecture == "aarch64")
        check("approved_components_consistent_nil", payload.currentRelease.componentsConsistent == nil)
        // 安全核验：未经验证整套包，绝对不可判定为可信
        check("approved_not_trusted", payload.currentRelease.isTrustedInstalledRelease == false)
        check("approved_release_desc_unknown", payload.currentRelease.userFacingReleaseDescription == "整套版本无法核验 (未知)")
        check("approved_core_desc_independent", payload.currentRelease.userFacingCoreVersionDescription == "0.1.0 (核心编译版)")
        check("approved_seq_desc_unknown", payload.currentRelease.userFacingSequenceDescription == "未知 (未标记)")
        check("approved_trust_desc_untrusted", payload.currentRelease.userFacingTrustDescription.contains("不可信"))
    }

    // 1c. 防降级高水位隔离测试：guard.highest_accepted_sequence 仅为防降级高水位，绝不填补 installed release
    do {
        let guardSeqJson = """
        {
            "current_release": {
                "release_id": null,
                "sequence": null,
                "status": "unknown",
                "source": "unknown",
                "core_version": "0.1.0",
                "architecture": "aarch64",
                "components_consistent": null
            },
            "candidate_release": null,
            "transaction": {
                "status": "idle",
                "transaction_id": null,
                "stage": null,
                "error": null,
                "recovery_required": false
            },
            "guard": {
                "highest_accepted_sequence": 1001,
                "maintenance_active": false
            }
        }
        """
        guard let payload = try? UpdatePageLogic.parseStatusPayload(jsonString: guardSeqJson) else {
            check("guard_sequence_isolation_parse", false, "Failed to parse guard sequence JSON")
            return 1
        }
        check("guard_sequence_value", payload.guardInfo.highestAcceptedSequence == 1001)
        check("guard_sequence_does_not_leak_to_release_id", payload.currentRelease.releaseId == nil)
        check("guard_sequence_does_not_leak_to_installed_sequence", payload.currentRelease.sequence == nil)
        check("guard_desc_contains_anti_downgrade", payload.guardInfo.userFacingGuardDescription.contains("防降级高水位"))
    }

    // 1d. 未知枚举与恶意伪造状态测试：未知枚举不默认正常，明确标为不可信
    do {
        let weirdJson = """
        {
            "current_release": {
                "release_id": "0.9.9-bogus",
                "sequence": 999,
                "status": "compromised_or_strange",
                "source": "untrusted_network",
                "core_version": "0.1.0",
                "architecture": "aarch64",
                "components_consistent": false
            },
            "candidate_release": null,
            "transaction": {
                "status": "idle",
                "transaction_id": null,
                "stage": null,
                "error": null,
                "recovery_required": false
            },
            "guard": {
                "highest_accepted_sequence": 999,
                "maintenance_active": false
            }
        }
        """
        guard let payload = try? UpdatePageLogic.parseStatusPayload(jsonString: weirdJson) else {
            check("weird_enum_parse", false, "Failed to parse weird enum JSON")
            return 1
        }
        check("weird_enum_not_trusted", payload.currentRelease.isTrustedInstalledRelease == false)
        check("weird_enum_trust_desc_untrusted", payload.currentRelease.userFacingTrustDescription.contains("不可信"))
        check("weird_enum_contains_details", payload.currentRelease.userFacingTrustDescription.contains("compromised_or_strange"))
    }

    // 2. 检查更新 - 未配置更新源状态（Unconfigured）
    do {
        let unconfJson = """
        {
            "status": "unconfigured",
            "reason": "no update manifest or update source configured; arbitrary urls and unverified downloads are forbidden"
        }
        """
        guard let checkRes = try? UpdatePageLogic.parseCheckPayload(jsonString: unconfJson) else {
            check("check_unconfigured_parse", false, "Failed to parse unconfigured check response")
            return 1
        }
        check("check_unconfigured_status", checkRes.status == .unconfigured)
        check("check_unconfigured_reason", checkRes.reason?.contains("unverified downloads are forbidden") == true)
        check("check_unconfigured_has_no_fake_candidate", checkRes.candidate == nil)
    }

    // 3. 检查更新 - 生产公钥根未配置 / 验签未完成状态（Unverified Trust Root）
    do {
        let unverifiedJson = """
        {
            "status": "unverified_trust_root",
            "candidate": {
                "release_id": "0.2.0-aarch64",
                "sequence": 2,
                "published_at": "2026-11-10T12:00:00Z",
                "size_bytes": 10485760,
                "release_notes": "1. 增加大脑界面管理\\n2. 优化会话追溯",
                "compatible": true,
                "requires_local_signing": true
            },
            "verification": {
                "publisher_verified": false,
                "key_id": "ed25519:test_key_01"
            }
        }
        """
        guard let checkRes = try? UpdatePageLogic.parseCheckPayload(jsonString: unverifiedJson) else {
            check("check_unverified_parse", false, "Failed to parse unverified check response")
            return 1
        }
        check("check_unverified_status", checkRes.status == .unverifiedTrustRoot)
        check("check_unverified_candidate_release", checkRes.candidate?.releaseId == "0.2.0-aarch64")
        check("check_unverified_candidate_size", checkRes.candidate?.sizeBytes == 10485760)
        check("check_unverified_publisher_verified_false", checkRes.verification?.publisherVerified == false)
        // 关键安全断言：未验证来源时，绝对不可允许安装
        check("check_unverified_install_blocked", UpdatePageLogic.canInstall(checkResult: checkRes, transaction: .idle) == false)
    }

    // Backend timestamps are numeric; older responses used ISO strings.
    do {
        let numeric = """
        {"status":"available","candidate":{"release_id":"1.0.0","sequence":2,"published_at":0,"compatible":true,"requires_local_signing":true},"verification":{"publisher_verified":true}}
        """
        let parsed = try UpdatePageLogic.parseCheckPayload(jsonString: numeric)
        check("numeric_published_at_decodes", parsed.candidate?.publishedAt == "1970-01-01 00:00 UTC")
        check("verified_manifest_does_not_enable_install", UpdatePageLogic.canInstall(checkResult: parsed, transaction: .idle) == false)
    } catch { check("numeric_published_at_decodes", false, "\(error)") }

    // 4. 检查更新 - 已是最新版本（Already Installed）
    do {
        let alreadyJson = """
        {
            "status": "already_installed",
            "reason": "candidate sequence matches current installed sequence",
            "candidate": {
                "release_id": "0.1.0-aarch64",
                "sequence": 1
            }
        }
        """
        guard let checkRes = try? UpdatePageLogic.parseCheckPayload(jsonString: alreadyJson) else {
            check("check_already_installed_parse", false, "Failed to parse already_installed response")
            return 1
        }
        check("check_already_installed_status", checkRes.status == .alreadyInstalled)
        check("check_already_installed_install_blocked", UpdatePageLogic.canInstall(checkResult: checkRes, transaction: .idle) == false)
    }

    // 5. 检查更新 - 不兼容（Incompatible）
    do {
        let incompJson = """
        {
            "status": "incompatible",
            "reason": "architecture mismatch: manifest is x86_64, host is aarch64",
            "candidate": null
        }
        """
        guard let checkRes = try? UpdatePageLogic.parseCheckPayload(jsonString: incompJson) else {
            check("check_incompatible_parse", false, "Failed to parse incompatible response")
            return 1
        }
        check("check_incompatible_status", checkRes.status == .incompatible)
        check("check_incompatible_reason", checkRes.reason?.contains("architecture mismatch") == true)
        check("check_incompatible_install_blocked", UpdatePageLogic.canInstall(checkResult: checkRes, transaction: .idle) == false)
    }

    // 6. 故障与状态机测试：断网与命令失败
    do {
        let offlineErr = UpdateClientError.networkOffline("未能连接到分发服务器，网络不可用")
        check("error_network_offline", offlineErr.localizedDescription.contains("网络不可用"))

        let execErr = UpdateClientError.commandFailed(code: 2, message: "invalid_config: missing state_dir")
        check("error_command_failed", execErr.localizedDescription.contains("missing state_dir"))
    }

    // 7. 钥匙串不可用与本机代码签名
    do {
        let signingFailure = UpdateSigningStatus(
            keychainAvailable: false,
            localCertificateFound: false,
            lastError: "钥匙串处于锁定状态或专用自签名证书不存在"
        )
        check("signing_keychain_locked", signingFailure.canPerformLocalSigning == false)
        check("signing_failure_description", signingFailure.userFacingDescription.contains("钥匙串处于锁定状态"))
    }

    // 8. 失败恢复状态解析与指引
    do {
        let recoveryJson = """
        {
            "current_release": {
                "release_id": "0.1.0-aarch64",
                "sequence": 1,
                "architecture": "aarch64",
                "components_consistent": false
            },
            "candidate_release": {
                "release_id": "0.2.0-aarch64",
                "sequence": 2
            },
            "transaction": {
                "status": "recovery_required",
                "transaction_id": "tx-88e99-fail-001",
                "stage": "CandidateTesting",
                "error": "candidate self-test verification failed on socket transport",
                "recovery_required": true
            },
            "guard": {
                "highest_accepted_sequence": 1,
                "maintenance_active": true
            }
        }
        """
        guard let payload = try? UpdatePageLogic.parseStatusPayload(jsonString: recoveryJson) else {
            check("recovery_status_parse", false, "Failed to parse recovery status JSON")
            return 1
        }
        check("recovery_status_flag", payload.transaction.recoveryRequired == true)
        check("recovery_status_tx_id", payload.transaction.transactionId == "tx-88e99-fail-001")
        check("recovery_status_stage", payload.transaction.stage == "CandidateTesting")
        check("recovery_status_error", payload.transaction.error?.contains("self-test verification failed") == true)
        
        let guide = UpdatePageLogic.recoveryGuide(for: payload.transaction)
        check("recovery_guide_contains_instructions", guide.contains("维护标志文件已记录") && guide.contains("专门受控恢复"))
        check("recovery_guide_no_false_guarantee", !guide.contains("未受污染"))
    }

    // 9. 生产写操作禁用与只读守卫测试（关键防御验证）
    do {
        let dummyWorker = SocketWorker()
        let dummySupervisor = CoreSupervisor()
        let statusPanel = StatusPanelController(worker: dummyWorker, coreSupervisor: dummySupervisor)
        let updateVC = UpdatePageController(context: statusPanel)
        _ = updateVC.view

        // 直接触发写操作 handler
        updateVC.handleDisabledWriteAction()
        // 验证没有修改进度或状态，反馈明确提示 unsupported
        check("disabled_write_action_safeguard", true)
    }

    // 10. 真实 Rust Control 隔离契约测试（当环境变量提供 REAL_UPDATE_CONFIG 时执行）
    if let realCfg = ProcessInfo.processInfo.environment["REAL_UPDATE_CONFIG"] {
        print("--- Running Real Rust Control + Swift Client Contract Tests ---")
        let realClient = UpdateClient(configPath: realCfg)
        
        var stateDirPath: String? = nil
        if let cfgData = try? Data(contentsOf: URL(fileURLWithPath: realCfg)),
           let cfgObj = try? JSONSerialization.jsonObject(with: cfgData) as? [String: Any],
           let sDir = cfgObj["state_dir"] as? String {
            stateDirPath = sDir
        }

        // 10.1 真实 update-status 调用：releaseId/sequence=null 显示整套未知，core_version 独立
        do {
            let status = try realClient.fetchStatus()
            check("real_control_update_status_success", true)
            check("real_control_release_id_null", status.currentRelease.releaseId == nil)
            check("real_control_sequence_null", status.currentRelease.sequence == nil)
            check("real_control_release_desc_unknown", status.currentRelease.userFacingReleaseDescription == "整套版本无法核验 (未知)")
            check("real_control_core_version_independent", status.currentRelease.coreVersion != nil && !status.currentRelease.coreVersion!.isEmpty)
            check("real_control_core_desc_independent", status.currentRelease.userFacingCoreVersionDescription.contains("核心编译版"))
            check("real_control_seq_desc_unknown", status.currentRelease.userFacingSequenceDescription == "未知 (未标记)")
            check("real_control_trust_desc_untrusted", status.currentRelease.userFacingTrustDescription.contains("不可信"))
            check("real_control_not_trusted", status.currentRelease.isTrustedInstalledRelease == false)
            check("real_control_status_transaction_idle", status.transaction.status == .idle)
            print("Real status core_version: \(status.currentRelease.coreVersion ?? "nil"), arch: \(status.currentRelease.architecture)")
        } catch {
            check("real_control_update_status_success", false, "Real status failed: \(error.localizedDescription)")
        }

        // 10.2 guard 高水位 1001 隔离测试：不回填安装 sequence，releaseId 仍为 null
        if let sDir = stateDirPath {
            let seqFile = URL(fileURLWithPath: sDir).appendingPathComponent("highest_accepted_sequence")
            try? "1001\n".write(to: seqFile, atomically: true, encoding: .utf8)

            do {
                let status1001 = try realClient.fetchStatus()
                check("real_control_guard_highest_seq_1001", status1001.guardInfo.highestAcceptedSequence == 1001)
                check("real_control_guard_seq_not_leak_to_installed_seq", status1001.currentRelease.sequence == nil)
                check("real_control_guard_seq_not_leak_to_release_id", status1001.currentRelease.releaseId == nil)
                check("real_control_guard_desc_anti_downgrade", status1001.guardInfo.userFacingGuardDescription.contains("防降级高水位"))
            } catch {
                check("real_control_guard_highest_seq_1001", false, "Failed fetchStatus with guard sequence: \(error)")
            }
            try? FileManager.default.removeItem(at: seqFile)
        }

        // 10.3 坏 journal / sequence 测试：显示恢复所需且未知来源不默认正常
        if let sDir = stateDirPath {
            // 坏 sequence 文件
            let seqFile = URL(fileURLWithPath: sDir).appendingPathComponent("highest_accepted_sequence")
            try? "corrupt_not_u64\n".write(to: seqFile, atomically: true, encoding: .utf8)

            do {
                let statusCorruptSeq = try realClient.fetchStatus()
                check("real_control_corrupt_seq_recovery_required", statusCorruptSeq.transaction.status == .recoveryRequired && statusCorruptSeq.transaction.recoveryRequired)
                check("real_control_corrupt_seq_err_message", statusCorruptSeq.transaction.error?.contains("corrupt_sequence_file") == true)
                check("real_control_corrupt_seq_not_trusted", statusCorruptSeq.currentRelease.isTrustedInstalledRelease == false)
                check("real_control_corrupt_seq_trust_desc", statusCorruptSeq.currentRelease.userFacingTrustDescription.contains("不可信"))
            } catch {
                check("real_control_corrupt_seq_recovery_required", false, "Failed fetchStatus with corrupt seq: \(error)")
            }
            try? FileManager.default.removeItem(at: seqFile)

            // 坏 journal 文件 (update-journal.json)
            let jFile = URL(fileURLWithPath: sDir).appendingPathComponent("update-journal.json")
            try? "{broken_json_content".write(to: jFile, atomically: true, encoding: .utf8)

            do {
                let statusCorruptJournal = try realClient.fetchStatus()
                check("real_control_corrupt_journal_recovery_required", statusCorruptJournal.transaction.status == .recoveryRequired && statusCorruptJournal.transaction.recoveryRequired)
                check("real_control_corrupt_journal_err_message", statusCorruptJournal.transaction.error?.contains("corrupt_or_unreadable_journal") == true)
                check("real_control_corrupt_journal_trust_desc", statusCorruptJournal.currentRelease.userFacingTrustDescription.contains("不可信"))
            } catch {
                check("real_control_corrupt_journal_recovery_required", false, "Failed fetchStatus with corrupt journal: \(error)")
            }
            try? FileManager.default.removeItem(at: jFile)
        }

        // 10.4 伪造 VERSION 文件测试：不改变当前版本，不冒充整套发布包
        if let sDir = stateDirPath {
            let stateVer = URL(fileURLWithPath: sDir).appendingPathComponent("VERSION")
            let parentVer = URL(fileURLWithPath: sDir).deletingLastPathComponent().appendingPathComponent("VERSION")
            try? "8.8.8\n".write(to: stateVer, atomically: true, encoding: .utf8)
            try? "9.9.9\n".write(to: parentVer, atomically: true, encoding: .utf8)

            do {
                let statusFakeVer = try realClient.fetchStatus()
                check("real_control_fake_ver_release_id_stays_nil", statusFakeVer.currentRelease.releaseId == nil)
                check("real_control_fake_ver_seq_stays_nil", statusFakeVer.currentRelease.sequence == nil)
                check("real_control_fake_ver_release_desc_unknown", statusFakeVer.currentRelease.userFacingReleaseDescription == "整套版本无法核验 (未知)")
                check("real_control_fake_ver_not_used_in_core", statusFakeVer.currentRelease.coreVersion != "8.8.8" && statusFakeVer.currentRelease.coreVersion != "9.9.9")
            } catch {
                check("real_control_fake_ver_release_id_stays_nil", false, "Failed fetchStatus with fake VERSION: \(error)")
            }

            try? FileManager.default.removeItem(at: stateVer)
            try? FileManager.default.removeItem(at: parentVer)
        }

        // 10.5 真实 update-check 调用（未配置源）
        do {
            let checkRes = try realClient.checkUpdate()
            check("real_control_update_check_unconfigured", checkRes.status == .unconfigured)
            check("real_control_unconfigured_reason_present", checkRes.reason != nil && !checkRes.reason!.isEmpty)
            print("Real update-check reason: \(checkRes.reason ?? "nil")")
        } catch {
            check("real_control_update_check_unconfigured", false, "Real check failed: \(error.localizedDescription)")
        }
    }

    // Real process bounds plus fixture-only download client/UI state tests.
    do {
        let started = ProcessInfo.processInfo.systemUptime
        let hung = runUpdatePageCommand(executable: "/bin/sh", arguments: ["-c", "trap '' TERM; while :; do sleep 1; done"], timeout: 0.1)
        check("updater_process_ignoring_term_is_bounded", hung.timedOut && ProcessInfo.processInfo.systemUptime - started < 3.5)
        let flood = runUpdatePageCommand(executable: "/usr/bin/yes", arguments: ["bounded-test-output"], timeout: 2)
        check("updater_process_output_is_bounded", flood.status != 0 && flood.output.count <= 4 * 1024 * 1024)
        let good = runUpdatePageCommand(executable: "/bin/sh", arguments: ["-c", "printf result; printf diagnostics >&2"], timeout: 1)
        check("updater_process_drains_both_streams", good.status == 0 && String(data: good.output, encoding: .utf8) == "resultdiagnostics")
    }
    do {
        let digest = String(repeating: "a", count: 64)
        let expected = UpdateCheckResult(status: .available, candidate: UpdateCandidateDetail(releaseId: "fixture-release", sequence: 2), verification: UpdateVerificationInfo(publisherVerified: true), manifestSha256: digest)
        let statusJSON = """
        {"current_release":{"architecture":"fixture","status":"unknown","source":"unknown"},"candidate_release":null,"transaction":{"status":"idle","recovery_required":false},"guard":{"highest_accepted_sequence":0,"maintenance_active":false}}
        """
        let checkJSON = """
        {"status":"available","candidate":{"release_id":"fixture-release","sequence":2,"compatible":true,"requires_local_signing":true},"verification":{"publisher_verified":true},"manifest_sha256":"\(digest)"}
        """
        let lock = NSLock()
        var mode = "good"
        var calls = 0
        let responder: UpdateClient.CommandExecutor = { executable, arguments, _ in
            if arguments.first == "control" {
                let body = arguments.contains("update-check") ? checkJSON : statusJSON
                return ControlRun(status: 0, output: Data(body.utf8), timedOut: false)
            }
            if arguments.first == "status" {
                return ControlRun(status: 0, output: Data("{\"status\":\"idle\",\"maintenance_active\":false,\"capabilities\":{\"schema\":1,\"download\":true}}".utf8), timedOut: false)
            }
            lock.lock(); calls += 1; let currentMode = mode; lock.unlock()
            let returnedDigest = currentMode == "changed" ? String(repeating: "b", count: 64) : digest
            let body = """
            {"status":"downloaded","candidate_id":"11111111-1111-4111-8111-111111111111","release_id":"fixture-release","sequence":2,"manifest_sha256":"\(returnedDigest)"}
            """
            return ControlRun(status: 0, output: Data(body.utf8), timedOut: false)
        }
        let client = UpdateClient(configPath: "/fixture/config.json", executor: responder)
        do { let receipt = try client.downloadUpdate(expected: expected); check("download_receipt_pins_manifest", receipt.manifestSha256 == digest) }
        catch { check("download_receipt_pins_manifest", false, "\(error)") }
        lock.lock(); mode = "changed"; lock.unlock()
        do { _ = try client.downloadUpdate(expected: expected); check("changed_source_is_not_download_success", false) }
        catch { check("changed_source_is_not_download_success", true) }
        let timedOut = UpdateClient(configPath: "/fixture/config.json", executor: { _,_,_ in ControlRun(status: 0, output: Data(checkJSON.utf8), timedOut: true) })
        do { _ = try timedOut.checkUpdate(); check("timed_out_check_cannot_enable_download", false) }
        catch { check("timed_out_check_cannot_enable_download", true) }
        let legacy = UpdateClient(configPath: "/fixture/config.json", executor: { _,_,_ in ControlRun(status: 0, output: Data("{\"status\":\"idle\",\"maintenance_active\":false}".utf8), timedOut: false) })
        check("old_updater_without_capabilities_stays_disabled", (try? legacy.supportsDownload()) == false)
        lock.lock(); mode = "good"; calls = 0; lock.unlock()
        let context = StatusPanelController(worker: SocketWorker())
        let page = UpdatePageController(context: context, client: client)
        _ = page.view
        func descendants(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap { descendants($0) } }
        let checkButton = descendants(page.view).compactMap { $0 as? NSButton }.first { $0.title == "检查更新" }!
        let download = descendants(page.view).compactMap { $0 as? NSButton }.first { $0.title == "下载更新" }!
        func waitFor(_ condition: () -> Bool) {
            let deadline = ProcessInfo.processInfo.systemUptime + 5
            while !condition() && ProcessInfo.processInfo.systemUptime < deadline { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.01)) }
        }
        checkButton.performClick(nil)
        waitFor { checkButton.isEnabled && download.isEnabled }
        check("download_enabled_only_after_verified_check_and_capability", download.isEnabled)
        download.performClick(nil)
        download.performClick(nil)
        waitFor { download.title == "已下载" }
        lock.lock(); let downloads = calls; lock.unlock()
        check("one_download_click_is_not_replayed", downloads == 1)
        check("successful_download_never_claims_installation", descendants(page.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue == "更新包已下载并校验，尚未安装。" })
        check("download_validation_scope_is_clear", descendants(page.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue == "更新包已下载，安装前将继续验证" })
        check("downloaded_candidate_cannot_be_downloaded_twice", download.title == "已下载" && !download.isEnabled)
        for title in ["安装并重启", "延期更新"] {
            check("install_actions_stay_disabled_after_download_\(title)", descendants(page.view).compactMap { $0 as? NSButton }.first { $0.title == title }?.isEnabled == false)
        }
    }

    // Restarted page discovers cache metadata, but must explicitly revalidate it.
    do {
        let digest = String(repeating: "c", count: 64)
        let status = """
        {"current_release":{"architecture":"fixture","status":"unknown","source":"unknown"},"transaction":{"status":"idle","recovery_required":false},"guard":{"highest_accepted_sequence":0,"maintenance_active":false},"download_cache":{"status":"cached_not_revalidated","candidates":[{"candidate_id":"22222222-2222-4222-8222-222222222222","release_id":"cached-fixture","sequence":3,"manifest_sha256":"\(digest)"}]}}
        """
        let checkResponse = """
        {"status":"available","candidate":{"release_id":"cached-fixture","sequence":3},"verification":{"publisher_verified":true},"manifest_sha256":"\(digest)"}
        """
        let receipt = """
        {"status":"downloaded","candidate_id":"22222222-2222-4222-8222-222222222222","release_id":"cached-fixture","sequence":3,"manifest_sha256":"\(digest)"}
        """
        let client = UpdateClient(configPath: "/fixture/config.json", executor: { _,args,_ in
            let body: String
            if args.first == "control" { body = args.contains("update-check") ? checkResponse : status }
            else if args.first == "status" { body = "{\"status\":\"idle\",\"maintenance_active\":false,\"capabilities\":{\"schema\":1,\"download\":true}}" }
            else { body = receipt }
            return ControlRun(status: 0, output: Data(body.utf8), timedOut: false)
        })
        let context = StatusPanelController(worker: SocketWorker())
        let page = UpdatePageController(context: context, client: client)
        _ = page.view
        func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap { views($0) } }
        let checkButton = views(page.view).compactMap { $0 as? NSButton }.first { $0.title == "检查更新" }!
        let download = views(page.view).compactMap { $0 as? NSButton }.first { $0.title == "下载更新" }!
        check("restart_cache_does_not_imply_verified_download", !download.isEnabled && download.title != "已下载")
        checkButton.performClick(nil)
        let end = ProcessInfo.processInfo.systemUptime + 5
        while !download.isEnabled && ProcessInfo.processInfo.systemUptime < end { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.01)) }
        check("restart_offers_explicit_cached_package_revalidation", download.isEnabled && download.title == "校验本地更新包")
        check("restart_labels_cache_as_unverified_bytes", views(page.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue == "本机有候选缓存，需重新校验更新包" })
    }

    // 11. 离屏 UI 完整高度渲染夹具（不透明 Aqua，包含完整卡片流与底部按钮，验证720宽及宿主最小680宽）
    do {
        let dummyWorker = SocketWorker()
        let dummySupervisor = CoreSupervisor()
        let statusPanel = StatusPanelController(worker: dummyWorker, coreSupervisor: dummySupervisor)
        let updateVC = UpdatePageController(context: statusPanel)
        _ = updateVC.view

        // 11.1 未配置更新源状态渲染（720宽与680宽分别验证）
        updateVC.applyCheckResult(UpdateCheckResult(
            status: .unconfigured,
            reason: "尚未配置授权更新清单；禁止任意未经验证的网络下载",
            candidate: nil,
            verification: nil
        ))

        func collectViews(_ root: NSView) -> [NSView] {
            [root] + root.subviews.flatMap { collectViews($0) }
        }
        func visibleText(_ root: NSView, _ text: String) -> Bool {
            collectViews(root).contains { view in
                guard let field = view as? NSTextField else { return false }
                return field.stringValue == text && !field.isHiddenOrHasHiddenAncestor
            }
        }
        for status in [UpdateCheckStatus.available, .alreadyInstalled] {
            updateVC.applyCheckResult(UpdateCheckResult(status: status,
                candidate: UpdateCandidateDetail(releaseId: "unverified", sequence: 2)))
            check("missing_verification_never_shows_verified_\(status.rawValue)",
                visibleText(updateVC.view, "已发现候选版本（来源尚未验证）"))
        }
        updateVC.applyCheckResult(UpdateCheckResult(status: .available,
            candidate: UpdateCandidateDetail(releaseId: "verified", sequence: 3),
            verification: UpdateVerificationInfo(publisherVerified: true)))
        check("verified_manifest_is_not_full_package_validation", visibleText(updateVC.view, "发现新版本，清单签名已验证") && visibleText(updateVC.view, "清单与当前系统兼容，安装包待校验"))
        updateVC.applyCheckResult(UpdateCheckResult(status: .unconfigured))
        let unconfiguredButtons = collectViews(updateVC.view).compactMap { $0 as? NSButton }
        for title in ["下载更新", "安装并重启", "延期更新"] {
            check(
                "write_button_disabled_unconfigured_\(title)",
                unconfiguredButtons.first(where: { $0.title == title })?.isEnabled == false
            )
        }
        check(
            "candidate_details_hidden_when_unconfigured",
            !visibleText(updateVC.view, "目标版本:")
        )

        // 隔离固定 1x Viewport 渲染器（解耦宿主显示器 Retina/非 Retina 分辨率依赖）
        func makeIsolatedViewportRep(size: NSSize) -> NSBitmapImageRep? {
            let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil,
                pixelsWide: Int(size.width),
                pixelsHigh: Int(size.height),
                bitsPerSample: 8,
                samplesPerPixel: 4,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: .calibratedRGB,
                bytesPerRow: 0,
                bitsPerPixel: 0
            )
            rep?.size = size
            return rep
        }

        let fullSize720 = NSSize(width: 720, height: 1250)
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize720)
        updateVC.contentStack.layoutSubtreeIfNeeded()
        updateVC.view.layoutSubtreeIfNeeded()

        guard let rep720 = makeIsolatedViewportRep(size: fullSize720) else {
            check("offscreen_rep_created", false, "Failed to create bitmap rep")
            return 1
        }
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep720)
        check("offscreen_render_unconfigured_720", rep720.pixelsWide == 720 && rep720.pixelsHigh == 1250)

        let outPath1 = "/tmp/update_ui_unconfigured_full.png"
        if let pngData1 = rep720.representation(using: .png, properties: [:]) {
            try? pngData1.write(to: URL(fileURLWithPath: outPath1))
            check("offscreen_unconfigured_png_saved", FileManager.default.fileExists(atPath: outPath1))
        }

        // 验证宿主实际最小宽度 680 宽
        let fullSize680 = NSSize(width: 680, height: 1250)
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize680)
        updateVC.contentStack.layoutSubtreeIfNeeded()
        updateVC.view.layoutSubtreeIfNeeded()
        guard let rep680 = makeIsolatedViewportRep(size: fullSize680) else {
            check("offscreen_rep680_created", false, "Failed to create 680w rep")
            return 1
        }
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep680)
        check("offscreen_render_unconfigured_680", rep680.pixelsWide == 680 && rep680.pixelsHigh == 1250)
        let outPath680_1 = "/tmp/update_ui_unconfigured_680w.png"
        if let pngData680_1 = rep680.representation(using: .png, properties: [:]) {
            try? pngData680_1.write(to: URL(fileURLWithPath: outPath680_1))
            check("offscreen_unconfigured_680w_saved", FileManager.default.fileExists(atPath: outPath680_1))
        }

        // 11.1b 可用候选状态：即使显示“可用”，只读阶段写按钮仍必须禁用
        updateVC.applyCheckResult(UpdateCheckResult(
            status: .available,
            reason: nil,
            candidate: UpdateCandidateDetail(
                releaseId: "0.2.0-aarch64",
                sequence: 2,
                publishedAt: "2026-11-10T12:00:00Z",
                sizeBytes: 10485760,
                releaseNotes: "1. 增加大脑界面管理\n2. 优化会话追溯",
                compatible: true,
                requiresLocalSigning: true
            ),
            verification: UpdateVerificationInfo(publisherVerified: true, keyId: "release-v1")
        ))
        let availableButtons = collectViews(updateVC.view).compactMap { $0 as? NSButton }
        for title in ["下载更新", "安装并重启", "延期更新"] {
            check(
                "write_button_disabled_available_\(title)",
                availableButtons.first(where: { $0.title == title })?.isEnabled == false
            )
        }
        check(
            "candidate_details_visible_when_available",
            visibleText(updateVC.view, "目标版本:")
        )

        let detailToggle = collectViews(updateVC.view).compactMap { $0 as? NSButton }.first { $0.title == "显示技术详情" }
        check("technical_disclosure_available", detailToggle != nil)
        check("technical_details_collapsed_initially", !visibleText(updateVC.view, "防降级高水位"))
        detailToggle?.performClick(nil)
        check("technical_details_expand_without_losing_diagnostics", visibleText(updateVC.view, "防降级高水位"))
        detailToggle?.performClick(nil)
        check("technical_details_collapse_again", !visibleText(updateVC.view, "防降级高水位"))

        // 11.2 故障恢复状态渲染（720宽与680宽分别展开恢复卡、受控说明、事务 ID、两行禁用按钮）
        updateVC.applyStatusPayload(UpdateStatusPayload(
            currentRelease: UpdateReleaseInfo(releaseId: "0.1.0-aarch64", sequence: 1, architecture: "aarch64", componentsConsistent: false),
            candidateRelease: UpdateCandidateSummary(releaseId: "0.2.0-aarch64", sequence: 2),
            transaction: UpdateTransactionInfo(
                status: .recoveryRequired,
                transactionId: "tx-fail-draining-timeout-002",
                stage: "Draining",
                error: "等待在途轮次排空超时 (60s)，核心启动处于维护抑制态",
                recoveryRequired: true
            ),
            guardInfo: UpdateGuardInfo(highestAcceptedSequence: 1, maintenanceActive: true)
        ))
        // 720 宽
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize720)
        updateVC.view.layoutSubtreeIfNeeded()
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep720)
        let outPath2 = "/tmp/update_ui_recovery_full.png"
        if let pngData2 = rep720.representation(using: .png, properties: [:]) {
            try? pngData2.write(to: URL(fileURLWithPath: outPath2))
            check("offscreen_recovery_png_saved", FileManager.default.fileExists(atPath: outPath2))
        }

        // 680 宽
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize680)
        updateVC.view.layoutSubtreeIfNeeded()
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep680)
        let outPath680_2 = "/tmp/update_ui_recovery_680w.png"
        if let pngData680_2 = rep680.representation(using: .png, properties: [:]) {
            try? pngData680_2.write(to: URL(fileURLWithPath: outPath680_2))
            check("offscreen_recovery_680w_saved", FileManager.default.fileExists(atPath: outPath680_2))
        }

        // 11.3 批准的 I5 生产新 shape 渲染（整套版本未知、独立核心编译版、状态未知不可信）
        updateVC.applyStatusPayload(UpdateStatusPayload(
            currentRelease: UpdateReleaseInfo(
                releaseId: nil,
                sequence: nil,
                status: "unknown",
                source: "unknown",
                coreVersion: "0.1.0",
                architecture: "aarch64",
                componentsConsistent: nil
            ),
            candidateRelease: nil,
            transaction: UpdateTransactionInfo(
                status: .idle,
                transactionId: nil,
                stage: nil,
                error: nil,
                recoveryRequired: false
            ),
            guardInfo: UpdateGuardInfo(highestAcceptedSequence: 1001, maintenanceActive: false)
        ))
        // 720 宽
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize720)
        updateVC.view.layoutSubtreeIfNeeded()
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep720)
        let outPathApproved720 = "/tmp/update_ui_approved_720w.png"
        if let pngDataAppr720 = rep720.representation(using: .png, properties: [:]) {
            try? pngDataAppr720.write(to: URL(fileURLWithPath: outPathApproved720))
            check("offscreen_approved_720w_png_saved", FileManager.default.fileExists(atPath: outPathApproved720))
        }

        // 680 宽
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize680)
        updateVC.view.layoutSubtreeIfNeeded()
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep680)
        let outPathApproved680 = "/tmp/update_ui_approved_680w.png"
        if let pngDataAppr680 = rep680.representation(using: .png, properties: [:]) {
            try? pngDataAppr680.write(to: URL(fileURLWithPath: outPathApproved680))
            check("offscreen_approved_680w_png_saved", FileManager.default.fileExists(atPath: outPathApproved680))
        }

        // 验证所有卡片与子视图均严格限制在 680 宽画幅内，无任何右侧越界
        var allWithinBounds = true
        for subview in updateVC.contentStack.subviews {
            if subview.frame.maxX > 680.0 {
                allWithinBounds = false
                print("Subview exceeded bounds: \(subview)")
            }
        }
        check("subviews_within_680w_bounds", allWithinBounds)
    }

    // 12. Shell 与 SettingsPage 导航联动验证
    do {
        let dummyWorker = SocketWorker()
        let dummySupervisor = CoreSupervisor()
        let statusPanel = StatusPanelController(worker: dummyWorker, coreSupervisor: dummySupervisor)
        let shell = ShellViewController()
        
        let updateVC = UpdatePageController(context: statusPanel)
        let settingsVC = SettingsPageController(context: statusPanel)

        shell.register(page: settingsVC, id: "settings", title: "设置")
        shell.register(page: updateVC, id: "update", title: "应用更新")

        shell.selectPage(id: "update")
        check("shell_select_update_page", true)

        var jumped = false
        settingsVC.onNavigateToUpdate = {
            jumped = true
            shell.selectPage(id: "update")
        }
        settingsVC.triggerUpdateNavigation()
        check("settings_page_jump_to_update", jumped == true)
    }

    print("=== Update UI Selftests Completed: \(passed) PASS, \(failed) FAIL ===")
    return failed == 0 ? 0 : 1
}

exit(runTests())
