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
    print("T4 status, five-language, session decoding and read-only client checks passed")
} catch {
    fputs("T4 fixture or decoding check failed\n", stderr)
    exit(1)
}
