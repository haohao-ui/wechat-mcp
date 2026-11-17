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
    public static var ranges: [String: (ClosedRange<Int>, String, String)] { [
        "cooldown_seconds": (0...300, L10n.t(.fieldCooldown), L10n.t(.unitSeconds)),
        "max_replies_per_hour": (1...100, L10n.t(.fieldMaxReplies), L10n.t(.unitMessages)),
        "turn_quiet_seconds": (0...120, L10n.t(.fieldQuiet), L10n.t(.unitSeconds)),
        "turn_max_seconds": (10...600, L10n.t(.fieldTurnMax), L10n.t(.unitSeconds)),
        "context_messages": (0...100, L10n.t(.fieldContext), L10n.t(.unitMessages)),
        "max_attachment_mb": (1...200, L10n.t(.fieldAttachment), L10n.t(.unitMB))
    ] }

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
                        return .failure(.message(L10n.t(.errNotInteger, label, field)))
                    }
                    intVal = parsed
                } else if let num = val as? Int {
                    intVal = num
                } else {
                    return .failure(.message(L10n.t(.errWrongType, label, field)))
                }

                if !range.contains(intVal) {
                    return .failure(.message(L10n.t(.errOutOfRange, label, field, range.lowerBound, range.upperBound, unit)))
                }
            }
        }

        do {
            let updated = try ConfigFormLogic.buildUpdatedConfig(original: original, formInputs: formInputs)
            return .success(updated)
        } catch {
            return .failure(.message(L10n.t(.validationError, L10n.t(.valueUnavailable))))
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
    private let manageWindowCheck = NSButton(checkboxWithTitle: L10n.t(.manageWindow), target: nil, action: nil)

    private let saveButton = NSButton(title: L10n.t(.actionSaveConfig), target: nil, action: nil)
    private let restoreDefaultsButton = NSButton(title: L10n.t(.actionRestoreDefaults), target: nil, action: nil)
    private let statusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // Permission rows
    private let connectionLabel = NSTextField(wrappingLabelWithString: "")
    private var statusObserver: NSObjectProtocol?
    private let languagePicker = NSPopUpButton()


    // Update navigation
    public var onNavigateToUpdate: (() -> Void)?
    private let jumpToUpdateButton = NSButton(title: L10n.t(.actionGotoUpdate), target: nil, action: nil)

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
        _ = context.collectLatestStatus()
        updatePermissionStatuses()
        statusObserver = NotificationCenter.default.addObserver(forName: StatusPanelController.statusDidChange, object: context, queue: .main) { [weak self] _ in self?.updatePermissionStatuses() }
    }

    deinit { if let observer = statusObserver { NotificationCenter.default.removeObserver(observer) } }

    @objc private func changeLanguage() {
        let index = languagePicker.indexOfSelectedItem
        guard AppLanguage.allCases.indices.contains(index) else { return }
        L10n.setLanguage(AppLanguage.allCases[index])
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

        let languageBox = createSectionBox(title: L10n.t(.settingsLanguageBox))
        languagePicker.addItems(withTitles: AppLanguage.allCases.map { $0.displayName })
        languagePicker.selectItem(at: AppLanguage.allCases.firstIndex(of: L10n.language) ?? 0)
        languagePicker.target = self
        languagePicker.action = #selector(changeLanguage)
        let languageStack = NSStackView(views: [languagePicker, NSTextField(wrappingLabelWithString: L10n.t(.languageNote))])
        languageStack.orientation = .vertical
        languageStack.alignment = .leading
        languageBox.setContent(languageStack)
        contentStack.addArrangedSubview(languageBox)
        languageBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        // 1. 系统权限
        let permBox = createSectionBox(title: L10n.t(.settingsPermBox))
        let permStack = NSStackView()
        permStack.orientation = .vertical
        permStack.alignment = .leading
        permStack.spacing = 8
        permStack.translatesAutoresizingMaskIntoConstraints = false

        permStack.addArrangedSubview(connectionLabel)
        for (kind, key) in [(PermissionKind.screen, L10nKey.permScreen), (.accessibility, .permAccessibility), (.desktop, .permFolders)] {
            let button = NSButton(title: L10n.t(key), target: self, action: #selector(handleRequestPermission(_:)))
            button.identifier = NSUserInterfaceItemIdentifier(kind.rawValue)
            permStack.addArrangedSubview(button)
        }
        permStack.addArrangedSubview(NSButton(title: L10n.t(.actionOpenSystemSettings), target: self, action: #selector(handleOpenSettings(_:))))
        permStack.addArrangedSubview(NSButton(title: L10n.t(.actionRecheck), target: self, action: #selector(handleRecheckPermissions)))

        permBox.setContent(permStack)
        contentStack.addArrangedSubview(permBox)

        // 2. 账号与模型
        let accountBox = createSectionBox(title: L10n.t(.settingsAccountBox))
        let accGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.labelAccount)), accountLabel],
            [NSTextField(labelWithString: L10n.t(.labelBackend)), backendLabel],
            [NSTextField(labelWithString: L10n.t(.labelModelName)), modelLabel],
            [NSTextField(labelWithString: L10n.t(.labelApiKey)), keyStatusLabel]
        ])
        accGrid.translatesAutoresizingMaskIntoConstraints = false
        accGrid.rowSpacing = 6
        accGrid.columnSpacing = 12
        let accountStack = NSStackView(views: [accGrid, NSTextField(wrappingLabelWithString: L10n.t(.modelGlobalNote))])
        accountStack.orientation = .vertical
        accountStack.alignment = .leading
        accountBox.setContent(accountStack)
        contentStack.addArrangedSubview(accountBox)

        // 3. 回复节奏与附件配置
        let paceBox = createSectionBox(title: L10n.t(.paceBox))
        let paceGrid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.t(.fieldCooldown)), cooldownField, NSTextField(labelWithString: L10n.t(.rangeHint, 0, 300, L10n.t(.unitSeconds), 15))],
            [NSTextField(labelWithString: L10n.t(.fieldMaxReplies)), maxRepliesField, NSTextField(labelWithString: L10n.t(.rangeHint, 1, 100, L10n.t(.unitMessages), 20))],
            [NSTextField(labelWithString: L10n.t(.fieldQuiet)), quietSecField, NSTextField(labelWithString: L10n.t(.rangeHint, 0, 120, L10n.t(.unitSeconds), 10))],
            [NSTextField(labelWithString: L10n.t(.fieldTurnMax)), maxSecField, NSTextField(labelWithString: L10n.t(.rangeHint, 10, 600, L10n.t(.unitSeconds), 60))],
            [NSTextField(labelWithString: L10n.t(.fieldContext)), contextMsgField, NSTextField(labelWithString: L10n.t(.rangeHint, 0, 100, L10n.t(.unitMessages), 5))],
            [NSTextField(labelWithString: L10n.t(.fieldAttachment)), maxAttachmentField, NSTextField(labelWithString: L10n.t(.rangeHint, 1, 200, L10n.t(.unitMB), 50))]
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

        let noteLbl = NSTextField(wrappingLabelWithString: L10n.t(.settingsMovedNote, L10n.replyModesPair))
        noteLbl.font = NSFont.systemFont(ofSize: 11)
        noteLbl.textColor = .secondaryLabelColor
        paceStack.addArrangedSubview(noteLbl)

        paceBox.setContent(paceStack)
        contentStack.addArrangedSubview(paceBox)

        // 4. 应用版本与更新导航
        let updateBox = createSectionBox(title: L10n.t(.updateBox))
        let updateStack = NSStackView()
        updateStack.orientation = .vertical
        updateStack.alignment = .leading
        updateStack.spacing = 8

        let verRow = NSStackView()
        verRow.orientation = .horizontal
        verRow.spacing = 8
        let verTitle = NSTextField(labelWithString: L10n.t(.hostVersion))
        verTitle.font = NSFont.systemFont(ofSize: 12)
        let verVal = NSTextField(labelWithString: hostAppVersion())
        verVal.font = NSFont.systemFont(ofSize: 12, weight: .semibold)
        verRow.addArrangedSubview(verTitle)
        verRow.addArrangedSubview(verVal)
        updateStack.addArrangedSubview(verRow)

        let updateIntro = NSTextField(wrappingLabelWithString: L10n.t(.updateIntro))
        updateIntro.font = NSFont.systemFont(ofSize: 11)
        updateIntro.textColor = .secondaryLabelColor
        updateStack.addArrangedSubview(updateIntro)

        jumpToUpdateButton.target = self
        jumpToUpdateButton.action = #selector(handleJumpToUpdate)
        updateStack.addArrangedSubview(jumpToUpdateButton)

        updateBox.setContent(updateStack)
        contentStack.addArrangedSubview(updateBox)

        // 5. 底部操作按钮
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
            paceBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            updateBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor)
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
        keyStatusLabel.stringValue = L10n.t((masked["api_key_status"] as? String) == L10n.text(.modelConfigured, in: .zh) ? .modelConfigured : .modelUnconfigured)

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
        guard let summary = context.statusSummary else { return }
        let screen = L10n.t(summary.screenRecordingOK ? .statusOK : .statusMissing)
        let accessibility = L10n.t(summary.accessibilityOK ? .statusOK : .statusMissing)
        let folder = summary.folderBlockedDetail == nil ? L10n.t(.folderNoProblem) : L10n.t(.statusBlocked)
        connectionLabel.stringValue = [summary.cardText, L10n.t(.permScreen) + ": " + screen,
            L10n.t(.permAccessibility) + ": " + accessibility, L10n.t(.permFolders) + ": " + folder,
            L10n.t(.navModel) + ": " + summary.modelState.label].joined(separator: "\n")
    }

    @objc private func handleRequestPermission(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue, let kind = PermissionKind(rawValue: id) else { return }
        requestPermission(kind)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            self?.context.refresh()
        }
    }

    @objc private func handleOpenSettings(_ sender: NSButton) {
        let kind = PermissionKind(rawValue: sender.identifier?.rawValue ?? "") ?? .screen
        openSystemSettings(for: kind)
    }

    @objc private func handleRecheckPermissions() {
        context.refresh()
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
                statusMessageLabel.stringValue = L10n.t(.savedNeedsRestart)
                statusMessageLabel.textColor = .systemGreen
            } catch {
                statusMessageLabel.stringValue = L10n.t(.saveFailed, L10n.t(.valueUnavailable))
                statusMessageLabel.textColor = .systemRed
            }
        case .failure(let err):
            statusMessageLabel.stringValue = L10n.t(.validationError, err.localizedDescription)
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
        statusMessageLabel.stringValue = L10n.t(.defaultsLoaded)
        statusMessageLabel.textColor = .systemBlue
    }

    public func triggerUpdateNavigation() {
        onNavigateToUpdate?()
    }

    @objc private func handleJumpToUpdate() {
        triggerUpdateNavigation()
    }
}
