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
    case writeFailed(String)
    case reloadFailed(String)

    public var errorDescription: String? {
        switch self {
        case .validationFailed(let msg): return msg
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
                // 如果环境变量名已配置，也视为已配置
                hasKey = true
            }
        }

        masked["model_backend"] = modelBackend
        masked["model_name"] = modelName
        masked["api_key_status"] = hasKey ? "已配置" : "未配置"

        // Editable fields allowed by spec §5.5
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

    /// Performs the save workflow:
    /// 1. Write temp file
    /// 2. Validate via checker
    /// 3. If invalid -> discard temp file, return error without touching target
    /// 4. If valid -> backup existing config as config.json.bak-<timestamp> -> atomic replace -> reload
    public static func saveConfig(
        targetPath: URL,
        newConfigData: Data,
        checker: (URL) throws -> (ok: Bool, error: String?),
        reloader: (() throws -> Void)? = nil
    ) throws {
        let tempDir = FileManager.default.temporaryDirectory
        let tempFile = tempDir.appendingPathComponent("wechat-config-check-\(UUID().uuidString).json")

        // 1. Write to temp file
        try newConfigData.write(to: tempFile, options: .atomic)
        defer {
            try? FileManager.default.removeItem(at: tempFile)
        }

        // 2. Validate
        let checkRes = try checker(tempFile)
        guard checkRes.ok else {
            let errMsg = checkRes.error ?? "配置校验失败"
            throw ConfigFormError.validationFailed(errMsg)
        }

        // 3. Backup existing target
        if FileManager.default.fileExists(atPath: targetPath.path) {
            let timestamp = Int(Date().timeIntervalSince1970)
            let bakPath = targetPath.deletingPathExtension().appendingPathExtension("json.bak-\(timestamp)")
            try? FileManager.default.copyItem(at: targetPath, to: bakPath)
        }

        // 4. Atomic write / replace target
        let parentDir = targetPath.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: parentDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try newConfigData.write(to: targetPath, options: .atomic)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: targetPath.path)

        // 5. Reload (e.g. launchctl kickstart)
        try reloader?()
    }

    /// Default python-based config-check invocation
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

    /// Default bot reload invocation
    public static func defaultReloadBot() throws {
        let uid = geteuid()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["kickstart", "-k", "gui/\(uid)/local.wechat-rpa.bot"]
        try process.run()
        process.waitUntilExit()
    }
}
