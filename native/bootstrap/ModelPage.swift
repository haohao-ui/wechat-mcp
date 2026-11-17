// ModelPage.swift: Global model configuration view controller and validation logic (Issue #2 AC10).
// Minimum deployment target: macOS 12.0
import Foundation
import AppKit
import Darwin

// MARK: - Localized Strings Constants (Centralized for future migration to L10n.swift by T4)

public enum ModelPageStrings {
    // 页面标题与导言
    public static var pageTitle: String { L10n.t(.navModel) }
    public static var scopeNote: String { L10n.t(.modelPageScopeNote) }
    public static var statusConfigured: String { L10n.t(.modelConfigured) }
    public static var statusUnconfigured: String { L10n.t(.modelUnconfigured) }
    public static var tagDefault: String { L10n.t(.modelDefault) }

    // 状态横幅
    public static var bannerDefaultCodex: String { L10n.t(.modelPageBannerDefaultCodex) }
    public static var bannerUnset: String { L10n.t(.modelPageBannerUnset) }
    public static var bannerPendingRestart: String { L10n.t(.modelPageBannerPendingRestart) }
    public static var bannerErrorSummary: String { L10n.t(.modelPageBannerErrorSummary) }

    // 卡片 1: 接入方式
    public static var sectionBackend: String { L10n.t(.modelPageSectionBackend) }
    public static let backendCodex = "codex-exec"
    public static let backendOpenAI = "openai-compatible"
    public static var backendCodexDesc: String { L10n.t(.modelPageBackendCodexDesc) }
    public static var backendOpenAIDesc: String { L10n.t(.modelPageBackendOpenAIDesc) }

    // 卡片 2: 连接 (Connection) - 当 backend == "openai-compatible" 时展示
    public static var sectionConnection: String { L10n.t(.modelPageSectionConnection) }
    public static var fieldBaseUrl: String { L10n.t(.modelPageFieldBaseUrl) }
    public static let fieldBaseUrlPlaceholder = "https://"
    public static var fieldBaseUrlHelper: String { L10n.t(.modelPageFieldBaseUrlHelper) }

    public static var fieldModel: String { L10n.t(.modelPageFieldModel) }
    public static var fieldModelPlaceholder: String { L10n.t(.modelPageFieldModelPlaceholder) }

    public static var fieldKeySource: String { L10n.t(.modelPageFieldKeySource) }
    public static var keySourceEnv: String { L10n.t(.modelPageKeySourceEnv) }
    public static var keySourceFile: String { L10n.t(.modelPageKeySourceFile) }
    public static var keySourceEnvHelper: String { L10n.t(.modelPageKeySourceEnvHelper) }
    public static var keySourceFileHelper: String { L10n.t(.modelPageKeySourceFileHelper) }

    public static var fieldKeyStatus: String { L10n.t(.modelPageFieldKeyStatus) }
    public static var keyStatusConfigured: String { L10n.t(.modelConfigured) }
    public static var keyStatusUnconfigured: String { L10n.t(.modelUnconfigured) }
    public static var keyStatusEnvNotFound: String { L10n.t(.modelPageKeyStatusEnvNotFound) }
    public static var keyNeverExposed: String { L10n.t(.modelPageKeyNeverExposed) }

    // 卡片 3: 参数设置 (Model Parameters)
    public static var sectionParameters: String { L10n.t(.modelPageSectionParameters) }
    public static var fieldTimeout: String { L10n.t(.modelPageFieldTimeout) }
    public static var timeoutUnit: String { L10n.t(.unitSeconds) }
    public static var timeoutHelper: String { L10n.t(.modelPageTimeoutHelper) }

    public static var fieldSupportsImages: String { L10n.t(.modelPageFieldSupportsImages) }
    public static var supportsImagesHelper: String { L10n.t(.modelPageSupportsImagesHelper) }

    public static var fieldImagesPerCall: String { L10n.t(.modelPageFieldImagesPerCall) }
    public static var imagesPerCallHelper: String { L10n.t(.modelPageImagesPerCallHelper) }

    public static var fieldJsonOutput: String { L10n.t(.modelPageFieldJsonOutput) }
    public static var jsonOutputHelper: String { L10n.t(.modelPageJsonOutputHelper) }

    public static var fieldMaxTokens: String { L10n.t(.modelPageFieldMaxTokens) }
    public static var maxTokensReadOnly: String { L10n.t(.modelPageMaxTokensReadOnly) }

    // 卡片 4: 操作栏与按钮
    public static var buttonTest: String { L10n.t(.modelPageButtonTest) }
    public static var buttonSave: String { L10n.t(.actionSaveConfig) }
    public static var buttonDiscard: String { L10n.t(.modelPageButtonDiscard) }
    public static var buttonRestartCore: String { L10n.t(.actionRestartCore) }

    // 测试连接结果横幅
    public static var testPassedTitle: String { L10n.t(.modelPageTestPassedTitle) }
    public static var testPassedNote: String { L10n.t(.modelPageTestPassedNote) }
    public static var testFailedTitle: String { L10n.t(.modelPageTestFailedTitle) }
    public static var testFailedNote: String { L10n.t(.modelPageTestFailedNote) }

    // 二次确认弹窗
    public static var dialogConfirmTitle: String { L10n.t(.modelPageDialogConfirmTitle) }
    public static var dialogConfirmBody: String { L10n.t(.modelPageDialogConfirmBody) }
    public static var dialogConfirmCheckbox: String { L10n.t(.modelPageDialogConfirmCheckbox) }
    public static var dialogConfirmButton: String { L10n.t(.modelPageDialogConfirmButton) }
    public static var dialogCancelButton: String { L10n.t(.cancel) }

    // 字段错误提示
    public static var errorUrlRequired: String { L10n.t(.modelPageErrorUrlRequired) }
    public static var errorUrlInvalid: String { L10n.t(.modelPageErrorUrlInvalid) }
    public static var errorHttpsRequired: String { L10n.t(.modelPageErrorHttpsRequired) }
    public static var errorUrlCredentials: String { L10n.t(.modelPageErrorUrlCredentials) }
    public static var errorModelRequired: String { L10n.t(.modelPageErrorModelRequired) }
    public static var errorTimeoutRange: String { L10n.t(.modelPageErrorTimeoutRange) }
    public static var errorImagesRange: String { L10n.t(.modelPageErrorImagesRange) }
    public static var errorEnvRequired: String { L10n.t(.modelPageErrorEnvRequired) }
    public static var errorEnvInvalid: String { L10n.t(.modelPageErrorEnvInvalid) }
    public static var errorFileRequired: String { L10n.t(.modelPageErrorFileRequired) }
    public static var errorFileNotAbsolute: String { L10n.t(.modelPageErrorFileNotAbsolute) }
    public static var errorFilePermission: String { L10n.t(.modelPageErrorFilePermission) }
    public static var errorFileNotFoundOrAccess: String { L10n.t(.modelPageErrorFileNotFoundOrAccess) }
}

// MARK: - Model Draft Model

public struct ModelPageDraft: Equatable {
    public var backend: String
    public var baseUrl: String
    public var model: String
    public var keySource: String // "env" | "file"
    public var apiKeyEnv: String
    public var apiKeyFile: String
    public var supportsImages: Bool
    public var timeoutSeconds: String
    public var jsonOutput: Bool
    public var imagesPerModelCall: String

    public init(
        backend: String = "codex-exec",
        baseUrl: String = "",
        model: String = "",
        keySource: String = "env",
        apiKeyEnv: String = "",
        apiKeyFile: String = "",
        supportsImages: Bool = false,
        timeoutSeconds: String = "60",
        jsonOutput: Bool = true,
        imagesPerModelCall: String = "4"
    ) {
        self.backend = backend
        self.baseUrl = baseUrl
        self.model = model
        self.keySource = keySource
        self.apiKeyEnv = apiKeyEnv
        self.apiKeyFile = apiKeyFile
        self.supportsImages = supportsImages
        self.timeoutSeconds = timeoutSeconds
        self.jsonOutput = jsonOutput
        self.imagesPerModelCall = imagesPerModelCall
    }

    public static func fromConfig(_ raw: [String: Any]) -> ModelPageDraft {
        let mc = raw["model_config"] as? [String: Any] ?? [:]
        let backend = mc["backend"] as? String ?? "codex-exec"
        let name = mc["model"] as? String ?? ""
        let base = mc["base_url"] as? String ?? ""
        let file = mc["api_key_file"] as? String ?? ""
        let env = mc["api_key_env"] as? String ?? ""
        let source = !file.isEmpty ? "file" : "env"
        let timeout = mc["timeout_seconds"] as? Int ?? 60
        let supports = mc["supports_images"] as? Bool ?? false
        let json = mc["json_output"] as? Bool ?? true
        let images = raw["images_per_model_call"] as? Int ?? 4

        return ModelPageDraft(
            backend: backend,
            baseUrl: base,
            model: name,
            keySource: source,
            apiKeyEnv: env.isEmpty && source == "env" && base.contains("deepseek") ? "DEEPSEEK_API_KEY" : (env.isEmpty && source == "env" ? "OPENAI_API_KEY" : env),
            apiKeyFile: file,
            supportsImages: supports,
            timeoutSeconds: String(timeout),
            jsonOutput: json,
            imagesPerModelCall: String(images)
        )
    }

    public func toModelConfigForm() -> [String: Any] {
        var form: [String: Any] = [
            "backend": backend,
            "supports_images": supportsImages,
            "json_output": jsonOutput,
            "timeout_seconds": Int(timeoutSeconds) ?? 60,
            "images_per_model_call": Int(imagesPerModelCall) ?? 4
        ]
        if backend == "openai-compatible" {
            form["base_url"] = baseUrl.trimmingCharacters(in: .whitespacesAndNewlines)
            form["model"] = model.trimmingCharacters(in: .whitespacesAndNewlines)
            form["key_source"] = keySource
            if keySource == "env" {
                form["key_env_name"] = apiKeyEnv.trimmingCharacters(in: .whitespacesAndNewlines)
                form["api_key_env"] = apiKeyEnv.trimmingCharacters(in: .whitespacesAndNewlines)
            } else {
                form["key_file_path"] = apiKeyFile.trimmingCharacters(in: .whitespacesAndNewlines)
                form["api_key_file"] = apiKeyFile.trimmingCharacters(in: .whitespacesAndNewlines)
            }
        }
        return form
    }
}

// MARK: - Validation Logic

public struct ModelPageValidator {
    public static func validModelURL(_ value: String) -> (ok: Bool, reason: String?) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return (false, ModelPageStrings.errorUrlRequired)
        }
        guard let url = URLComponents(string: trimmed), let host = url.host, !host.isEmpty else {
            return (false, ModelPageStrings.errorUrlInvalid)
        }
        if url.user != nil || url.password != nil || url.query != nil || trimmed.contains("?") || url.fragment != nil || trimmed.contains("#") {
            return (false, ModelPageStrings.errorUrlCredentials)
        }
        let isLocal = ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host)
        if url.scheme == "https" || (url.scheme == "http" && isLocal) {
            return (true, nil)
        }
        return (false, ModelPageStrings.errorHttpsRequired)
    }

    public static func validEnvName(_ value: String) -> (ok: Bool, reason: String?) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return (false, ModelPageStrings.errorEnvRequired)
        }
        let regex = try! NSRegularExpression(pattern: "^[A-Za-z_][A-Za-z0-9_]*$")
        let range = NSRange(location: 0, length: trimmed.utf16.count)
        if regex.firstMatch(in: trimmed, options: [], range: range) != nil {
            return (true, nil)
        }
        return (false, ModelPageStrings.errorEnvInvalid)
    }

    public static func validKeyFile(_ path: String) -> (ok: Bool, reason: String?) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return (false, ModelPageStrings.errorFileRequired)
        }
        guard trimmed.hasPrefix("/") else {
            return (false, ModelPageStrings.errorFileNotAbsolute)
        }
        let fd = open(trimmed, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard fd >= 0 else {
            return (false, ModelPageStrings.errorFilePermission)
        }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? handle.close() }
        var info = stat()
        guard fstat(fd, &info) == 0,
              info.st_mode & S_IFMT == S_IFREG,
              info.st_mode & 0o777 == 0o600,
              info.st_uid == getuid() else {
            return (false, ModelPageStrings.errorFilePermission)
        }
        guard let data = try? handle.read(upToCount: 4097),
              let key = String(data: data, encoding: .utf8),
              validKeyContent(key) else {
            return (false, ModelPageStrings.errorFilePermission)
        }
        return (true, nil)
    }

    private static func validKeyContent(_ value: String) -> Bool {
        let key = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return !key.isEmpty && key.utf8.count <= 4096 && !key.contains(where: { $0.isWhitespace })
    }

    public static func isKeyConfigured(draft: ModelPageDraft) -> Bool {
        if draft.backend == "codex-exec" {
            return true
        }
        if draft.keySource == "env" {
            let env = draft.apiKeyEnv.trimmingCharacters(in: .whitespacesAndNewlines)
            guard validEnvName(env).ok else { return false }
            guard let val = ProcessInfo.processInfo.environment[env] else { return false }
            return validKeyContent(val)
        } else {
            return validKeyFile(draft.apiKeyFile).ok
        }
    }

    public static func validate(draft: ModelPageDraft) -> [String: String] {
        var errors: [String: String] = [:]

        // 超时时间
        let timeoutTrim = draft.timeoutSeconds.trimmingCharacters(in: .whitespacesAndNewlines)
        if let t = Int(timeoutTrim), (1...180).contains(t) {
            // ok
        } else {
            errors["timeout_seconds"] = ModelPageStrings.errorTimeoutRange
        }

        // 图片数
        if draft.supportsImages {
            let imgTrim = draft.imagesPerModelCall.trimmingCharacters(in: .whitespacesAndNewlines)
            if let img = Int(imgTrim), (1...16).contains(img) {
                // ok
            } else {
                errors["images_per_model_call"] = ModelPageStrings.errorImagesRange
            }
        }

        if draft.backend == "openai-compatible" {
            // base_url
            let urlCheck = validModelURL(draft.baseUrl)
            if !urlCheck.ok, let r = urlCheck.reason {
                errors["base_url"] = r
            }

            // model
            let modelTrim = draft.model.trimmingCharacters(in: .whitespacesAndNewlines)
            if modelTrim.isEmpty {
                errors["model"] = ModelPageStrings.errorModelRequired
            }

            // key
            if draft.keySource == "env" {
                let envCheck = validEnvName(draft.apiKeyEnv)
                if !envCheck.ok, let r = envCheck.reason {
                    errors["api_key_env"] = r
                }
            } else {
                let fileCheck = validKeyFile(draft.apiKeyFile)
                if !fileCheck.ok, let r = fileCheck.reason {
                    errors["api_key_file"] = r
                }
            }
        }

        return errors
    }

    public static func isUnconfigured(draft: ModelPageDraft) -> Bool {
        if draft.backend == "codex-exec" {
            return false
        }
        let baseValid = validModelURL(draft.baseUrl).ok
        let modelValid = !draft.model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let keyValid = isKeyConfigured(draft: draft)
        return !(baseValid && modelValid && keyValid)
    }
}

// MARK: - Probe Result & Runner

public enum ModelProbeResult: Equatable {
    case idle
    case success
    case failure(reason: String)
}

public protocol ModelProbeRunnerProtocol {
    func probe(config: ModelPageDraft, completion: @escaping (ModelProbeResult) -> Void)
}

public final class DefaultModelProbeRunner: ModelProbeRunnerProtocol {
    public init() {}

    public func probe(config: ModelPageDraft, completion: @escaping (ModelProbeResult) -> Void) {
        // 在后台线程执行只读探测，绝不暴露供应商原始报错或密钥
        DispatchQueue.global(qos: .userInitiated).async {
            if config.backend == "codex-exec" {
                // 检查本机 codex 是否存在且可执行
                let bin = ProcessInfo.processInfo.environment["WECHAT_CODEX_BIN"] ?? "codex"
                let ok: Bool
                if bin.contains("/") {
                    ok = FileManager.default.isExecutableFile(atPath: bin)
                } else if let paths = ProcessInfo.processInfo.environment["PATH"]?.split(separator: ":") {
                    ok = paths.contains { FileManager.default.isExecutableFile(atPath: String($0) + "/" + bin) }
                } else {
                    ok = false
                }
                DispatchQueue.main.async {
                    if ok {
                        completion(.success)
                    } else {
                        completion(.failure(reason: L10n.t(.modelCodexMissing)))
                    }
                }
                return
            }

            // openai-compatible 探测
            guard let url = URL(string: config.baseUrl.trimmingCharacters(in: .whitespacesAndNewlines))?.appendingPathComponent("models") else {
                DispatchQueue.main.async {
                    completion(.failure(reason: L10n.t(.modelPageErrorUrlInvalid)))
                }
                return
            }

            var key = ""
            if config.keySource == "env" {
                let env = config.apiKeyEnv.trimmingCharacters(in: .whitespacesAndNewlines)
                key = ProcessInfo.processInfo.environment[env] ?? ""
            } else {
                let path = config.apiKeyFile.trimmingCharacters(in: .whitespacesAndNewlines)
                if let d = try? Data(contentsOf: URL(fileURLWithPath: path)),
                   let str = String(data: d, encoding: .utf8) {
                    key = str.trimmingCharacters(in: .whitespacesAndNewlines)
                }
            }

            guard !key.isEmpty else {
                DispatchQueue.main.async {
                    completion(.failure(reason: L10n.t(.modelKeyMissing)))
                }
                return
            }

            var request = URLRequest(url: url)
            request.httpMethod = "GET"
            request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
            request.timeoutInterval = Double(Int(config.timeoutSeconds) ?? 60)

            let task = URLSession.shared.dataTask(with: request) { data, response, error in
                let isOk: Bool
                if let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode), let data = data {
                    if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                       let list = json["data"] as? [[String: Any]] {
                        let targetModel = config.model.trimmingCharacters(in: .whitespacesAndNewlines)
                        isOk = list.contains { ($0["id"] as? String) == targetModel }
                    } else {
                        isOk = false
                    }
                } else {
                    isOk = false
                }

                DispatchQueue.main.async {
                    if isOk {
                        completion(.success)
                    } else {
                        // 失败时不显示密钥或供应商原始报错
                        completion(.failure(reason: ModelPageStrings.testFailedNote))
                    }
                }
            }
            task.resume()
        }
    }
}

// MARK: - Flipped Document View

public final class ModelPageFlippedView: NSView {
    public override var isFlipped: Bool { true }
}

// MARK: - ModelPageController

public final class ModelPageController: NSViewController {
    public let pageId = "model"

    private weak var context: AnyObject?
    private var probeRunner: ModelProbeRunnerProtocol
    private var isCoreRunningHandler: (() -> Bool)?
    private var saveHandler: (([String: Any]) throws -> Void)?

    // 表单状态
    public private(set) var savedDraft: ModelPageDraft
    public private(set) var currentDraft: ModelPageDraft
    public private(set) var hasPendingRestartBanner: Bool = false
    public private(set) var probeResult: ModelProbeResult = .idle

    // UI - Header
    private let titleLabel = NSTextField(labelWithString: ModelPageStrings.pageTitle)
    private let scopeLabel = NSTextField(labelWithString: ModelPageStrings.scopeNote)
    private let topStatusTag = ThemeStatusTag(status: .normal, customText: ModelPageStrings.statusConfigured)
    private let topDefaultTag = ThemeStatusTag(status: .inactive, customText: ModelPageStrings.tagDefault, showsDot: false)

    // UI - Banners
    private let bannerContainer = NSStackView()
    private var activeBanners: [NSView] = []

    // UI - Section 1: 接入方式
    private let backendSegment = NSSegmentedControl(labels: [ModelPageStrings.backendCodex, ModelPageStrings.backendOpenAI], trackingMode: .selectOne, target: nil, action: nil)
    private let backendDescLabel = NSTextField(wrappingLabelWithString: "")

    // UI - Section 2: 连接 (Connection Card)
    private let connectionBox = NSBox()
    private let baseUrlField = ThemeFormField(label: ModelPageStrings.fieldBaseUrl, placeholder: ModelPageStrings.fieldBaseUrlPlaceholder, helper: ModelPageStrings.fieldBaseUrlHelper)
    private let modelField = ThemeFormField(label: ModelPageStrings.fieldModel, placeholder: ModelPageStrings.fieldModelPlaceholder)
    private let keySourceSegment = NSSegmentedControl(labels: [ModelPageStrings.keySourceEnv, ModelPageStrings.keySourceFile], trackingMode: .selectOne, target: nil, action: nil)
    private let keyInputField = ThemeFormField(label: ModelPageStrings.fieldKeySource, placeholder: "OPENAI_API_KEY", helper: ModelPageStrings.keySourceEnvHelper)
    private let keyStatusTag = ThemeStatusTag(status: .normal, customText: ModelPageStrings.keyStatusConfigured)
    private let keyStatusNoteLabel = NSTextField(wrappingLabelWithString: ModelPageStrings.keyNeverExposed)

    // UI - Section 3: 参数设置 (Parameters Card)
    private let parametersBox = NSBox()
    private let timeoutField = ThemeFormField(label: ModelPageStrings.fieldTimeout, placeholder: "60", helper: ModelPageStrings.timeoutHelper)
    private let supportsImagesCheck = NSButton(checkboxWithTitle: ModelPageStrings.fieldSupportsImages, target: nil, action: nil)
    private let supportsImagesDesc = NSTextField(wrappingLabelWithString: ModelPageStrings.supportsImagesHelper)
    private let imagesPerCallField = ThemeFormField(label: ModelPageStrings.fieldImagesPerCall, placeholder: "4", helper: ModelPageStrings.imagesPerCallHelper)
    private let jsonOutputCheck = NSButton(checkboxWithTitle: ModelPageStrings.fieldJsonOutput, target: nil, action: nil)
    private let jsonOutputDesc = NSTextField(wrappingLabelWithString: ModelPageStrings.jsonOutputHelper)
    private let maxTokensLabel = NSTextField(labelWithString: ModelPageStrings.fieldMaxTokens)
    private let maxTokensValueLabel = NSTextField(labelWithString: ModelPageStrings.maxTokensReadOnly)

    // UI - Section 4: 操作栏
    private let testButton = NSButton(title: ModelPageStrings.buttonTest, target: nil, action: nil)
    private let saveButton = ThemePrimaryButton(title: ModelPageStrings.buttonSave)
    private let discardButton = NSButton(title: ModelPageStrings.buttonDiscard, target: nil, action: nil)

    // MARK: - Initializer

    public init(
        context: AnyObject? = nil,
        initialDraft: ModelPageDraft? = nil,
        probeRunner: ModelProbeRunnerProtocol = DefaultModelProbeRunner(),
        isCoreRunningHandler: (() -> Bool)? = nil,
        saveHandler: (([String: Any]) throws -> Void)? = nil
    ) {
        self.context = context
        self.probeRunner = probeRunner
        self.isCoreRunningHandler = isCoreRunningHandler
        self.saveHandler = saveHandler

        let draft: ModelPageDraft
        if let d = initialDraft {
            draft = d
        } else {
            draft = ModelPageDraft.fromConfig((context as? StatusPanelController)?.currentRawConfig ?? [:])
        }
        self.savedDraft = draft
        self.currentDraft = draft

        super.init(nibName: nil, bundle: nil)
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 760, height: 860))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        syncDraftToUI()
        validateAndRefreshUI()
    }

    // MARK: - UI Setup

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let docView = ModelPageFlippedView()
        docView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.documentView = docView
        view.addSubview(scrollView)

        let rootStack = NSStackView()
        rootStack.orientation = .vertical
        rootStack.alignment = .leading
        rootStack.spacing = 16
        rootStack.translatesAutoresizingMaskIntoConstraints = false
        docView.addSubview(rootStack)

        NSLayoutConstraint.activate([
            docView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            docView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            docView.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            docView.bottomAnchor.constraint(greaterThanOrEqualTo: rootStack.bottomAnchor, constant: 32),

            rootStack.topAnchor.constraint(equalTo: docView.topAnchor, constant: 20),
            rootStack.leadingAnchor.constraint(equalTo: docView.leadingAnchor, constant: 24),
            rootStack.trailingAnchor.constraint(equalTo: docView.trailingAnchor, constant: -24),
            rootStack.widthAnchor.constraint(equalTo: docView.widthAnchor, constant: -48)
        ])

        // 1. 顶部 Header
        let headerRow = NSStackView()
        headerRow.orientation = .horizontal
        headerRow.alignment = .firstBaseline
        headerRow.spacing = 10
        headerRow.translatesAutoresizingMaskIntoConstraints = false

        titleLabel.font = NSFont.systemFont(ofSize: 20, weight: .bold)
        scopeLabel.font = NSFont.systemFont(ofSize: 12)
        scopeLabel.textColor = NSColor.secondaryLabelColor

        headerRow.addArrangedSubview(titleLabel)
        headerRow.addArrangedSubview(scopeLabel)
        headerRow.addArrangedSubview(topStatusTag)
        headerRow.addArrangedSubview(topDefaultTag)
        rootStack.addArrangedSubview(headerRow)

        // 2. 横幅容器
        bannerContainer.orientation = .vertical
        bannerContainer.alignment = .leading
        bannerContainer.spacing = 10
        bannerContainer.translatesAutoresizingMaskIntoConstraints = false
        rootStack.addArrangedSubview(bannerContainer)
        bannerContainer.widthAnchor.constraint(equalTo: rootStack.widthAnchor).isActive = true

        // 3. Section 1: 接入方式
        let backendBox = createSectionBox(title: ModelPageStrings.sectionBackend)
        let backendContent = NSStackView()
        backendContent.orientation = .vertical
        backendContent.alignment = .leading
        backendContent.spacing = 8
        backendContent.translatesAutoresizingMaskIntoConstraints = false

        backendSegment.target = self
        backendSegment.action = #selector(handleBackendChanged(_:))
        backendDescLabel.font = NSFont.systemFont(ofSize: 12)
        backendDescLabel.textColor = NSColor.secondaryLabelColor

        backendContent.addArrangedSubview(backendSegment)
        backendContent.addArrangedSubview(backendDescLabel)
        backendBox.setContent(backendContent)
        rootStack.addArrangedSubview(backendBox)
        backendBox.widthAnchor.constraint(equalTo: rootStack.widthAnchor).isActive = true

        // 4. Section 2: 连接 (Connection Card)
        connectionBox.title = ModelPageStrings.sectionConnection
        connectionBox.titleFont = NSFont.systemFont(ofSize: 14, weight: .bold)
        let connContent = NSStackView()
        connContent.orientation = .vertical
        connContent.alignment = .leading
        connContent.spacing = 14
        connContent.translatesAutoresizingMaskIntoConstraints = false

        baseUrlField.translatesAutoresizingMaskIntoConstraints = false
        baseUrlField.input.textField.target = self
        baseUrlField.input.textField.action = #selector(handleFieldChanged)

        modelField.translatesAutoresizingMaskIntoConstraints = false
        modelField.input.textField.target = self
        modelField.input.textField.action = #selector(handleFieldChanged)

        // 密钥来源与输入
        let keyRow = NSStackView()
        keyRow.orientation = .vertical
        keyRow.alignment = .leading
        keyRow.spacing = 6
        keyRow.translatesAutoresizingMaskIntoConstraints = false

        keySourceSegment.target = self
        keySourceSegment.action = #selector(handleKeySourceChanged(_:))
        keyInputField.translatesAutoresizingMaskIntoConstraints = false
        keyInputField.input.textField.target = self
        keyInputField.input.textField.action = #selector(handleFieldChanged)

        keyRow.addArrangedSubview(keySourceSegment)
        keyRow.addArrangedSubview(keyInputField)
        keyInputField.widthAnchor.constraint(equalTo: keyRow.widthAnchor).isActive = true

        // 密钥状态行
        let keyStatusRow = NSStackView()
        keyStatusRow.orientation = .horizontal
        keyStatusRow.alignment = .centerY
        keyStatusRow.spacing = 8

        let keyStatusLabel = NSTextField(labelWithString: ModelPageStrings.fieldKeyStatus)
        keyStatusLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        keyStatusNoteLabel.font = NSFont.systemFont(ofSize: 11)
        keyStatusNoteLabel.textColor = NSColor.secondaryLabelColor

        keyStatusRow.addArrangedSubview(keyStatusLabel)
        keyStatusRow.addArrangedSubview(keyStatusTag)

        let keyStatusStack = NSStackView(views: [keyStatusRow, keyStatusNoteLabel])
        keyStatusStack.orientation = .vertical
        keyStatusStack.alignment = .leading
        keyStatusStack.spacing = 4

        connContent.addArrangedSubview(baseUrlField)
        connContent.addArrangedSubview(modelField)
        connContent.addArrangedSubview(keyRow)
        connContent.addArrangedSubview(keyStatusStack)

        baseUrlField.widthAnchor.constraint(equalTo: connContent.widthAnchor).isActive = true
        modelField.widthAnchor.constraint(equalTo: connContent.widthAnchor).isActive = true
        keyRow.widthAnchor.constraint(equalTo: connContent.widthAnchor).isActive = true

        connectionBox.setContent(connContent)
        rootStack.addArrangedSubview(connectionBox)
        connectionBox.widthAnchor.constraint(equalTo: rootStack.widthAnchor).isActive = true

        // 5. Section 3: 参数设置 (Parameters Card)
        parametersBox.title = ModelPageStrings.sectionParameters
        parametersBox.titleFont = NSFont.systemFont(ofSize: 14, weight: .bold)
        let paramContent = NSStackView()
        paramContent.orientation = .vertical
        paramContent.alignment = .leading
        paramContent.spacing = 14
        paramContent.translatesAutoresizingMaskIntoConstraints = false

        timeoutField.translatesAutoresizingMaskIntoConstraints = false
        timeoutField.input.textField.target = self
        timeoutField.input.textField.action = #selector(handleFieldChanged)

        // 图片设置
        supportsImagesCheck.target = self
        supportsImagesCheck.action = #selector(handleSupportsImagesChanged(_:))
        supportsImagesDesc.font = NSFont.systemFont(ofSize: 11)
        supportsImagesDesc.textColor = NSColor.secondaryLabelColor

        let imagesCheckStack = NSStackView(views: [supportsImagesCheck, supportsImagesDesc])
        imagesCheckStack.orientation = .vertical
        imagesCheckStack.alignment = .leading
        imagesCheckStack.spacing = 2

        imagesPerCallField.translatesAutoresizingMaskIntoConstraints = false
        imagesPerCallField.input.textField.target = self
        imagesPerCallField.input.textField.action = #selector(handleFieldChanged)

        // JSON 输出
        jsonOutputCheck.target = self
        jsonOutputCheck.action = #selector(handleJsonOutputChanged(_:))
        jsonOutputDesc.font = NSFont.systemFont(ofSize: 11)
        jsonOutputDesc.textColor = NSColor.secondaryLabelColor

        let jsonCheckStack = NSStackView(views: [jsonOutputCheck, jsonOutputDesc])
        jsonCheckStack.orientation = .vertical
        jsonCheckStack.alignment = .leading
        jsonCheckStack.spacing = 2

        // max_tokens 只读
        let maxTokRow = NSStackView()
        maxTokRow.orientation = .horizontal
        maxTokRow.alignment = .firstBaseline
        maxTokRow.spacing = 8
        maxTokensLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        maxTokensValueLabel.font = NSFont.systemFont(ofSize: 13)
        maxTokensValueLabel.textColor = NSColor.secondaryLabelColor
        maxTokRow.addArrangedSubview(maxTokensLabel)
        maxTokRow.addArrangedSubview(maxTokensValueLabel)

        paramContent.addArrangedSubview(timeoutField)
        paramContent.addArrangedSubview(imagesCheckStack)
        paramContent.addArrangedSubview(imagesPerCallField)
        paramContent.addArrangedSubview(jsonCheckStack)
        paramContent.addArrangedSubview(maxTokRow)

        timeoutField.widthAnchor.constraint(equalTo: paramContent.widthAnchor).isActive = true
        imagesPerCallField.widthAnchor.constraint(equalTo: paramContent.widthAnchor).isActive = true

        parametersBox.setContent(paramContent)
        rootStack.addArrangedSubview(parametersBox)
        parametersBox.widthAnchor.constraint(equalTo: rootStack.widthAnchor).isActive = true

        // 6. Section 4: 操作栏
        let actionRow = NSStackView()
        actionRow.orientation = .horizontal
        actionRow.alignment = .centerY
        actionRow.spacing = 12
        actionRow.translatesAutoresizingMaskIntoConstraints = false

        testButton.target = self
        testButton.action = #selector(handleTestConnection)
        testButton.bezelStyle = .rounded

        saveButton.target = self
        saveButton.action = #selector(handleSave)

        discardButton.target = self
        discardButton.action = #selector(handleDiscard)
        discardButton.bezelStyle = .rounded

        actionRow.addArrangedSubview(testButton)
        actionRow.addArrangedSubview(saveButton)
        actionRow.addArrangedSubview(discardButton)
        rootStack.addArrangedSubview(actionRow)

        // 注册 Notification 监听输入框实时变化
        NotificationCenter.default.addObserver(self, selector: #selector(textDidChangeNotification(_:)), name: NSControl.textDidChangeNotification, object: nil)
    }

    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 14, weight: .bold)
        return box
    }

    // MARK: - State & UI Sync

    private func syncDraftToUI() {
        let isCodex = currentDraft.backend == "codex-exec"
        backendSegment.selectedSegment = isCodex ? 0 : 1
        backendDescLabel.stringValue = isCodex ? ModelPageStrings.backendCodexDesc : ModelPageStrings.backendOpenAIDesc

        // 选 codex-exec 时隐藏三个字段与连接卡片
        connectionBox.isHidden = isCodex

        baseUrlField.input.stringValue = currentDraft.baseUrl
        modelField.input.stringValue = currentDraft.model

        let isEnv = currentDraft.keySource == "env"
        keySourceSegment.selectedSegment = isEnv ? 0 : 1
        if isEnv {
            keyInputField.label.stringValue = ModelPageStrings.fieldKeySource + " (" + ModelPageStrings.keySourceEnv + ")"
            keyInputField.input.placeholderString = "OPENAI_API_KEY"
            keyInputField.setNormal(helper: ModelPageStrings.keySourceEnvHelper)
            keyInputField.input.stringValue = currentDraft.apiKeyEnv
        } else {
            keyInputField.label.stringValue = ModelPageStrings.fieldKeySource + " (" + ModelPageStrings.keySourceFile + ")"
            keyInputField.input.placeholderString = "/Users/me/.wechat-mcp/model.key"
            keyInputField.setNormal(helper: ModelPageStrings.keySourceFileHelper)
            keyInputField.input.stringValue = currentDraft.apiKeyFile
        }

        timeoutField.input.stringValue = currentDraft.timeoutSeconds
        supportsImagesCheck.state = currentDraft.supportsImages ? .on : .off
        imagesPerCallField.input.stringValue = currentDraft.imagesPerModelCall
        imagesPerCallField.input.isEditable = currentDraft.supportsImages
        if !currentDraft.supportsImages {
            imagesPerCallField.setDisabled(helper: ModelPageStrings.imagesPerCallHelper)
        } else {
            imagesPerCallField.setNormal(helper: ModelPageStrings.imagesPerCallHelper)
        }

        jsonOutputCheck.state = currentDraft.jsonOutput ? .on : .off
    }

    private func syncUIToDraft() {
        let isCodex = backendSegment.selectedSegment == 0
        currentDraft.backend = isCodex ? "codex-exec" : "openai-compatible"

        currentDraft.baseUrl = baseUrlField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        currentDraft.model = modelField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)

        let isEnv = keySourceSegment.selectedSegment == 0
        currentDraft.keySource = isEnv ? "env" : "file"
        if isEnv {
            currentDraft.apiKeyEnv = keyInputField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            currentDraft.apiKeyFile = keyInputField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        currentDraft.timeoutSeconds = timeoutField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        currentDraft.supportsImages = supportsImagesCheck.state == .on
        currentDraft.imagesPerModelCall = imagesPerCallField.input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        currentDraft.jsonOutput = jsonOutputCheck.state == .on
    }

    public func validateAndRefreshUI() {
        syncUIToDraft()

        let errors = ModelPageValidator.validate(draft: currentDraft)
        let isUnconfigured = ModelPageValidator.isUnconfigured(draft: currentDraft)
        let isKeyOk = ModelPageValidator.isKeyConfigured(draft: currentDraft)
        let isCodex = currentDraft.backend == "codex-exec"
        let isDirty = currentDraft != savedDraft

        // 1. 刷新输入框错误态
        if isCodex {
            baseUrlField.setNormal(helper: ModelPageStrings.fieldBaseUrlHelper)
            modelField.setNormal()
            keyInputField.setNormal(helper: ModelPageStrings.keySourceEnvHelper)
        } else {
            if let err = errors["base_url"] {
                baseUrlField.setError(err)
            } else {
                baseUrlField.setNormal(helper: ModelPageStrings.fieldBaseUrlHelper)
            }

            if let err = errors["model"] {
                modelField.setError(err)
            } else {
                modelField.setNormal()
            }

            if currentDraft.keySource == "env" {
                if let err = errors["api_key_env"] {
                    keyInputField.setError(err)
                } else {
                    keyInputField.setNormal(helper: ModelPageStrings.keySourceEnvHelper)
                }
            } else {
                if let err = errors["api_key_file"] {
                    keyInputField.setError(err)
                } else {
                    keyInputField.setNormal(helper: ModelPageStrings.keySourceFileHelper)
                }
            }
        }

        if let err = errors["timeout_seconds"] {
            timeoutField.setError(err)
        } else {
            timeoutField.setNormal(helper: ModelPageStrings.timeoutHelper)
        }

        if currentDraft.supportsImages {
            if let err = errors["images_per_model_call"] {
                imagesPerCallField.setError(err)
            } else {
                imagesPerCallField.setNormal(helper: ModelPageStrings.imagesPerCallHelper)
            }
        } else {
            imagesPerCallField.setDisabled(helper: ModelPageStrings.imagesPerCallHelper)
        }

        // 2. 刷新密钥状态标签
        if isCodex {
            keyStatusTag.status = .normal
            keyStatusTag.customText = ModelPageStrings.keyStatusConfigured
            keyStatusNoteLabel.stringValue = ModelPageStrings.keyNeverExposed
        } else {
            if isKeyOk {
                keyStatusTag.status = .normal
                keyStatusTag.customText = ModelPageStrings.keyStatusConfigured
                keyStatusNoteLabel.stringValue = ModelPageStrings.keyNeverExposed
            } else {
                keyStatusTag.status = .warning
                keyStatusTag.customText = ModelPageStrings.keyStatusUnconfigured
                if currentDraft.keySource == "env" && errors["api_key_env"] == nil && !currentDraft.apiKeyEnv.isEmpty {
                    keyStatusNoteLabel.stringValue = ModelPageStrings.keyStatusEnvNotFound + " · " + ModelPageStrings.keyNeverExposed
                } else {
                    keyStatusNoteLabel.stringValue = ModelPageStrings.keyNeverExposed
                }
            }
        }

        // 3. 刷新顶部状态
        if isCodex {
            topStatusTag.status = .normal
            topStatusTag.customText = ModelPageStrings.statusConfigured
            topDefaultTag.isHidden = false
        } else {
            topDefaultTag.isHidden = true
            if isUnconfigured {
                topStatusTag.status = .warning
                topStatusTag.customText = ModelPageStrings.statusUnconfigured
            } else {
                topStatusTag.status = .normal
                topStatusTag.customText = ModelPageStrings.statusConfigured
            }
        }

        // 4. 刷新横幅
        updateBanners(errors: errors, isUnconfigured: isUnconfigured, isCodex: isCodex)

        // 5. 刷新按钮可用性
        discardButton.isEnabled = isDirty
        testButton.isEnabled = errors.isEmpty
    }

    private func updateBanners(errors: [String: String], isUnconfigured: Bool, isCodex: Bool) {
        // 清理旧横幅
        for view in activeBanners {
            view.removeFromSuperview()
        }
        activeBanners.removeAll()

        // 1. 重启后生效横幅 (pending restart)
        if hasPendingRestartBanner {
            let banner = ThemeStatusBanner(
                status: .warning,
                title: L10n.t(.modelPendingTitle),
                message: ModelPageStrings.bannerPendingRestart,
                actionTitle: ModelPageStrings.buttonRestartCore,
                actionTarget: self,
                actionSelector: #selector(handleRestartCore)
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        }

        // 2. 默认 codex 接入横幅
        if isCodex {
            let banner = ThemeStatusBanner(
                status: .normal,
                title: L10n.t(.modelDefaultTitle),
                message: ModelPageStrings.bannerDefaultCodex
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        }

        // 3. 未配置横幅 (未设兼容接口或缺少地址/模型/密钥)
        if isUnconfigured && !isCodex {
            let banner = ThemeStatusBanner(
                status: .warning,
                title: L10n.t(.modelMissingTitle),
                message: ModelPageStrings.bannerUnset
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        }

        // 4. 校验错误横幅
        if !errors.isEmpty {
            let banner = ThemeStatusBanner(
                status: .error,
                title: L10n.t(.modelValidationTitle),
                message: String(format: ModelPageStrings.bannerErrorSummary, errors.count)
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        }

        // 5. 测试连接结果横幅
        switch probeResult {
        case .idle:
            break
        case .success:
            let banner = ThemeStatusBanner(
                status: .normal,
                title: ModelPageStrings.testPassedTitle,
                message: ModelPageStrings.testPassedNote
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        case .failure:
            let banner = ThemeStatusBanner(
                status: .error,
                title: ModelPageStrings.testFailedTitle,
                message: ModelPageStrings.testFailedNote
            )
            bannerContainer.addArrangedSubview(banner)
            banner.widthAnchor.constraint(equalTo: bannerContainer.widthAnchor).isActive = true
            activeBanners.append(banner)
        }
    }

    // MARK: - Action Handlers

    @objc private func handleBackendChanged(_ sender: NSSegmentedControl) {
        probeResult = .idle
        validateAndRefreshUI()
        syncDraftToUI()
    }

    @objc private func handleKeySourceChanged(_ sender: NSSegmentedControl) {
        probeResult = .idle
        let isEnv = sender.selectedSegment == 0
        currentDraft.keySource = isEnv ? "env" : "file"
        validateAndRefreshUI()
        syncDraftToUI()
    }

    @objc private func handleSupportsImagesChanged(_ sender: NSButton) {
        probeResult = .idle
        currentDraft.supportsImages = sender.state == .on
        validateAndRefreshUI()
    }

    @objc private func handleJsonOutputChanged(_ sender: NSButton) {
        probeResult = .idle
        currentDraft.jsonOutput = sender.state == .on
        validateAndRefreshUI()
    }

    @objc private func handleFieldChanged() {
        probeResult = .idle
        validateAndRefreshUI()
    }

    @objc private func textDidChangeNotification(_ note: Notification) {
        probeResult = .idle
        validateAndRefreshUI()
    }

    @objc public func handleDiscard() {
        probeResult = .idle
        currentDraft = savedDraft
        syncDraftToUI()
        validateAndRefreshUI()
    }

    @objc public func handleTestConnection() {
        syncUIToDraft()
        let errors = ModelPageValidator.validate(draft: currentDraft)
        guard errors.isEmpty else {
            validateAndRefreshUI()
            return
        }

        testButton.isEnabled = false
        probeRunner.probe(config: currentDraft) { [weak self] result in
            guard let self = self else { return }
            self.testButton.isEnabled = true
            self.probeResult = result
            self.validateAndRefreshUI()
        }
    }

    @objc public func handleSave() {
        syncUIToDraft()
        let errors = ModelPageValidator.validate(draft: currentDraft)
        guard errors.isEmpty else {
            validateAndRefreshUI()
            return
        }

        let isRunning: Bool
        if let handler = isCoreRunningHandler {
            isRunning = handler()
        } else {
            isRunning = (context as? StatusPanelController)?.isBotRunning ?? false
        }

        let isDirty = currentDraft != savedDraft
        let changedNames = changedFieldDescriptions()

        // Bot 运行中修改任何模型字段并保存，都要弹二次确认（勾选才能确认），提示「重启后生效」
        if isRunning && isDirty {
            let namesStr = changedNames.joined(separator: "、")
            let body = String(format: ModelPageStrings.dialogConfirmBody, namesStr)
            let confirmed = ThemeConfirmDialog.show(
                title: ModelPageStrings.dialogConfirmTitle,
                message: body,
                checkboxText: ModelPageStrings.dialogConfirmCheckbox,
                confirmTitle: ModelPageStrings.dialogConfirmButton,
                cancelTitle: ModelPageStrings.dialogCancelButton
            )
            guard confirmed else {
                return
            }
        }

        do {
            try performSave()
            savedDraft = currentDraft
            hasPendingRestartBanner = isRunning && isDirty
            probeResult = .idle
            validateAndRefreshUI()
        } catch {
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = L10n.t(.saveFailed)
            alert.informativeText = L10n.t(.saveRejected)
            alert.runModal()
        }
    }

    private func performSave() throws {
        let formDict = currentDraft.toModelConfigForm()
        if let handler = saveHandler {
            try handler(formDict)
        } else if let context = context as? StatusPanelController {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: context.currentRawConfig, formInputs: ["model_config_form": formDict])
            try context.persistConfig(updated)
        } else {
            throw ConfigFormError.writeFailed(L10n.t(.configUnavailable))
        }
    }

    @objc private func handleRestartCore() {
        let alert = NSAlert()
        alert.messageText = L10n.t(.confirmRestartBotTitle)
        alert.informativeText = L10n.t(.confirmRestartBotBody)
        alert.addButton(withTitle: L10n.t(.confirmRestart))
        alert.addButton(withTitle: L10n.t(.cancel))
        if alert.runModal() == .alertFirstButtonReturn {
            (context as? StatusPanelController)?.restartBot()
        }
    }

    private func changedFieldDescriptions() -> [String] {
        var names: [String] = []
        if currentDraft.backend != savedDraft.backend { names.append(ModelPageStrings.sectionBackend) }
        if currentDraft.baseUrl != savedDraft.baseUrl { names.append(ModelPageStrings.fieldBaseUrl) }
        if currentDraft.model != savedDraft.model { names.append(ModelPageStrings.fieldModel) }
        if currentDraft.keySource != savedDraft.keySource ||
           currentDraft.apiKeyEnv != savedDraft.apiKeyEnv ||
           currentDraft.apiKeyFile != savedDraft.apiKeyFile {
            names.append(ModelPageStrings.fieldKeySource)
        }
        if currentDraft.timeoutSeconds != savedDraft.timeoutSeconds { names.append(ModelPageStrings.fieldTimeout) }
        if currentDraft.supportsImages != savedDraft.supportsImages { names.append(ModelPageStrings.fieldSupportsImages) }
        if currentDraft.imagesPerModelCall != savedDraft.imagesPerModelCall { names.append(ModelPageStrings.fieldImagesPerCall) }
        if currentDraft.jsonOutput != savedDraft.jsonOutput { names.append(ModelPageStrings.fieldJsonOutput) }
        return names
    }

    // MARK: - Inspection Helpers (For Selftests and Automated Contract Tests)

    public var isConnectionCardHidden: Bool {
        return connectionBox.isHidden
    }

    public var isTopStatusConfigured: Bool {
        return topStatusTag.status == .normal
    }

    public var isTopStatusUnconfigured: Bool {
        return topStatusTag.status == .warning
    }

    public var isKeyStatusConfigured: Bool {
        return keyStatusTag.status == .normal
    }

    public var isKeyStatusUnconfigured: Bool {
        return keyStatusTag.status == .warning
    }

    public var renderedBannerCount: Int {
        return activeBanners.count
    }
}
