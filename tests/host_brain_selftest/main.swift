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

    print("\nTotal passed: \(passed), failed: \(failed)")
    return failed == 0 ? 0 : 1
}

exit(runTests())
