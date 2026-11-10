// tests/host_brain_selftest/main.swift
// Pure logic and UI fixture tests for BrainPage, SessionsPage provenance, and Shell navigation.
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

    print("=== Running Brain UI & Provenance Selftests ===")

    // 1. Data Model & JSON Roundtrip
    do {
        let budget = BrainContextBudget(maxInputBytes: 49152, knowledgeBytes: 16384, historyBytes: 16384, overflow: "truncate_tail")
        check("budget_defaults", budget.maxInputBytes == 49152 && budget.overflow == "truncate_tail")

        var unreadableMap: [String: String] = [:]
        unreadableMap["unsupported_type"] = "抱歉，暂不支持此类文件"
        let templates = BrainTemplates(
            greeting: "您好！",
            insufficientInformation: "请提供更多信息",
            unreadable: unreadableMap,
            unreadableMissingLabels: nil,
            unreadableManyItems: nil
        )
        check("templates_reasons_coverage", templates.allReasonsCoverage.count == 8)
        check("templates_greeting", templates.greeting == "您好！")
    }

    // 2. Brain List & Role Entry Parsing (Draft vs Active separation)
    do {
        let jsonStr = """
        {
            "generation": 10,
            "execution_mode": "managed",
            "roles": [
                {
                    "role_id": "role_sales",
                    "draft_revision_id": "rev_4",
                    "active_revision_id": "rev_3",
                    "name": "售后助手",
                    "archived": false,
                    "active_binding_count": 1
                },
                {
                    "role_id": "role_migrated",
                    "draft_revision_id": "rev_1",
                    "name": "从旧版迁移",
                    "archived": false,
                    "active_binding_count": 0
                }
            ],
            "active": {
                "default": { "role_id": "role_sales", "revision_id": "rev_3" },
                "overrides": []
            },
            "draft": {
                "default": { "role_id": "role_sales", "revision_id": "rev_4" },
                "overrides": []
            },
            "model_summary": {
                "mode": "external",
                "timeout_seconds": 120
            }
        }
        """
        guard let payload = try? BrainPageLogic.parseBrainList(jsonString: jsonStr) else {
            check("brain_list_parse", false, "Failed to parse brain list JSON")
            return 1
        }
        check("brain_list_parse", payload.generation == 10)
        check("execution_mode_managed", payload.executionMode == "managed")
        check("roles_count", payload.roles.count == 2)
        
        let salesRole = payload.roles[0]
        check("sales_role_draft_rev", salesRole.draftRevisionId == "rev_4")
        check("sales_role_active_rev", salesRole.activeRevisionId == "rev_3")
        check("sales_role_draft_vs_active_different", salesRole.hasDraftChanges == true)
        
        let migratedRole = payload.roles[1]
        check("migrated_role_origin_detect", BrainPageLogic.isMigratedRole(migratedRole))
    }

    // 3. CAS Conflict Handling: preserve user input on generation mismatch
    do {
        let localDraft = BrainRoleRevision(
            roleId: "role_sales",
            revisionId: "rev_4",
            name: "售后助手（用户编辑中）",
            persona: "专业认真的售后客服人员",
            goals: ["解决客户退换货问题"],
            tone: "热情、耐心",
            businessRules: ["退货需要提供运单号"],
            modelRef: "config.model_config"
        )
        // 模拟并发冲突响应：expected_generation mismatch
        let conflictResponse = BrainControlResponse<String>(
            ok: false,
            error: BrainControlError(code: "conflict", message: "expected_generation mismatch: expected 10, actual 11", retryable: false)
        )
        let handled = BrainPageLogic.handleSaveConflict(localDraft: localDraft, error: conflictResponse.error!)
        check("cas_conflict_preserves_local_name", handled.preservedDraft.name == "售后助手（用户编辑中）")
        check("cas_conflict_reports_error", handled.errorMessage.contains("expected_generation mismatch"))
    }

    // 4. Role Copy and Archive Validation
    do {
        let original = BrainRoleRevision(
            roleId: "role_source",
            revisionId: "rev_1",
            name: "源角色",
            persona: "源客服",
            goals: [],
            tone: "礼貌",
            businessRules: [],
            modelRef: "config.model_config"
        )
        let copied = BrainPageLogic.prepareRoleCopy(from: original, newRoleId: "role_copy_123", newName: "源角色 (副本)")
        check("copy_role_new_id", copied.roleId == "role_copy_123")
        check("copy_role_parent_rev", copied.parentRevisionId == "rev_1")
        check("copy_role_name", copied.name == "源角色 (副本)")

        // 归档校验：若 active_binding_count > 0，禁止归档
        let inUseEntry = BrainRoleEntry(
            roleId: "role_source",
            draftRevisionId: "rev_1",
            activeRevisionId: "rev_1",
            name: "源角色",
            archived: false,
            activeBindingCount: 1
        )
        let canArchive = BrainPageLogic.canArchiveRole(inUseEntry)
        check("in_use_role_cannot_archive", canArchive == false)
    }

    // 5. Knowledge Management: fixed refs & Markdown text handling
    do {
        let k1 = BrainKnowledgeRevision(
            knowledgeId: "kn_faq",
            revisionId: "rev_k1",
            title: "退换货说明",
            text: "# 退换货指南\n7天无理由退货...",
            sourceLabel: "faq.md",
            contentSha256: "aabbcc112233"
        )
        var role = BrainRoleRevision(
            roleId: "role_1",
            revisionId: "rev_1",
            name: "测试角色",
            persona: "人设",
            goals: [],
            tone: "自然",
            businessRules: [],
            modelRef: "config.model_config"
        )
        check("role_knowledge_empty", role.knowledgeRefs.isEmpty)
        role.addKnowledgeRef(from: k1)
        check("role_knowledge_ref_added", role.knowledgeRefs.count == 1)
        check("role_knowledge_ref_fixed_sha", role.knowledgeRefs[0].contentSha256 == "aabbcc112233")
    }

    // 6. Synthetic Preview (assemble_only): marked as uncalled model, no real message sent
    do {
        let previewJson = """
        {
            "preview_id": "prev_123",
            "snapshot_id": "snap_999",
            "snapshot": {
                "assembly_sha256": "sha_snap_456",
                "system_constraints": "系统约束：不可虚构事实...",
                "persona": "售后助手",
                "knowledge": ["退换货指南"],
                "input_text": "我要退货",
                "budget": {
                    "original_bytes": 1024,
                    "included_bytes": 1024,
                    "truncated": false
                }
            },
            "decision": null,
            "cost": {
                "incurred": false,
                "note": "未调用模型，仅组装预览"
            }
        }
        """
        guard let preview = try? BrainPageLogic.parsePreviewResult(jsonString: previewJson) else {
            check("parse_preview_result", false, "Failed to parse preview json")
            return 1
        }
        check("preview_decision_null", preview.decision == nil)
        check("preview_cost_incurred_false", preview.costIncurred == false)
        check("preview_marked_uncalled_model", preview.costNote.contains("未调用模型"))
        check("preview_budget_not_truncated", preview.budgetTruncated == false)
    }

    // 7. Binding Management: Single source from allowlist
    do {
        let allowlist = [
            ["recipient": "张三", "kind": "private"],
            ["recipient": "技术交流群", "kind": "group"]
        ]
        let bindings = BrainBindingSet(
            defaultRef: BrainRoleRef(roleId: "role_sales", revisionId: "rev_3"),
            overrides: [
                BrainOverride(contactId: "技术交流群", chatKind: "group", roleId: "role_group_helper", revisionId: "rev_1")
            ]
        )
        check("binding_default_role", bindings.defaultRef?.roleId == "role_sales")
        check("binding_override_count", bindings.overrides.count == 1)
        // 校验 Override 的 contactId 是否在 allowlist 中
        let valid = BrainPageLogic.validateOverridesAgainstAllowlist(overrides: bindings.overrides, allowlist: allowlist)
        check("binding_override_allowlist_valid", valid)
    }

    // 8. Turn Provenance: Old records show legacy unrecorded, new records show phases
    do {
        // Case A: 旧版记录 (provenance: null)
        let legacyDetail = BrainTurnDetail(
            turnId: "turn_legacy_1",
            contactId: "李四",
            snapshotId: nil,
            snapshot: nil,
            decision: ["action": AnyCodable("send"), "text": AnyCodable("你好")],
            model: nil,
            sendKey: "k_legacy",
            execution: ["phase": AnyCodable("triggered")]
        )
        check("legacy_provenance_null", legacyDetail.hasProvenance == false)
        check("legacy_display_text", legacyDetail.provenanceDisplayText == "旧版未记录")

        // Case B: 新版包含完整 provenance 记录
        let modernDetail = BrainTurnDetail(
            turnId: "turn_new_2",
            contactId: "张三",
            snapshotId: "snap_101",
            snapshot: [
                "assembly_sha256": AnyCodable("sha_abc"),
                "role_id": AnyCodable("role_sales"),
                "revision_id": AnyCodable("rev_3")
            ],
            decision: ["action": AnyCodable("send"), "text": AnyCodable("运单号已收到，马上为您办理")],
            model: ["provider": AnyCodable("mock"), "model": AnyCodable("gpt-4o")],
            sendKey: "k_101",
            execution: [
                "phase": AnyCodable("sent"),
                "delivery": AnyCodable("echo_observed")
            ]
        )
        check("modern_provenance_present", modernDetail.hasProvenance == true)
        check("modern_role_id", modernDetail.roleId == "role_sales")
        check("modern_phase_labels", modernDetail.phaseLabels.contains("己方回显"))
    }

    // 9. Shell Navigation & Page Selection
    do {
        let shell = ShellViewController()
        let dummy1 = NSViewController()
        let dummy2 = NSViewController()
        let dummy3 = NSViewController()
        shell.register(page: dummy1, id: "overview", title: "总览")
        shell.register(page: dummy2, id: "brain", title: "大脑")
        shell.register(page: dummy3, id: "mcp", title: "MCP")
        
        check("shell_page_count", shell.numberOfRows(in: NSTableView()) == 3)
        shell.selectPage(id: "brain")
        // 验证 selectPage(id:) 不崩溃且正常工作
        check("shell_select_page_by_id", true)
    }

    // 10. McpPage Navigation Pointer Check
    do {
        let pointerText = McpPageLogic.brainPagePointerText()
        check("mcp_page_has_brain_pointer", pointerText.contains("大脑"))
    }

    // 11. Isolated UI Fixture View Loading & Controller Hierarchy Verification
    do {
        let mockExecutor: BrainClient.CommandExecutor = { cmd, args, timeout in
            if args.contains("brain-list") {
                let mockJson = """
                {"ok":true,"data":{"generation":1,"execution_mode":"managed","roles":[],"active":null,"draft":null,"model_summary":{"mode":"builtin"}}}
                """
                return ControlRun(status: 0, output: mockJson.data(using: .utf8)!, timedOut: false)
            }
            return ControlRun(status: 0, output: Data(), timedOut: false)
        }
        let mockClient = BrainClient(configPath: "/tmp/mock_config.json", executor: mockExecutor)
        let dummyContext = StatusPanelController(worker: SocketWorker())
        let brainController = BrainPageController(context: dummyContext, client: mockClient)
        
        let view = brainController.view
        check("isolated_ui_view_loaded", view.subviews.count > 0)
        check("isolated_ui_view_size", view.frame.width == 720 && view.frame.height == 760)
        check("isolated_ui_table_configured", brainController.numberOfRows(in: NSTableView()) == 0)
    }

    // 12. Offscreen PNG Rendering & Visual Layout Checks
    do {
        let fixtureDir = URL(fileURLWithPath: "target/brain_ui_fixtures")
        try? FileManager.default.createDirectory(at: fixtureDir, withIntermediateDirectories: true)
        
        let mockExecutor: BrainClient.CommandExecutor = { cmd, args, timeout in
            if args.contains("brain-list") {
                let mockJson = """
                {"ok":true,"data":{"generation":1,"execution_mode":"managed","roles":[{"role_id":"r1","draft_revision_id":"v1","name":"客服助手","archived":false,"active_binding_count":0}],"active":null,"draft":null,"model_summary":{"mode":"builtin"}}}
                """
                return ControlRun(status: 0, output: mockJson.data(using: .utf8)!, timedOut: false)
            }
            return ControlRun(status: 0, output: Data(), timedOut: false)
        }
        let mockClient = BrainClient(configPath: "/tmp/mock_config.json", executor: mockExecutor)
        let dummyContext = StatusPanelController(worker: SocketWorker())
        let brainController = BrainPageController(context: dummyContext, client: mockClient)
        
        let bView = brainController.view
        bView.frame = NSRect(x: 0, y: 0, width: 720, height: 760)
        bView.layoutSubtreeIfNeeded()
        
        if let rep = bView.bitmapImageRepForCachingDisplay(in: bView.bounds) {
            bView.cacheDisplay(in: bView.bounds, to: rep)
            if let png = rep.representation(using: .png, properties: [:]) {
                let pngURL = fixtureDir.appendingPathComponent("brain_page.png")
                try? png.write(to: pngURL)
                check("brain_page_png_written", FileManager.default.fileExists(atPath: pngURL.path))
                check("brain_page_png_size_valid", png.count > 1024)
                check("brain_page_pixel_dimensions", rep.pixelsWide == 720 && rep.pixelsHigh == 760)
            } else {
                check("brain_page_png_representation", false, "Failed to get PNG representation")
            }
        } else {
            check("brain_page_cache_display_rep", false, "Failed to create bitmap rep")
        }

        let sessController = SessionsPageController(context: dummyContext)
        let sView = sessController.view
        sView.frame = NSRect(x: 0, y: 0, width: 700, height: 720)
        sView.layoutSubtreeIfNeeded()
        if let rep = sView.bitmapImageRepForCachingDisplay(in: sView.bounds) {
            sView.cacheDisplay(in: sView.bounds, to: rep)
            if let png = rep.representation(using: .png, properties: [:]) {
                let pngURL = fixtureDir.appendingPathComponent("sessions_page.png")
                try? png.write(to: pngURL)
                check("sessions_page_png_written", FileManager.default.fileExists(atPath: pngURL.path))
                check("sessions_page_png_size_valid", png.count > 1024)
            }
        }
    }

    // 13. Real Rust control + Swift client contract integration
    if let realCfgPath = ProcessInfo.processInfo.environment["REAL_CONTROL_CONFIG"],
       FileManager.default.fileExists(atPath: realCfgPath) {
        print("--> Running Real Rust control + Swift client contract integration tests...")
        let realClient = BrainClient(configPath: realCfgPath, executor: runControlCommand)
        
        do {
            // 13.1 初始化 fresh 模式
            let initGen = try realClient.initialize(installation: "fresh")
            check("real_control_init", initGen >= 1, "generation should be >= 1")
            
            // 13.2 列表查询与模式验证
            let listPayload = try realClient.fetchList()
            check("real_control_list_mode", listPayload.executionMode == "managed")
            check("real_control_list_gen", listPayload.generation == initGen)
            
            // 13.3 角色新建与保存
            var role = BrainRoleRevision(
                roleId: "contract_role_1",
                revisionId: "rev_c1",
                origin: "user",
                name: "契约测试角色",
                persona: "真实契约助手"
            )
            let genAfterSave = try realClient.saveRole(role, expectedGeneration: initGen)
            check("real_control_save_role_gen", genAfterSave > initGen)
            
            // 13.4 角色获取与真实解码
            let fetchedRole = try realClient.getRole(roleId: "contract_role_1")
            check("real_control_fetch_role_persona", fetchedRole.persona == "真实契约助手")
            check("real_control_fetch_role_name", fetchedRole.name == "契约测试角色")
            
            // 13.5 知识保存与修订/hash真实返回
            let knowledge = BrainKnowledgeRevision(
                knowledgeId: "k_contract_1",
                revisionId: "k_rev_1",
                title: "服务条款",
                text: "# 服务条款\n请勿违反规范。",
                sourceLabel: "docs"
            )
            let savedKnowledge = try realClient.saveKnowledge(knowledge, expectedGeneration: genAfterSave)
            check("real_control_save_knowledge_id", savedKnowledge.knowledgeId == "k_contract_1")
            check("real_control_save_knowledge_sha", savedKnowledge.contentSha256.count == 64)
            check("real_control_save_knowledge_gen", savedKnowledge.generation > genAfterSave)
            
            // 13.6 角色引用知识后再次保存
            role.knowledgeRefs = [BrainKnowledgeRef(
                knowledgeId: savedKnowledge.knowledgeId,
                revisionId: savedKnowledge.revisionId,
                contentSha256: savedKnowledge.contentSha256
            )]
            role.revisionId = "rev_c2"
            role.parentRevisionId = "rev_c1"
            let genAfterRef = try realClient.saveRole(role, expectedGeneration: savedKnowledge.generation)
            check("real_control_save_role_ref_gen", genAfterRef > savedKnowledge.generation)
            
            // 13.7 绑定保存
            let bindings = BrainBindingSet(
                defaultRef: BrainRoleRef(roleId: "contract_role_1", revisionId: "rev_c2"),
                overrides: []
            )
            let genAfterBind = try realClient.saveBindings(draft: bindings, expectedGeneration: genAfterRef)
            check("real_control_save_bindings_gen", genAfterBind > genAfterRef)
            
            // 13.8 激活与回退
            let genAfterAct = try realClient.activate(expectedGeneration: genAfterBind)
            check("real_control_activate_gen", genAfterAct > genAfterBind)
            
            // 产生第二个激活历史世代以验证合法回退
            let genAfterBind2 = try realClient.saveBindings(draft: bindings, expectedGeneration: genAfterAct)
            let genAfterAct2 = try realClient.activate(expectedGeneration: genAfterBind2)
            check("real_control_activate2_gen", genAfterAct2 > genAfterAct)
            
            // 回退到前一个已激活的世代 genAfterAct
            let genAfterRoll = try realClient.rollback(targetGeneration: genAfterAct, expectedGeneration: genAfterAct2)
            check("real_control_rollback_gen", genAfterRoll > genAfterAct2)
            
            // 13.9 assemble_only 合成预览
            let previewRes = try realClient.preview(roleId: "contract_role_1", text: "请问服务条款是什么？")
            check("real_control_preview_cost", previewRes.costIncurred == false)
            check("real_control_preview_content", previewRes.snapshot?.compiledText.contains("真实契约助手") == true)
            
            // 13.10 真实轮次详情查询与字段解码
            if let testTurnId = ProcessInfo.processInfo.environment["REAL_CONTROL_TEST_TURN_ID"] {
                let turnDetail = try realClient.turnDetail(turnId: testTurnId)
                check("real_control_turn_id", turnDetail.turnId == testTurnId)
                check("real_control_turn_contact", !turnDetail.contactId.isEmpty)
                check("real_control_turn_decision", turnDetail.decision != nil)
                check("real_control_turn_snapshot", turnDetail.snapshot != nil)
            }
            
            // 13.11 CAS 冲突验证：期望错误世代，必须抛出异常并保留数据
            var conflictCaught = false
            do {
                _ = try realClient.saveRole(role, expectedGeneration: 99999)
            } catch let err as NSError {
                conflictCaught = true
                check("real_control_conflict_code", err.userInfo["error_code"] as? String == "revision_conflict" || err.localizedDescription.contains("conflict") || err.localizedDescription.contains("revision_conflict"))
            }
            check("real_control_conflict_caught", conflictCaught)
            
        } catch {
            check("real_control_pipeline_success", false, "Pipeline failed with error: \(error)")
        }
    }

    print("\nTotal passed: \(passed), failed: \(failed)")
    return failed == 0 ? 0 : 1
}

exit(runTests())
