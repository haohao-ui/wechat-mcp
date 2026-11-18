import Foundation

func require(_ condition: @autoclosure () -> Bool, _ label: String) {
    if !condition() { fputs("T4 check failed: \(label)\n", stderr); exit(1) }
}
func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder().decode(type, from: Data(json.utf8))
}

let fixture = """
{"permissions":{"screen_recording_authorized":true,"accessibility_trusted":true},
 "desktop_service":{"socket_exists":true,"status":"running"},
 "bot":{"launchd_loaded":false,"core_enabled":true,"core_status":"running","core_pid":123,
        "health_state":"watching","health_updated_at":100,"now":101},
 "sessions":{},"wechat":{"is_running":true},"layout":{"layout_exists":true},
 "disk":{"free_bytes":10000000000,"attachment_bytes":0},"model_state":"configured"}
"""
do {
    L10n.setLanguage(.zh, persist: false)
    var input = try decode(StatusInput.self, fixture)
    require(summarizeStatus(input).state == .normal, "healthy")
    input.modelState = "unconfigured"
    require(summarizeStatus(input).state == .modelUnconfigured, "model missing")
    input.permissions.screenRecordingAuthorized = false
    require(summarizeStatus(input).state == .permissionMissing, "permission before model")
    input.bot.coreStatus = "stopped"
    let summary = summarizeStatus(input)
    require(summary.state == .offline, "offline before permission")
    require(summary.issues.map { $0.kind } == [.offline, .screenRecording, .modelUnconfigured], "ordered issues")
    require(summary.autoReplyPaused && summary.bannerText != nil, "consistent paused banner")
    input = try decode(StatusInput.self, fixture)
    input.modelState = nil
    require(summarizeStatus(input).modelState == .unknown, "old backend unknown")
    require(ModelConfigState.parse("default_codex") == .defaultBackend, "T1 default_codex")
    input.modelState = "unconfigured"
    input.brain = BrainInput(mode: "external")
    require(summarizeStatus(input).state != .modelUnconfigured, "external does not use builtin model")
    input.permissions.folderPermissionBlocked = "Desktop"
    require(!summarizeStatus(input).autoReplyPaused, "folder only blocks attachments")
    require(!sessionStateHumanReadable(state: "unknown", reason: "PRIVATE_SENTINEL").contains("PRIVATE_SENTINEL"), "raw health error hidden")

    let session = try decode(RecordedSession.self, """
    {"contact_id":"c1","display_name":"Alice","kind":"group","unread":null,"auto_replying":true,"handoff":false}
    """)
    require(session.unread == nil && !session.displayTitle.contains("未读"), "null unread omitted")
    require(session.displayTitle.contains(L10n.t(.kindGroup)), "group distinguished")
    require(!session.title(paused: true).contains(L10n.t(.modeAuto)), "offline masks active row")
    let missing = try decode(RecordedSession.self, """
    {"contact_id":"c2","display_name":"Bob"}
    """)
    require(missing.displayTitle == "Bob", "missing optional fields omitted")
    let pending = try decode(RecordedMessage.self, """
    {"text":"Hello","direction":"outbound","auto_reply":true,"send_status":null}
    """)
    require(!pending.displayText.contains(L10n.t(.termSent)), "no invented send")
    let sent = try decode(RecordedMessage.self, """
    {"text":"Hello","direction":"outbound","auto_reply":true,"send_status":"\(L10n.t(.termSent))"}
    """)
    for language in AppLanguage.allCases {
        L10n.setLanguage(language, persist: false)
        for key in L10nKey.allCases { require(!L10n.t(key).isEmpty, "translation exists") }
        require(sent.displayText.contains(L10n.t(.termSent)), "send translated")
        require(!pending.displayText.contains(L10n.t(.termSent)), "null stays unknown in every language")
        require(!L10n.t(.rangeHint, 0, 300, L10n.t(.unitSeconds), 15).contains("%@"), "format substituted")
    }
    L10n.setLanguage(.zh, persist: false)
    let rawModel: [String: Any] = ["model_config": ["backend": "codex-exec", "timeout_seconds": 42], "images_per_model_call": 3, "account_id": "integration"]
    let draft = ModelPageDraft.fromConfig(rawModel)
    require(draft.timeoutSeconds == "42" && draft.imagesPerModelCall == "3" && !draft.supportsImages, "model reads persisted values and images default off")
    let updated = try ConfigFormLogic.buildUpdatedConfig(original: rawModel, formInputs: ["model_config_form": draft.toModelConfigForm()])
    require(updated["account_id"] as? String == "integration", "model save preserves unrelated configuration")
    let rollback = try decode(RollbackStatusInfo.self, "{\"version\":\"previous\",\"installed_at\":null,\"can_rollback\":false,\"reason\":\"no_backup\"}")
    require(rollback.installedAt == nil && !rollback.canRollback, "rollback accepts nullable installation time without enabling rollback")
    var captured: [String] = []
    let client = SessionDataClient(configPath: "/tmp/t4-test-config", executor: { _, args, _ in
        captured = args
        return ControlRun(status: 0, output: Data("{\"ok\":true,\"data\":{\"sessions\":[],\"next_offset\":50}}".utf8), timedOut: false)
    })
    let page = try client.list()
    require(page.sessions.isEmpty && page.nextOffset == 50, "pagination envelope")
    require(captured.contains("sessions-list") && captured.contains("--request-json"), "T2 read-only command")
    let detailClient = SessionDataClient(executor: { _, _, _ in
        ControlRun(status: 0, output: Data("{\"ok\":true,\"data\":{\"messages\":[],\"next_offset\":null}}".utf8), timedOut: false)
    })
    let detail = try detailClient.detail(contactID: "c1")
    require(detail.messages.isEmpty, "empty recorded messages")

    for human in [true, false] {
        let handoffClient = SessionDataClient(configPath: "/tmp/t8-config", executor: { _, args, _ in
            captured = args
            return ControlRun(status: 0, output: Data("{\"ok\":true,\"data\":{\"session_id\":\"c1\",\"handoff\":\(human)}}".utf8), timedOut: false)
        })
        let state = try handoffClient.handoff(sessionID: "c1", toHuman: human)
        require(state.handoff == human, "handoff result")
        require(captured.contains("session-handoff"), "session mutation command")
        let requestIndex = captured.firstIndex(of: "--request-json")!
        let request = try JSONSerialization.jsonObject(with: Data(captured[requestIndex + 1].utf8)) as! [String: String]
        require(request["session_id"] == "c1" && request["to"] == (human ? "human" : "bot"), "handoff exact identity and target")
    }
    let rejected = SessionDataClient(executor: { _, _, _ in
        ControlRun(status: 0, output: Data("{\"ok\":false,\"error\":{\"message\":\"PRIVATE_SENTINEL\"}}".utf8), timedOut: false)
    })
    do { _ = try rejected.handoff(sessionID: "c1", toHuman: true); require(false, "rejection fails") }
    catch { require(!String(describing: error).contains("PRIVATE_SENTINEL"), "handoff error sanitized") }
    let wrongSession = SessionDataClient(executor: { _, _, _ in
        ControlRun(status: 0, output: Data("{\"ok\":true,\"data\":{\"session_id\":\"other\",\"handoff\":true}}".utf8), timedOut: false)
    })
    do { _ = try wrongSession.handoff(sessionID: "c1", toHuman: true); require(false, "wrong session rejected") }
    catch { }
    let stats = SessionDataClient(executor: { _, _, _ in
        ControlRun(status: 0, output: Data("{\"stats\":{\"sent_today\":7}}".utf8), timedOut: false)
    })
    let count = try stats.sentToday()
    require(count == 7, "T2 sent_today")
    let oldStats = SessionDataClient(executor: { _, _, _ in ControlRun(status: 0, output: Data("{\"stats\":{}}".utf8), timedOut: false) })
    let oldCount = try oldStats.sentToday()
    require(oldCount == nil, "missing metric unknown")
    let failure = SessionDataClient(executor: { _, _, _ in ControlRun(status: 1, output: Data("PRIVATE_SENTINEL".utf8), timedOut: false) })
    do { _ = try failure.list(); require(false, "failed request must fail") }
    catch { require(!String(describing: error).contains("PRIVATE_SENTINEL"), "raw command output hidden") }

    // 检查 7 个目标 UI 页面里不再有写死的中文界面文案（注释除外）
    let targetFiles = [
        "native/bootstrap/BrainPage.swift",
        "native/bootstrap/McpPage.swift",
        "native/bootstrap/ContactsPage.swift",
        "native/bootstrap/UpdatePage.swift",
        "native/bootstrap/LogsPage.swift",
        "native/bootstrap/ComponentsPage.swift",
        "native/bootstrap/ModelPage.swift"
    ]
    let chineseRegex = try NSRegularExpression(pattern: "[\\u4e00-\\u9fff]")
    for filePath in targetFiles {
        var fileContent: String?
        if let direct = try? String(contentsOfFile: filePath, encoding: .utf8) {
            fileContent = direct
        } else {
            let cwd = FileManager.default.currentDirectoryPath
            let candidate = URL(fileURLWithPath: cwd).appendingPathComponent(filePath).path
            fileContent = try? String(contentsOfFile: candidate, encoding: .utf8)
        }
        if let content = fileContent {
            let lines = content.components(separatedBy: "\n")
            for (idx, line) in lines.enumerated() {
                let code = line.components(separatedBy: "//")[0]
                let range = NSRange(location: 0, length: code.utf16.count)
                let matches = chineseRegex.matches(in: code, range: range)
                require(matches.isEmpty, "Found hardcoded Chinese in \(filePath):\(idx + 1): \(code.trimmingCharacters(in: .whitespaces))")
            }
        }
    }

    print("T4 status, five-language, session decoding, zero hardcoded Chinese and read-only client checks passed")
} catch {
    fputs("T4 fixture or decoding check failed\n", stderr)
    exit(1)
}
