import Foundation

// SessionDataClient.swift: 只读会话数据客户端（issue #2 T2 的 sessions-list / session-detail / session-handoff / status）；
// 设计基线里这些 Swift 类型原来长在 SessionsPage.swift，合并时把页面换成设计版，这里把数据层单独拆出来。

public struct RecordedSession: Decodable {
    public let contactID: String
    public let displayName: String
    public let kind: String?
    public let unread: Int?
    public let autoReplying: Bool?
    public let handoff: Bool?
    enum CodingKeys: String, CodingKey {
        case contactID = "contact_id", displayName = "display_name", kind, unread
        case autoReplying = "auto_replying", handoff
    }
    public var displayTitle: String { title(paused: false) }
    public func title(paused: Bool) -> String {
        var parts = [displayName]
        if kind == "group" { parts.append(L10n.t(.kindGroup)) }
        if kind == "private" { parts.append(L10n.t(.kindPrivate)) }
        if let unread = unread { parts.append(L10n.t(.unreadCount, unread)) }
        if handoff == true { parts.append(L10n.t(.modeManual)) }
        else if let active = autoReplying { parts.append(L10n.t(active && !paused ? .modeAuto : .modePaused)) }
        return parts.joined(separator: " · ")
    }
}

public struct RecordedMessage: Decodable {
    public let text: String?
    public let direction: String?
    public let autoReply: Bool?
    public let sendStatus: String?
    enum CodingKeys: String, CodingKey {
        case text, direction, autoReply = "auto_reply", sendStatus = "send_status"
    }
    public var displayText: String {
        var marks: [String] = []
        if direction == "inbound" { marks.append(L10n.t(.messageFromCustomer)) }
        if direction == "outbound" { marks.append(L10n.t(.messageFromUs)) }
        if autoReply == true { marks.append(L10n.t(.termAutoReply)) }
        // Only the fixed, evidenced T2 status marks a send. Null never becomes "sent".
        if sendStatus == L10n.text(.termSent, in: .zh) {
            marks.append(L10n.t(.termSent))
            marks.append(L10n.t(.deliveryUnknown))
        }
        return marks.joined(separator: " · ") + "\n" + (text ?? "")
    }
}

public struct SessionListPage: Decodable {
    public let sessions: [RecordedSession]
    public let nextOffset: Int?
    enum CodingKeys: String, CodingKey { case sessions, nextOffset = "next_offset" }
}
public struct SessionMessagePage: Decodable {
    public let session: RecordedSession?
    public let messages: [RecordedMessage]
    public let nextOffset: Int?
    enum CodingKeys: String, CodingKey { case session, messages, nextOffset = "next_offset" }
}

/// Uses T2's versioned JSON envelope and never includes command output in errors.
public final class SessionDataClient {
    public typealias Executor = (String, [String], Double) -> ControlRun
    private let executor: Executor
    private let configPath: String
    public init(configPath: String? = nil, executor: @escaping Executor = runControlCommand) {
        self.executor = executor
        self.configPath = configPath ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".WeChatRPABot/config.json").path
    }
    private func run(_ action: String, request: [String: Any]? = nil) throws -> Data {
        var args = ["control", action, "--config", configPath]
        if let request = request {
            let data = try JSONSerialization.data(withJSONObject: request, options: .sortedKeys)
            args += ["--request-json", String(decoding: data, as: UTF8.self)]
        }
        let result = executor(controlBinaryPath(), args, 15)
        guard result.status == 0 && !result.timedOut else { throw SessionDataError.unavailable }
        return result.output
    }
    private struct Envelope<Value: Decodable>: Decodable { let ok: Bool; let data: Value? }
    private func payload<T: Decodable>(_ action: String, request: [String: Any]) throws -> T {
        let envelope = try JSONDecoder().decode(Envelope<T>.self, from: run(action, request: request))
        guard envelope.ok, let value = envelope.data else { throw SessionDataError.unavailable }
        return value
    }
    public func list(offset: Int = 0) throws -> SessionListPage {
        try payload("sessions-list", request: ["limit": 50, "offset": offset])
    }
    public func detail(contactID: String, offset: Int = 0) throws -> SessionMessagePage {
        try payload("session-detail", request: ["contact_id": contactID, "limit": 50, "offset": offset])
    }
    public struct HandoffState: Decodable { public let sessionID: String; public let handoff: Bool
        enum CodingKeys: String, CodingKey { case sessionID = "session_id", handoff }
    }
    public func handoff(sessionID: String, toHuman: Bool) throws -> HandoffState {
        let state: HandoffState = try payload("session-handoff", request: ["session_id": sessionID, "to": toHuman ? "human" : "bot"])
        guard state.sessionID == sessionID, state.handoff == toHuman else { throw SessionDataError.unavailable }
        return state
    }
    public func sentToday() throws -> Int? {
        let object = try JSONSerialization.jsonObject(with: run("status")) as? [String: Any]
        let stats = object?["stats"] as? [String: Any]
        guard let count = stats?["sent_today"] as? Int, count >= 0 else { return nil }
        return count
    }
}

public enum SessionDataError: Error { case unavailable }
