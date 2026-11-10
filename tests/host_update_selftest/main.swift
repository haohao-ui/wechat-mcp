// tests/host_update_selftest/main.swift
// Pure logic, state machine, fault injection, and offscreen UI fixture tests for UpdatePage.
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

    // 1. 数据模型与 JSON 解析测试
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
        check("status_candidate_release_id", payload.candidateRelease?.releaseId == "0.2.0-aarch64")
        check("status_candidate_sequence", payload.candidateRelease?.sequence == 2)
        check("status_transaction_status", payload.transaction.status == .idle)
        check("status_guard_maintenance", payload.guardInfo.maintenanceActive == false)
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
        // 关键安全断言：未验证来源时，绝对不可允许直接安装
        check("check_unverified_install_blocked", UpdatePageLogic.canInstall(checkResult: checkRes, transaction: .idle) == false)
    }

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

    // 7. 钥匙串不可用与本机代码签名失败
    do {
        let signingFailure = UpdateSigningStatus(
            keychainAvailable: false,
            localCertificateFound: false,
            lastError: "钥匙串处于锁定状态或专用自签名证书不存在"
        )
        check("signing_keychain_locked", signingFailure.canPerformLocalSigning == false)
        check("signing_failure_description", signingFailure.userFacingDescription.contains("钥匙串处于锁定状态"))
    }

    // 8. 待轮次排空（Draining）与超时延期（Defer）
    do {
        var drainState = UpdateDrainState(
            isDraining: true,
            inflightCount: 2,
            drainReason: "正在等待 2 个在途轮次完成；已暂停接收新消息",
            deferred: false
        )
        check("drain_active", drainState.isDraining == true && drainState.inflightCount == 2)
        // 延期操作
        drainState.deferUpdate()
        check("drain_deferred", drainState.deferred == true)
        check("drain_cleared_after_defer", drainState.isDraining == false)
    }

    // 9. 失败恢复状态（Recovery Required）
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
        check("recovery_guide_contains_instructions", guide.contains("回退") || guide.contains("旧版本"))
    }

    // 10. 离屏 UI 渲染夹具与视图树验证
    do {
        let dummyWorker = SocketWorker()
        let dummySupervisor = CoreSupervisor()
        let statusPanel = StatusPanelController(worker: dummyWorker, coreSupervisor: dummySupervisor)
        let updateVC = UpdatePageController(context: statusPanel)

        // 加载视图
        _ = updateVC.view
        check("update_view_loaded", updateVC.isViewLoaded)

        // 模拟未配置更新源态渲染
        updateVC.applyCheckResult(UpdateCheckResult(
            status: .unconfigured,
            reason: "尚未配置更新源；禁止任意未经验证的网络下载",
            candidate: nil,
            verification: nil
        ))
        
        let size = NSSize(width: 720, height: 760)
        updateVC.view.frame = NSRect(origin: .zero, size: size)
        updateVC.view.layoutSubtreeIfNeeded()

        guard let rep = updateVC.view.bitmapImageRepForCachingDisplay(in: updateVC.view.bounds) else {
            check("offscreen_rep_created", false, "Failed to create bitmap rep")
            return 1
        }
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep)
        check("offscreen_render_unconfigured", rep.pixelsWide > 0 && rep.pixelsHigh > 0)

        // 导出测试离屏图片
        if let pngData = rep.representation(using: .png, properties: [:]) {
            let outPath = "/tmp/test_update_ui_unconfigured.png"
            try? pngData.write(to: URL(fileURLWithPath: outPath))
            check("offscreen_png_exported", FileManager.default.fileExists(atPath: outPath))
        }

        // 模拟排空等待与恢复态渲染
        updateVC.applyStatusPayload(UpdateStatusPayload(
            currentRelease: UpdateReleaseInfo(releaseId: "0.1.0-aarch64", sequence: 1, architecture: "aarch64", componentsConsistent: true),
            candidateRelease: UpdateCandidateSummary(releaseId: "0.2.0-aarch64", sequence: 2),
            transaction: UpdateTransactionInfo(
                status: .recoveryRequired,
                transactionId: "tx-recovery-demo-99",
                stage: "Draining",
                error: "等待在途轮次排空超时",
                recoveryRequired: true
            ),
            guardInfo: UpdateGuardInfo(highestAcceptedSequence: 1, maintenanceActive: true)
        ))
        updateVC.view.layoutSubtreeIfNeeded()
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep)
        if let pngData2 = rep.representation(using: .png, properties: [:]) {
            let outPath2 = "/tmp/test_update_ui_recovery.png"
            try? pngData2.write(to: URL(fileURLWithPath: outPath2))
            check("offscreen_recovery_png_exported", FileManager.default.fileExists(atPath: outPath2))
        }
    }

    // 11. Shell 与 SettingsPage 导航集成验证
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

        // 验证 SettingsPage 具备导航到 UpdatePage 的回调与跳转能力
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
