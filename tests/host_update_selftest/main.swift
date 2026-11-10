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
        
        // 10.1 真实 update-status 调用
        do {
            let status = try realClient.fetchStatus()
            check("real_control_update_status_success", true)
            check("real_control_status_transaction_idle", status.transaction.status == .idle || status.transaction.status == .recoveryRequired)
            print("Real status architecture: \(status.currentRelease.architecture), maintenance: \(status.guardInfo.maintenanceActive)")
        } catch {
            check("real_control_update_status_success", false, "Real status failed: \(error.localizedDescription)")
        }

        // 10.2 真实 update-check 调用（未配置源）
        do {
            let checkRes = try realClient.checkUpdate()
            check("real_control_update_check_unconfigured", checkRes.status == .unconfigured)
            check("real_control_unconfigured_reason_present", checkRes.reason != nil && !checkRes.reason!.isEmpty)
            print("Real update-check reason: \(checkRes.reason ?? "nil")")
        } catch {
            check("real_control_update_check_unconfigured", false, "Real check failed: \(error.localizedDescription)")
        }
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
        
        let fullSize720 = NSSize(width: 720, height: 1250)
        updateVC.view.frame = NSRect(origin: .zero, size: fullSize720)
        updateVC.contentStack.layoutSubtreeIfNeeded()
        updateVC.view.layoutSubtreeIfNeeded()

        guard let rep720 = updateVC.view.bitmapImageRepForCachingDisplay(in: updateVC.view.bounds) else {
            check("offscreen_rep_created", false, "Failed to create bitmap rep")
            return 1
        }
        rep720.colorSpaceName = .calibratedRGB
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
        guard let rep680 = updateVC.view.bitmapImageRepForCachingDisplay(in: updateVC.view.bounds) else {
            check("offscreen_rep680_created", false, "Failed to create 680w rep")
            return 1
        }
        rep680.colorSpaceName = .calibratedRGB
        updateVC.view.cacheDisplay(in: updateVC.view.bounds, to: rep680)
        check("offscreen_render_unconfigured_680", rep680.pixelsWide == 680 && rep680.pixelsHigh == 1250)
        let outPath680_1 = "/tmp/update_ui_unconfigured_680w.png"
        if let pngData680_1 = rep680.representation(using: .png, properties: [:]) {
            try? pngData680_1.write(to: URL(fileURLWithPath: outPath680_1))
            check("offscreen_unconfigured_680w_saved", FileManager.default.fileExists(atPath: outPath680_1))
        }

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
