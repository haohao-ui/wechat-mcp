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
        "cooldown_seconds": (0...300, L10n.t(.fieldCooldown), L10n.t(.unitSeconds)),
        "max_replies_per_hour": (1...100, L10n.t(.fieldMaxReplies), L10n.t(.unitMessages)),
        "turn_quiet_seconds": (0...120, L10n.t(.fieldQuiet), L10n.t(.unitSeconds)),
        "turn_max_seconds": (10...600, L10n.t(.fieldTurnMax), L10n.t(.unitSeconds)),
        "context_messages": (0...100, L10n.t(.fieldContext), L10n.t(.unitMessages)),
        "max_attachment_mb": (1...200, L10n.t(.fieldAttachment), "MB")
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
                        return .failure(.message(L10n.t(.uiSettings021, String(describing: label), String(describing: field))))
                    }
                    intVal = parsed
                } else if let num = val as? Int {
                    intVal = num
                } else {
                    return .failure(.message(L10n.t(.uiSettings022, String(describing: label), String(describing: field))))
                }

                if !range.contains(intVal) {
                    return .failure(.message(L10n.t(.uiSettings023, String(describing: label), String(describing: field), String(describing: range.lowerBound), String(describing: range.upperBound), String(describing: unit))))
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

    /// 统一的字段标签列宽（设计稿：标签左、控件右，标签列 180）。
    private static let labelColumnWidth: CGFloat = 180
    private static let cardInset: CGFloat = 20

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

    private let saveButton = ThemePrimaryButton(title: L10n.t(.actionSaveConfig), target: nil, action: nil)
    private let restoreDefaultsButton = NSButton(title: L10n.t(.actionRestoreDefaults), target: nil, action: nil)
    private let statusMessageLabel = NSTextField(wrappingLabelWithString: "")

    // Permission rows
    private var permissionViews: [PermissionKind: (status: NSTextField, reqBtn: NSButton, setBtn: NSButton)] = [:]

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
        updatePermissionStatuses()
    }

    private let languagePopup = NSPopUpButton()

    // MARK: - Shared visual helpers (令牌全部来自 Theme.Colors)

    private func makePrimaryLabel(_ text: String, size: CGFloat = 13) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: size)
        label.textColor = Theme.Colors.textPrimary
        label.lineBreakMode = .byTruncatingTail
        return label
    }

    private func makeFieldLabel(_ text: String) -> NSTextField {
        let label = makePrimaryLabel(text)
        label.setContentHuggingPriority(.required, for: .horizontal)
        label.setContentCompressionResistancePriority(.required, for: .horizontal)
        label.widthAnchor.constraint(equalToConstant: Self.labelColumnWidth).isActive = true
        return label
    }

    private func makeValueLabel(_ text: String) -> NSTextField {
        let label = makePrimaryLabel(text)
        label.setContentHuggingPriority(.defaultLow, for: .horizontal)
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    private func styleValueLabel(_ label: NSTextField) {
        label.font = NSFont.systemFont(ofSize: 13)
        label.textColor = Theme.Colors.textPrimary
        label.lineBreakMode = .byTruncatingTail
        label.setContentHuggingPriority(.defaultLow, for: .horizontal)
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }

    private func makeHintLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = Theme.Colors.textSecondary
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    private func makeWrappingHintLabel(_ text: String) -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: text)
        label.font = NSFont.systemFont(ofSize: 12)
        label.textColor = Theme.Colors.textSecondary
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    private func styleSecondaryButton(_ button: NSButton) {
        button.bezelStyle = .rounded
        button.controlSize = .regular
        button.font = NSFont.systemFont(ofSize: 13)
        button.setContentHuggingPriority(.required, for: .horizontal)
    }

    private func makeSpacer() -> NSView {
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        spacer.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return spacer
    }

    private func makeRow(_ views: [NSView], spacing: CGFloat = 12) -> NSStackView {
        let row = NSStackView(views: views)
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = spacing
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }

    private func makeVerticalStack(spacing: CGFloat = 12) -> NSStackView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = spacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        return stack
    }

    /// 白卡片：标题 17 semibold + 内边距 20（NSBox.setContent 由 Shell 统一提供白底/圆角/描边）。
    private func createSectionBox(title: String) -> NSBox {
        let box = NSBox()
        box.title = title
        box.boxType = .primary
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    private func addFullWidthRow(_ row: NSView, to stack: NSStackView) {
        stack.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }

    private func makeDivider() -> NSView {
        let divider = NSView()
        divider.wantsLayer = true
        divider.layer?.backgroundColor = Theme.Colors.cardBorder.cgColor
        divider.translatesAutoresizingMaskIntoConstraints = false
        divider.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return divider
    }

    private func makeStyleField(_ field: NSTextField) {
        field.font = NSFont.systemFont(ofSize: 13)
        field.alignment = .center
        field.textColor = Theme.Colors.textPrimary
        field.widthAnchor.constraint(equalToConstant: 100).isActive = true
    }

    // MARK: - Layout

    private func setupUI() {
        view.wantsLayer = true
        view.layer?.backgroundColor = Theme.Colors.pageBackground.cgColor

        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        let contentStack = makeVerticalStack(spacing: 20)

        // 1. 系统权限
        let permBox = createSectionBox(title: L10n.t(.uiSettings047))
        let permStack = makeVerticalStack(spacing: 12)

        let perms: [(PermissionKind, String)] = [
            (.screen, L10n.t(.uiSettings024)),
            (.accessibility, L10n.t(.uiSettings025)),
            (.desktop, L10n.t(.uiSettings026)),
            (.documents, L10n.t(.uiSettings027)),
            (.downloads, L10n.t(.uiSettings028))
        ]

        for (kind, title) in perms {
            let nameLbl = makePrimaryLabel(title)
            nameLbl.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

            let stLbl = NSTextField(labelWithString: L10n.t(.statusChecking))
            stLbl.font = NSFont.systemFont(ofSize: 12, weight: .medium)
            stLbl.textColor = Theme.Colors.textSecondary
            stLbl.alignment = .left
            stLbl.widthAnchor.constraint(equalToConstant: 96).isActive = true
            stLbl.setContentHuggingPriority(.required, for: .horizontal)

            let reqBtn = NSButton(title: L10n.t(.uiSettings029), target: self, action: #selector(handleRequestPermission(_:)))
            reqBtn.identifier = NSUserInterfaceItemIdentifier(kind.rawValue)
            styleSecondaryButton(reqBtn)

            let setBtn = NSButton(title: L10n.t(.actionOpenSystemSettings), target: self, action: #selector(handleOpenSettings(_:)))
            setBtn.identifier = NSUserInterfaceItemIdentifier(kind.rawValue)
            styleSecondaryButton(setBtn)

            let row = makeRow([nameLbl, stLbl, makeSpacer(), reqBtn, setBtn])
            addFullWidthRow(row, to: permStack)
            permissionViews[kind] = (stLbl, reqBtn, setBtn)
        }

        let recheckBtn = NSButton(title: L10n.t(.uiSettings030), target: self, action: #selector(handleRecheckPermissions))
        styleSecondaryButton(recheckBtn)
        addFullWidthRow(makeRow([recheckBtn, makeSpacer()]), to: permStack)

        permBox.setContent(permStack, inset: Self.cardInset)
        contentStack.addArrangedSubview(permBox)

        // 2. 运行规则（回复节奏与运行参数）
        let paceBox = createSectionBox(title: L10n.t(.uiSettings049))
        let paceStack = makeVerticalStack(spacing: 12)

        for f in [cooldownField, maxRepliesField, quietSecField, maxSecField, contextMsgField, maxAttachmentField] {
            makeStyleField(f)
        }

        let paceRows: [(NSTextField, NSTextField)] = [
            (makeFieldLabel(L10n.t(.uiSettings031)), makeHintLabel(L10n.t(.uiSettings032))),
            (makeFieldLabel(L10n.t(.uiSettings033)), makeHintLabel(L10n.t(.uiSettings034))),
            (makeFieldLabel(L10n.t(.uiSettings035)), makeHintLabel(L10n.t(.uiSettings036))),
            (makeFieldLabel(L10n.t(.uiSettings037)), makeHintLabel(L10n.t(.uiSettings038))),
            (makeFieldLabel(L10n.t(.uiSettings039)), makeHintLabel(L10n.t(.uiSettings040))),
            (makeFieldLabel(L10n.t(.uiSettings041)), makeHintLabel(L10n.t(.uiSettings042)))
        ]
        let paceFields = [cooldownField, maxRepliesField, quietSecField, maxSecField, contextMsgField, maxAttachmentField]
        for (idx, pair) in paceRows.enumerated() {
            let row = makeRow([pair.0, paceFields[idx], pair.1, makeSpacer()])
            addFullWidthRow(row, to: paceStack)
        }

        manageWindowCheck.font = NSFont.systemFont(ofSize: 13)
        manageWindowCheck.contentTintColor = Theme.Colors.brand
        paceStack.addArrangedSubview(manageWindowCheck)

        let noteLbl = makeWrappingHintLabel(L10n.t(.uiSettings043))
        addFullWidthRow(noteLbl, to: paceStack)

        let divider = makeDivider()
        addFullWidthRow(divider, to: paceStack)

        restoreDefaultsButton.target = self
        restoreDefaultsButton.action = #selector(handleRestoreDefaults)
        styleSecondaryButton(restoreDefaultsButton)
        saveButton.target = self
        saveButton.action = #selector(handleSaveConfig)

        statusMessageLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        statusMessageLabel.textColor = Theme.Colors.textSecondary
        statusMessageLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let actionsRow = makeRow([saveButton, restoreDefaultsButton, makeSpacer()])
        addFullWidthRow(actionsRow, to: paceStack)
        statusMessageLabel.maximumNumberOfLines = 2
        addFullWidthRow(statusMessageLabel, to: paceStack)

        paceBox.setContent(paceStack, inset: Self.cardInset)
        contentStack.addArrangedSubview(paceBox)

        // 3. 界面语言（五语）：切换后由宿主重建侧栏，导航与页面文案一起更新。
        let langBox = createSectionBox(title: L10n.t(.uiSettings050))
        let langStack = makeVerticalStack(spacing: 8)
        languagePopup.addItems(withTitles: AppLanguage.allCases.map { $0.displayName })
        if let idx = AppLanguage.allCases.firstIndex(of: L10n.language) {
            languagePopup.selectItem(at: idx)
        }
        languagePopup.target = self
        languagePopup.action = #selector(handleLanguageChange)
        let langRow = makeRow([makeFieldLabel(L10n.t(.languageLabel)), languagePopup, makeSpacer()])
        addFullWidthRow(langRow, to: langStack)
        let langNote = makeWrappingHintLabel(L10n.t(.languageNote))
        addFullWidthRow(langNote, to: langStack)
        langBox.setContent(langStack, inset: Self.cardInset)
        contentStack.addArrangedSubview(langBox)

        // 4. 账号与模型
        let accountBox = createSectionBox(title: L10n.t(.uiSettings051))
        let accountStack = makeVerticalStack(spacing: 12)
        let accountRows: [(String, NSTextField)] = [
            (L10n.t(.labelAccount), accountLabel),
            (L10n.t(.labelBackend), backendLabel),
            (L10n.t(.labelModelName), modelLabel),
            (L10n.t(.labelApiKey), keyStatusLabel)
        ]
        for (title, value) in accountRows {
            styleValueLabel(value)
            addFullWidthRow(makeRow([makeFieldLabel(title), value]), to: accountStack)
        }
        accountBox.setContent(accountStack, inset: Self.cardInset)
        contentStack.addArrangedSubview(accountBox)

        // 5. 应用与更新
        let updateBox = createSectionBox(title: L10n.t(.uiSettings052))
        let updateStack = makeVerticalStack(spacing: 10)
        let verRow = makeRow([
            makeFieldLabel(L10n.t(.hostVersion)),
            makeValueLabel(hostAppVersion()),
            makeSpacer()
        ])
        addFullWidthRow(verRow, to: updateStack)
        let updateIntro = makeWrappingHintLabel(L10n.t(.updateIntro))
        addFullWidthRow(updateIntro, to: updateStack)
        jumpToUpdateButton.target = self
        jumpToUpdateButton.action = #selector(handleJumpToUpdate)
        styleSecondaryButton(jumpToUpdateButton)
        let jumpRow = makeRow([jumpToUpdateButton, makeSpacer()])
        addFullWidthRow(jumpRow, to: updateStack)
        updateBox.setContent(updateStack, inset: Self.cardInset)
        contentStack.addArrangedSubview(updateBox)

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(contentStack)
        scrollView.documentView = document
        view.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: view.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 24),
            contentStack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -24),
            contentStack.topAnchor.constraint(equalTo: document.topAnchor, constant: 8),
            contentStack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24)
        ])
        for card in [permBox, paceBox, langBox, accountBox, updateBox] {
            card.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
    }

    // MARK: - Actions

    @objc private func handleLanguageChange() {
        let idx = languagePopup.indexOfSelectedItem
        guard idx >= 0, idx < AppLanguage.allCases.count else { return }
        L10n.setLanguage(AppLanguage.allCases[idx])
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
                views.status.stringValue = L10n.t(.uiSettings044)
                views.status.textColor = Theme.Colors.statusNormalText
                views.reqBtn.isEnabled = false
            case .some(false):
                views.status.stringValue = L10n.t(.uiSettings048)
                views.status.textColor = Theme.Colors.statusWarningText
                views.reqBtn.isEnabled = true
            case .none:
                views.status.stringValue = L10n.t(.uiResidualUnknown)
                views.status.textColor = Theme.Colors.statusInactiveText
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
                statusMessageLabel.stringValue = L10n.t(.uiSettings045)
                statusMessageLabel.textColor = Theme.Colors.statusNormalText
            } catch {
                statusMessageLabel.stringValue = L10n.t(.saveFailed, String(describing: error.localizedDescription))
                statusMessageLabel.textColor = Theme.Colors.statusErrorText
            }
        case .failure(let err):
            statusMessageLabel.stringValue = L10n.t(.validationError, String(describing: err))
            statusMessageLabel.textColor = Theme.Colors.statusErrorText
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
        statusMessageLabel.stringValue = L10n.t(.uiSettings046)
        statusMessageLabel.textColor = Theme.Colors.textSecondary
    }

    public func triggerUpdateNavigation() {
        onNavigateToUpdate?()
    }

    @objc private func handleJumpToUpdate() {
        triggerUpdateNavigation()
    }
}
