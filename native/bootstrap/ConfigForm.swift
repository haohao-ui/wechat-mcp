// ConfigForm.swift: Logic and UI model for bot configuration form and atomic save workflow.
import Foundation
import Darwin

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

        let mc = raw["model_config"] as? [String: Any] ?? [:]
        let backend = mc["backend"] as? String ?? "codex-exec"
        let name = mc["model"] as? String ?? ""
        let base = mc["base_url"] as? String ?? ""
        let file = mc["api_key_file"] as? String ?? ""
        let explicitEnv = mc["api_key_env"] as? String ?? ""
        let defaultEnv = URLComponents(string: base)?.host == "api.deepseek.com" ? "DEEPSEEK_API_KEY" : "OPENAI_API_KEY"
        let env = explicitEnv.isEmpty ? defaultEnv : explicitEnv
        let source = backend == "codex-exec" ? "none" : (!file.isEmpty ? "file" : (!explicitEnv.isEmpty ? "env" : "default_env"))
        let hasKey = source == "file" ? validKeyFile(file) : (source != "none" && validKey(ProcessInfo.processInfo.environment[env] ?? ""))
        masked["model_backend"] = backend
        masked["model_name"] = backend == "codex-exec" ? "" : name
        masked["base_url"] = backend == "codex-exec" || !validModelURL(base) ? "" : base
        masked["key_source"] = source
        masked["key_env_name"] = source == "env" || source == "default_env" ? env : ""
        masked["key_file_path"] = source == "file" ? file : ""
        masked["model_state"] = backend == "codex-exec" ? "default_codex" : (hasKey && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && validModelURL(base) ? "configured" : "unconfigured")
        masked["api_key_status"] = hasKey ? "已配置" : "未配置"
        masked["timeout_seconds"] = mc["timeout_seconds"] as? Int ?? 60
        masked["supports_images"] = mc["supports_images"] as? Bool ?? false
        masked["json_output"] = mc["json_output"] as? Bool ?? true
        masked["images_per_model_call"] = raw["images_per_model_call"] as? Int ?? 4

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
        if let brain = raw["brain"] { editable["brain"] = brain }

        masked["editable"] = editable
        return masked
    }

    private static func validKey(_ value: String) -> Bool {
        let key = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return !key.isEmpty && key.utf8.count <= 4096 && !key.contains(where: { $0.isWhitespace })
    }

    private static func validKeyFile(_ path: String) -> Bool {
        guard path.hasPrefix("/") else { return false }
        let fd = open(path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard fd >= 0 else { return false }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? handle.close() }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_mode & 0o777 == 0o600, info.st_uid == getuid(),
              let data = try? handle.read(upToCount: 4097),
              let key = String(data: data, encoding: .utf8) else { return false }
        return validKey(key)
    }

    private static func validModelURL(_ value: String) -> Bool {
        guard let url = URLComponents(string: value), let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else { return false }
        return url.scheme == "https" || (url.scheme == "http" && ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host))
    }

    private static func modelForm(_ form: [String: Any], original: [String: Any]) throws -> (model: [String: Any], images: Int) {
        func fail(_ code: String, _ message: String) -> ConfigFormError { .validationFailed("\(code): \(message)") }
        func integer(_ key: String, _ fallback: Int, _ range: ClosedRange<Int>) throws -> Int {
            let text = form[key].map { String(describing: $0) } ?? String(fallback)
            guard text.range(of: "^[0-9]+$", options: .regularExpression) != nil, let n = Int(text), range.contains(n) else {
                throw fail(key == "timeout_seconds" ? "MODEL_TIMEOUT_INVALID" : "MODEL_IMAGES_INVALID", key == "timeout_seconds" ? "超时必须为 1–180 之间的整数" : "图片数必须为 1–16 之间的整数")
            }
            return n
        }
        let supports = form["supports_images"] as? Bool ?? false
        let images = supports ? try integer("images_per_model_call", original["images_per_model_call"] as? Int ?? 4, 1...16) : original["images_per_model_call"] as? Int ?? 4
        let backend = form["backend"] as? String ?? "codex-exec"
        if backend == "codex-exec" { return (["backend": backend], images) }
        guard backend == "openai-compatible" else { throw fail("MODEL_BACKEND_INVALID", "请选择有效的接入方式") }
        let base = (form["base_url"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard validModelURL(base) else { throw fail("MODEL_URL_INVALID", "接口地址必须使用 HTTPS（本机可用 HTTP），且不能包含凭据、查询或片段") }
        let model = (form["model"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !model.isEmpty else { throw fail("MODEL_NAME_REQUIRED", "请填写模型名") }
        var result: [String: Any] = ["backend": backend, "base_url": base, "model": model,
            "timeout_seconds": try integer("timeout_seconds", 60, 1...180), "supports_images": supports,
            "json_output": form["json_output"] as? Bool ?? true]
        let source = form["key_source"] as? String ?? "env"
        if source == "env" {
            let env = form["key_env_name"] as? String ?? form["api_key_env"] as? String ?? ""
            guard env.range(of: "^[A-Za-z_][A-Za-z0-9_]*$", options: .regularExpression) != nil else { throw fail("MODEL_KEY_ENV_INVALID", "请填写有效的环境变量名") }
            result["api_key_env"] = env
        } else if source == "file" {
            let path = form["key_file_path"] as? String ?? form["api_key_file"] as? String ?? ""
            guard validKeyFile(path) else { throw fail("MODEL_KEY_FILE_INVALID", "密钥文件必须为当前用户所有的绝对路径普通文件、权限 0600、非符号链接且内容有效") }
            result["api_key_file"] = path
        } else if source != "default_env" { throw fail("MODEL_KEY_SOURCE_INVALID", "请选择有效的密钥来源") }
        return (result, images)
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
                    // Empty: keep the original value, or leave the key out so the bot uses its default.
                    if str.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
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

        // 5. brain setting
        if let brainModeRaw = formInputs["brain_mode"] as? String {
            let brainMode = brainModeRaw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if brainMode == "builtin" {
                updated["brain"] = ["mode": "builtin"]
            } else if brainMode == "external" {
                var extDict: [String: Any] = [:]

                // timeout_seconds: 1..3600 (default 120)
                var timeoutSec = 120
                if let tVal = formInputs["brain_timeout_seconds"] {
                    if let tStr = tVal as? String {
                        let trimmed = tStr.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !trimmed.isEmpty {
                            timeoutSec = try parseIntStrict(field: "brain_timeout_seconds", value: trimmed)
                        }
                    } else if let tInt = tVal as? Int {
                        timeoutSec = tInt
                    } else {
                        throw ConfigFormError.validationFailed("brain_timeout_seconds 必须为整数")
                    }
                }
                guard timeoutSec >= 1 && timeoutSec <= 3600 else {
                    throw ConfigFormError.validationFailed("brain.external.timeout_seconds 必须在 1 到 3600 秒之间")
                }
                extDict["timeout_seconds"] = timeoutSec

                // timeout_action: "skip" | "builtin" | "handoff" (default "skip")
                var timeoutAct = "skip"
                if let aVal = formInputs["brain_timeout_action"] as? String {
                    let aStr = aVal.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                    if !aStr.isEmpty {
                        timeoutAct = aStr
                    }
                }
                guard ["skip", "builtin", "handoff"].contains(timeoutAct) else {
                    throw ConfigFormError.validationFailed("brain.external.timeout_action 必须是 skip、builtin 或 handoff")
                }
                extDict["timeout_action"] = timeoutAct

                // queue_minutes: 1..1440 (default 30)
                var queueMin = 30
                if let qVal = formInputs["brain_queue_minutes"] {
                    if let qStr = qVal as? String {
                        let trimmed = qStr.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !trimmed.isEmpty {
                            queueMin = try parseIntStrict(field: "brain_queue_minutes", value: trimmed)
                        }
                    } else if let qInt = qVal as? Int {
                        queueMin = qInt
                    } else {
                        throw ConfigFormError.validationFailed("brain_queue_minutes 必须为整数")
                    }
                }
                guard queueMin >= 1 && queueMin <= 1440 else {
                    throw ConfigFormError.validationFailed("brain.external.queue_minutes 必须在 1 到 1440 分钟之间")
                }
                extDict["queue_minutes"] = queueMin

                updated["brain"] = [
                    "mode": "external",
                    "external": extDict
                ]
            } else {
                throw ConfigFormError.validationFailed("brain.mode 必须是 builtin 或 external")
            }
        } else if let brainObj = formInputs["brain"] as? [String: Any] {
            guard let mode = brainObj["mode"] as? String else {
                throw ConfigFormError.validationFailed("brain.mode 缺失")
            }
            if mode == "builtin" {
                guard brainObj.count == 1 else {
                    throw ConfigFormError.validationFailed("brain.mode=builtin 时不允许包含其他字段")
                }
                updated["brain"] = ["mode": "builtin"]
            } else if mode == "external" {
                for (k, _) in brainObj {
                    if !["mode", "external"].contains(k) {
                        throw ConfigFormError.validationFailed("brain 包含未知字段: \(k)")
                    }
                }
                var extDict: [String: Any] = [:]
                if let ext = brainObj["external"] as? [String: Any] {
                    for (k, _) in ext {
                        if !["timeout_seconds", "timeout_action", "queue_minutes"].contains(k) {
                            throw ConfigFormError.validationFailed("brain.external 包含未知字段: \(k)")
                        }
                    }
                    let t = ext["timeout_seconds"] as? Int ?? 120
                    guard t >= 1 && t <= 3600 else {
                        throw ConfigFormError.validationFailed("brain.external.timeout_seconds 必须在 1 到 3600 秒之间")
                    }
                    extDict["timeout_seconds"] = t

                    let a = ext["timeout_action"] as? String ?? "skip"
                    guard ["skip", "builtin", "handoff"].contains(a) else {
                        throw ConfigFormError.validationFailed("brain.external.timeout_action 必须是 skip、builtin 或 handoff")
                    }
                    extDict["timeout_action"] = a

                    let q = ext["queue_minutes"] as? Int ?? 30
                    guard q >= 1 && q <= 1440 else {
                        throw ConfigFormError.validationFailed("brain.external.queue_minutes 必须在 1 到 1440 分钟之间")
                    }
                    extDict["queue_minutes"] = q
                } else {
                    extDict = ["timeout_seconds": 120, "timeout_action": "skip", "queue_minutes": 30]
                }
                updated["brain"] = ["mode": "external", "external": extDict]
            } else {
                throw ConfigFormError.validationFailed("brain.mode 必须是 builtin 或 external")
            }
        }

        if let input = formInputs["model_config_form"] {
            guard let form = input as? [String: Any] else { throw ConfigFormError.validationFailed("MODEL_FORM_INVALID: 模型配置格式无效") }
            let model = try modelForm(form, original: original)
            updated["model_config"] = model.model
            updated["images_per_model_call"] = model.images
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

    /// 默认 Rust config-check 调用，共用宿主带超时的命令执行器。
    public static func defaultCheckConfig(fileURL: URL) throws -> (ok: Bool, error: String?) {
        let result = runControlCommand(executable: controlBinaryPath(), arguments: [
            "control", "config-check", "--config", fileURL.path, "--json"
        ], timeout: 20)
        if result.timedOut { return (false, "CONFIG_CHECK_TIMEOUT: 配置校验超时") }
        if result.status == 0,
           let json = try? JSONSerialization.jsonObject(with: result.output) as? [String: Any], json["ok"] as? Bool == true { return (true, nil) }
        return (false, "CONFIG_CHECK_FAILED: 配置校验失败，请检查配置")
    }

    static func configCheckResult(_ result: ControlRun) -> (ok: Bool, error: String?) {
        if result.timedOut { return (false, "config-check 超时") }
        if let json = try? JSONSerialization.jsonObject(with: result.output) as? [String: Any] {
            if result.status == 0 && json["ok"] as? Bool == true { return (true, nil) }
            return (false, json["error"] as? String ?? "config-check 校验失败 (exit \(result.status))")
        }
        let diagnostic = String(data: result.output, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return (false, diagnostic.isEmpty ? "无法执行 config-check (exit \(result.status))" : diagnostic)
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

public struct ControlRun {
    public let status: Int32
    public let output: Data
    public let timedOut: Bool
}

/// 运行 wechat-mcp control 子命令并取标准输出。
/// 必须边运行边读完输出再等结束：先 waitUntilExit 再读管道，输出超过管道缓冲（约 64KB）就会互相等死。
/// 标准错误合并到同一输出管道，供失败诊断；超过 timeout 秒仍未结束则终止并返回 timedOut。
public func runControlCommand(executable: String, arguments: [String], timeout: Double) -> ControlRun {
    let proc = Process()
    proc.executableURL = URL(fileURLWithPath: executable)
    proc.arguments = arguments
    let pipe = Pipe()
    proc.standardOutput = pipe
    proc.standardError = pipe
    do { try proc.run() } catch { return ControlRun(status: -1, output: Data(), timedOut: false) }

    let lock = NSLock()
    var collected = Data()
    let readDone = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .utility).async {
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        lock.lock(); collected = data; lock.unlock()
        readDone.signal()
    }
    var timedOut = false
    if readDone.wait(timeout: .now() + timeout) == .timedOut {
        timedOut = true
        proc.terminate()
        _ = readDone.wait(timeout: .now() + 2)
    }
    proc.waitUntilExit()
    lock.lock(); let out = collected; lock.unlock()
    return ControlRun(status: proc.terminationStatus, output: out, timedOut: timedOut)
}

/// 宿主调用的 wechat-mcp 路径
public func controlBinaryPath() -> String {
    if let envBin = ProcessInfo.processInfo.environment["WECHAT_MCP_BIN"], FileManager.default.isExecutableFile(atPath: envBin) {
        return envBin
    }
    let rpaBin = (NSHomeDirectory() as NSString).appendingPathComponent(".wechat-rpa/bin/wechat-mcp")
    return FileManager.default.isExecutableFile(atPath: rpaBin) ? rpaBin : "/usr/local/bin/wechat-mcp"
}
