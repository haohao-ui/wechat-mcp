// ConfigForm.swift: Logic and UI model for bot configuration form and atomic save workflow.
import Foundation

public struct ConfigMaskResult: Codable {
    public let accountId: String?
    public let modelBackend: String?
    public let modelName: String?
    public let apiKeyStatus: String
    public let editable: [String: AnyCodable]

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case modelBackend = "model_backend"
        case modelName = "model_name"
        case apiKeyStatus = "api_key_status"
        case editable
    }
}

public enum ConfigFormError: Error, LocalizedError {
    case validationFailed(String)
    case backupFailed(String)
    case writeFailed(String)
    case reloadFailed(String)

    public var errorDescription: String? {
        switch self {
        case .validationFailed(let msg): return msg
        case .backupFailed(let msg): return msg
        case .writeFailed(let msg): return msg
        case .reloadFailed(let msg): return msg
        }
    }
}

public struct ConfigFormLogic {
    /// Masks sensitive fields from raw config object. Secrets are never exposed.
    public static func maskConfig(_ raw: [String: Any]) -> [String: Any] {
        var masked: [String: Any] = [:]
        masked["account_id"] = raw["account_id"] as? String ?? ""

        var modelBackend = ""
        var modelName = ""
        var hasKey = false

        if let mc = raw["model_config"] as? [String: Any] {
            modelBackend = mc["backend"] as? String ?? ""
            modelName = mc["model"] as? String ?? ""
            if let key = mc["api_key"] as? String, !key.isEmpty {
                hasKey = true
            }
            if let keyEnv = mc["api_key_env"] as? String, !keyEnv.isEmpty {
                hasKey = true
            }
        }

        masked["model_backend"] = modelBackend
        masked["model_name"] = modelName
        masked["api_key_status"] = hasKey ? "已配置" : "未配置"

        var editable: [String: Any] = [:]
        if let allowlist = raw["allowlist"] { editable["allowlist"] = allowlist }
        if let ignorelist = raw["ignorelist"] { editable["ignorelist"] = ignorelist }
        if let maxAtt = raw["max_attachment_mb"] { editable["max_attachment_mb"] = maxAtt }
        if let maxRep = raw["max_replies_per_hour"] { editable["max_replies_per_hour"] = maxRep }
        if let cd = raw["cooldown_seconds"] { editable["cooldown_seconds"] = cd }
        if let tq = raw["turn_quiet_seconds"] { editable["turn_quiet_seconds"] = tq }
        if let tm = raw["turn_max_seconds"] { editable["turn_max_seconds"] = tm }
        if let cm = raw["context_messages"] { editable["context_messages"] = cm }
        if let mw = raw["manage_window"] { editable["manage_window"] = mw }

        masked["editable"] = editable
        return masked
    }

    /// 严格解析整数字段：不允许包含小数点、非数字字符，必须是有效 Int
    public static func parseIntStrict(field: String, value: String) throws -> Int {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw ConfigFormError.validationFailed("\(field) 不能为空，必须为整数")
        }
        // 只能包含可选的前导负号及连续纯数字
        let regex = try! NSRegularExpression(pattern: "^-?[0-9]+$")
        let range = NSRange(location: 0, length: trimmed.utf16.count)
        guard regex.firstMatch(in: trimmed, options: [], range: range) != nil,
              let intVal = Int(trimmed) else {
            throw ConfigFormError.validationFailed("\(field) 必须为整数，不能包含小数或非数字字符")
        }
        return intVal
    }

    /// 将由 rows: (recipient, kind) 组成的列表或多行文本解析为 contact 数组，
    /// 保留原 entry 的所有其他 key（如 aliases 等）。空输入返回空数组 []。
    public static func parseContactEntries(
        rows: [[String: String]],
        originalList: [[String: Any]]?
    ) -> [[String: Any]] {
        var results: [[String: Any]] = []
        let origs = originalList ?? []

        for row in rows {
            guard let name = row["recipient"]?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else {
                continue
            }
            let rawKind = row["kind"]?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? "private"
            let kind = (rawKind == "group") ? "group" : "private"

            // 查找原 entry，保留其他未知 key（如 aliases）
            if let matched = origs.first(where: { ($0["recipient"] as? String) == name && ($0["kind"] as? String) == kind }) {
                var entry = matched
                entry["recipient"] = name
                entry["kind"] = kind
                results.append(entry)
            } else if let matchedByName = origs.first(where: { ($0["recipient"] as? String) == name }) {
                var entry = matchedByName
                entry["recipient"] = name
                entry["kind"] = kind
                results.append(entry)
            } else {
                results.append(["recipient": name, "kind": kind])
            }
        }
        return results
    }

    /// 解析多行文本形式的联系人列表，如 "群名称, group\n联系人, private"
    public static func parseContactText(
        text: String,
        originalList: [[String: Any]]?
    ) -> [[String: Any]] {
        let lines = text.split(whereSeparator: \.isNewline)
        var rows: [[String: String]] = []
        for line in lines {
            let str = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if str.isEmpty { continue }

            var name = str
            var kind = "private"

            if str.contains(",") {
                let parts = str.split(separator: ",", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                if parts.count >= 1 { name = parts[0] }
                if parts.count >= 2 { kind = parts[1] }
            } else if str.contains(":") {
                let parts = str.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                if parts.count >= 1 { name = parts[0] }
                if parts.count >= 2 { kind = parts[1] }
            }

            rows.append(["recipient": name, "kind": kind])
        }
        return parseContactEntries(rows: rows, originalList: originalList)
    }

    /// 格式化联系人列表为每行 "recipient, kind" 的文本
    public static func formatContactText(_ entries: [[String: Any]]) -> String {
        return entries.map { entry in
            let r = entry["recipient"] as? String ?? ""
            let k = entry["kind"] as? String ?? "private"
            return "\(r), \(k)"
        }.joined(separator: "\n")
    }

    /// 将表单输入合并回原配置，未展示的 keys 保证 deep-equal 原封不动保留
    public static func buildUpdatedConfig(
        original: [String: Any],
        formInputs: [String: Any]
    ) throws -> [String: Any] {
        var updated = original // 基础复制，保留所有 untouched keys

        // 1. allowlist
        if let allowRows = formInputs["allowlist_rows"] as? [[String: String]] {
            let origAllow = original["allowlist"] as? [[String: Any]]
            updated["allowlist"] = parseContactEntries(rows: allowRows, originalList: origAllow)
        } else if let allowText = formInputs["allowlist"] as? String {
            let origAllow = original["allowlist"] as? [[String: Any]]
            updated["allowlist"] = parseContactText(text: allowText, originalList: origAllow)
        }

        // 2. ignorelist
        if let ignoreRows = formInputs["ignorelist_rows"] as? [[String: String]] {
            let origIgnore = original["ignorelist"] as? [[String: Any]]
            updated["ignorelist"] = parseContactEntries(rows: ignoreRows, originalList: origIgnore)
        } else if let ignoreText = formInputs["ignorelist"] as? String {
            let origIgnore = original["ignorelist"] as? [[String: Any]]
            updated["ignorelist"] = parseContactText(text: ignoreText, originalList: origIgnore)
        }

        // 3. Integer fields - 严格整数校验
        let intFieldNames = [
            "max_attachment_mb",
            "max_replies_per_hour",
            "cooldown_seconds",
            "turn_quiet_seconds",
            "turn_max_seconds",
            "context_messages"
        ]
        for field in intFieldNames {
            if let val = formInputs[field] {
                if let str = val as? String {
                    let intVal = try parseIntStrict(field: field, value: str)
                    updated[field] = intVal
                } else if let intVal = val as? Int {
                    updated[field] = intVal
                } else {
                    throw ConfigFormError.validationFailed("\(field) 必须为整数")
                }
            }
        }

        // 4. manage_window
        if let mw = formInputs["manage_window"] as? Bool {
            updated["manage_window"] = mw
        }

        return updated
    }

    /// 获取安全的临时文件目录 (~/.wechat-rpa/tmp)，确保目录权限为 0700
    public static func getSecureTempDirectory() throws -> URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let dir = home.appendingPathComponent(".wechat-rpa/tmp", isDirectory: true)
        if !FileManager.default.fileExists(atPath: dir.path) {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        } else {
            try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
        }
        return dir
    }

    /// 执行保存工作流：
    /// 1. 0700 专有目录下创建 0600 临时文件
    /// 2. 执行 checker 校验
    /// 3. 若有旧文件且备份失败，中止并报错（绝不无备份替换）
    /// 4. 原子替换目标配置
    /// 5. 调用 reloader
    public static func saveConfig(
        targetPath: URL,
        newConfigData: Data,
        checker: (URL) throws -> (ok: Bool, error: String?),
        backupHandler: ((URL, URL) throws -> Void)? = nil,
        reloader: (() throws -> Void)? = nil
    ) throws {
        // 1. 安全临时目录与文件
        let secureDir = try getSecureTempDirectory()
        let tempFile = secureDir.appendingPathComponent("wechat-config-check-\(UUID().uuidString).json")

        try newConfigData.write(to: tempFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: tempFile.path)
        defer {
            try? FileManager.default.removeItem(at: tempFile)
        }

        // 2. 校验
        let checkRes = try checker(tempFile)
        guard checkRes.ok else {
            let errMsg = checkRes.error ?? "配置校验失败"
            throw ConfigFormError.validationFailed(errMsg)
        }

        // 3. 备份（如果失败，中止保存并报错）
        if FileManager.default.fileExists(atPath: targetPath.path) {
            let timestamp = Int(Date().timeIntervalSince1970)
            var bakPath = targetPath.deletingPathExtension().appendingPathExtension("json.bak-\(timestamp)")
            if FileManager.default.fileExists(atPath: bakPath.path) {
                let ms = Int64(Date().timeIntervalSince1970 * 1000)
                bakPath = targetPath.deletingPathExtension().appendingPathExtension("json.bak-\(ms)")
                if FileManager.default.fileExists(atPath: bakPath.path) {
                    try? FileManager.default.removeItem(at: bakPath)
                }
            }
            do {
                if let customBackup = backupHandler {
                    try customBackup(targetPath, bakPath)
                } else {
                    try FileManager.default.copyItem(at: targetPath, to: bakPath)
                }
            } catch {
                throw ConfigFormError.backupFailed("配置文件备份失败: \(error.localizedDescription)，已中止保存")
            }
        }

        // 4. 原子替换目标文件
        let parentDir = targetPath.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: parentDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try newConfigData.write(to: targetPath, options: .atomic)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: targetPath.path)

        // 5. 触发重载
        try reloader?()
    }

    /// 默认 python-based config-check 调用
    public static func defaultCheckConfig(fileURL: URL) throws -> (ok: Bool, error: String?) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")

        let botDir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".wechat-rpa/bot", isDirectory: true)

        var env = ProcessInfo.processInfo.environment
        let existingPyPath = env["PYTHONPATH"] ?? ""
        env["PYTHONPATH"] = botDir.path + ":" + existingPyPath
        process.environment = env
        process.arguments = ["-m", "scripts.wechat_bot.control", "config-check", fileURL.path]

        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe

        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return (false, "无法执行 config-check: \(error.localizedDescription)")
        }

        let outData = stdoutPipe.fileHandleForReading.readDataToEndOfFile()
        if let json = try? JSONSerialization.jsonObject(with: outData, options: []) as? [String: Any] {
            let ok = json["ok"] as? Bool ?? false
            let err = json["error"] as? String
            return (ok, err)
        }

        if process.terminationStatus == 0 {
            return (true, nil)
        } else {
            let errData = stderrPipe.fileHandleForReading.readDataToEndOfFile()
            let errStr = String(data: errData, encoding: .utf8) ?? "未知校验错误"
            return (false, errStr)
        }
    }

    /// 默认机器人重载调用
    public static func defaultReloadBot() throws {
        let uid = geteuid()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["kickstart", "-k", "gui/\(uid)/local.wechat-rpa.bot"]
        try process.run()
        process.waitUntilExit()
    }
}
