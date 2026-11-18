// BrainPage.swift: Persona, knowledge, prompt templates, bindings, and version activation management.
import Foundation
import AppKit

// MARK: - Brain Data Models

public struct BrainContextBudget: Codable, Equatable {
    public var maxInputBytes: UInt64
    public var knowledgeBytes: UInt64
    public var historyBytes: UInt64
    public var overflow: String

    enum CodingKeys: String, CodingKey {
        case maxInputBytes = "max_input_bytes"
        case knowledgeBytes = "knowledge_bytes"
        case historyBytes = "history_bytes"
        case overflow
    }

    public init(maxInputBytes: UInt64 = 49152, knowledgeBytes: UInt64 = 16384, historyBytes: UInt64 = 16384, overflow: String = "truncate_tail") {
        self.maxInputBytes = maxInputBytes
        self.knowledgeBytes = knowledgeBytes
        self.historyBytes = historyBytes
        self.overflow = overflow
    }
}

struct DynamicCodingKey: CodingKey {
    var stringValue: String
    var intValue: Int?
    init(stringValue: String) { self.stringValue = stringValue; self.intValue = nil }
    init?(intValue: Int) { self.stringValue = "\(intValue)"; self.intValue = intValue }
}

public struct BrainTemplates: Codable, Equatable {
    public static let allowedReasons = [
        "unsupported_type",
        "too_large",
        "download_failed",
        "snapshot_changed",
        "extract_failed",
        "empty",
        "transcribe_failed",
        "unknown"
    ]

    public var greeting: String?
    public var insufficientInformation: String?
    public var unreadable: [String: String]
    public var unreadableMissingLabels: String?
    public var unreadableManyItems: String?

    public var allReasonsCoverage: [String: String?] {
        var map: [String: String?] = [:]
        for reason in BrainTemplates.allowedReasons {
            map[reason] = unreadable[reason]
        }
        return map
    }

    enum CodingKeys: String, CodingKey {
        case greeting
        case insufficientInformation = "insufficient_information"
        case unreadable
        case unreadableMissingLabels = "unreadable_missing_labels"
        case unreadableManyItems = "unreadable_many_items"
    }

    public init(
        greeting: String? = nil,
        insufficientInformation: String? = nil,
        unreadable: [String: String] = [:],
        unreadableMissingLabels: String? = nil,
        unreadableManyItems: String? = nil
    ) {
        self.greeting = greeting
        self.insufficientInformation = insufficientInformation
        self.unreadable = unreadable
        self.unreadableMissingLabels = unreadableMissingLabels
        self.unreadableManyItems = unreadableManyItems
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.greeting = try container.decodeIfPresent(String.self, forKey: .greeting)
        self.insufficientInformation = try container.decodeIfPresent(String.self, forKey: .insufficientInformation)
        self.unreadableMissingLabels = try container.decodeIfPresent(String.self, forKey: .unreadableMissingLabels)
        self.unreadableManyItems = try container.decodeIfPresent(String.self, forKey: .unreadableManyItems)

        var map: [String: String] = [:]
        if let rawUnreadable = try? container.decode([String: String?].self, forKey: .unreadable) {
            for (k, v) in rawUnreadable {
                if let str = v {
                    map[k] = str
                }
            }
        }
        self.unreadable = map
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        if let g = greeting { try container.encode(g, forKey: .greeting) } else { try container.encodeNil(forKey: .greeting) }
        if let inf = insufficientInformation { try container.encode(inf, forKey: .insufficientInformation) } else { try container.encodeNil(forKey: .insufficientInformation) }
        if let m = unreadableMissingLabels { try container.encode(m, forKey: .unreadableMissingLabels) } else { try container.encodeNil(forKey: .unreadableMissingLabels) }
        if let mi = unreadableManyItems { try container.encode(mi, forKey: .unreadableManyItems) } else { try container.encodeNil(forKey: .unreadableManyItems) }

        var unreadableContainer = container.nestedContainer(keyedBy: DynamicCodingKey.self, forKey: .unreadable)
        for reason in BrainTemplates.allowedReasons {
            let key = DynamicCodingKey(stringValue: reason)
            if let val = unreadable[reason] {
                try unreadableContainer.encode(val, forKey: key)
            } else {
                try unreadableContainer.encodeNil(forKey: key)
            }
        }
    }
}

public struct BrainKnowledgeRef: Codable, Equatable {
    public var knowledgeId: String
    public var revisionId: String
    public var contentSha256: String

    enum CodingKeys: String, CodingKey {
        case knowledgeId = "knowledge_id"
        case revisionId = "revision_id"
        case contentSha256 = "content_sha256"
    }

    public init(knowledgeId: String, revisionId: String, contentSha256: String) {
        self.knowledgeId = knowledgeId
        self.revisionId = revisionId
        self.contentSha256 = contentSha256
    }
}

public struct BrainRoleRef: Codable, Equatable {
    public var roleId: String
    public var revisionId: String

    enum CodingKeys: String, CodingKey {
        case roleId = "role_id"
        case revisionId = "revision_id"
    }

    public init(roleId: String, revisionId: String) {
        self.roleId = roleId
        self.revisionId = revisionId
    }
}

/// Editable starter content is shipped as a resource, then saved through the normal role editor.
public struct BrainDefaultRoleTemplate: Decodable {
    public var name: String
    public var persona: String
    public var goals: [String]
    public var tone: String
    public var businessRules: [String]
    public var templates: BrainTemplates

    public static func load(from url: URL) throws -> BrainDefaultRoleTemplate {
        try JSONDecoder().decode(BrainDefaultRoleTemplate.self, from: Data(contentsOf: url))
    }

    public func makeRole() -> BrainRoleRevision {
        BrainRoleRevision(roleId: "role_\(UUID().uuidString.prefix(8))", revisionId: "rev_1",
                          name: name, persona: persona, goals: goals, tone: tone,
                          businessRules: businessRules, templates: templates)
    }
}

public struct BrainRoleRevision: Codable, Equatable {
    public var schemaVersion: UInt32
    public var ownerAccountId: String
    public var roleId: String
    public var revisionId: String
    public var parentRevisionId: String?
    public var createdAt: UInt64
    public var origin: String
    public var name: String
    public var persona: String
    public var goals: [String]
    public var tone: String
    public var businessRules: [String]
    public var modelRef: String
    public var knowledgeRefs: [BrainKnowledgeRef]
    public var templates: BrainTemplates
    public var contextBudget: BrainContextBudget
    public var contentSha256: String

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case ownerAccountId = "owner_account_id"
        case roleId = "role_id"
        case revisionId = "revision_id"
        case parentRevisionId = "parent_revision_id"
        case createdAt = "created_at"
        case origin
        case name
        case persona
        case goals
        case tone
        case businessRules = "business_rules"
        case modelRef = "model_ref"
        case knowledgeRefs = "knowledge_refs"
        case templates
        case contextBudget = "context_budget"
        case contentSha256 = "content_sha256"
    }

    public init(
        schemaVersion: UInt32 = 1,
        ownerAccountId: String = "",
        roleId: String = "",
        revisionId: String = "",
        parentRevisionId: String? = nil,
        createdAt: UInt64 = UInt64(Date().timeIntervalSince1970),
        origin: String = "user",
        name: String = "",
        persona: String = "",
        goals: [String] = [],
        tone: String = "",
        businessRules: [String] = [],
        modelRef: String = "config.model_config",
        knowledgeRefs: [BrainKnowledgeRef] = [],
        templates: BrainTemplates = BrainTemplates(),
        contextBudget: BrainContextBudget = BrainContextBudget(),
        contentSha256: String = ""
    ) {
        self.schemaVersion = schemaVersion
        self.ownerAccountId = ownerAccountId
        self.roleId = roleId
        self.revisionId = revisionId
        self.parentRevisionId = parentRevisionId
        self.createdAt = createdAt
        self.origin = origin
        self.name = name
        self.persona = persona
        self.goals = goals
        self.tone = tone
        self.businessRules = businessRules
        self.modelRef = modelRef
        self.knowledgeRefs = knowledgeRefs
        self.templates = templates
        self.contextBudget = contextBudget
        self.contentSha256 = contentSha256
    }

    public mutating func addKnowledgeRef(from knowledge: BrainKnowledgeRevision) {
        if !knowledgeRefs.contains(where: { $0.knowledgeId == knowledge.knowledgeId }) {
            knowledgeRefs.append(BrainKnowledgeRef(
                knowledgeId: knowledge.knowledgeId,
                revisionId: knowledge.revisionId,
                contentSha256: knowledge.contentSha256
            ))
        }
    }

    public mutating func removeKnowledgeRef(knowledgeId: String) {
        knowledgeRefs.removeAll(where: { $0.knowledgeId == knowledgeId })
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(schemaVersion, forKey: .schemaVersion)
        try container.encode(ownerAccountId, forKey: .ownerAccountId)
        try container.encode(roleId, forKey: .roleId)
        try container.encode(revisionId, forKey: .revisionId)
        if let pid = parentRevisionId {
            try container.encode(pid, forKey: .parentRevisionId)
        } else {
            try container.encodeNil(forKey: .parentRevisionId)
        }
        try container.encode(createdAt, forKey: .createdAt)
        try container.encode(origin, forKey: .origin)
        try container.encode(name, forKey: .name)
        try container.encode(persona, forKey: .persona)
        try container.encode(goals, forKey: .goals)
        try container.encode(tone, forKey: .tone)
        try container.encode(businessRules, forKey: .businessRules)
        try container.encode(modelRef, forKey: .modelRef)
        try container.encode(knowledgeRefs, forKey: .knowledgeRefs)
        try container.encode(templates, forKey: .templates)
        try container.encode(contextBudget, forKey: .contextBudget)
        try container.encode(contentSha256, forKey: .contentSha256)
    }
}

public struct BrainKnowledgeRevision: Codable, Equatable {
    public var schemaVersion: UInt32
    public var ownerAccountId: String
    public var knowledgeId: String
    public var revisionId: String
    public var parentRevisionId: String?
    public var createdAt: UInt64
    public var title: String
    public var text: String
    public var contentSha256: String
    public var sourceLabel: String

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case ownerAccountId = "owner_account_id"
        case knowledgeId = "knowledge_id"
        case revisionId = "revision_id"
        case parentRevisionId = "parent_revision_id"
        case createdAt = "created_at"
        case title
        case text
        case contentSha256 = "content_sha256"
        case sourceLabel = "source_label"
    }

    public init(
        schemaVersion: UInt32 = 1,
        ownerAccountId: String = "",
        knowledgeId: String = "",
        revisionId: String = "",
        parentRevisionId: String? = nil,
        createdAt: UInt64 = UInt64(Date().timeIntervalSince1970),
        title: String = "",
        text: String = "",
        sourceLabel: String = "",
        contentSha256: String = ""
    ) {
        self.schemaVersion = schemaVersion
        self.ownerAccountId = ownerAccountId
        self.knowledgeId = knowledgeId
        self.revisionId = revisionId
        self.parentRevisionId = parentRevisionId
        self.createdAt = createdAt
        self.title = title
        self.text = text
        self.sourceLabel = sourceLabel
        self.contentSha256 = contentSha256
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(schemaVersion, forKey: .schemaVersion)
        try container.encode(ownerAccountId, forKey: .ownerAccountId)
        try container.encode(knowledgeId, forKey: .knowledgeId)
        try container.encode(revisionId, forKey: .revisionId)
        if let pid = parentRevisionId {
            try container.encode(pid, forKey: .parentRevisionId)
        } else {
            try container.encodeNil(forKey: .parentRevisionId)
        }
        try container.encode(createdAt, forKey: .createdAt)
        try container.encode(title, forKey: .title)
        try container.encode(text, forKey: .text)
        try container.encode(contentSha256, forKey: .contentSha256)
        try container.encode(sourceLabel, forKey: .sourceLabel)
    }
}

public struct BrainRoleEntry: Codable, Equatable {
    public var roleId: String
    public var draftRevisionId: String
    public var activeRevisionId: String?
    public var name: String
    public var archived: Bool
    public var activeBindingCount: Int

    enum CodingKeys: String, CodingKey {
        case roleId = "role_id"
        case draftRevisionId = "draft_revision_id"
        case activeRevisionId = "active_revision_id"
        case name
        case archived
        case activeBindingCount = "active_binding_count"
    }

    public init(
        roleId: String,
        draftRevisionId: String,
        activeRevisionId: String? = nil,
        name: String = "",
        archived: Bool = false,
        activeBindingCount: Int = 0
    ) {
        self.roleId = roleId
        self.draftRevisionId = draftRevisionId
        self.activeRevisionId = activeRevisionId
        self.name = name.isEmpty ? roleId : name
        self.archived = archived
        self.activeBindingCount = activeBindingCount
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.roleId = try container.decode(String.self, forKey: .roleId)
        self.draftRevisionId = try container.decode(String.self, forKey: .draftRevisionId)
        self.activeRevisionId = try container.decodeIfPresent(String.self, forKey: .activeRevisionId)
        let decodedName = try container.decodeIfPresent(String.self, forKey: .name)
        self.name = (decodedName != nil && !decodedName!.isEmpty) ? decodedName! : roleId
        self.archived = (try? container.decodeIfPresent(Bool.self, forKey: .archived)) ?? false
        self.activeBindingCount = (try? container.decodeIfPresent(Int.self, forKey: .activeBindingCount)) ?? 0
    }

    public var hasDraftChanges: Bool {
        return activeRevisionId == nil || activeRevisionId != draftRevisionId
    }
}

public struct BrainOverride: Codable, Equatable {
    public var contactId: String
    public var chatKind: String
    public var roleId: String
    public var revisionId: String

    enum CodingKeys: String, CodingKey {
        case contactId = "contact_id"
        case chatKind = "chat_kind"
        case roleId = "role_id"
        case revisionId = "revision_id"
    }

    public init(contactId: String, chatKind: String, roleId: String, revisionId: String) {
        self.contactId = contactId
        self.chatKind = chatKind
        self.roleId = roleId
        self.revisionId = revisionId
    }
}

public struct BrainBindingSet: Codable, Equatable {
    public var defaultRef: BrainRoleRef?
    public var overrides: [BrainOverride]

    enum CodingKeys: String, CodingKey {
        case defaultRef = "default"
        case overrides
    }

    public init(defaultRef: BrainRoleRef? = nil, overrides: [BrainOverride] = []) {
        self.defaultRef = defaultRef
        self.overrides = overrides
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        if let d = defaultRef {
            try container.encode(d, forKey: .defaultRef)
        } else {
            try container.encodeNil(forKey: .defaultRef)
        }
        try container.encode(overrides, forKey: .overrides)
    }
}

public struct BrainAllowlistContact: Codable, Equatable {
    public var contactId: String
    public var displayName: String
    public var chatKind: String

    enum CodingKeys: String, CodingKey {
        case contactId = "contact_id"
        case displayName = "display_name"
        case chatKind = "chat_kind"
    }

    public init(contactId: String, displayName: String, chatKind: String = "private") {
        self.contactId = contactId
        self.displayName = displayName
        self.chatKind = chatKind
    }
}

public enum BrainBindingError: Error, Equatable {
    case notInAllowlist
    case duplicateContact
    case overrideNotFound
    case invalidRole
}

public struct BrainModelSummary: Codable, Equatable {
    public var mode: String
    public var timeoutSeconds: Int?

    enum CodingKeys: String, CodingKey {
        case mode
        case timeoutSeconds = "timeout_seconds"
    }
}

public struct BrainListPayload: Codable, Equatable {
    public var accountId: String?
    public var generation: UInt64
    public var executionMode: String
    public var roles: [BrainRoleEntry]
    public var active: BrainBindingSet?
    public var draft: BrainBindingSet?
    public var modelSummary: BrainModelSummary?
    public var allowlistContacts: [BrainAllowlistContact]
    public var allowlistContactsStatus: String
    public var allowlistContactsError: String?

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case generation
        case executionMode = "execution_mode"
        case roles
        case active
        case draft
        case modelSummary = "model_summary"
        case allowlistContacts = "allowlist_contacts"
        case allowlistContactsStatus = "allowlist_contacts_status"
        case allowlistContactsError = "allowlist_contacts_error"
    }

    public init(
        generation: UInt64,
        executionMode: String,
        roles: [BrainRoleEntry],
        active: BrainBindingSet? = nil,
        draft: BrainBindingSet? = nil,
        modelSummary: BrainModelSummary? = nil,
        allowlistContacts: [BrainAllowlistContact] = [],
        allowlistContactsStatus: String = "ready",
        allowlistContactsError: String? = nil,
        accountId: String? = nil
    ) {
        self.accountId = accountId
        self.generation = generation
        self.executionMode = executionMode
        self.roles = roles
        self.active = active
        self.draft = draft
        self.modelSummary = modelSummary
        self.allowlistContacts = allowlistContacts
        self.allowlistContactsStatus = allowlistContactsStatus
        self.allowlistContactsError = allowlistContactsError
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.accountId = try container.decodeIfPresent(String.self, forKey: .accountId)
        self.generation = try container.decode(UInt64.self, forKey: .generation)
        self.executionMode = try container.decode(String.self, forKey: .executionMode)
        self.roles = try container.decode([BrainRoleEntry].self, forKey: .roles)
        self.active = try container.decodeIfPresent(BrainBindingSet.self, forKey: .active)
        self.draft = try container.decodeIfPresent(BrainBindingSet.self, forKey: .draft)
        self.modelSummary = try container.decodeIfPresent(BrainModelSummary.self, forKey: .modelSummary)
        self.allowlistContacts = (try? container.decodeIfPresent([BrainAllowlistContact].self, forKey: .allowlistContacts)) ?? []
        // 兼容旧服务端响应缺失字段的情况：缺省按 unavailable 处理
        self.allowlistContactsStatus = (try? container.decodeIfPresent(String.self, forKey: .allowlistContactsStatus)) ?? "unavailable"
        self.allowlistContactsError = try container.decodeIfPresent(String.self, forKey: .allowlistContactsError)
    }
}

// MARK: - Brain History Models（issue-2 历史版本，字段对齐后端 brain-history）
// 新增文案暂用中文硬编码，后续统一迁入 L10n。

/// 代次记录。字段取自后端 `brain-history` 的 `generations`：
/// generation/at/operation/role_id/revision_id/is_activation/active/draft。
/// `draft` 只有当前代次快照才有值；为空表示后端没有该代次的完整草稿，
/// 界面必须如实标注「未保存完整草稿」，不得补全或编造。
public struct BrainHistoryGeneration: Codable, Equatable {
    public let generation: UInt64
    public let at: UInt64?
    public let operation: String?
    public let roleId: String?
    public let revisionId: String?
    public let isActivation: Bool?
    public let active: BrainBindingSet?
    public let draft: BrainBindingSet?

    enum CodingKeys: String, CodingKey {
        case generation
        case at
        case operation
        case roleId = "role_id"
        case revisionId = "revision_id"
        case isActivation = "is_activation"
        case active
        case draft
    }

    public init(
        generation: UInt64,
        at: UInt64? = nil,
        operation: String? = nil,
        roleId: String? = nil,
        revisionId: String? = nil,
        isActivation: Bool? = nil,
        active: BrainBindingSet? = nil,
        draft: BrainBindingSet? = nil
    ) {
        self.generation = generation
        self.at = at
        self.operation = operation
        self.roleId = roleId
        self.revisionId = revisionId
        self.isActivation = isActivation
        self.active = active
        self.draft = draft
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.generation = try container.decode(UInt64.self, forKey: .generation)
        self.at = try container.decodeIfPresent(UInt64.self, forKey: .at)
        self.operation = try container.decodeIfPresent(String.self, forKey: .operation)
        self.roleId = try container.decodeIfPresent(String.self, forKey: .roleId)
        self.revisionId = try container.decodeIfPresent(String.self, forKey: .revisionId)
        self.isActivation = try container.decodeIfPresent(Bool.self, forKey: .isActivation)
        // 绑定快照结构异常时按「未记录」处理，避免整份历史读取失败。
        self.active = try? container.decodeIfPresent(BrainBindingSet.self, forKey: .active)
        self.draft = try? container.decodeIfPresent(BrainBindingSet.self, forKey: .draft)
    }

    /// 后端是否为该代次保留了完整草稿快照。
    public var hasCompleteDraft: Bool {
        return draft != nil
    }

    public var runDefaultRoleId: String? {
        return active?.defaultRef?.roleId ?? draft?.defaultRef?.roleId
    }

    public var runDefaultRevisionId: String? {
        return active?.defaultRef?.revisionId ?? draft?.defaultRef?.revisionId
    }

    public var overrideCount: Int {
        return active?.overrides.count ?? draft?.overrides.count ?? 0
    }
}

/// 版本记录。字段取自后端 `brain-history` 的 `versions`：
/// role_id/revision_id/created_at/is_active/is_draft/content。
/// `content` 是后端保存的完整角色修订；解析失败只置空，绝不补造字段。
public struct BrainHistoryVersion: Codable, Equatable {
    public let roleId: String?
    public let revisionId: String
    public let createdAt: UInt64?
    public let isActive: Bool?
    public let isDraft: Bool?
    public let content: BrainRoleRevision?

    enum CodingKeys: String, CodingKey {
        case roleId = "role_id"
        case revisionId = "revision_id"
        case createdAt = "created_at"
        case isActive = "is_active"
        case isDraft = "is_draft"
        case content
    }

    public init(
        roleId: String? = nil,
        revisionId: String,
        createdAt: UInt64? = nil,
        isActive: Bool? = nil,
        isDraft: Bool? = nil,
        content: BrainRoleRevision? = nil
    ) {
        self.roleId = roleId
        self.revisionId = revisionId
        self.createdAt = createdAt
        self.isActive = isActive
        self.isDraft = isDraft
        self.content = content
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.revisionId = try container.decode(String.self, forKey: .revisionId)
        self.roleId = try container.decodeIfPresent(String.self, forKey: .roleId)
        self.createdAt = try container.decodeIfPresent(UInt64.self, forKey: .createdAt)
        self.isActive = try container.decodeIfPresent(Bool.self, forKey: .isActive)
        self.isDraft = try container.decodeIfPresent(Bool.self, forKey: .isDraft)
        self.content = try? container.decodeIfPresent(BrainRoleRevision.self, forKey: .content)
    }
}

public struct BrainHistoryPayload: Codable, Equatable {
    public let accountId: String?
    public let generation: UInt64?
    public let active: BrainBindingSet?
    public let draft: BrainBindingSet?
    public let generations: [BrainHistoryGeneration]?
    public let versions: [BrainHistoryVersion]?
    public let historyScope: String?

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case generation
        case active
        case draft
        case generations
        case versions
        case historyScope = "history_scope"
    }

    public init(
        accountId: String? = nil,
        generation: UInt64? = nil,
        active: BrainBindingSet? = nil,
        draft: BrainBindingSet? = nil,
        generations: [BrainHistoryGeneration]? = nil,
        versions: [BrainHistoryVersion]? = nil,
        historyScope: String? = nil
    ) {
        self.accountId = accountId
        self.generation = generation
        self.active = active
        self.draft = draft
        self.generations = generations
        self.versions = versions
        self.historyScope = historyScope
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.accountId = try container.decodeIfPresent(String.self, forKey: .accountId)
        self.generation = try container.decodeIfPresent(UInt64.self, forKey: .generation)
        self.active = try? container.decodeIfPresent(BrainBindingSet.self, forKey: .active)
        self.draft = try? container.decodeIfPresent(BrainBindingSet.self, forKey: .draft)
        self.generations = try? container.decodeIfPresent([BrainHistoryGeneration].self, forKey: .generations)
        self.versions = try? container.decodeIfPresent([BrainHistoryVersion].self, forKey: .versions)
        self.historyScope = try container.decodeIfPresent(String.self, forKey: .historyScope)
    }
}

public struct BrainControlError: Codable, Equatable {
    public var code: String
    public var message: String
    public var retryable: Bool?
    public var details: [String: AnyCodable]?

    public init(code: String, message: String, retryable: Bool? = nil, details: [String: AnyCodable]? = nil) {
        self.code = code
        self.message = message
        self.retryable = retryable
        self.details = details
    }

    public static func == (lhs: BrainControlError, rhs: BrainControlError) -> Bool {
        return lhs.code == rhs.code && lhs.message == rhs.message && lhs.retryable == rhs.retryable
    }
}

public struct BrainControlResponse<T: Codable>: Codable {
    public var ok: Bool
    public var data: T?
    public var error: BrainControlError?

    public init(ok: Bool, data: T? = nil, error: BrainControlError? = nil) {
        self.ok = ok
        self.data = data
        self.error = error
    }
}

public struct BrainPreviewBudget: Codable, Equatable {
    public var algorithm: String?
    public var maxInputBytes: UInt64?
    public var includedBytes: UInt64?
    public var omittedHistoryCount: Int?

    enum CodingKeys: String, CodingKey {
        case algorithm
        case maxInputBytes = "max_input_bytes"
        case includedBytes = "included_bytes"
        case omittedHistoryCount = "omitted_history_count"
    }

    public var originalBytes: UInt64? { return maxInputBytes }
    public var truncated: Bool? { return (omittedHistoryCount ?? 0) > 0 }
}

public struct BrainPreviewSnapshot: Codable, Equatable {
    public var assemblySha256: String?
    public var systemConstraints: String?
    public var persona: String?
    public var compiled: String?
    public var budget: BrainPreviewBudget?

    enum CodingKeys: String, CodingKey {
        case assemblySha256 = "assembly_sha256"
        case systemConstraints = "system_constraints"
        case persona
        case compiled
        case budget
    }

    public var compiledText: String {
        return compiled ?? ""
    }

    public static func == (lhs: BrainPreviewSnapshot, rhs: BrainPreviewSnapshot) -> Bool {
        return lhs.assemblySha256 == rhs.assemblySha256 && lhs.persona == rhs.persona && lhs.compiled == rhs.compiled
    }
}

public struct BrainPreviewCost: Codable, Equatable {
    public var incurred: Bool
    public var note: String?
}

public struct BrainPreviewResult: Codable, Equatable {
    public var previewId: String?
    public var snapshotId: String?
    public var snapshot: BrainPreviewSnapshot?
    public var decision: [String: AnyCodable]?
    public var cost: BrainPreviewCost?

    enum CodingKeys: String, CodingKey {
        case previewId = "preview_id"
        case snapshotId = "snapshot_id"
        case snapshot
        case decision
        case cost
    }

    public var costIncurred: Bool {
        return cost?.incurred == true
    }

    public var costNote: String {
        return cost?.note ?? "未调用模型，仅组装预览"
    }

    public var budgetTruncated: Bool {
        return snapshot?.budget?.truncated == true
    }

    public static func == (lhs: BrainPreviewResult, rhs: BrainPreviewResult) -> Bool {
        return lhs.previewId == rhs.previewId && lhs.snapshotId == rhs.snapshotId && lhs.cost == rhs.cost
    }
}

public struct BrainTurnDetail: Codable, Equatable {
    public var turnId: String
    public var contactId: String
    public var snapshotId: String?
    public var snapshot: [String: AnyCodable]?
    public var decision: [String: AnyCodable]?
    public var model: [String: AnyCodable]?
    public var sendKey: String?
    public var execution: [String: AnyCodable]?

    enum CodingKeys: String, CodingKey {
        case turnId = "turn_id"
        case contactId = "contact_id"
        case snapshotId = "snapshot_id"
        case snapshot
        case decision
        case model
        case sendKey = "send_key"
        case execution
    }

    public static func == (lhs: BrainTurnDetail, rhs: BrainTurnDetail) -> Bool {
        return lhs.turnId == rhs.turnId && lhs.contactId == rhs.contactId && lhs.sendKey == rhs.sendKey
    }

    public var hasProvenance: Bool {
        return snapshotId != nil || (snapshot != nil && !snapshot!.isEmpty)
    }

    public var provenanceDisplayText: String {
        return hasProvenance ? "包含完整轮次追溯" : "旧版未记录"
    }

    public var roleId: String? {
        return snapshot?["role_id"]?.value as? String
    }

    public var phaseLabels: [String] {
        var labels: [String] = []
        if decision != nil {
            labels.append("模型已生成")
        }
        if let exec = execution {
            if let phase = exec["phase"]?.value as? String {
                if phase == "triggered" {
                    labels.append("发送已触发")
                } else if phase == "sent" {
                    labels.append("已发送")
                }
            }
            if let delivery = exec["delivery"]?.value as? String {
                if delivery == "echo_observed" {
                    labels.append("己方回显")
                    labels.append("送达状态: 未知 (unknown)")
                } else if delivery == "unknown" {
                    labels.append("送达状态: 未知 (unknown)")
                } else {
                    labels.append("送达状态: \(delivery)")
                }
            } else {
                labels.append("送达状态: 未知 (unknown)")
            }
        }
        return labels.isEmpty ? ["状态未知"] : labels
    }
}

// MARK: - BrainPageLogic Pure Logic

public struct BrainPageLogic {
    public static func parseBrainList(jsonString: String) throws -> BrainListPayload {
        guard let data = jsonString.data(using: .utf8) else {
            throw NSError(domain: "BrainPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 字符串"])
        }
        let decoder = JSONDecoder()
        do {
            let resp = try decoder.decode(BrainControlResponse<BrainListPayload>.self, from: data)
            guard resp.ok, let payload = resp.data else {
                let msg = resp.error?.message ?? "获取大脑配置失败"
                throw NSError(domain: "BrainPage", code: 2, userInfo: [NSLocalizedDescriptionKey: msg])
            }
            return payload
        } catch {
            if let direct = try? decoder.decode(BrainListPayload.self, from: data) {
                return direct
            }
            throw error
        }
    }

    public static func parsePreviewResult(jsonString: String) throws -> BrainPreviewResult {
        guard let data = jsonString.data(using: .utf8) else {
            throw NSError(domain: "BrainPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 字符串"])
        }
        let decoder = JSONDecoder()
        if let resp = try? decoder.decode(BrainControlResponse<BrainPreviewResult>.self, from: data) {
            guard resp.ok, let payload = resp.data else {
                let msg = resp.error?.message ?? "组装预览失败"
                throw NSError(domain: "BrainPage", code: 2, userInfo: [NSLocalizedDescriptionKey: msg])
            }
            return payload
        }
        return try decoder.decode(BrainPreviewResult.self, from: data)
    }

    // MARK: - 历史版本（issue-2）

    public static func parseHistoryPayload(jsonString: String) throws -> BrainHistoryPayload {
        guard let data = jsonString.data(using: .utf8) else {
            throw NSError(domain: "BrainPage", code: 1, userInfo: [NSLocalizedDescriptionKey: "无效 UTF8 字符串"])
        }
        let decoder = JSONDecoder()
        if let resp = try? decoder.decode(BrainControlResponse<BrainHistoryPayload>.self, from: data), resp.ok, let payload = resp.data {
            return payload
        }
        return try decoder.decode(BrainHistoryPayload.self, from: data)
    }

    /// 后端 `at` 是 Unix 秒；0/缺省表示初始代次。
    public static func historyTimestampText(_ at: UInt64?) -> String {
        guard let ts = at, ts > 0 else { return "初始代次" }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone.current
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(ts)))
    }

    /// 后端 operation 是写入动作名；未知动作原样展示，便于排查，不臆造含义。
    public static func operationDisplayName(_ operation: String?) -> String {
        guard let op = operation, !op.isEmpty else { return "未记录操作" }
        switch op {
        case "brain-save-role": return "保存角色草稿"
        case "brain-save-knowledge": return "保存知识"
        case "brain-copy-role": return "复制角色"
        case "brain-archive-role": return "删除角色"
        case "brain-save-bindings": return "保存绑定草稿"
        case "brain-activate": return "启用配置"
        case "brain-rollback": return "版本回退"
        default: return op
        }
    }

    /// 版本对比：只展示后端实际返回的字段；任一侧没有完整内容时如实说明，不补造。
    public static func compareVersions(v1: BrainHistoryVersion, v2: BrainHistoryVersion) -> String {
        var lines: [String] = []
        let missing = "[缺失]"

        func flagText(_ flag: Bool?) -> String {
            guard let flag = flag else { return missing }
            return flag ? "是" : "否"
        }

        func personaSummary(_ text: String) -> String {
            if text.isEmpty { return missing }
            if text.count <= 120 { return text }
            return String(text.prefix(120)) + "…（共 \(text.count) 字）"
        }

        func listText(_ items: [String]) -> String {
            return items.isEmpty ? "无" : items.joined(separator: "、")
        }

        func knowledgeText(_ refs: [BrainKnowledgeRef]) -> String {
            if refs.isEmpty { return "无" }
            return refs.map { $0.knowledgeId }.joined(separator: "、") + "（\(refs.count) 项）"
        }

        func templatesText(_ templates: BrainTemplates) -> String {
            var parts: [String] = []
            if let g = templates.greeting, !g.isEmpty { parts.append("欢迎语") }
            if let i = templates.insufficientInformation, !i.isEmpty { parts.append("信息不足") }
            if let m = templates.unreadableMissingLabels, !m.isEmpty { parts.append("缺项标签") }
            if let many = templates.unreadableManyItems, !many.isEmpty { parts.append("多条标签") }
            let reasons = templates.unreadable.keys.sorted()
            if !reasons.isEmpty { parts.append("不可读原因 \(reasons.count) 项") }
            return parts.isEmpty ? "未设置自定义话术" : parts.joined(separator: "、")
        }

        func budgetText(_ budget: BrainContextBudget) -> String {
            return "max_input_bytes=\(budget.maxInputBytes) · knowledge_bytes=\(budget.knowledgeBytes) · history_bytes=\(budget.historyBytes) · overflow=\(budget.overflow)"
        }

        func compare<T: Equatable>(_ label: String, _ a: T?, _ b: T?, _ describe: (T) -> String) {
            switch (a, b) {
            case (nil, nil):
                lines.append("\(label)：双方均未返回内容（\(missing)）")
            case (let x?, nil):
                lines.append("\(label)：A=\(describe(x))，B=\(missing)")
            case (nil, let y?):
                lines.append("\(label)：A=\(missing)，B=\(describe(y))")
            case (let x?, let y?):
                if x == y {
                    lines.append("\(label)：无变化（\(describe(x))）")
                } else {
                    lines.append("\(label)：")
                    lines.append("  A: \(describe(x))")
                    lines.append("  B: \(describe(y))")
                }
            }
        }

        func appendMeta(_ version: BrainHistoryVersion, label: String) {
            lines.append("版本 \(label)：\(version.revisionId)")
            lines.append("  角色=\(version.roleId ?? missing) · 创建时间=\(historyTimestampText(version.createdAt)) · 草稿=\(flagText(version.isDraft)) · 运行中=\(flagText(version.isActive))")
        }

        appendMeta(v1, label: "A")
        appendMeta(v2, label: "B")
        lines.append(String(repeating: "-", count: 52))

        let contentA = v1.content
        let contentB = v2.content
        if contentA == nil || contentB == nil {
            lines.append("【完整性说明】")
            if contentA == nil {
                lines.append("• 版本 A（\(v1.revisionId)）后端未返回完整草稿内容，A 侧字段按「\(missing)」处理，不补全、不编造。")
            }
            if contentB == nil {
                lines.append("• 版本 B（\(v2.revisionId)）后端未返回完整草稿内容，B 侧字段按「\(missing)」处理，不补全、不编造。")
            }
            let available: [(BrainHistoryVersion, String)] = [(v1, "A"), (v2, "B")].filter { $0.0.content != nil }
            for (version, label) in available {
                guard let content = version.content else { continue }
                lines.append("")
                lines.append("【版本 \(label) 已保存内容】\(content.name)")
                lines.append("• 语气风格：\(content.tone.isEmpty ? missing : content.tone)")
                lines.append("• 身份与业务范围：\(personaSummary(content.persona))")
                lines.append("• 核心目标：\(listText(content.goals))")
                lines.append("• 业务规则：\(listText(content.businessRules))")
                lines.append("• 引用知识：\(knowledgeText(content.knowledgeRefs))")
                lines.append("• 异常回复模板：\(templatesText(content.templates))")
                lines.append("• 上下文预算：\(budgetText(content.contextBudget))")
            }
            lines.append("")
            lines.append("字段级差异不可用：需要两个版本都返回完整草稿内容；本次只展示上方的版本元信息与可用一侧的已保存内容。")
            return lines.joined(separator: "\n")
        }

        guard let contentA = contentA, let contentB = contentB else {
            return lines.joined(separator: "\n")
        }
        compare("名称", Optional(contentA.name), Optional(contentB.name)) { $0.isEmpty ? missing : $0 }
        compare("语气风格", Optional(contentA.tone), Optional(contentB.tone)) { $0.isEmpty ? missing : $0 }
        compare("身份与业务范围", Optional(contentA.persona), Optional(contentB.persona)) { personaSummary($0) }
        compare("核心目标", Optional(contentA.goals), Optional(contentB.goals)) { listText($0) }
        compare("业务规则", Optional(contentA.businessRules), Optional(contentB.businessRules)) { listText($0) }
        compare("引用知识", Optional(contentA.knowledgeRefs), Optional(contentB.knowledgeRefs)) { knowledgeText($0) }
        compare("异常回复模板", Optional(contentA.templates), Optional(contentB.templates)) { templatesText($0) }
        compare("上下文预算", Optional(contentA.contextBudget), Optional(contentB.contextBudget)) { budgetText($0) }
        compare("父版本", contentA.parentRevisionId, contentB.parentRevisionId) { $0 }
        compare("内容摘要 sha256", Optional(contentA.contentSha256), Optional(contentB.contentSha256)) { $0.isEmpty ? missing : $0 }
        return lines.joined(separator: "\n")
    }

    /// 试一试：只有后端真的返回 decision.reply 才展示回复；否则返回 nil，由界面明确提示不可用。
    public static func previewReplyText(_ result: BrainPreviewResult) -> String? {
        guard let decision = result.decision else { return nil }
        guard let raw = decision["reply"]?.value as? String else { return nil }
        let reply = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return reply.isEmpty ? nil : reply
    }

    /// 试一试：从组装内容中解析后端固定的系统约束；解析不到就返回 nil（不伪造）。
    public static func systemContractText(fromCompiled compiled: String?) -> String? {
        guard let compiled = compiled, !compiled.isEmpty, let data = compiled.data(using: .utf8) else { return nil }
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        guard let contract = object["system_contract"] as? String, !contract.isEmpty else { return nil }
        return contract
    }

    public static func isMigratedRole(_ entry: BrainRoleEntry) -> Bool {
        return entry.name.contains("从旧版迁移") || entry.roleId.hasPrefix("migrated_")
    }

    public static func handleSaveConflict(localDraft: BrainRoleRevision, error: BrainControlError) -> (preservedDraft: BrainRoleRevision, errorMessage: String) {
        let errDesc = "并发修改冲突: \(error.message)。已保留您当前编辑的草稿，请核对最新版本后再次保存。"
        return (localDraft, errDesc)
    }

    public static func prepareRoleCopy(from source: BrainRoleRevision, newRoleId: String, newName: String) -> BrainRoleRevision {
        var copy = source
        copy.roleId = newRoleId
        copy.revisionId = "rev_\(UUID().uuidString.prefix(8))"
        copy.parentRevisionId = source.revisionId
        copy.name = newName
        copy.createdAt = UInt64(Date().timeIntervalSince1970)
        copy.origin = "user"
        return copy
    }

    public static func canArchiveRole(_ entry: BrainRoleEntry) -> Bool {
        return entry.activeBindingCount == 0
    }

    public static func validateOverridesAgainstAllowlist(overrides: [BrainOverride], allowlist: [[String: Any]]) -> Bool {
        let authorizedRecipients = Set(allowlist.compactMap { $0["recipient"] as? String })
        for ov in overrides {
            if !authorizedRecipients.contains(ov.contactId) {
                return false
            }
        }
        return true
    }

    public static func setDefaultRole(in bindings: BrainBindingSet, roleId: String, revisionId: String) -> BrainBindingSet {
        var copy = bindings
        copy.defaultRef = BrainRoleRef(roleId: roleId, revisionId: revisionId)
        return copy
    }

    public static func addOverride(
        to bindings: BrainBindingSet,
        contact: BrainAllowlistContact,
        roleId: String,
        revisionId: String,
        allowlist: [BrainAllowlistContact]
    ) -> Result<BrainBindingSet, BrainBindingError> {
        let isAllowed = allowlist.contains {
            $0.contactId == contact.contactId && ($0.chatKind == contact.chatKind || contact.chatKind.isEmpty)
        }
        guard isAllowed else {
            return .failure(.notInAllowlist)
        }
        guard !bindings.overrides.contains(where: { $0.contactId == contact.contactId }) else {
            return .failure(.duplicateContact)
        }
        var copy = bindings
        copy.overrides.append(BrainOverride(
            contactId: contact.contactId,
            chatKind: contact.chatKind,
            roleId: roleId,
            revisionId: revisionId
        ))
        return .success(copy)
    }

    public static func updateOverride(
        in bindings: BrainBindingSet,
        contactId: String,
        newRoleId: String,
        newRevisionId: String
    ) -> Result<BrainBindingSet, BrainBindingError> {
        guard let idx = bindings.overrides.firstIndex(where: { $0.contactId == contactId }) else {
            return .failure(.overrideNotFound)
        }
        var copy = bindings
        let old = copy.overrides[idx]
        copy.overrides[idx] = BrainOverride(
            contactId: old.contactId,
            chatKind: old.chatKind,
            roleId: newRoleId,
            revisionId: newRevisionId
        )
        return .success(copy)
    }

    public static func activationBindings(role: BrainRoleRevision, bindings: BrainBindingSet) -> BrainBindingSet {
        var result = bindings
        let referenced = result.defaultRef?.roleId == role.roleId || result.overrides.contains { $0.roleId == role.roleId }
        if !referenced || result.defaultRef?.roleId == role.roleId {
            result.defaultRef = BrainRoleRef(roleId: role.roleId, revisionId: role.revisionId)
        }
        for index in result.overrides.indices where result.overrides[index].roleId == role.roleId {
            result.overrides[index].revisionId = role.revisionId
        }
        return result
    }

    public static func deleteOverride(from bindings: BrainBindingSet, contactId: String) -> BrainBindingSet {
        var copy = bindings
        copy.overrides.removeAll(where: { $0.contactId == contactId })
        return copy
    }

    public static func handleBindingsConflict(
        localDraft: BrainBindingSet,
        error: BrainControlError
    ) -> (preservedBindings: BrainBindingSet, errorMessage: String) {
        let msg = "保存绑定冲突 [\(error.code)]：\(error.message)。已保留您当前未提交的绑定草稿与表单，请稍后重试。"
        return (preservedBindings: localDraft, errorMessage: msg)
    }
}

// MARK: - BrainClient

public final class BrainClient {
    public typealias CommandExecutor = (String, [String], Double) -> ControlRun

    private let configPath: String
    private let executor: CommandExecutor

    public func getConfigPath() -> String {
        return configPath
    }

    public init(configPath: String? = nil, executor: @escaping CommandExecutor = runControlCommand) {
        self.configPath = configPath ?? (FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".WeChatRPABot/config.json").path)
        self.executor = executor
    }

    private func executeAction(_ action: String, requestObj: [String: Any]? = nil) -> ControlRun {
        var args = ["control", action, "--config", configPath]
        if let req = requestObj, let data = try? JSONSerialization.data(withJSONObject: req, options: [.sortedKeys]), let jsonStr = String(data: data, encoding: .utf8) {
            args.append("--request-json")
            args.append(jsonStr)
        }
        return executor(controlBinaryPath(), args, 15.0)
    }

    public func fetchList() throws -> BrainListPayload {
        let run = executeAction("brain-list")
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "未知错误"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        var payload = try BrainPageLogic.parseBrainList(jsonString: outStr)
        for index in payload.roles.indices {
            let entry = payload.roles[index]
            if entry.archived { continue }
            let role = try getRole(roleId: entry.roleId, revisionId: entry.draftRevisionId)
            payload.roles[index].name = role.name
            var revisions: [String] = []
            if let ref = payload.active?.defaultRef, ref.roleId == entry.roleId { revisions.append(ref.revisionId) }
            for ref in payload.active?.overrides ?? [] where ref.roleId == entry.roleId { revisions.append(ref.revisionId) }
            payload.roles[index].activeBindingCount = revisions.count
            payload.roles[index].activeRevisionId = revisions.contains(entry.draftRevisionId) ? entry.draftRevisionId : revisions.first
        }
        return payload
    }

    public func getRole(roleId: String, revisionId: String? = nil) throws -> BrainRoleRevision {
        var req: [String: Any] = ["role_id": roleId]
        if let r = revisionId { req["revision_id"] = r }
        let run = executeAction("brain-get", requestObj: req)
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "未知错误"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        let resp = try JSONDecoder().decode(BrainControlResponse<BrainRoleRevision>.self, from: run.output)
        guard resp.ok, let role = resp.data else {
            throw NSError(domain: "BrainClient", code: 2, userInfo: [NSLocalizedDescriptionKey: resp.error?.message ?? "获取角色失败"])
        }
        return role
    }

    private func parseGenerationResponse(_ run: ControlRun, actionName: String) throws -> UInt64 {
        guard run.status == 0 else {
            if let errResp = try? JSONDecoder().decode(BrainControlResponse<[String: AnyCodable]>.self, from: run.output),
               let err = errResp.error {
                throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: err.message, "error_code": err.code])
            }
            let errStr = String(data: run.output, encoding: .utf8) ?? "\(actionName)失败 (exit \(run.status))"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        guard let json = try? JSONSerialization.jsonObject(with: run.output) as? [String: Any],
              json["ok"] as? Bool == true,
              let data = json["data"] as? [String: Any],
              let gen = (data["generation"] as? NSNumber)?.uint64Value else {
            throw NSError(domain: "BrainClient", code: 2, userInfo: [NSLocalizedDescriptionKey: "\(actionName)返回无效数据，无法获取 generation"])
        }
        return gen
    }

    public func initialize(installation: String = "fresh") throws -> UInt64 {
        let req: [String: Any] = ["installation": installation]
        let run = executeAction("brain-initialize", requestObj: req)
        return try parseGenerationResponse(run, actionName: "初始化大脑")
    }

    /// 只读版本历史（后端 brain-history）。失败时抛出可展示的中文错误。
    public func fetchHistory(roleId: String? = nil) throws -> BrainHistoryPayload {
        var req: [String: Any] = [:]
        if let rid = roleId, !rid.isEmpty {
            req["role_id"] = rid
        }
        let run = executeAction("brain-history", requestObj: req)
        guard run.status == 0, !run.timedOut else {
            let fallback = run.timedOut ? "读取版本历史超时" : "读取版本历史失败 (exit \(run.status))"
            let errStr = String(data: run.output, encoding: .utf8) ?? fallback
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try BrainPageLogic.parseHistoryPayload(jsonString: outStr)
    }

    public func saveRole(_ role: BrainRoleRevision, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let roleData = try JSONEncoder().encode(role)
        let roleDict = try JSONSerialization.jsonObject(with: roleData)
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "role": roleDict
        ]
        let run = executeAction("brain-save-role", requestObj: req)
        return try parseGenerationResponse(run, actionName: "保存角色")
    }

    public struct SavedKnowledgeResult {
        public let generation: UInt64
        public let knowledgeId: String
        public let revisionId: String
        public let contentSha256: String
    }

    public func saveKnowledge(_ knowledge: BrainKnowledgeRevision, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> SavedKnowledgeResult {
        let kData = try JSONEncoder().encode(knowledge)
        let kDict = try JSONSerialization.jsonObject(with: kData)
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "knowledge": kDict
        ]
        let run = executeAction("brain-save-knowledge", requestObj: req)
        guard run.status == 0 else {
            if let errResp = try? JSONDecoder().decode(BrainControlResponse<[String: AnyCodable]>.self, from: run.output),
               let err = errResp.error {
                throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: err.message, "error_code": err.code])
            }
            let errStr = String(data: run.output, encoding: .utf8) ?? "保存知识失败"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        guard let json = try? JSONSerialization.jsonObject(with: run.output) as? [String: Any],
              json["ok"] as? Bool == true,
              let data = json["data"] as? [String: Any],
              let gen = (data["generation"] as? NSNumber)?.uint64Value,
              let kid = data["knowledge_id"] as? String,
              let rev = data["revision_id"] as? String,
              let sha = data["content_sha256"] as? String else {
            throw NSError(domain: "BrainClient", code: 2, userInfo: [NSLocalizedDescriptionKey: "保存知识返回无效响应"])
        }
        return SavedKnowledgeResult(generation: gen, knowledgeId: kid, revisionId: rev, contentSha256: sha)
    }

    public func copyRole(roleId: String, revisionId: String, newRoleId: String? = nil, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        var req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "role_id": roleId,
            "revision_id": revisionId
        ]
        if let nid = newRoleId { req["new_role_id"] = nid }
        let run = executeAction("brain-copy-role", requestObj: req)
        return try parseGenerationResponse(run, actionName: "复制角色")
    }

    public func archiveRole(roleId: String, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "role_id": roleId
        ]
        let run = executeAction("brain-archive-role", requestObj: req)
        return try parseGenerationResponse(run, actionName: "归档角色")
    }

    public func saveBindings(draft: BrainBindingSet, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let draftData = try JSONEncoder().encode(draft)
        let draftDict = try JSONSerialization.jsonObject(with: draftData)
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "draft": draftDict
        ]
        let run = executeAction("brain-save-bindings", requestObj: req)
        return try parseGenerationResponse(run, actionName: "保存绑定")
    }

    public func activate(expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased()
        ]
        let run = executeAction("brain-activate", requestObj: req)
        return try parseGenerationResponse(run, actionName: "启用配置")
    }

    public func rollback(targetGeneration: UInt64, expectedGeneration: UInt64, requestId: String = UUID().uuidString.lowercased()) throws -> UInt64 {
        let req: [String: Any] = [
            "expected_generation": expectedGeneration,
            "request_id": requestId.lowercased(),
            "target_generation": targetGeneration
        ]
        let run = executeAction("brain-rollback", requestObj: req)
        return try parseGenerationResponse(run, actionName: "回退配置")
    }

    public func preview(roleId: String, revisionId: String? = nil, text: String, chatKind: String = "private") throws -> BrainPreviewResult {
        var req: [String: Any] = [
            "role_id": roleId,
            "execution": "assemble_only",
            "cost_consent": false,
            "input": [
                "text": text,
                "chat_kind": chatKind,
                "files": [],
                "images": [],
                "failed": [],
                "context": []
            ]
        ]
        if let r = revisionId { req["revision_id"] = r }
        let run = executeAction("brain-preview", requestObj: req)
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "组装预览失败"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        let outStr = String(data: run.output, encoding: .utf8) ?? "{}"
        return try BrainPageLogic.parsePreviewResult(jsonString: outStr)
    }

    public func turnDetail(turnId: String) throws -> BrainTurnDetail {
        let req: [String: Any] = ["turn_id": turnId]
        let run = executeAction("brain-turn-detail", requestObj: req)
        guard run.status == 0 else {
            let errStr = String(data: run.output, encoding: .utf8) ?? "获取轮次详情失败"
            throw NSError(domain: "BrainClient", code: Int(run.status), userInfo: [NSLocalizedDescriptionKey: errStr])
        }
        let resp = try JSONDecoder().decode(BrainControlResponse<BrainTurnDetail>.self, from: run.output)
        guard resp.ok, let detail = resp.data else {
            throw NSError(domain: "BrainClient", code: 2, userInfo: [NSLocalizedDescriptionKey: resp.error?.message ?? "轮次不存在"])
        }
        return detail
    }
}

// MARK: - BrainPageController (AppKit View Controller)

public final class BrainPageController: NSViewController, NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate, NSTextViewDelegate {
    private let context: StatusPanelController
    private var client: BrainClient
    private var listPayload: BrainListPayload?
    private var selectedRoleIndex: Int = -1
    private var preferredRoleId: String?
    private var roleLoadToken = UUID()
    private var focusRoleAfterLoad: String?
    private var hasStoredRoles = false
    private var activationInFlight = false
    private var currentDraftRole: BrainRoleRevision?
    private var localUnsavedChanges: Bool = false

    // UI - Main Layout
    private let splitView = NSSplitView()
    private let roleTable = NSTableView()
    private let rightContainer = NSView()

    // UI - Header Area
    private let accountLabel = NSTextField(labelWithString: "当前账号: -")
    private let modeLabel = NSTextField(labelWithString: "回复方式：待加载")
    private let toMcpButton = NSButton(title: "模型与连接…", target: nil, action: nil)
    private let statusMessageLabel = NSTextField(wrappingLabelWithString: "")
    private let loadErrorLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Role Management Buttons
    private let newRoleButton = NSButton(title: "新增角色", target: nil, action: nil)
    private let useAsDefaultButton = NSButton(title: "设为默认角色…", target: nil, action: nil)
    private let copyRoleButton = NSButton(title: "复制角色", target: nil, action: nil)
    private let archiveRoleButton = NSButton(title: "删除角色", target: nil, action: nil)

    // UI - Tab Segment
    private let sectionSegment = NSSegmentedControl(labels: ["角色设定", "知识", "异常回复", "适用会话", "历史版本", "试一试"], trackingMode: .selectOne, target: nil, action: nil)
    private let tabContainer = NSView()

    // Tab Views
    private let definitionView = NSView()
    private let knowledgeView = NSView()
    private let templatesView = NSView()
    private let bindingsView = NSView()
    private let versionView = NSView()
    private let previewView = NSView()

    // 历史版本（issue-2）状态与控件（文案暂用中文硬编码，后续统一迁入 L10n）
    private var historyGenerations: [BrainHistoryGeneration] = []
    private var historyVersions: [BrainHistoryVersion] = []
    private var historyLoadToken = UUID()
    private let historyTable = NSTableView()
    private let historyEmptyLabel = NSTextField(wrappingLabelWithString: "")
    private let historyStatusLabel = NSTextField(wrappingLabelWithString: "版本历史：尚未加载")
    private let historyDetailNoticeLabel = NSTextField(wrappingLabelWithString: "")
    private let diffVerAPopup = NSPopUpButton()
    private let diffVerBPopup = NSPopUpButton()
    private let runDiffButton = NSButton(title: "对比差异", target: nil, action: nil)
    private let diffTextView = NSTextView()
    private let rollbackPicker = NSPopUpButton()
    private let rollbackToVersionButton = NSButton(title: "回滚至选定版本…", target: nil, action: nil)

    // 试一试（issue-2）状态与控件
    private let tryItModelStatusLabel = NSTextField(wrappingLabelWithString: "真实模型调用：状态待加载")

    // Form Inputs - Definition
    private let roleNameField = NSTextField()
    private let personaField = NSTextView()
    private let toneField = NSTextField()
    private let goalsField = NSTextField()
    private let rulesField = NSTextField()

    // Form Inputs - Knowledge
    private let knowledgeTitleField = NSTextField()
    private let knowledgeSourceField = NSTextField()
    private let knowledgeContentField = NSTextView()
    private let importKnowledgeButton = NSButton(title: "导入并关联至角色", target: nil, action: nil)
    private let knowledgeRefsSummaryLabel = NSTextField(wrappingLabelWithString: "当前角色未引用任何知识")

    // Form Inputs - Templates
    private let greetingField = NSTextField()
    private let insufficientInfoField = NSTextField()
    private let unreadableTypeField = NSTextField()
    private let unreadableLargeField = NSTextField()

    // Form Inputs - Bindings
    public private(set) var currentDraftBindings: BrainBindingSet = BrainBindingSet()
    public private(set) var hasUnsavedBindingsChanges: Bool = false
    public private(set) var allowlistContacts: [BrainAllowlistContact] = []
    public private(set) var allowlistContactsStatus: String = "ready"
    public private(set) var allowlistContactsError: String? = nil
    public private(set) var currentSaveToken: UInt64 = 0
    public var onSaveBindingsFinished: (() -> Void)? = nil

    public func isAddOverrideButtonEnabled() -> Bool {
        return addOverrideButton.isEnabled
    }
    public func isUpdateOverrideButtonEnabled() -> Bool {
        return updateOverrideButton.isEnabled
    }
    public func isDeleteOverrideButtonEnabled() -> Bool {
        return deleteOverrideButton.isEnabled
    }
    public func bumpSaveTokenForTest() {
        currentSaveToken += 1
    }
    public func triggerSaveBindingsDraftAction() {
        handleSaveBindingsDraft()
    }
    public func updateDraftBindingsForTest(_ draft: BrainBindingSet) {
        self.currentDraftBindings = draft
    }

    private let defaultRolePopup = NSPopUpButton()
    private let overridesSummaryLabel = NSTextField(wrappingLabelWithString: "暂无特定联系人覆盖")
    private let overridesTableView = NSTableView()
    private let overrideContactPopup = NSPopUpButton()
    private let overrideRolePopup = NSPopUpButton()
    private let addOverrideButton = NSButton(title: "添加覆盖", target: nil, action: nil)
    private let updateOverrideButton = NSButton(title: "更新选定覆盖", target: nil, action: nil)
    private let deleteOverrideButton = NSButton(title: "删除选定覆盖", target: nil, action: nil)
    private let saveBindingsDraftButton = NSButton(title: "保存绑定草稿", target: nil, action: nil)
    private let bindingsStatusLabel = NSTextField(wrappingLabelWithString: "")

    // Form Inputs - Version & Action
    private let saveDraftButton = NSButton(title: "保存草稿", target: nil, action: nil)
    private let activateButton = NSButton(title: "启用配置", target: nil, action: nil)
    private let rollbackButton = NSButton(title: "回退版本", target: nil, action: nil)
    private let revisionStatusLabel = NSTextField(wrappingLabelWithString: "草稿: - · 运行: -")

    // Form Inputs - Preview
    private let previewInputField = NSTextField()
    private let previewChatKindPopup = NSPopUpButton()
    private let runPreviewButton = NSButton(title: "组装预览", target: nil, action: nil)
    private let previewResultTextView = NSTextView()

    public init(context: StatusPanelController, client: BrainClient? = nil) {
        self.context = context
        self.client = client ?? BrainClient()
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 720, height: 760))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        reloadBrainData()
    }

    private func setupUI() {
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(splitView)

        // 左侧栏：角色列表与操作
        let leftView = NSView()
        leftView.translatesAutoresizingMaskIntoConstraints = false
        setupLeftSidebar(inside: leftView)

        // 右侧区：顶部摘要、Tab、内容、底部动作
        rightContainer.translatesAutoresizingMaskIntoConstraints = false
        setupRightContent(inside: rightContainer)

        splitView.addArrangedSubview(leftView)
        splitView.addArrangedSubview(rightContainer)

        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: view.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            leftView.widthAnchor.constraint(equalToConstant: 168)
        ])
    }

    private func setupLeftSidebar(inside parent: NSView) {
        let titleLabel = NSTextField(labelWithString: "角色列表")
        titleLabel.font = NSFont.systemFont(ofSize: 13, weight: .bold)

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.translatesAutoresizingMaskIntoConstraints = false

        let col = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("RoleCol"))
        col.title = "角色"
        roleTable.addTableColumn(col)
        roleTable.headerView = nil
        roleTable.dataSource = self
        roleTable.delegate = self
        roleTable.target = self
        roleTable.doubleAction = #selector(handleEditRole)
        roleTable.toolTip = "双击角色名编辑"
        roleTable.rowHeight = 44
        roleTable.style = .sourceList
        roleTable.setAccessibilityLabel("角色列表")
        scroll.documentView = roleTable

        newRoleButton.target = self
        newRoleButton.action = #selector(handleNewRole)
        copyRoleButton.target = self
        copyRoleButton.action = #selector(handleCopyRole)
        archiveRoleButton.target = self
        archiveRoleButton.action = #selector(handleArchiveRole)

        parent.addSubview(titleLabel)
        parent.addSubview(scroll)

        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: parent.topAnchor, constant: 14),
            titleLabel.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 12),
            titleLabel.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -12),

            scroll.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 12),
            scroll.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 8),
            scroll.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -8),
            scroll.bottomAnchor.constraint(equalTo: parent.bottomAnchor, constant: -12)
        ])
    }

    private func setupRightContent(inside parent: NSView) {
        let headerBox = NSBox()
        headerBox.titlePosition = .noTitle
        let headerStack = NSStackView()
        headerStack.orientation = .vertical
        headerStack.alignment = .leading
        headerStack.spacing = 6

        accountLabel.font = NSFont.systemFont(ofSize: 13, weight: .bold)
        modeLabel.font = NSFont.systemFont(ofSize: 12)
        toMcpButton.target = self
        toMcpButton.action = #selector(handleToMcp)

        let topRow = NSStackView(views: [accountLabel, toMcpButton])
        topRow.orientation = .horizontal
        topRow.spacing = 10
        headerStack.addArrangedSubview(topRow)
        let roleToolbar = NSStackView(views: [modeLabel, newRoleButton, copyRoleButton, archiveRoleButton])
        roleToolbar.spacing = 8
        headerStack.addArrangedSubview(roleToolbar)

        statusMessageLabel.font = NSFont.systemFont(ofSize: 11)
        loadErrorLabel.font = NSFont.systemFont(ofSize: 11)
        loadErrorLabel.textColor = .systemRed
        headerStack.addArrangedSubview(statusMessageLabel)
        headerStack.addArrangedSubview(loadErrorLabel)
        headerBox.setContent(headerStack, inset: 8)

        // Section Segment
        sectionSegment.selectedSegment = 0
        sectionSegment.target = self
        sectionSegment.action = #selector(handleSectionChanged)

        // Tabs
        tabContainer.translatesAutoresizingMaskIntoConstraints = false
        setupTabViews()

        // Bottom Action Bar: Two-line flexible layout (status on top, action buttons below)
        // Ensures buttons (save, activate, rollback) are never clipped at right edge in 720~760px windows.
        let bottomBar = NSBox()
        bottomBar.titlePosition = .noTitle
        let bottomStack = NSStackView()
        bottomStack.orientation = .vertical
        bottomStack.alignment = .leading
        bottomStack.spacing = 6

        revisionStatusLabel.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        revisionStatusLabel.maximumNumberOfLines = 0
        revisionStatusLabel.lineBreakMode = .byWordWrapping
        revisionStatusLabel.preferredMaxLayoutWidth = 500

        saveDraftButton.bezelStyle = .rounded
        saveDraftButton.bezelColor = .systemGreen
        saveDraftButton.contentTintColor = .white
        saveDraftButton.keyEquivalent = "s"
        saveDraftButton.keyEquivalentModifierMask = [.command]
        saveDraftButton.target = self
        saveDraftButton.action = #selector(handleSaveDraft)
        useAsDefaultButton.target = self
        useAsDefaultButton.action = #selector(handleUseAsDefault)
        activateButton.target = self
        activateButton.action = #selector(handleActivate)
        rollbackButton.target = self
        rollbackButton.action = #selector(handleRollback)

        let actionRow = NSStackView(views: [
            saveDraftButton,
            useAsDefaultButton,
            activateButton
        ])
        actionRow.orientation = .horizontal
        actionRow.spacing = 10
        actionRow.alignment = .centerY

        bottomStack.addArrangedSubview(revisionStatusLabel)
        bottomStack.addArrangedSubview(actionRow)
        bottomBar.setContent(bottomStack, inset: 8)

        parent.addSubview(headerBox)
        parent.addSubview(sectionSegment)
        parent.addSubview(tabContainer)
        parent.addSubview(bottomBar)

        headerBox.translatesAutoresizingMaskIntoConstraints = false
        sectionSegment.translatesAutoresizingMaskIntoConstraints = false
        bottomBar.translatesAutoresizingMaskIntoConstraints = false

        NSLayoutConstraint.activate([
            headerBox.topAnchor.constraint(equalTo: parent.topAnchor, constant: 10),
            headerBox.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 14),
            headerBox.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -14),

            sectionSegment.topAnchor.constraint(equalTo: headerBox.bottomAnchor, constant: 10),
            sectionSegment.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 14),

            tabContainer.topAnchor.constraint(equalTo: sectionSegment.bottomAnchor, constant: 10),
            tabContainer.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 14),
            tabContainer.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -14),
            tabContainer.bottomAnchor.constraint(equalTo: bottomBar.topAnchor, constant: -10),

            bottomBar.leadingAnchor.constraint(equalTo: parent.leadingAnchor, constant: 14),
            bottomBar.trailingAnchor.constraint(equalTo: parent.trailingAnchor, constant: -14),
            bottomBar.bottomAnchor.constraint(equalTo: parent.bottomAnchor, constant: -12)
        ])

        switchTab(index: 0)
    }

    private func setupTabViews() {
        // Tab 0: 定义
        setupDefinitionTab()
        // Tab 1: 知识
        setupKnowledgeTab()
        // Tab 2: 话术
        setupTemplatesTab()
        // Tab 3: 绑定
        setupBindingsTab()
        // Tab 4: 历史版本
        setupVersionTab()
        // Tab 5: 试一试
        setupPreviewTab()
    }

    private func setupDefinitionTab() {
        definitionView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let nameRow = NSStackView(views: [NSTextField(labelWithString: "角色名称:"), roleNameField])
        roleNameField.widthAnchor.constraint(equalToConstant: 240).isActive = true
        stack.addArrangedSubview(nameRow)

        let toneRow = NSStackView(views: [NSTextField(labelWithString: "语气风格:"), toneField])
        toneField.widthAnchor.constraint(equalToConstant: 240).isActive = true
        stack.addArrangedSubview(toneRow)

        stack.addArrangedSubview(NSTextField(labelWithString: "身份与业务范围："))
        let personaScroll = NSScrollView()
        personaScroll.hasVerticalScroller = true
        personaField.isRichText = false
        personaField.font = NSFont.systemFont(ofSize: 12)
        personaField.frame = NSRect(x: 0, y: 0, width: 460, height: 120)
        personaField.isVerticallyResizable = true
        personaField.isHorizontallyResizable = false
        personaField.autoresizingMask = [.width]
        personaField.minSize = NSSize(width: 0, height: 120)
        personaField.maxSize = NSSize(width: 460, height: CGFloat.greatestFiniteMagnitude)
        personaField.textContainer?.widthTracksTextView = false
        personaField.textContainer?.containerSize = NSSize(width: 444, height: CGFloat.greatestFiniteMagnitude)
        personaField.textContainerInset = NSSize(width: 8, height: 8)
        personaScroll.documentView = personaField
        personaScroll.heightAnchor.constraint(equalToConstant: 120).isActive = true
        personaScroll.widthAnchor.constraint(equalToConstant: 460).isActive = true
        stack.addArrangedSubview(personaScroll)

        let goalsRow = NSStackView(views: [NSTextField(labelWithString: "核心目标:"), goalsField])
        goalsField.placeholderString = "用逗号分隔多个目标"
        goalsField.widthAnchor.constraint(equalToConstant: 340).isActive = true
        stack.addArrangedSubview(goalsRow)

        let rulesRow = NSStackView(views: [NSTextField(labelWithString: "业务规则:"), rulesField])
        rulesField.placeholderString = "用逗号分隔业务规则"
        rulesField.widthAnchor.constraint(equalToConstant: 340).isActive = true
        stack.addArrangedSubview(rulesRow)

        let constraintsBox = NSBox()
        constraintsBox.title = "系统能力约束 (只读)"
        let cText = NSTextField(wrappingLabelWithString: "不可虚构事实、语音转写以本轮转写为准、未识别项点名并依提示回复。本约束由代码自动拼接在角色人设之后，不可在界面删改。")
        cText.font = NSFont.systemFont(ofSize: 11)
        cText.textColor = .secondaryLabelColor
        cText.maximumNumberOfLines = 0
        cText.lineBreakMode = .byWordWrapping
        cText.preferredMaxLayoutWidth = 460
        constraintsBox.setContent(cText, inset: 8)
        constraintsBox.widthAnchor.constraint(equalToConstant: 480).isActive = true
        stack.addArrangedSubview(constraintsBox)

        definitionView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: definitionView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: definitionView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: definitionView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: definitionView.bottomAnchor)
        ])
    }

    private func setupKnowledgeTab() {
        knowledgeView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let intro = NSTextField(wrappingLabelWithString: "业务知识 (Markdown/纯文本)。通过核心 brain-save-knowledge 控制接口保存，自动计算 SHA256 摘要并固定版本。")
        intro.font = NSFont.systemFont(ofSize: 12)
        intro.textColor = .secondaryLabelColor
        intro.maximumNumberOfLines = 0
        intro.lineBreakMode = .byWordWrapping
        intro.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(intro)

        stack.addArrangedSubview(NSTextField(labelWithString: "当前角色引用的知识列表:"))
        knowledgeRefsSummaryLabel.font = NSFont.systemFont(ofSize: 12)
        knowledgeRefsSummaryLabel.maximumNumberOfLines = 0
        knowledgeRefsSummaryLabel.lineBreakMode = .byWordWrapping
        knowledgeRefsSummaryLabel.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(knowledgeRefsSummaryLabel)

        let addBox = NSBox()
        addBox.title = "导入/新建知识"
        let addStack = NSStackView()
        addStack.orientation = .vertical
        addStack.alignment = .leading
        addStack.spacing = 8

        let titleRow = NSStackView(views: [NSTextField(labelWithString: "知识标题:"), knowledgeTitleField])
        knowledgeTitleField.widthAnchor.constraint(equalToConstant: 240).isActive = true
        addStack.addArrangedSubview(titleRow)

        let srcRow = NSStackView(views: [NSTextField(labelWithString: "来源标识:"), knowledgeSourceField])
        knowledgeSourceField.placeholderString = "例如 faq.md 或 manual.txt"
        knowledgeSourceField.widthAnchor.constraint(equalToConstant: 240).isActive = true
        addStack.addArrangedSubview(srcRow)

        addStack.addArrangedSubview(NSTextField(labelWithString: "知识正文 (Markdown):"))
        let textScroll = NSScrollView()
        textScroll.hasVerticalScroller = true
        knowledgeContentField.isRichText = false
        knowledgeContentField.font = NSFont.userFixedPitchFont(ofSize: 11)
        textScroll.documentView = knowledgeContentField
        textScroll.heightAnchor.constraint(equalToConstant: 120).isActive = true
        textScroll.widthAnchor.constraint(equalToConstant: 440).isActive = true
        addStack.addArrangedSubview(textScroll)

        importKnowledgeButton.target = self
        importKnowledgeButton.action = #selector(handleImportKnowledge)
        addStack.addArrangedSubview(importKnowledgeButton)

        addBox.setContent(addStack, inset: 8)
        stack.addArrangedSubview(addBox)

        knowledgeView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: knowledgeView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: knowledgeView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: knowledgeView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: knowledgeView.bottomAnchor)
        ])
    }

    private func setupTemplatesTab() {
        templatesView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false

        let intro = NSTextField(wrappingLabelWithString: "通用话术模板 (留空表示无自定义文案，进入待处理或人工接管):")
        intro.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        intro.maximumNumberOfLines = 0
        intro.lineBreakMode = .byWordWrapping
        intro.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(intro)

        let gRow = NSStackView(views: [NSTextField(labelWithString: "欢迎语:"), greetingField])
        greetingField.widthAnchor.constraint(equalToConstant: 320).isActive = true
        stack.addArrangedSubview(gRow)

        let iRow = NSStackView(views: [NSTextField(labelWithString: "信息不足:"), insufficientInfoField])
        insufficientInfoField.widthAnchor.constraint(equalToConstant: 320).isActive = true
        stack.addArrangedSubview(iRow)

        let u1Row = NSStackView(views: [NSTextField(labelWithString: "格式不支持:"), unreadableTypeField])
        unreadableTypeField.widthAnchor.constraint(equalToConstant: 320).isActive = true
        stack.addArrangedSubview(u1Row)

        let u2Row = NSStackView(views: [NSTextField(labelWithString: "文件超限:"), unreadableLargeField])
        unreadableLargeField.widthAnchor.constraint(equalToConstant: 320).isActive = true
        stack.addArrangedSubview(u2Row)

        templatesView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: templatesView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: templatesView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: templatesView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: templatesView.bottomAnchor)
        ])
    }

    private func setupBindingsTab() {
        bindingsView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let intro = NSTextField(wrappingLabelWithString: "账号与联系人绑定。联系人唯一来源为白名单，本处仅引用不复制联系人清单；修改仅暂存为草稿，点击「保存绑定草稿」通过 CAS 校验后落盘；启用配置后在下一轮对话生效。")
        intro.font = NSFont.systemFont(ofSize: 12)
        intro.textColor = .secondaryLabelColor
        intro.maximumNumberOfLines = 0
        intro.lineBreakMode = .byWordWrapping
        intro.preferredMaxLayoutWidth = 460
        stack.addArrangedSubview(intro)

        defaultRolePopup.widthAnchor.constraint(greaterThanOrEqualToConstant: 240).isActive = true
        defaultRolePopup.target = self
        defaultRolePopup.action = #selector(handleDefaultRoleChanged)
        let defaultRow = NSStackView(views: [NSTextField(labelWithString: "账号默认角色:"), defaultRolePopup])
        defaultRow.alignment = .centerY
        defaultRow.spacing = 8
        stack.addArrangedSubview(defaultRow)

        // 覆盖规则管理框 (包含列表表格、白名单下拉框、角色下拉框、增删改按钮与草稿保存)
        let ovBox = NSBox()
        ovBox.title = "特定联系人 / 群覆盖规则 (白名单唯一来源)"
        let ovStack = NSStackView()
        ovStack.orientation = .vertical
        ovStack.alignment = .leading
        ovStack.spacing = 6

        // 覆盖规则表格（自适应紧凑列宽并支持滚动）
        overridesTableView.delegate = self
        overridesTableView.dataSource = self
        overridesTableView.headerView = NSTableHeaderView()
        overridesTableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle

        let colContact = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_contact"))
        colContact.title = "联系人/群 (白名单)"
        colContact.width = 105
        colContact.minWidth = 70

        let colKind = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_kind"))
        colKind.title = "类型"
        colKind.width = 42
        colKind.minWidth = 35

        let colRole = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_role"))
        colRole.title = "绑定角色"
        colRole.width = 95
        colRole.minWidth = 70

        let colRev = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_rev"))
        colRev.title = "版本"
        colRev.width = 58
        colRev.minWidth = 45

        overridesTableView.addTableColumn(colContact)
        overridesTableView.addTableColumn(colKind)
        overridesTableView.addTableColumn(colRole)
        overridesTableView.addTableColumn(colRev)

        let scroll = NSScrollView()
        scroll.documentView = overridesTableView
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.heightAnchor.constraint(equalToConstant: 90).isActive = true
        scroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 310).isActive = true
        ovStack.addArrangedSubview(scroll)

        // 表单输入区：垂直流式分行（联系人一行、角色一行，在 680px 宿主宽度下各占整行，从容排布）
        let contactLbl = NSTextField(labelWithString: "白名单联系人:")
        contactLbl.font = NSFont.systemFont(ofSize: 12)
        contactLbl.widthAnchor.constraint(equalToConstant: 90).isActive = true
        overrideContactPopup.widthAnchor.constraint(greaterThanOrEqualToConstant: 240).isActive = true
        let rowContact = NSStackView(views: [contactLbl, overrideContactPopup])
        rowContact.alignment = .centerY
        rowContact.spacing = 8
        ovStack.addArrangedSubview(rowContact)

        let roleLbl = NSTextField(labelWithString: "绑定大脑角色:")
        roleLbl.font = NSFont.systemFont(ofSize: 12)
        roleLbl.widthAnchor.constraint(equalToConstant: 90).isActive = true
        overrideRolePopup.widthAnchor.constraint(greaterThanOrEqualToConstant: 240).isActive = true
        let rowRole = NSStackView(views: [roleLbl, overrideRolePopup])
        rowRole.alignment = .centerY
        rowRole.spacing = 8
        ovStack.addArrangedSubview(rowRole)

        // 按钮行：添加、更新、删除
        addOverrideButton.target = self
        addOverrideButton.action = #selector(handleAddOverride)
        updateOverrideButton.target = self
        updateOverrideButton.action = #selector(handleUpdateOverride)
        deleteOverrideButton.target = self
        deleteOverrideButton.action = #selector(handleDeleteOverride)

        let btnRow = NSStackView(views: [
            addOverrideButton,
            updateOverrideButton,
            deleteOverrideButton
        ])
        btnRow.alignment = .centerY
        btnRow.spacing = 8
        ovStack.addArrangedSubview(btnRow)

        // 只读摘要保留（展示当前所有生效或草稿覆盖）
        overridesSummaryLabel.font = NSFont.systemFont(ofSize: 11)
        overridesSummaryLabel.textColor = .secondaryLabelColor
        overridesSummaryLabel.maximumNumberOfLines = 0
        overridesSummaryLabel.lineBreakMode = .byWordWrapping
        overridesSummaryLabel.preferredMaxLayoutWidth = 460
        ovStack.addArrangedSubview(overridesSummaryLabel)

        ovBox.setContent(ovStack, inset: 8)
        ovBox.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(ovBox)

        // 保存绑定草稿行
        saveBindingsDraftButton.target = self
        saveBindingsDraftButton.action = #selector(handleSaveBindingsDraft)
        bindingsStatusLabel.font = NSFont.systemFont(ofSize: 11)
        bindingsStatusLabel.maximumNumberOfLines = 0
        bindingsStatusLabel.lineBreakMode = .byWordWrapping
        bindingsStatusLabel.preferredMaxLayoutWidth = 450

        let saveRow = NSStackView(views: [saveBindingsDraftButton, bindingsStatusLabel])
        saveRow.alignment = .centerY
        saveRow.spacing = 10
        stack.addArrangedSubview(saveRow)

        bindingsView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: bindingsView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: bindingsView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: bindingsView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: bindingsView.bottomAnchor),
            ovBox.widthAnchor.constraint(lessThanOrEqualTo: bindingsView.widthAnchor, constant: -8)
        ])
    }

    private func setupVersionTab() {
        versionView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let intro = NSTextField(wrappingLabelWithString: "版本管理：保存草稿不影响运行版本；启用配置仅对新轮次生效。支持并发修改检测 (CAS) 与原子激活/回退。")
        intro.font = NSFont.systemFont(ofSize: 12)
        intro.textColor = .secondaryLabelColor
        intro.maximumNumberOfLines = 0
        intro.lineBreakMode = .byWordWrapping
        intro.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(intro)

        let historyIntro = NSTextField(wrappingLabelWithString: "历史版本：只读列出后端保存的代次与版本，可对比任意两个版本的差异，并可带二次确认回滚到选定代次。旧代次没有完整草稿时只展示后端已保存内容，不补全、不编造。")
        historyIntro.font = NSFont.systemFont(ofSize: 12)
        historyIntro.textColor = .secondaryLabelColor
        historyIntro.maximumNumberOfLines = 0
        historyIntro.lineBreakMode = .byWordWrapping
        historyIntro.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(historyIntro)

        // 版本与代次规则说明（保留设计版原有说明）
        let verBox = NSBox()
        verBox.title = "版本与代次规则说明"
        let verStack = NSStackView()
        verStack.orientation = .vertical
        verStack.alignment = .leading
        verStack.spacing = 6

        let l1 = NSTextField(wrappingLabelWithString: "• 草稿与运行分离：保存草稿仅存入本地草稿区，不影响当前服务对外响应。")
        l1.maximumNumberOfLines = 0; l1.lineBreakMode = .byWordWrapping; l1.preferredMaxLayoutWidth = 460
        let l2 = NSTextField(wrappingLabelWithString: "• 原子激活：点击「启用配置」时将草稿生成新版本并原子推进激活代次。")
        l2.maximumNumberOfLines = 0; l2.lineBreakMode = .byWordWrapping; l2.preferredMaxLayoutWidth = 460
        let l3 = NSTextField(wrappingLabelWithString: "• 安全回退：点击「回退版本」将配置无损切换回上一稳定激活代次。")
        l3.maximumNumberOfLines = 0; l3.lineBreakMode = .byWordWrapping; l3.preferredMaxLayoutWidth = 460

        verStack.addArrangedSubview(l1)
        verStack.addArrangedSubview(l2)
        verStack.addArrangedSubview(l3)
        verBox.setContent(verStack, inset: 8)
        verBox.widthAnchor.constraint(equalToConstant: 480).isActive = true
        stack.addArrangedSubview(verBox)

        // 1. 代次与版本历史（只读 brain-history）
        let tableBox = NSBox()
        tableBox.title = "版本与代次历史"
        let tableStack = NSStackView()
        tableStack.orientation = .vertical
        tableStack.alignment = .leading
        tableStack.spacing = 4

        historyStatusLabel.font = NSFont.systemFont(ofSize: 11)
        historyStatusLabel.textColor = .secondaryLabelColor
        historyStatusLabel.maximumNumberOfLines = 0
        historyStatusLabel.lineBreakMode = .byWordWrapping
        historyStatusLabel.preferredMaxLayoutWidth = 460
        tableStack.addArrangedSubview(historyStatusLabel)

        historyTable.delegate = self
        historyTable.dataSource = self
        historyTable.headerView = NSTableHeaderView()
        historyTable.rowHeight = 22
        historyTable.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        historyTable.setAccessibilityLabel("版本与代次历史")

        let colGen = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_gen"))
        colGen.title = "代次"
        colGen.width = 54
        let colAt = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_at"))
        colAt.title = "生成时间"
        colAt.width = 106
        let colOp = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_op"))
        colOp.title = "操作"
        colOp.width = 92
        let colRev = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_rev"))
        colRev.title = "默认版本ID"
        colRev.width = 126
        let colStatus = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("col_status"))
        colStatus.title = "状态"
        colStatus.width = 104

        historyTable.addTableColumn(colGen)
        historyTable.addTableColumn(colAt)
        historyTable.addTableColumn(colOp)
        historyTable.addTableColumn(colRev)
        historyTable.addTableColumn(colStatus)

        let scroll = NSScrollView()
        scroll.documentView = historyTable
        scroll.hasVerticalScroller = true
        scroll.heightAnchor.constraint(equalToConstant: 110).isActive = true
        scroll.widthAnchor.constraint(equalToConstant: 470).isActive = true

        historyEmptyLabel.translatesAutoresizingMaskIntoConstraints = false
        historyEmptyLabel.font = NSFont.systemFont(ofSize: 12)
        historyEmptyLabel.textColor = .secondaryLabelColor
        historyEmptyLabel.alignment = .center
        historyEmptyLabel.stringValue = "版本历史：尚未加载"
        historyEmptyLabel.isHidden = true
        scroll.addSubview(historyEmptyLabel)

        NSLayoutConstraint.activate([
            historyEmptyLabel.centerXAnchor.constraint(equalTo: scroll.centerXAnchor),
            historyEmptyLabel.centerYAnchor.constraint(equalTo: scroll.centerYAnchor),
            historyEmptyLabel.widthAnchor.constraint(lessThanOrEqualTo: scroll.widthAnchor, constant: -20)
        ])

        tableStack.addArrangedSubview(scroll)

        historyDetailNoticeLabel.font = NSFont.systemFont(ofSize: 11)
        historyDetailNoticeLabel.textColor = .secondaryLabelColor
        historyDetailNoticeLabel.maximumNumberOfLines = 0
        historyDetailNoticeLabel.lineBreakMode = .byWordWrapping
        historyDetailNoticeLabel.preferredMaxLayoutWidth = 460
        tableStack.addArrangedSubview(historyDetailNoticeLabel)

        tableBox.setContent(tableStack, inset: 6)
        stack.addArrangedSubview(tableBox)

        // 2. 版本差异对比
        let diffBox = NSBox()
        diffBox.title = "两版本差异对比"
        let diffStack = NSStackView()
        diffStack.orientation = .vertical
        diffStack.alignment = .leading
        diffStack.spacing = 6

        let diffRow = NSStackView(views: [
            NSTextField(labelWithString: "版本 A:"),
            diffVerAPopup,
            NSTextField(labelWithString: "版本 B:"),
            diffVerBPopup,
            runDiffButton
        ])
        diffRow.alignment = .centerY
        diffRow.spacing = 6
        diffVerAPopup.widthAnchor.constraint(equalToConstant: 130).isActive = true
        diffVerBPopup.widthAnchor.constraint(equalToConstant: 130).isActive = true
        runDiffButton.target = self
        runDiffButton.action = #selector(handleRunDiff)
        diffStack.addArrangedSubview(diffRow)

        let diffScroll = NSScrollView()
        diffScroll.hasVerticalScroller = true
        diffTextView.isEditable = false
        diffTextView.font = NSFont.userFixedPitchFont(ofSize: 10)
        diffScroll.documentView = diffTextView
        diffScroll.heightAnchor.constraint(equalToConstant: 90).isActive = true
        diffScroll.widthAnchor.constraint(equalToConstant: 470).isActive = true
        diffStack.addArrangedSubview(diffScroll)
        diffBox.setContent(diffStack, inset: 6)
        stack.addArrangedSubview(diffBox)

        // 3. 回滚到历史版本（二次确认）
        let rollbackBox = NSBox()
        rollbackBox.title = "回滚到历史版本 (二次确认)"
        let rbStack = NSStackView()
        rbStack.orientation = .vertical
        rbStack.alignment = .leading
        rbStack.spacing = 6

        rollbackPicker.widthAnchor.constraint(equalToConstant: 160).isActive = true
        rollbackToVersionButton.target = self
        rollbackToVersionButton.action = #selector(handleRollbackToSelected)

        let rbRow = NSStackView(views: [
            NSTextField(labelWithString: "目标代次:"),
            rollbackPicker,
            rollbackToVersionButton,
            rollbackButton
        ])
        rbRow.alignment = .centerY
        rbRow.spacing = 8
        rbStack.addArrangedSubview(rbRow)
        rollbackBox.setContent(rbStack, inset: 6)
        stack.addArrangedSubview(rollbackBox)

        versionView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: versionView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: versionView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: versionView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: versionView.bottomAnchor)
        ])
    }

    private func setupPreviewTab() {
        previewView.translatesAutoresizingMaskIntoConstraints = false
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false

        let notice = NSTextField(wrappingLabelWithString: "试一试：输入一段客户问题，查看大脑如何组装上下文与预览回复。合成测试仅执行确定性组装预览 (assemble_only)，标明【未调用模型】，不产生费用，绝不发送真实微信消息。")
        notice.font = NSFont.systemFont(ofSize: 12, weight: .bold)
        notice.textColor = .systemBlue
        notice.maximumNumberOfLines = 0
        notice.lineBreakMode = .byWordWrapping
        notice.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(notice)

        tryItModelStatusLabel.font = NSFont.systemFont(ofSize: 11)
        tryItModelStatusLabel.textColor = .secondaryLabelColor
        tryItModelStatusLabel.maximumNumberOfLines = 0
        tryItModelStatusLabel.lineBreakMode = .byWordWrapping
        tryItModelStatusLabel.preferredMaxLayoutWidth = 480
        stack.addArrangedSubview(tryItModelStatusLabel)

        previewChatKindPopup.addItems(withTitles: ["私聊 (private)", "群聊 (group)"])
        previewInputField.placeholderString = "例如：你们支持七天无理由退货吗？"
        let inputRow = NSStackView(views: [
            NSTextField(labelWithString: "客户问题:"),
            previewInputField
        ])
        previewInputField.widthAnchor.constraint(equalToConstant: 200).isActive = true
        runPreviewButton.target = self
        runPreviewButton.action = #selector(handleRunPreview)
        stack.addArrangedSubview(inputRow)
        let previewActions = NSStackView(views: [previewChatKindPopup, runPreviewButton])
        previewActions.spacing = 10
        stack.addArrangedSubview(previewActions)

        stack.addArrangedSubview(NSTextField(labelWithString: "试一试结果（预览回复 + 组装预览快照与决策）:"))
        let resScroll = NSScrollView()
        resScroll.hasVerticalScroller = true
        previewResultTextView.isEditable = false
        previewResultTextView.font = NSFont.userFixedPitchFont(ofSize: 11)
        resScroll.documentView = previewResultTextView
        resScroll.heightAnchor.constraint(equalToConstant: 240).isActive = true
        resScroll.widthAnchor.constraint(equalToConstant: 480).isActive = true
        stack.addArrangedSubview(resScroll)

        previewView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: previewView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: previewView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: previewView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: previewView.bottomAnchor)
        ])
    }

    private func switchTab(index: Int) {
        tabContainer.subviews.forEach { $0.removeFromSuperview() }
        let targetView: NSView
        switch index {
        case 0: targetView = definitionView
        case 1: targetView = knowledgeView
        case 2: targetView = templatesView
        case 3: targetView = bindingsView
        case 4: targetView = versionView
        case 5: targetView = previewView
        default: targetView = definitionView
        }
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.drawsBackground = false
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(targetView)
        scroll.documentView = document
        tabContainer.addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: tabContainer.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: tabContainer.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: tabContainer.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: tabContainer.bottomAnchor),
            document.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
            document.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scroll.contentView.trailingAnchor),
            targetView.topAnchor.constraint(equalTo: document.topAnchor),
            targetView.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            targetView.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            targetView.bottomAnchor.constraint(equalTo: document.bottomAnchor)
        ])
    }

    @objc private func handleSectionChanged() {
        switchTab(index: sectionSegment.selectedSegment)
    }

    @objc private func handleToMcp() {
        // 跳转到 MCP 页面
        if let shell = view.window?.contentViewController as? ShellViewController {
            shell.selectPage(id: "mcp")
        }
    }

    public func reloadBrainData() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                var payload = try self.client.fetchList()
                let hasStoredRoles = !payload.roles.isEmpty
                payload.roles.removeAll { $0.archived }
                DispatchQueue.main.async {
                    self.hasStoredRoles = hasStoredRoles
                    self.listPayload = payload
                    self.updateUIWithList(payload)
                }
            } catch {
                DispatchQueue.main.async {
                    self.loadErrorLabel.stringValue = "加载大脑配置失败: \(error.localizedDescription)"
                }
            }
        }
        loadHistory()
    }

    /// 只读加载版本历史；任何失败都落到界面状态，不抛到主线程之外。
    /// 用令牌丢弃过期响应，避免回滚后旧请求覆盖新状态。
    private func loadHistory() {
        let token = UUID()
        historyLoadToken = token
        historyStatusLabel.textColor = .secondaryLabelColor
        historyStatusLabel.stringValue = "版本历史：正在加载…"
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let payload = try self.client.fetchHistory()
                DispatchQueue.main.async {
                    guard self.historyLoadToken == token else { return }
                    self.updateHistoryUI(payload, error: nil)
                }
            } catch {
                DispatchQueue.main.async {
                    guard self.historyLoadToken == token else { return }
                    self.updateHistoryUI(nil, error: error.localizedDescription)
                }
            }
        }
    }

    private func updateHistoryUI(_ payload: BrainHistoryPayload?, error: String?) {
        guard let payload = payload else {
            historyGenerations = []
            historyVersions = []
            historyTable.reloadData()
            historyEmptyLabel.stringValue = "版本历史暂不可用"
            historyEmptyLabel.isHidden = false
            historyDetailNoticeLabel.stringValue = ""
            diffVerAPopup.removeAllItems()
            diffVerBPopup.removeAllItems()
            rollbackPicker.removeAllItems()
            diffTextView.string = ""
            historyStatusLabel.textColor = .systemRed
            historyStatusLabel.stringValue = "版本历史暂不可用：\(error ?? "未知原因")"
            return
        }

        historyGenerations = (payload.generations ?? []).sorted { $0.generation > $1.generation }
        historyVersions = payload.versions ?? []
        historyTable.reloadData()
        historyEmptyLabel.isHidden = !historyGenerations.isEmpty
        if historyGenerations.isEmpty {
            historyEmptyLabel.stringValue = "暂无版本历史记录"
        }

        historyStatusLabel.textColor = .secondaryLabelColor
        let currentGenText = payload.generation.map { "第 \($0) 代" } ?? "未知"
        var status = "版本历史：当前 \(currentGenText) · 代次记录 \(historyGenerations.count) 条 · 版本 \(historyVersions.count) 个"
        if let scope = payload.historyScope, !scope.isEmpty {
            status += "\n后端历史口径：\(scope)（只保留已生效激活、已提交写入与可达版本；更早的编辑历史未保留）"
        }
        historyStatusLabel.stringValue = status

        diffVerAPopup.removeAllItems()
        diffVerBPopup.removeAllItems()
        rollbackPicker.removeAllItems()

        for version in historyVersions {
            var title = version.revisionId
            if version.isActive == true { title += " (运行中)" }
            if version.isDraft == true { title += " (草稿)" }
            diffVerAPopup.addItem(withTitle: title)
            diffVerAPopup.lastItem?.representedObject = version.revisionId
            diffVerBPopup.addItem(withTitle: title)
            diffVerBPopup.lastItem?.representedObject = version.revisionId
        }
        if diffVerBPopup.numberOfItems > 1 { diffVerBPopup.selectItem(at: 1) }
        diffTextView.string = diffVerAPopup.numberOfItems > 0 ? "请选择两个版本后点击「对比差异」。" : "当前没有可对比的版本记录。"

        for generation in historyGenerations {
            var title = "第 \(generation.generation) 代"
            if let current = payload.generation, current == generation.generation { title += " (当前)" }
            rollbackPicker.addItem(withTitle: title)
            rollbackPicker.lastItem?.representedObject = generation.generation
        }
        if let current = payload.generation, let index = historyGenerations.firstIndex(where: { $0.generation == current }) {
            rollbackPicker.selectItem(at: index)
        } else if rollbackPicker.numberOfItems > 0 {
            rollbackPicker.selectItem(at: 0)
        }
        historyDetailNoticeLabel.stringValue = ""
    }

    private func updateUIWithList(_ payload: BrainListPayload) {
        accountLabel.stringValue = "当前账号: \(payload.accountId ?? "未知")"
        if let summary = payload.modelSummary {
            modeLabel.stringValue = "回复方式：\(summary.mode == "builtin" ? "内置大脑" : summary.mode)"
            updateTryItModelStatus(mode: summary.mode)
        } else {
            updateTryItModelStatus(mode: nil)
        }
        loadErrorLabel.stringValue = ""

        roleTable.reloadData()
        updateBindingsUI()
        if !payload.roles.isEmpty {
            if let preferred = preferredRoleId, let index = payload.roles.firstIndex(where: { $0.roleId == preferred }) { selectedRoleIndex = index }
            preferredRoleId = nil
            if selectedRoleIndex < 0 || selectedRoleIndex >= payload.roles.count {
                selectedRoleIndex = 0
            }
            roleTable.selectRowIndexes(IndexSet(integer: selectedRoleIndex), byExtendingSelection: false)
            loadSelectedRole()
        } else {
            if currentDraftRole == nil && !hasStoredRoles { handleAddDefaultRole() }
            refreshDraftRoleRow()
        }
    }

    /// 试一试页顶部的真实模型可用性说明：本构建后端只接受 assemble_only，
    /// 因此明确标注「不会生成真实回复文本」，不假装成功。
    private func updateTryItModelStatus(mode: String?) {
        let modeText: String
        if let mode = mode, !mode.isEmpty {
            modeText = mode == "builtin" ? "内置大脑" : mode
        } else {
            modeText = "未知（未返回模型摘要）"
        }
        tryItModelStatusLabel.textColor = .secondaryLabelColor
        tryItModelStatusLabel.stringValue = "回复方式：\(modeText) · 真实模型调用：当前构建仅支持 assemble_only 确定性组装预览（不调用模型、不产生费用），因此不会生成真实回复文本。"
    }

    private func updateBindingsUI() {
        guard let payload = listPayload else { return }
        defaultRolePopup.removeAllItems()
        for role in payload.roles {
            let title = "\(role.name) (\(role.roleId))"
            defaultRolePopup.addItem(withTitle: title)
            defaultRolePopup.lastItem?.representedObject = role.roleId
        }

        if !hasUnsavedBindingsChanges {
            currentDraftBindings = payload.draft ?? payload.active ?? BrainBindingSet(defaultRef: nil, overrides: [])
        }

        if let defaultRoleId = currentDraftBindings.defaultRef?.roleId {
            for (index, item) in defaultRolePopup.itemArray.enumerated() {
                if (item.representedObject as? String) == defaultRoleId || item.title.contains(defaultRoleId) {
                    defaultRolePopup.selectItem(at: index)
                    break
                }
            }
        }

        overrideRolePopup.removeAllItems()
        for role in payload.roles {
            let title = "\(role.name) [\(role.draftRevisionId)]"
            overrideRolePopup.addItem(withTitle: title)
            overrideRolePopup.lastItem?.representedObject = role.roleId
        }

        // 白名单联系人来源状态处理（由核心只读投影权威提供，绝不在前端凭 recipient 造 ID 或用已有 override 扩权）
        self.allowlistContacts = payload.allowlistContacts
        self.allowlistContactsStatus = payload.allowlistContactsStatus
        self.allowlistContactsError = payload.allowlistContactsError

        overrideContactPopup.removeAllItems()
        if allowlistContactsStatus == "unavailable" {
            // 来源不可用：必须禁用新增/编辑表单，不得拿既有覆盖扩权
            overrideContactPopup.isEnabled = false
            overrideRolePopup.isEnabled = false
            addOverrideButton.isEnabled = false
            updateOverrideButton.isEnabled = false
            deleteOverrideButton.isEnabled = !currentDraftBindings.overrides.isEmpty
            let errText = allowlistContactsError ?? "未就绪"
            bindingsStatusLabel.stringValue = "白名单联系人来源不可用 [\(errText)]，已禁用新增与编辑"
            bindingsStatusLabel.textColor = .systemRed
        } else {
            // ready 状态
            for c in allowlistContacts {
                let kindStr = c.chatKind == "group" ? "群聊" : "私聊"
                let title = "\(c.displayName) (\(c.contactId), \(kindStr))"
                overrideContactPopup.addItem(withTitle: title)
                overrideContactPopup.lastItem?.representedObject = c.contactId
            }
            if allowlistContacts.isEmpty {
                overrideContactPopup.isEnabled = false
                overrideRolePopup.isEnabled = false
                addOverrideButton.isEnabled = false
                updateOverrideButton.isEnabled = false
                deleteOverrideButton.isEnabled = !currentDraftBindings.overrides.isEmpty
                if !hasUnsavedBindingsChanges {
                    bindingsStatusLabel.stringValue = "当前白名单无可用联系人，新增覆盖前请先确认白名单配置与通讯录同步"
                    bindingsStatusLabel.textColor = .secondaryLabelColor
                }
            } else {
                overrideContactPopup.isEnabled = true
                overrideRolePopup.isEnabled = true
                addOverrideButton.isEnabled = true
                updateOverrideButton.isEnabled = true
                deleteOverrideButton.isEnabled = true
                if !hasUnsavedBindingsChanges {
                    bindingsStatusLabel.stringValue = ""
                }
            }
        }

        updateOverridesListUI()
    }

    private func updateOverridesListUI() {
        overridesTableView.reloadData()

        if !currentDraftBindings.overrides.isEmpty {
            var lines: [String] = []
            for ov in currentDraftBindings.overrides {
                let roleName = listPayload?.roles.first(where: { $0.roleId == ov.roleId })?.name ?? ov.roleId
                let kindStr = ov.chatKind == "group" ? "群聊" : "私聊"
                lines.append("• 联系人: \(ov.contactId) [\(kindStr)] ➔ \(roleName) (\(ov.roleId)) [版本: \(ov.revisionId)]")
            }
            overridesSummaryLabel.stringValue = lines.joined(separator: "\n")
        } else {
            overridesSummaryLabel.stringValue = "暂无特定联系人覆盖（所有联系人均使用默认角色）"
        }
        if allowlistContactsStatus == "unavailable" || allowlistContacts.isEmpty {
            deleteOverrideButton.isEnabled = !currentDraftBindings.overrides.isEmpty
        }
    }

    // MARK: - Bindings Form Actions & Fixtures

    public func setAllowlistContacts(_ contacts: [BrainAllowlistContact], status: String = "ready", error: String? = nil) {
        self.allowlistContacts = contacts
        self.allowlistContactsStatus = status
        self.allowlistContactsError = error
        if let p = listPayload {
            var copy = p
            copy.allowlistContacts = contacts
            copy.allowlistContactsStatus = status
            copy.allowlistContactsError = error
            self.listPayload = copy
        }
        updateBindingsUI()
    }

    public func addOverrideDraft(contact: BrainAllowlistContact, roleId: String, revisionId: String) {
        guard allowlistContactsStatus != "unavailable" else { return }
        let res = BrainPageLogic.addOverride(to: currentDraftBindings, contact: contact, roleId: roleId, revisionId: revisionId, allowlist: allowlistContacts)
        if case .success(let b) = res {
            currentDraftBindings = b
            hasUnsavedBindingsChanges = true
            updateOverridesListUI()
        }
    }

    public func updateOverrideDraft(contactId: String, roleId: String, revisionId: String) {
        guard allowlistContactsStatus != "unavailable" else { return }
        let res = BrainPageLogic.updateOverride(in: currentDraftBindings, contactId: contactId, newRoleId: roleId, newRevisionId: revisionId)
        if case .success(let b) = res {
            currentDraftBindings = b
            hasUnsavedBindingsChanges = true
            updateOverridesListUI()
        }
    }

    public func deleteOverrideDraft(contactId: String) {
        currentDraftBindings = BrainPageLogic.deleteOverride(from: currentDraftBindings, contactId: contactId)
        hasUnsavedBindingsChanges = true
        updateOverridesListUI()
    }

    public func selectDefaultRoleDraft(roleId: String) {
        guard let payload = listPayload, let role = payload.roles.first(where: { $0.roleId == roleId }) else { return }
        currentDraftBindings = BrainPageLogic.setDefaultRole(in: currentDraftBindings, roleId: roleId, revisionId: role.draftRevisionId)
        hasUnsavedBindingsChanges = true
        updateBindingsUI()
    }

    public func setBindingsConflictFixture(message: String) {
        bindingsStatusLabel.stringValue = message
        bindingsStatusLabel.textColor = .systemRed
    }

    @objc private func handleDefaultRoleChanged() {
        guard let selectedRoleId = defaultRolePopup.selectedItem?.representedObject as? String,
              let payload = listPayload,
              let roleEntry = payload.roles.first(where: { $0.roleId == selectedRoleId }) else { return }
        let rev = roleEntry.draftRevisionId
        currentDraftBindings = BrainPageLogic.setDefaultRole(in: currentDraftBindings, roleId: selectedRoleId, revisionId: rev)
        hasUnsavedBindingsChanges = true
        bindingsStatusLabel.stringValue = "默认角色草稿已选择为「\(roleEntry.name)」，待点击「保存绑定草稿」"
        bindingsStatusLabel.textColor = .labelColor
        updateOverridesListUI()
    }

    private func selectedAllowlistContactFromPopup() -> BrainAllowlistContact? {
        guard let contactId = overrideContactPopup.selectedItem?.representedObject as? String else {
            return nil
        }
        return allowlistContacts.first(where: { $0.contactId == contactId })
    }

    private func selectedRoleRefFromOverridePopup() -> BrainRoleRef? {
        guard let roleId = overrideRolePopup.selectedItem?.representedObject as? String,
              let payload = listPayload,
              let role = payload.roles.first(where: { $0.roleId == roleId }) else {
            return nil
        }
        return BrainRoleRef(roleId: roleId, revisionId: role.draftRevisionId)
    }

    @objc private func handleAddOverride() {
        guard let contact = selectedAllowlistContactFromPopup() else {
            bindingsStatusLabel.stringValue = "添加失败：请先选择合法的白名单联系人"
            bindingsStatusLabel.textColor = .systemRed
            return
        }
        guard let roleRef = selectedRoleRefFromOverridePopup() else {
            bindingsStatusLabel.stringValue = "添加失败：请选择有效的角色"
            bindingsStatusLabel.textColor = .systemRed
            return
        }
        let res = BrainPageLogic.addOverride(
            to: currentDraftBindings,
            contact: contact,
            roleId: roleRef.roleId,
            revisionId: roleRef.revisionId,
            allowlist: allowlistContacts
        )
        switch res {
        case .success(let newBindings):
            currentDraftBindings = newBindings
            hasUnsavedBindingsChanges = true
            bindingsStatusLabel.stringValue = "已添加针对「\(contact.displayName)」的覆盖规则草稿，待保存"
            bindingsStatusLabel.textColor = .labelColor
            updateOverridesListUI()
        case .failure(let err):
            switch err {
            case .duplicateContact:
                bindingsStatusLabel.stringValue = "添加失败：联系人「\(contact.displayName)」已存在覆盖规则，可直接编辑"
            case .notInAllowlist:
                bindingsStatusLabel.stringValue = "添加失败：联系人必须源自真实白名单，禁止凭姓名构造ID"
            default:
                bindingsStatusLabel.stringValue = "添加覆盖失败"
            }
            bindingsStatusLabel.textColor = .systemRed
        }
    }

    @objc private func handleUpdateOverride() {
        guard let contact = selectedAllowlistContactFromPopup() else {
            bindingsStatusLabel.stringValue = "更新失败：请选择要更新的白名单联系人"
            bindingsStatusLabel.textColor = .systemRed
            return
        }
        guard let roleRef = selectedRoleRefFromOverridePopup() else {
            bindingsStatusLabel.stringValue = "更新失败：请选择有效的角色"
            bindingsStatusLabel.textColor = .systemRed
            return
        }
        let res = BrainPageLogic.updateOverride(
            in: currentDraftBindings,
            contactId: contact.contactId,
            newRoleId: roleRef.roleId,
            newRevisionId: roleRef.revisionId
        )
        switch res {
        case .success(let newBindings):
            currentDraftBindings = newBindings
            hasUnsavedBindingsChanges = true
            bindingsStatusLabel.stringValue = "已更新针对「\(contact.displayName)」的覆盖规则草稿，待保存"
            bindingsStatusLabel.textColor = .labelColor
            updateOverridesListUI()
        case .failure:
            bindingsStatusLabel.stringValue = "更新失败：未找到对应的覆盖规则，请先点击「添加覆盖」"
            bindingsStatusLabel.textColor = .systemRed
        }
    }

    @objc private func handleDeleteOverride() {
        let contactIdToDelete: String?
        if let contact = selectedAllowlistContactFromPopup() {
            contactIdToDelete = contact.contactId
        } else {
            let row = overridesTableView.selectedRow
            if row >= 0 && row < currentDraftBindings.overrides.count {
                contactIdToDelete = currentDraftBindings.overrides[row].contactId
            } else {
                contactIdToDelete = nil
            }
        }
        guard let cid = contactIdToDelete else {
            bindingsStatusLabel.stringValue = "删除失败：请在表格中或下拉框中选择要删除的覆盖规则"
            bindingsStatusLabel.textColor = .systemRed
            return
        }
        currentDraftBindings = BrainPageLogic.deleteOverride(from: currentDraftBindings, contactId: cid)
        hasUnsavedBindingsChanges = true
        bindingsStatusLabel.stringValue = "已从草稿中删除覆盖规则 [\(cid)]，待点击「保存绑定草稿」"
        bindingsStatusLabel.textColor = .labelColor
        updateOverridesListUI()
    }

    @objc private func handleSaveBindingsDraft() {
        guard let payload = listPayload else { return }
        currentSaveToken += 1
        let token = currentSaveToken

        saveBindingsDraftButton.isEnabled = false
        addOverrideButton.isEnabled = false
        updateOverrideButton.isEnabled = false
        deleteOverrideButton.isEnabled = false

        bindingsStatusLabel.stringValue = "正在保存绑定草稿至核心..."
        bindingsStatusLabel.textColor = .labelColor
        let gen = payload.generation
        let draft = currentDraftBindings
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                _ = try self.client.saveBindings(draft: draft, expectedGeneration: gen)
                DispatchQueue.main.async {
                    defer { self.onSaveBindingsFinished?() }
                    guard self.currentSaveToken == token else { return }
                    self.saveBindingsDraftButton.isEnabled = true
                    self.addOverrideButton.isEnabled = (self.allowlistContactsStatus == "ready" && !self.allowlistContacts.isEmpty)
                    self.updateOverrideButton.isEnabled = (self.allowlistContactsStatus == "ready" && !self.allowlistContacts.isEmpty)
                    self.deleteOverrideButton.isEnabled = true
                    self.hasUnsavedBindingsChanges = false
                    self.bindingsStatusLabel.stringValue = "绑定草稿保存成功！新配置将在启用后作用于新轮次。"
                    self.bindingsStatusLabel.textColor = .systemGreen
                    self.reloadBrainData()
                }
            } catch {
                DispatchQueue.main.async {
                    defer { self.onSaveBindingsFinished?() }
                    guard self.currentSaveToken == token else { return }
                    self.saveBindingsDraftButton.isEnabled = true
                    self.addOverrideButton.isEnabled = (self.allowlistContactsStatus == "ready" && !self.allowlistContacts.isEmpty)
                    self.updateOverrideButton.isEnabled = (self.allowlistContactsStatus == "ready" && !self.allowlistContacts.isEmpty)
                    self.deleteOverrideButton.isEnabled = true
                    let handled = BrainPageLogic.handleBindingsConflict(
                        localDraft: draft,
                        error: BrainControlError(code: "conflict", message: error.localizedDescription)
                    )
                    self.currentDraftBindings = handled.preservedBindings
                    self.bindingsStatusLabel.stringValue = handled.errorMessage
                    self.bindingsStatusLabel.textColor = .systemRed
                    self.updateOverridesListUI()
                }
            }
        }
    }

    private var displayedRoles: [BrainRoleEntry] {
        var roles = listPayload?.roles ?? []
        if let role = currentDraftRole, !roles.contains(where: { $0.roleId == role.roleId }) {
            roles.append(BrainRoleEntry(roleId: role.roleId, draftRevisionId: role.revisionId, name: role.name))
        }
        return roles
    }

    private func refreshDraftRoleRow() {
        roleLoadToken = UUID()
        roleTable.reloadData()
        guard let role = currentDraftRole,
              let index = displayedRoles.firstIndex(where: { $0.roleId == role.roleId }) else { return }
        selectedRoleIndex = index
        roleTable.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        archiveRoleButton.isEnabled = true
    }

    @objc private func handleEditRole() {
        let row = roleTable.clickedRow >= 0 ? roleTable.clickedRow : roleTable.selectedRow
        guard row >= 0, row < displayedRoles.count else { return }
        selectedRoleIndex = row
        let entry = displayedRoles[row]
        sectionSegment.selectedSegment = 0
        switchTab(index: 0)
        if currentDraftRole?.roleId == entry.roleId {
            view.window?.makeFirstResponder(roleNameField)
        } else {
            focusRoleAfterLoad = entry.roleId
            loadSelectedRole()
        }
    }

    private func loadSelectedRole() {
        if selectedRoleIndex == (listPayload?.roles.count ?? 0), let role = currentDraftRole {
            populateRoleForm(role, entry: BrainRoleEntry(roleId: role.roleId, draftRevisionId: role.revisionId, name: role.name))
            localUnsavedChanges = true
            return
        }
        guard let payload = listPayload, selectedRoleIndex >= 0 && selectedRoleIndex < payload.roles.count else {
            return
        }
        let entry = payload.roles[selectedRoleIndex]
        let loadToken = UUID()
        roleLoadToken = loadToken
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let role = try self.client.getRole(roleId: entry.roleId, revisionId: entry.draftRevisionId)
                DispatchQueue.main.async {
                    guard self.roleLoadToken == loadToken else { return }
                    self.currentDraftRole = role
                    self.populateRoleForm(role, entry: entry)
                    if self.focusRoleAfterLoad == role.roleId {
                        self.focusRoleAfterLoad = nil
                        self.view.window?.makeFirstResponder(self.roleNameField)
                    }
                }
            } catch {
                DispatchQueue.main.async {
                    guard self.roleLoadToken == loadToken else { return }
                    self.loadErrorLabel.stringValue = "读取角色草稿失败: \(error.localizedDescription)"
                }
            }
        }
    }

    public func selectTab(index: Int) {
        sectionSegment.selectedSegment = index
        switchTab(index: index)
    }

    public func setPreviewFixture(input: String, result: String) {
        previewInputField.stringValue = input
        previewResultTextView.string = result
    }

    public func applyFixtureState(payload: BrainListPayload, role: BrainRoleRevision, accountId: String = "wx_fixture_account") {
        self.listPayload = payload
        self.currentDraftRole = role
        self.accountLabel.stringValue = "当前账号: \(accountId)"
        if let summary = payload.modelSummary {
            self.modeLabel.stringValue = "回复方式：\(summary.mode == "builtin" ? "内置大脑" : summary.mode)"
            self.updateTryItModelStatus(mode: summary.mode)
        } else {
            self.updateTryItModelStatus(mode: nil)
        }
        self.statusMessageLabel.stringValue = "草稿已加载，可编辑后保存或合成预览"
        self.loadErrorLabel.stringValue = ""
        self.roleTable.reloadData()
        self.updateBindingsUI()
        if !payload.roles.isEmpty {
            self.selectedRoleIndex = 0
            self.roleTable.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            self.populateRoleForm(role, entry: payload.roles[0])
        }
    }

    private func populateRoleForm(_ role: BrainRoleRevision, entry: BrainRoleEntry) {
        localUnsavedChanges = false
        for field in [roleNameField, toneField, goalsField, rulesField, greetingField, insufficientInfoField, unreadableTypeField, unreadableLargeField] { field.delegate = self }
        personaField.delegate = self
        roleNameField.stringValue = role.name
        toneField.stringValue = role.tone
        personaField.string = role.persona
        goalsField.stringValue = role.goals.joined(separator: ", ")
        rulesField.stringValue = role.businessRules.joined(separator: ", ")

        greetingField.stringValue = role.templates.greeting ?? ""
        insufficientInfoField.stringValue = role.templates.insufficientInformation ?? ""
        unreadableTypeField.stringValue = (role.templates.unreadable["unsupported_type"] ?? nil) ?? ""
        unreadableLargeField.stringValue = (role.templates.unreadable["too_large"] ?? nil) ?? ""

        let activeRev = entry.activeRevisionId ?? "未启用"
        revisionStatusLabel.stringValue = "角色: \(role.name) · 草稿: \(role.revisionId) · 运行: \(activeRev)"
        archiveRoleButton.isEnabled = BrainPageLogic.canArchiveRole(entry)

        if role.knowledgeRefs.isEmpty {
            knowledgeRefsSummaryLabel.stringValue = "当前角色未引用任何知识"
        } else {
            let refTexts = role.knowledgeRefs.map { "\($0.knowledgeId) (\($0.revisionId), sha:\($0.contentSha256.prefix(8))...)" }
            knowledgeRefsSummaryLabel.stringValue = refTexts.joined(separator: "\n")
        }
    }

    public func controlTextDidChange(_ notification: Notification) {
        localUnsavedChanges = true
        statusMessageLabel.stringValue = "角色有未保存的修改 · ⌘S 保存草稿"
    }

    public func textDidChange(_ notification: Notification) {
        localUnsavedChanges = true
        statusMessageLabel.stringValue = "角色有未保存的修改 · ⌘S 保存草稿"
    }

    @objc private func handleImportKnowledge() {
        guard var role = currentDraftRole, let payload = listPayload else { return }
        let title = knowledgeTitleField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = knowledgeContentField.string.trimmingCharacters(in: .whitespacesAndNewlines)
        let src = knowledgeSourceField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, !text.isEmpty else {
            statusMessageLabel.stringValue = "导入知识失败：标题与正文不能为空"
            return
        }

        let newKnowledge = BrainKnowledgeRevision(
            knowledgeId: "kn_\(UUID().uuidString.prefix(8))",
            revisionId: "rev_1",
            title: title,
            text: text,
            sourceLabel: src.isEmpty ? "manual" : src
        )

        statusMessageLabel.stringValue = "正在通过核心接口保存知识..."
        let gen = payload.generation
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let saved = try self.client.saveKnowledge(newKnowledge, expectedGeneration: gen)
                DispatchQueue.main.async {
                    role.knowledgeRefs.append(BrainKnowledgeRef(
                        knowledgeId: saved.knowledgeId,
                        revisionId: saved.revisionId,
                        contentSha256: saved.contentSha256
                    ))
                    self.currentDraftRole = role
                    let refTexts = role.knowledgeRefs.map { "\($0.knowledgeId) (\($0.revisionId), sha:\($0.contentSha256.prefix(8))...)" }
                    self.knowledgeRefsSummaryLabel.stringValue = refTexts.joined(separator: "\n")
                    self.knowledgeTitleField.stringValue = ""
                    self.knowledgeContentField.string = ""
                    self.knowledgeSourceField.stringValue = ""
                    self.statusMessageLabel.stringValue = "知识已成功保存至核心并关联到当前草稿！"
                }
            } catch {
                DispatchQueue.main.async {
                    self.statusMessageLabel.stringValue = "保存知识失败: \(error.localizedDescription)"
                }
            }
        }
    }

    @objc private func handleAddDefaultRole() {
        guard !localUnsavedChanges else {
            statusMessageLabel.stringValue = "请先保存当前角色，再添加默认角色。"
            return
        }
        guard let url = Bundle.main.url(forResource: "brain-default-role", withExtension: "json") else {
            statusMessageLabel.stringValue = "默认角色模板缺失，请重新安装完整应用。"
            return
        }
        do {
            let role = try BrainDefaultRoleTemplate.load(from: url).makeRole()
            currentDraftRole = role
            populateRoleForm(role, entry: BrainRoleEntry(roleId: role.roleId, draftRevisionId: role.revisionId, name: role.name))
            localUnsavedChanges = true
            sectionSegment.selectedSegment = 0
            switchTab(index: 0)
            refreshDraftRoleRow()
            statusMessageLabel.stringValue = "默认角色草稿已准备好，可直接编辑并保存。"
        } catch {
            statusMessageLabel.stringValue = "读取默认角色模板失败：\(error.localizedDescription)"
        }
    }

    @objc private func handleUseAsDefault() {
        guard let role = currentDraftRole, !localUnsavedChanges,
              listPayload?.roles.contains(where: { $0.roleId == role.roleId }) == true else {
            statusMessageLabel.stringValue = "请先保存角色草稿，再设为默认角色。"
            return
        }
        selectDefaultRoleDraft(roleId: role.roleId)
        sectionSegment.selectedSegment = 3
        switchTab(index: 3)
        bindingsStatusLabel.stringValue = "已选择「\(role.name)」为默认角色。请保存绑定草稿，再启用配置。"
        statusMessageLabel.stringValue = "默认角色用于白名单内未指定专属角色的会话；专属角色保持优先。"
    }

    @objc private func handleNewRole() {
        guard !localUnsavedChanges else {
            statusMessageLabel.stringValue = "请先保存当前角色，再新建角色。"
            return
        }
        let newRole = BrainRoleRevision(
            roleId: "role_\(UUID().uuidString.prefix(8))",
            revisionId: "rev_1",
            name: "新助手",
            persona: "您是一位专业友善的客服助手。",
            goals: ["解答用户咨询"],
            tone: "温和自然",
            businessRules: []
        )
        currentDraftRole = newRole
        populateRoleForm(newRole, entry: BrainRoleEntry(roleId: newRole.roleId, draftRevisionId: newRole.revisionId, name: newRole.name))
        localUnsavedChanges = true
        sectionSegment.selectedSegment = 0
        switchTab(index: 0)
        refreshDraftRoleRow()
        view.window?.makeFirstResponder(roleNameField)
        statusMessageLabel.stringValue = "尚未保存：填写角色设定后按 ⌘S 保存，再设置适用会话"
    }

    @objc private func handleCopyRole() {
        guard let current = currentDraftRole else { return }
        let copy = BrainPageLogic.prepareRoleCopy(from: current, newRoleId: "role_\(UUID().uuidString.prefix(8))", newName: "\(current.name) (副本)")
        currentDraftRole = copy
        populateRoleForm(copy, entry: BrainRoleEntry(roleId: copy.roleId, draftRevisionId: copy.revisionId, name: copy.name))
        localUnsavedChanges = true
        refreshDraftRoleRow()
        statusMessageLabel.stringValue = "已复制角色，编辑后点击「保存草稿」"
    }

    @objc private func handleArchiveRole() {
        guard let payload = listPayload, selectedRoleIndex >= 0,
              selectedRoleIndex < displayedRoles.count else { return }
        let entry = displayedRoles[selectedRoleIndex]
        let isSaved = payload.roles.contains { $0.roleId == entry.roleId }
        let inUse = [payload.active, payload.draft, Optional(currentDraftBindings)].compactMap { $0 }.contains {
            $0.defaultRef?.roleId == entry.roleId || $0.overrides.contains { $0.roleId == entry.roleId }
        }
        guard !inUse else {
            statusMessageLabel.stringValue = "无法删除：该角色仍被会话配置引用，请先解除默认或专属角色绑定。"
            return
        }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "删除角色「\(entry.name)」？"
        alert.informativeText = isSaved
            ? "删除后将从角色列表移除，历史对话和版本记录会保留。"
            : "该角色尚未保存，删除后将丢弃这份草稿。"
        alert.addButton(withTitle: "取消")
        alert.addButton(withTitle: "删除角色")
        guard alert.runModal() == .alertSecondButtonReturn else { return }
        if !isSaved {
            clearDeletedRole()
            statusMessageLabel.stringValue = "未保存角色已删除"
            return
        }
        archiveRoleButton.isEnabled = false
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                _ = try self.client.archiveRole(roleId: entry.roleId, expectedGeneration: payload.generation)
                DispatchQueue.main.async {
                    self.clearDeletedRole()
                    self.statusMessageLabel.stringValue = "角色已删除"
                    self.reloadBrainData()
                }
            } catch {
                DispatchQueue.main.async {
                    self.archiveRoleButton.isEnabled = true
                    self.statusMessageLabel.stringValue = "删除失败：\(error.localizedDescription)"
                }
            }
        }
    }

    private func clearDeletedRole() {
        roleLoadToken = UUID()
        currentDraftRole = nil
        selectedRoleIndex = -1
        localUnsavedChanges = false
        for field in [roleNameField, toneField, goalsField, rulesField, greetingField, insufficientInfoField, unreadableTypeField, unreadableLargeField] {
            field.stringValue = ""
        }
        personaField.string = ""
        revisionStatusLabel.stringValue = "请选择角色，或点击「新增角色」"
        archiveRoleButton.isEnabled = false
        roleTable.deselectAll(nil)
        roleTable.reloadData()
    }

    @objc private func handleSaveDraft() {
        guard var draft = currentDraftRole, let payload = listPayload else { return }
        draft.name = roleNameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        draft.tone = toneField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        draft.persona = personaField.string.trimmingCharacters(in: .whitespacesAndNewlines)
        draft.goals = goalsField.stringValue.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        draft.businessRules = rulesField.stringValue.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }

        var t = draft.templates
        let g = greetingField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        t.greeting = g.isEmpty ? nil : g
        let inf = insufficientInfoField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        t.insufficientInformation = inf.isEmpty ? nil : inf
        let ut = unreadableTypeField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        t.unreadable["unsupported_type"] = ut.isEmpty ? nil : ut
        let ul = unreadableLargeField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        t.unreadable["too_large"] = ul.isEmpty ? nil : ul
        draft.templates = t

        if let existing = payload.roles.first(where: { $0.roleId == draft.roleId }) {
            draft.parentRevisionId = existing.draftRevisionId
            draft.revisionId = "rev_(UUID().uuidString.lowercased())"
            draft.createdAt = UInt64(Date().timeIntervalSince1970)
            draft.contentSha256 = ""
        }
        statusMessageLabel.stringValue = "正在保存草稿..."
        let gen = payload.generation
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                _ = try self.client.saveRole(draft, expectedGeneration: gen)
                DispatchQueue.main.async {
                    self.localUnsavedChanges = false
                    self.preferredRoleId = draft.roleId
                    self.statusMessageLabel.stringValue = "草稿已保存。请在「适用会话」设置回复范围，再启用配置。"
                    self.reloadBrainData()
                }
            } catch {
                DispatchQueue.main.async {
                    let handled = BrainPageLogic.handleSaveConflict(localDraft: draft, error: BrainControlError(code: "conflict", message: error.localizedDescription))
                    self.statusMessageLabel.stringValue = handled.errorMessage
                }
            }
        }
    }

    @objc private func handleActivate() {
        guard !activationInFlight, let payload = listPayload, let role = currentDraftRole else { return }
        guard !localUnsavedChanges, !hasUnsavedBindingsChanges,
              payload.roles.contains(where: { $0.roleId == role.roleId && $0.draftRevisionId == role.revisionId }) else {
            statusMessageLabel.stringValue = "尚有未保存的修改，请先保存角色草稿及会话绑定，再启用。"
            return
        }
        let savedBindings = payload.draft ?? BrainBindingSet(defaultRef: nil, overrides: [])
        let bindings = BrainPageLogic.activationBindings(role: role, bindings: savedBindings)
        let wasReferenced = savedBindings.defaultRef?.roleId == role.roleId || savedBindings.overrides.contains { $0.roleId == role.roleId }
        let alert = NSAlert()
        alert.messageText = "启用角色「\(role.name)」？"
        alert.informativeText = wasReferenced
            ? "将更新该角色已有的默认或专属绑定到当前已保存版本，并启用整份会话配置。其他角色绑定保持原样。仅影响新轮次。"
            : "将把该角色设为账号默认角色，并启用整份会话配置。白名单内没有专属角色的会话将使用它；已有专属角色保持不变。仅影响新轮次。"
        alert.addButton(withTitle: "确认启用")
        alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        activationInFlight = true
        activateButton.isEnabled = false
        statusMessageLabel.stringValue = "正在保存适用范围并启用角色..."
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                var generation = payload.generation
                if bindings != savedBindings {
                    generation = try self.client.saveBindings(draft: bindings, expectedGeneration: generation)
                }
                _ = try self.client.activate(expectedGeneration: generation)
                var verified = try self.client.fetchList()
                guard verified.active == bindings else {
                    throw NSError(domain: "BrainClient", code: 3, userInfo: [NSLocalizedDescriptionKey: "启用结果与预期范围不一致，请重新读取配置确认。"])
                }
                let hasStoredRoles = !verified.roles.isEmpty
                verified.roles.removeAll { $0.archived }
                DispatchQueue.main.async {
                    self.activationInFlight = false
                    self.activateButton.isEnabled = true
                    self.hasStoredRoles = hasStoredRoles
                    self.preferredRoleId = role.roleId
                    self.listPayload = verified
                    self.updateUIWithList(verified)
                    self.statusMessageLabel.stringValue = "角色「\(role.name)」已启用（\(role.revisionId)），适用范围已核对。"
                }
            } catch {
                DispatchQueue.main.async {
                    self.activationInFlight = false
                    self.activateButton.isEnabled = true
                    self.statusMessageLabel.stringValue = "启用未确认：\(error.localizedDescription)。请重新读取配置核对；已保存的绑定草稿会保留。"
                    self.reloadBrainData()
                }
            }
        }
    }

    @objc private func handleRollback() {
        guard let payload = listPayload else {
            statusMessageLabel.stringValue = "回退失败：尚未加载大脑配置"
            statusMessageLabel.textColor = .systemRed
            return
        }
        let targetGen = payload.generation > 1 ? payload.generation - 1 : 1
        let target = historyGenerations.first(where: { $0.generation == targetGen })
        confirmAndRollback(targetGeneration: targetGen, currentGeneration: payload.generation, historyTarget: target)
    }

    @objc private func handleRollbackToSelected() {
        guard let payload = listPayload else {
            statusMessageLabel.stringValue = "回退失败：尚未加载大脑配置"
            statusMessageLabel.textColor = .systemRed
            return
        }
        guard let selected = rollbackPicker.selectedItem?.representedObject as? UInt64 else {
            statusMessageLabel.stringValue = "请选择要回滚的目标代次"
            statusMessageLabel.textColor = .systemRed
            return
        }
        if selected == payload.generation {
            statusMessageLabel.stringValue = "选定代次已是当前代次，无需回滚"
            statusMessageLabel.textColor = .systemOrange
            return
        }
        let target = historyGenerations.first(where: { $0.generation == selected })
        confirmAndRollback(targetGeneration: selected, currentGeneration: payload.generation, historyTarget: target)
    }

    /// 回滚统一走二次确认：弹窗列出目标代次后端实际记录的内容与草稿完整性，
    /// 勾选确认后才执行；缺失内容如实标注，不补全。
    private func confirmAndRollback(targetGeneration: UInt64, currentGeneration: UInt64, historyTarget: BrainHistoryGeneration?) {
        let missing = "[缺失]"
        let alert = NSAlert()
        alert.messageText = "回滚大脑配置到第 \(targetGeneration) 代？"
        alert.alertStyle = .warning

        var info = "回滚将把账号配置原子切换到第 \(targetGeneration) 代的已保存状态；当前未保存的草稿将丢失。仅影响新轮次。\n"
        if let target = historyTarget {
            let draftText = target.hasCompleteDraft ? "已保存完整草稿" : "未保存完整草稿（仅显示后端已保存内容，不补全）"
            let activeText = target.active == nil ? "该代次不是激活快照，仅记录写入结果" : "有激活快照"
            info += "\n• 目标代次：第 \(target.generation) 代 · \(BrainPageLogic.historyTimestampText(target.at)) · 操作=\(BrainPageLogic.operationDisplayName(target.operation))"
            info += "\n• 运行配置：默认角色=\(target.runDefaultRoleId ?? missing)，版本=\(target.runDefaultRevisionId ?? missing)，覆盖规则=\(target.overrideCount) 条（\(activeText)）"
            info += "\n• 草稿状态：\(draftText)"
        } else {
            info += "\n• 后端未记录该代次的详细快照；回滚只恢复该代次实际保存的配置。"
        }
        alert.informativeText = info

        let checkbox = NSButton(checkboxWithTitle: "我已知晓回滚将恢复至选定版本并覆盖当前运行配置", target: nil, action: nil)
        checkbox.font = NSFont.systemFont(ofSize: 12)
        alert.accessoryView = checkbox
        alert.addButton(withTitle: "确认回滚")
        alert.addButton(withTitle: "取消")

        guard alert.runModal() == .alertFirstButtonReturn else {
            statusMessageLabel.stringValue = "已取消回滚"
            statusMessageLabel.textColor = .secondaryLabelColor
            return
        }
        guard checkbox.state == .on else {
            statusMessageLabel.stringValue = "回滚已取消：请勾选确认方可执行回滚。"
            statusMessageLabel.textColor = .systemRed
            return
        }

        statusMessageLabel.textColor = .labelColor
        statusMessageLabel.stringValue = "正在执行版本回退到第 \(targetGeneration) 代..."
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                _ = try self.client.rollback(targetGeneration: targetGeneration, expectedGeneration: currentGeneration)
                DispatchQueue.main.async {
                    var msg = "已回退至代次 \(targetGeneration)"
                    if let target = historyTarget, !target.hasCompleteDraft {
                        msg += "（该代次未保存完整草稿，仅恢复后端已保存内容）"
                    }
                    self.statusMessageLabel.stringValue = msg
                    self.statusMessageLabel.textColor = .systemGreen
                    self.reloadBrainData()
                }
            } catch {
                DispatchQueue.main.async {
                    self.statusMessageLabel.stringValue = "回退失败: \(error.localizedDescription)"
                    self.statusMessageLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc private func handleRunDiff() {
        guard let revA = diffVerAPopup.selectedItem?.representedObject as? String,
              let revB = diffVerBPopup.selectedItem?.representedObject as? String,
              let versionA = historyVersions.first(where: { $0.revisionId == revA }),
              let versionB = historyVersions.first(where: { $0.revisionId == revB }) else {
            diffTextView.string = "请选择两个版本进行对比"
            return
        }
        diffTextView.string = BrainPageLogic.compareVersions(v1: versionA, v2: versionB)
    }

    @objc private func handleRunPreview() {
        guard !localUnsavedChanges else {
            previewResultTextView.string = "请先保存角色草稿，再执行「试一试」。"
            return
        }
        guard let role = currentDraftRole else {
            previewResultTextView.string = "请先选择一个角色，再执行「试一试」。"
            return
        }
        let text = previewInputField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else {
            previewResultTextView.string = "请输入客户问题正文"
            return
        }
        let kind = (previewChatKindPopup.indexOfSelectedItem == 1) ? "group" : "private"
        previewResultTextView.string = "正在执行组装预览 (assemble_only)..."
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else { return }
            do {
                let res = try self.client.preview(roleId: role.roleId, revisionId: role.revisionId, text: text, chatKind: kind)
                DispatchQueue.main.async {
                    self.previewResultTextView.string = self.renderTryItResult(res, customerText: text)
                }
            } catch {
                DispatchQueue.main.async {
                    var msg = "试一试失败: \(error.localizedDescription)\n"
                    msg += "真实模型调用不可用：当前构建只提供 assemble_only 确定性组装预览，不会生成真实回复。\n"
                    self.previewResultTextView.string = msg
                }
            }
        }
    }

    /// 试一试结果：只有后端真的返回 decision.reply 才展示回复，否则明确标注不可用。
    private func renderTryItResult(_ result: BrainPreviewResult, customerText: String) -> String {
        var out = "【试一试结果】\n"
        out += "客户问题: \(customerText)\n"
        out += "是否调用模型: \(result.costIncurred ? "是" : "否")\n"
        out += "费用说明: \(result.costNote)\n"
        out += "快照摘要: \(result.snapshot?.assemblySha256 ?? "-")\n"
        if let budget = result.snapshot?.budget {
            out += "字节预算: 原始=\(budget.originalBytes ?? 0), 纳入=\(budget.includedBytes ?? 0), 是否截断=\(budget.truncated == true ? "是" : "否")\n"
        }
        out += "\n【预览回复】\n"
        if let reply = BrainPageLogic.previewReplyText(result) {
            out += "\(reply)\n"
        } else if result.decision != nil {
            out += "后端返回了决策，但未包含 reply 字段；不编造回复内容。\n"
        } else {
            out += "不可用：本次为确定性组装预览 (assemble_only)，未调用真实模型，无法生成回复文本。\n"
        }
        out += "\n【系统约束】\n"
        if let contract = BrainPageLogic.systemContractText(fromCompiled: result.snapshot?.compiled) {
            out += "\(contract)\n"
        } else {
            out += "后端未返回独立系统约束字段（组装内容中未解析到 system_contract）；不做推测。\n"
        }
        out += "\n【组装内容 (compiled)】\n"
        out += "\(result.snapshot?.compiled ?? "（后端未返回组装内容）")\n"
        return out
    }

    // MARK: - NSTableViewDataSource & Delegate

    public func numberOfRows(in tableView: NSTableView) -> Int {
        if tableView == overridesTableView {
            return currentDraftBindings.overrides.count
        }
        if tableView == historyTable {
            return historyGenerations.count
        }
        return displayedRoles.count
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        if tableView == historyTable {
            guard row >= 0, row < historyGenerations.count else { return nil }
            let generation = historyGenerations[row]
            let colId = tableColumn?.identifier.rawValue ?? ""
            let text: String
            switch colId {
            case "col_gen":
                text = "第 \(generation.generation) 代"
            case "col_at":
                text = BrainPageLogic.historyTimestampText(generation.at)
            case "col_op":
                text = BrainPageLogic.operationDisplayName(generation.operation)
            case "col_rev":
                text = generation.runDefaultRevisionId ?? "-"
            case "col_status":
                if let current = listPayload?.generation, current == generation.generation {
                    text = generation.hasCompleteDraft ? "当前激活 (含草稿)" : "当前激活 (未存草稿)"
                } else if generation.hasCompleteDraft {
                    text = "已存草稿"
                } else {
                    text = "未保存完整草稿"
                }
            default:
                text = "-"
            }
            let field = NSTextField(labelWithString: text)
            field.font = NSFont.systemFont(ofSize: 11)
            field.lineBreakMode = .byTruncatingTail
            field.toolTip = text
            if colId == "col_status" {
                field.textColor = generation.hasCompleteDraft ? .secondaryLabelColor : .systemOrange
            }
            return field
        }

        if tableView == overridesTableView {
            guard row < currentDraftBindings.overrides.count else { return nil }
            let ov = currentDraftBindings.overrides[row]
            let colId = tableColumn?.identifier.rawValue ?? ""
            let text: String
            switch colId {
            case "col_contact":
                let match = allowlistContacts.first(where: { $0.contactId == ov.contactId })
                text = match != nil ? "\(match!.displayName) (\(ov.contactId))" : ov.contactId
            case "col_kind":
                text = ov.chatKind == "group" ? "群聊" : "私聊"
            case "col_role":
                let role = listPayload?.roles.first(where: { $0.roleId == ov.roleId })
                text = role != nil ? "\(role!.name) (\(ov.roleId))" : ov.roleId
            case "col_rev":
                text = ov.revisionId
            default:
                text = "-"
            }
            let tf = NSTextField(labelWithString: text)
            tf.font = NSFont.systemFont(ofSize: 11)
            tf.lineBreakMode = .byTruncatingTail
            return tf
        }

        let roles = displayedRoles
        guard row >= 0, row < roles.count else { return nil }
        let entry = roles[row]
        let cell = NSStackView()
        cell.orientation = .vertical
        cell.alignment = .leading
        cell.spacing = 2

        let nameLbl = NSTextField(labelWithString: entry.name)
        nameLbl.font = NSFont.systemFont(ofSize: 12, weight: .bold)

        let isLocal = !(listPayload?.roles.contains(where: { $0.roleId == entry.roleId }) ?? false)
        let isDefault = listPayload?.draft?.defaultRef?.roleId == entry.roleId || listPayload?.active?.defaultRef?.roleId == entry.roleId
        let state = entry.activeRevisionId == nil ? "已保存 · 未启用" : entry.hasDraftChanges ? "有新草稿 · 运行: \(entry.activeRevisionId!)" : "已启用: \(entry.activeRevisionId!)"
        let statusText = isLocal ? "未保存草稿" : (isDefault ? "默认 · " : "") + state
        let statusLbl = NSTextField(labelWithString: statusText)
        statusLbl.font = NSFont.systemFont(ofSize: 10)
        statusLbl.textColor = entry.hasDraftChanges ? .systemOrange : .secondaryLabelColor

        cell.addArrangedSubview(nameLbl)
        cell.addArrangedSubview(statusLbl)
        return cell
    }

    public func tableViewSelectionDidChange(_ notification: Notification) {
        if let tbl = notification.object as? NSTableView, tbl == historyTable {
            let row = historyTable.selectedRow
            if row >= 0, row < historyGenerations.count {
                let generation = historyGenerations[row]
                let missing = "[缺失]"
                var lines: [String] = []
                lines.append("第 \(generation.generation) 代 · \(BrainPageLogic.historyTimestampText(generation.at)) · 操作=\(BrainPageLogic.operationDisplayName(generation.operation))")
                if let roleId = generation.roleId, !roleId.isEmpty {
                    lines.append("写入角色: \(roleId) · 写入版本: \(generation.revisionId ?? missing)")
                }
                if generation.active == nil && generation.draft == nil {
                    lines.append("运行/草稿配置: 后端未记录该代次的绑定快照（仅写入结果）")
                } else {
                    lines.append("运行配置: 默认角色=\(generation.runDefaultRoleId ?? missing) · 版本=\(generation.runDefaultRevisionId ?? missing) · 覆盖规则=\(generation.overrideCount) 条")
                }
                if generation.hasCompleteDraft {
                    lines.append("草稿状态: 已保存完整草稿")
                    historyDetailNoticeLabel.textColor = .secondaryLabelColor
                } else {
                    lines.append("草稿状态: 该代次未保存完整草稿，仅显示后端已保存内容（不补全、不编造）")
                    historyDetailNoticeLabel.textColor = .systemOrange
                }
                historyDetailNoticeLabel.stringValue = lines.joined(separator: "\n")
            } else {
                historyDetailNoticeLabel.stringValue = ""
            }
            return
        }

        if let tbl = notification.object as? NSTableView, tbl == overridesTableView {
            let row = overridesTableView.selectedRow
            if row >= 0 && row < currentDraftBindings.overrides.count {
                let ov = currentDraftBindings.overrides[row]
                for (i, item) in overrideContactPopup.itemArray.enumerated() {
                    if (item.representedObject as? String) == ov.contactId {
                        overrideContactPopup.selectItem(at: i)
                        break
                    }
                }
                for (i, item) in overrideRolePopup.itemArray.enumerated() {
                    if (item.representedObject as? String) == ov.roleId {
                        overrideRolePopup.selectItem(at: i)
                        break
                    }
                }
            }
            return
        }

        let row = roleTable.selectedRow
        if row >= 0 && row != selectedRoleIndex {
            selectedRoleIndex = row
            loadSelectedRole()
        }
    }
}
