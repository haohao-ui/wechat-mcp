// SettingsPage.swift: Host application settings view controller and form validation.
import Foundation
import AppKit

public enum SettingsValidationError: LocalizedError, Equatable {
    case message(String)
    public var errorDescription: String? {
        switch self {
        case .message(let msg): return msg
        }
    }
}

public struct SettingsFormLogic {
    public static let ranges: [String: (ClosedRange<Int>, String, String)] = [
        "cooldown_seconds": (0...300, "冷却时间", "秒"),
        "max_replies_per_hour": (1...100, "每小时回复上限", "条"),
        "turn_quiet_seconds": (0...120, "静默等待时间", "秒"),
        "turn_max_seconds": (10...600, "一轮最长耗时", "秒"),
        "context_messages": (0...100, "上下文消息数", "条"),
        "max_attachment_mb": (1...200, "附件大小上限", "MB")
    ]

    public static func validateAndSave(
        original: [String: Any],
        formInputs: [String: Any]
    ) -> Result<[String: Any], SettingsValidationError> {
        // 范围验证
        for (field, (range, label, unit)) in ranges {
            if let val = formInputs[field] {
                let intVal: Int
                if let str = val as? String {
                    let trimmed = str.trimmingCharacters(in: .whitespacesAndNewlines)
                    if trimmed.isEmpty { continue }
                    guard let parsed = Int(trimmed) else {
                        return .failure(.message("\(label) (\(field)) 必须为有效整数"))
                    }
                    intVal = parsed
                } else if let num = val as? Int {
                    intVal = num
                } else {
                    return .failure(.message("\(label) (\(field)) 类型不正确"))
                }

                if !range.contains(intVal) {
                    return .failure(.message("\(label) (\(field)) 超出范围 (\(range.lowerBound)-\(range.upperBound) \(unit))"))
                }
            }
        }

        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: original, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(.message(error.localizedDescription))
        }
    }
}

public final class SettingsPageController: NSViewController {
    private let context: StatusPanelController

    // UI Fields
    private let accountLabel = NSTextField(labelWithString: "-")
    private let backendLabel = NSTextField(labelWithString: "-")
    private let modelLabel = NSTextField(labelWithString: "-")
    private let keyStatusLabel = NSTextField(labelWithString: "-")

    private let cooldownField = NSTextField()
    private let maxRepliesField = NSTextField()
    private let quietSecField = NSTextField()
    private let maxSecField = NSTextField()
    private let contextMsgField = NSTextField()
    private let maxAttachmentField = NSTextField()
    private let manageWindowCheck = NSButton(checkboxWithTitle: "自动管理微信主窗口 (调整至 880x640)", target: nil, action: nil)

    private let saveButton = NSButton(title: "保存配置", target: nil, action: nil)
    private let restoreDefaultsButton = NSButton(title: "恢复默认值", target: nil, action: nil)
    private let statusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // Permission rows
    private var permissionViews: [PermissionKind: (status: NSTextField, reqBtn: NSButton, setBtn: NSButton)] = [:]

    public init(context: StatusPanelController) {
        self.context = context
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 700, height: 720))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
        populateCurrentValues()
        updatePermissionStatuses()
    }

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let contentStack = NSStackView()
        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = 18
        contentStack.translatesAutoresizingMaskIntoConstraints = false

        // 1. 系统权限
        let permBox = createSectionBox(title: "系统权限")
        let permStack = NSStackView()
        permStack.orientation = .vertical
        permStack.alignment = .leading
        permStack.spacing = 8
        permStack.translatesAutoresizingMaskIntoConstraints = false

        let perms: [(PermissionKind, String)] = [
            (.screen, "屏幕录制权限（微信窗口识别）"),
            (.accessibility, "辅助功能权限（窗口控制与点击）"),
            (.desktop, "桌面文件夹（附件与临时文件）"),
            (.documents, "文稿文件夹（附件读写）"),
            (.downloads, "下载文件夹（附件读写）")
        ]

        for (kind, title) in perms {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .centerY
            row.spacing = 10

            let nameLbl = NSTextField(labelWithString: title)
            nameLbl.font = NSFont.systemFont(ofSize: 12)
            nameLbl.widthAnchor.constraint(equalToConstant: 260).isActive = true

            let stLbl = NSTextField(labelWithString: "检查中...")
            stLbl.font = NSFont.systemFont(ofSize: 12, weight: .medium)
            stLbl.widthAnchor.constraint(equalToConstant: 70).isActive = true

            let reqBtn = NSButton(title: "请求权限", target: self, action: #selector(handleRequestPermission(_:)))
            reqBtn.identifier = NSUserInterfaceItemIdentifier(kind.rawValue)

            let setBtn = NSButton(title: "打开系统设置", target: self, action: #selector(handleOpenSettings(_:)))
            setBtn.identifier = NSUserInterfaceItemIdentifier(kind.rawValue)

            row.addArrangedSubview(nameLbl)
            row.addArrangedSubview(stLbl)
            row.addArrangedSubview(reqBtn)
            row.addArrangedSubview(setBtn)

            permStack.addArrangedSubview(row)
            permissionViews[kind] = (stLbl, reqBtn, setBtn)
        }

        let recheckBtn = NSButton(title: "重新检查权限", target: self, action: #selector(handleRecheckPermissions))
        permStack.addArrangedSubview(recheckBtn)

        permBox.setContent(permStack)
        contentStack.addArrangedSubview(permBox)

        // 2. 账号与模型
        let accountBox = createSectionBox(title: "账号与模型")
        let accGrid = NSGridView(views: [
            [NSTextField(labelWithString: "微信账号:"), accountLabel],
            [NSTextField(labelWithString: "模型接口:"), backendLabel],
            [NSTextField(labelWithString: "模型名称:"), modelLabel],
            [NSTextField(labelWithString: "API 密钥:"), keyStatusLabel]
        ])
        accGrid.translatesAutoresizingMaskIntoConstraints = false
        accGrid.rowSpacing = 6
        accGrid.columnSpacing = 12
        accountBox.setContent(accGrid)
        contentStack.addArrangedSubview(accountBox)

        // 3. 回复节奏与附件配置
        let paceBox = createSectionBox(title: "回复节奏与运行参数")
        let paceGrid = NSGridView(views: [
            [NSTextField(labelWithString: "冷却时间 (cooldown_seconds):"), cooldownField, NSTextField(labelWithString: "0–300 秒 (默认 15)")],
            [NSTextField(labelWithString: "每小时上限 (max_replies_per_hour):"), maxRepliesField, NSTextField(labelWithString: "1–100 条 (默认 20)")],
            [NSTextField(labelWithString: "静默等待 (turn_quiet_seconds):"), quietSecField, NSTextField(labelWithString: "0–120 秒 (默认 10)")],
            [NSTextField(labelWithString: "单轮最长 (turn_max_seconds):"), maxSecField, NSTextField(labelWithString: "10–600 秒 (默认 60)")],
            [NSTextField(labelWithString: "上下文条数 (context_messages):"), contextMsgField, NSTextField(labelWithString: "0–100 条 (默认 5)")],
            [NSTextField(labelWithString: "附件上限 (max_attachment_mb):"), maxAttachmentField, NSTextField(labelWithString: "1–200 MB (默认 50)")]
        ])
        paceGrid.translatesAutoresizingMaskIntoConstraints = false
        paceGrid.rowSpacing = 6
        paceGrid.columnSpacing = 12

        for f in [cooldownField, maxRepliesField, quietSecField, maxSecField, contextMsgField, maxAttachmentField] {
            f.widthAnchor.constraint(equalToConstant: 100).isActive = true
        }

        let paceStack = NSStackView()
        paceStack.orientation = .vertical
        paceStack.alignment = .leading
        paceStack.spacing = 10
        paceStack.addArrangedSubview(paceGrid)
        paceStack.addArrangedSubview(manageWindowCheck)

        let noteLbl = NSTextField(wrappingLabelWithString: "注：允许名单与忽略名单已移至「联系人」页；外部大脑配置已移至「MCP」页。")
        noteLbl.font = NSFont.systemFont(ofSize: 11)
        noteLbl.textColor = .secondaryLabelColor
        paceStack.addArrangedSubview(noteLbl)

        paceBox.setContent(paceStack)
        contentStack.addArrangedSubview(paceBox)

        // 4. 底部操作按钮
        let btnRow = NSStackView()
        btnRow.orientation = .horizontal
        btnRow.spacing = 12
        saveButton.target = self
        saveButton.action = #selector(handleSaveConfig)
        restoreDefaultsButton.target = self
        restoreDefaultsButton.action = #selector(handleRestoreDefaults)

        btnRow.addArrangedSubview(saveButton)
        btnRow.addArrangedSubview(restoreDefaultsButton)
        btnRow.addArrangedSubview(statusMessageLabel)
        contentStack.addArrangedSubview(btnRow)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 20),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -20),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 16),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -16),
            permBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            accountBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            paceBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
        ])
    }

    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.titleFont = NSFont.systemFont(ofSize: 13, weight: .bold)
        box.boxType = .primary
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    public func populateCurrentValues() {
        let masked = ConfigFormLogic.maskConfig(context.currentRawConfig)
        accountLabel.stringValue = masked["account_id"] as? String ?? "-"
        backendLabel.stringValue = masked["model_backend"] as? String ?? "-"
        modelLabel.stringValue = masked["model_name"] as? String ?? "-"
        keyStatusLabel.stringValue = masked["api_key_status"] as? String ?? "-"

        let editable = masked["editable"] as? [String: Any] ?? [:]
        if let cd = editable["cooldown_seconds"] { cooldownField.stringValue = "\(cd)" } else { cooldownField.stringValue = "15" }
        if let mr = editable["max_replies_per_hour"] { maxRepliesField.stringValue = "\(mr)" } else { maxRepliesField.stringValue = "20" }
        if let tq = editable["turn_quiet_seconds"] { quietSecField.stringValue = "\(tq)" } else { quietSecField.stringValue = "10" }
        if let tm = editable["turn_max_seconds"] { maxSecField.stringValue = "\(tm)" } else { maxSecField.stringValue = "60" }
        if let cm = editable["context_messages"] { contextMsgField.stringValue = "\(cm)" } else { contextMsgField.stringValue = "5" }
        if let att = editable["max_attachment_mb"] { maxAttachmentField.stringValue = "\(att)" } else { maxAttachmentField.stringValue = "50" }
        let mw = (editable["manage_window"] as? Bool) ?? true
        manageWindowCheck.state = mw ? .on : .off
    }

    public func updatePermissionStatuses() {
        for kind in PermissionKind.allCases {
            guard let views = permissionViews[kind] else { continue }
            let st = permissionStatus(kind)
            switch st {
            case .some(true):
                views.status.stringValue = "已授权"
                views.status.textColor = .systemGreen
                views.reqBtn.isEnabled = false
            case .some(false):
                views.status.stringValue = "未授权"
                views.status.textColor = .systemRed
                views.reqBtn.isEnabled = true
            case .none:
                views.status.stringValue = "未知"
                views.status.textColor = .secondaryLabelColor
                views.reqBtn.isEnabled = true
            }
        }
    }

    @objc private func handleRequestPermission(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue, let kind = PermissionKind(rawValue: id) else { return }
        requestPermission(kind)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            self?.updatePermissionStatuses()
        }
    }

    @objc private func handleOpenSettings(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue, let kind = PermissionKind(rawValue: id) else { return }
        openSystemSettings(for: kind)
    }

    @objc private func handleRecheckPermissions() {
        updatePermissionStatuses()
    }

    @objc private func handleSaveConfig() {
        var formInputs: [String: Any] = [:]
        formInputs["cooldown_seconds"] = cooldownField.stringValue
        formInputs["max_replies_per_hour"] = maxRepliesField.stringValue
        formInputs["turn_quiet_seconds"] = quietSecField.stringValue
        formInputs["turn_max_seconds"] = maxSecField.stringValue
        formInputs["context_messages"] = contextMsgField.stringValue
        formInputs["max_attachment_mb"] = maxAttachmentField.stringValue
        formInputs["manage_window"] = (manageWindowCheck.state == .on)

        let result = SettingsFormLogic.validateAndSave(original: context.currentRawConfig, formInputs: formInputs)
        switch result {
        case .success(let updated):
            do {
                try context.persistConfig(updated)
                statusMessageLabel.stringValue = "配置保存成功"
                statusMessageLabel.textColor = .systemGreen
            } catch {
                statusMessageLabel.stringValue = "保存失败: \(error.localizedDescription)"
                statusMessageLabel.textColor = .systemRed
            }
        case .failure(let err):
            statusMessageLabel.stringValue = "校验错误: \(err)"
            statusMessageLabel.textColor = .systemRed
        }
    }

    @objc private func handleRestoreDefaults() {
        cooldownField.stringValue = "15"
        maxRepliesField.stringValue = "20"
        quietSecField.stringValue = "10"
        maxSecField.stringValue = "60"
        contextMsgField.stringValue = "5"
        maxAttachmentField.stringValue = "50"
        manageWindowCheck.state = .on
        statusMessageLabel.stringValue = "已载入默认值，点击「保存配置」后生效"
        statusMessageLabel.textColor = .systemBlue
    }
}
