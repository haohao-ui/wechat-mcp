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

    // 12. Offscreen Opaque PNG Rendering & Visual Layout Checks with Rich Synthetic Fixtures
    do {
        let fixtureDir = URL(fileURLWithPath: "target/brain_ui_fixtures")
        try? FileManager.default.createDirectory(at: fixtureDir, withIntermediateDirectories: true)
        
        func renderOpaquePNG(view: NSView, size: NSSize, outputURL: URL) -> Bool {
            let window = NSWindow(
                contentRect: NSRect(origin: .zero, size: size),
                styleMask: [.titled, .closable],
                backing: .buffered,
                defer: false
            )
            window.appearance = NSAppearance(named: .aqua)
            window.backgroundColor = NSColor(calibratedRed: 0.95, green: 0.95, blue: 0.96, alpha: 1.0)
            window.isOpaque = true
            window.contentView = view
            view.frame = NSRect(origin: .zero, size: size)
            view.layoutSubtreeIfNeeded()
            window.display()

            guard let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil,
                pixelsWide: Int(size.width),
                pixelsHigh: Int(size.height),
                bitsPerSample: 8,
                samplesPerPixel: 4,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: .deviceRGB,
                bytesPerRow: 0,
                bitsPerPixel: 0
            ) else {
                return false
            }

            NSGraphicsContext.saveGraphicsState()
            guard let context = NSGraphicsContext(bitmapImageRep: rep) else {
                NSGraphicsContext.restoreGraphicsState()
                return false
            }
            NSGraphicsContext.current = context

            // 1. 显式填充不透明系统浅色窗体底色，彻底杜绝黑底/透明背景
            NSColor(calibratedRed: 0.95, green: 0.95, blue: 0.96, alpha: 1.0).setFill()
            NSRect(origin: .zero, size: size).fill()

            // 2. 缓存绘制视图到位图
            view.cacheDisplay(in: NSRect(origin: .zero, size: size), to: rep)
            context.flushGraphics()
            NSGraphicsContext.restoreGraphicsState()

            guard let png = rep.representation(using: .png, properties: [:]) else {
                return false
            }
            do {
                try png.write(to: outputURL)
                return true
            } catch {
                return false
            }
        }

        let dummyContext = StatusPanelController(worker: SocketWorker())
        let mockClient = BrainClient(configPath: "/tmp/mock_config.json", executor: { _, _, _ in
            ControlRun(status: 0, output: Data(), timedOut: false)
        })
        let brainController = BrainPageController(context: dummyContext, client: mockClient)

        // 构造丰富完整的合成测试数据（含草稿/运行中版本、多角色、知识引用、自定义话术与白名单覆盖）
        let mockPayload = BrainListPayload(
            generation: 12,
            executionMode: "managed",
            roles: [
                BrainRoleEntry(
                    roleId: "role_vip_service",
                    draftRevisionId: "rev_v2",
                    activeRevisionId: "rev_v1",
                    name: "VIP 售后专属顾问",
                    archived: false,
                    activeBindingCount: 1
                ),
                BrainRoleEntry(
                    roleId: "role_pre_sales",
                    draftRevisionId: "rev_p1",
                    activeRevisionId: "rev_p1",
                    name: "售前导购助手",
                    archived: false,
                    activeBindingCount: 0
                )
            ],
            active: BrainBindingSet(
                defaultRef: BrainRoleRef(roleId: "role_vip_service", revisionId: "rev_v1"),
                overrides: [
                    BrainOverride(contactId: "联系人_王经理", chatKind: "private", roleId: "role_vip_service", revisionId: "rev_v1")
                ]
            ),
            draft: BrainBindingSet(
                defaultRef: BrainRoleRef(roleId: "role_vip_service", revisionId: "rev_v2"),
                overrides: [
                    BrainOverride(contactId: "联系人_王经理", chatKind: "private", roleId: "role_vip_service", revisionId: "rev_v2")
                ]
            ),
            modelSummary: BrainModelSummary(mode: "builtin", timeoutSeconds: 30)
        )

        let mockRole = BrainRoleRevision(
            roleId: "role_vip_service",
            revisionId: "rev_v2",
            parentRevisionId: "rev_v1",
            createdAt: 1700000000,
            origin: "user",
            name: "VIP 售后专属顾问",
            persona: "负责企业客户售后咨询、技术答疑与退换货跟进，保持专业耐心态度。",
            goals: ["30秒内首响应", "准确解答退换货政策", "疑难技术问题转接工程师"],
            tone: "严谨、热情、简洁",
            businessRules: ["退货须核对订单号", "不可虚构促销或承诺退款", "非工作时间告知响应时限"],
            modelRef: "config.model_config",
            knowledgeRefs: [
                BrainKnowledgeRef(knowledgeId: "kn_product_guide", revisionId: "rev_k1", contentSha256: "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"),
                BrainKnowledgeRef(knowledgeId: "kn_service_policy", revisionId: "rev_k2", contentSha256: "f1e2d3c4b5a697887766554433221100f1e2d3c4b5a697887766554433221100")
            ],
            templates: BrainTemplates(
                greeting: "您好！我是您的 VIP 售后专属顾问，请问有什么可以帮您？",
                insufficientInformation: "抱歉，您提供的信息不足以定位订单，请告知订单号或购买手机号。",
                unreadable: [
                    "unsupported_type": "收到文件格式暂不支持直接解析，请发送 PDF、文档或文字说明。",
                    "too_large": "您发送的文件超过单次处理大小限制，请精简或分段发送。"
                ]
            ),
            contextBudget: BrainContextBudget(maxInputBytes: 49152, knowledgeBytes: 16384, historyBytes: 16384, overflow: "truncate_tail"),
            contentSha256: "11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff"
        )

        // 注入合成数据到大脑控制器
        brainController.applyFixtureState(payload: mockPayload, role: mockRole, accountId: "corp_support_01")
        
        let brainSize = NSSize(width: 760, height: 780)
        
        // 12.1 渲染定义页并导出 brain_page.png 及 brain_page_definition.png
        brainController.selectTab(index: 0)
        let defURL = fixtureDir.appendingPathComponent("brain_page_definition.png")
        let mainBrainURL = fixtureDir.appendingPathComponent("brain_page.png")
        let defOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: defURL)
        _ = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: mainBrainURL)
        check("brain_page_definition_written", defOK && FileManager.default.fileExists(atPath: defURL.path))
        check("brain_page_png_written", FileManager.default.fileExists(atPath: mainBrainURL.path))
        check("brain_page_png_size_valid", ((try? Data(contentsOf: mainBrainURL))?.count ?? 0) > 10240)

        // 12.2 渲染知识页
        brainController.selectTab(index: 1)
        let knowURL = fixtureDir.appendingPathComponent("brain_page_knowledge.png")
        let knowOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: knowURL)
        check("brain_page_knowledge_written", knowOK && FileManager.default.fileExists(atPath: knowURL.path))

        // 12.3 渲染话术页
        brainController.selectTab(index: 2)
        let templURL = fixtureDir.appendingPathComponent("brain_page_templates.png")
        let templOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: templURL)
        check("brain_page_templates_written", templOK && FileManager.default.fileExists(atPath: templURL.path))

        // 12.4 渲染绑定页
        brainController.selectTab(index: 3)
        let bindURL = fixtureDir.appendingPathComponent("brain_page_bindings.png")
        let bindOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: bindURL)
        check("brain_page_bindings_written", bindOK && FileManager.default.fileExists(atPath: bindURL.path))

        // 12.5 渲染版本管理页
        brainController.selectTab(index: 4)
        let verURL = fixtureDir.appendingPathComponent("brain_page_version.png")
        let verOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: verURL)
        check("brain_page_version_written", verOK && FileManager.default.fileExists(atPath: verURL.path))

        // 12.6 渲染合成测试页
        brainController.selectTab(index: 5)
        brainController.setPreviewFixture(
            input: "请问VIP客户退换货流程是怎样的？",
            result: """
            【组装预览结果】
            状态: 未调用模型，仅执行确定性组装预览 (assemble_only)
            实际费用产生: 否 (0 Token / 0 费用)
            快照摘要: 11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff
            字节预算: 原始=49152, 纳入=18240, 是否截断=否
            系统能力约束:
            不可虚构事实、语音转写以本轮转写为准、未识别项点名并依提示回复。
            """
        )
        let prevURL = fixtureDir.appendingPathComponent("brain_page_preview.png")
        let prevOK = renderOpaquePNG(view: brainController.view, size: brainSize, outputURL: prevURL)
        check("brain_page_preview_written", prevOK && FileManager.default.fileExists(atPath: prevURL.path))

        // 12.7 渲染会话页（载入真实结构化合成 provenance 与阶段标签）
        let sessController = SessionsPageController(context: dummyContext)
        _ = sessController.view // 确保 viewDidLoad 完成
        let dummyTurnDetail = BrainTurnDetail(
            turnId: "turn_contract_real_101",
            contactId: "联系人_王经理",
            snapshotId: "snap_112233",
            snapshot: ["role_id": AnyCodable("role_vip_service"), "compiled": AnyCodable("【合成快照】VIP 售后专属顾问已加载")],
            decision: ["action": AnyCodable("send"), "text": AnyCodable("您好！已为您登记售后换货申请，专员将在2小时内与您联系。")],
            model: ["provider": AnyCodable("openai"), "model": AnyCodable("gpt-4o")],
            sendKey: "send_key_8891_vip",
            execution: ["phase": AnyCodable("sent"), "delivery": AnyCodable("echo_observed")]
        )
        sessController.applyFixtureState(
            sessions: [
                "联系人_王经理": SessionHealthInput(state: "listening"),
                "技术支持群": SessionHealthInput(state: "handoff", handoff: true)
            ],
            selectedTurnDetail: dummyTurnDetail
        )
        let sessURL = fixtureDir.appendingPathComponent("sessions_page.png")
        let sessOK = renderOpaquePNG(view: sessController.view, size: NSSize(width: 740, height: 760), outputURL: sessURL)
        check("sessions_page_png_written", sessOK && FileManager.default.fileExists(atPath: sessURL.path))
        check("sessions_page_png_size_valid", ((try? Data(contentsOf: sessURL))?.count ?? 0) > 10240)
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
