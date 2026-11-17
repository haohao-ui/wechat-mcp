// ComponentsPage.swift: Design tokens and UI components showcase page for issue-2/theme-tokens.
// Minimum deployment target: macOS 12.0
import Foundation
import AppKit

// MARK: - Localized Strings Constants (To be migrated to L10n.swift by T4)
// All strings displayed on this page are centralized here for easy localization migration.

public enum ComponentsPageStrings {
    // 页面标题与导言
    public static var pageTitle: String { L10n.t(.navComponents) }
    public static var pageSubtitle: String { L10n.t(.componentsPageSubtitle) }

    // 色彩规范板块
    public static var sectionColorsTitle: String { L10n.t(.componentsSectionColorsTitle) }
    public static var subsectionButtonsAndBrand: String { L10n.t(.componentsSubsectionButtonsAndBrand) }
    public static var subsectionInputBorderAndStates: String { L10n.t(.componentsSubsectionInputBorderAndStates) }
    public static var subsectionStatusTokens: String { L10n.t(.componentsSubsectionStatusTokens) }

    // 按钮与品牌色用途说明
    public static var tokenPrimaryButtonDefault: String { L10n.t(.componentsTokenPrimaryButtonDefault) }
    public static var tokenPrimaryButtonHover: String { L10n.t(.componentsTokenPrimaryButtonHover) }
    public static var tokenPrimaryButtonText: String { L10n.t(.componentsTokenPrimaryButtonText) }
    public static var tokenPrimaryButtonDisabledBg: String { L10n.t(.componentsTokenPrimaryButtonDisabledBg) }
    public static var tokenPrimaryButtonDisabledText: String { L10n.t(.componentsTokenPrimaryButtonDisabledText) }
    public static var tokenBrand: String { L10n.t(.componentsTokenBrand) }
    public static var tokenBrandLight: String { L10n.t(.componentsTokenBrandLight) }

    // 输入框状态用途说明
    public static var tokenInputBorder: String { L10n.t(.componentsTokenInputBorder) }
    public static var tokenInputErrorBorder: String { L10n.t(.componentsTokenInputErrorBorder) }
    public static var tokenInputErrorText: String { L10n.t(.componentsTokenInputErrorText) }
    public static var tokenInputDisabledBg: String { L10n.t(.componentsTokenInputDisabledBg) }
    public static var tokenInputDisabledText: String { L10n.t(.componentsTokenInputDisabledText) }

    // 四状态用途说明
    public static var tokenNormalDot: String { L10n.t(.componentsTokenNormalDot) }
    public static var tokenNormalText: String { L10n.t(.componentsTokenNormalText) }
    public static var tokenNormalBg: String { L10n.t(.componentsTokenNormalBg) }

    public static var tokenWarningDot: String { L10n.t(.componentsTokenWarningDot) }
    public static var tokenWarningText: String { L10n.t(.componentsTokenWarningText) }
    public static var tokenWarningBg: String { L10n.t(.componentsTokenWarningBg) }

    public static var tokenErrorDot: String { L10n.t(.componentsTokenErrorDot) }
    public static var tokenErrorText: String { L10n.t(.componentsTokenErrorText) }
    public static var tokenErrorBg: String { L10n.t(.componentsTokenErrorBg) }

    public static var tokenInactiveDot: String { L10n.t(.componentsTokenInactiveDot) }
    public static var tokenInactiveText: String { L10n.t(.componentsTokenInactiveText) }
    public static var tokenInactiveBg: String { L10n.t(.componentsTokenInactiveBg) }

    // 通用组件板块
    public static var sectionComponentsTitle: String { L10n.t(.componentsSectionComponentsTitle) }

    // 主按钮展示
    public static var subsectionPrimaryButtons: String { L10n.t(.componentsSubsectionPrimaryButtons) }
    public static var buttonNormalTitle: String { L10n.t(.componentsButtonNormalTitle) }
    public static var buttonDisabledTitle: String { L10n.t(.componentsButtonDisabledTitle) }
    public static var buttonDescription: String { L10n.t(.componentsButtonDescription) }

    // 状态标签展示
    public static var subsectionStatusTags: String { L10n.t(.componentsSubsectionStatusTags) }
    public static var tagNormalTitle: String { L10n.t(.tagNormal) }
    public static var tagWarningTitle: String { L10n.t(.tagAttention) }
    public static var tagErrorTitle: String { L10n.t(.tagError) }
    public static var tagInactiveTitle: String { L10n.t(.tagOff) }

    // 状态横幅展示
    public static var subsectionStatusBanners: String { L10n.t(.componentsSubsectionStatusBanners) }
    public static var bannerNormalTitle: String { L10n.t(.componentsBannerNormalTitle) }
    public static var bannerNormalMsg: String { L10n.t(.componentsBannerNormalMsg) }
    public static var bannerWarningTitle: String { L10n.t(.componentsBannerWarningTitle) }
    public static var bannerWarningMsg: String { L10n.t(.componentsBannerWarningMsg) }
    public static var bannerWarningAction: String { L10n.t(.componentsBannerWarningAction) }
    public static var bannerErrorTitle: String { L10n.t(.componentsBannerErrorTitle) }
    public static var bannerErrorMsg: String { L10n.t(.componentsBannerErrorMsg) }

    // 二次确认弹窗展示
    public static var subsectionConfirmDialog: String { L10n.t(.componentsSubsectionConfirmDialog) }
    public static var dialogTriggerButtonTitle: String { L10n.t(.componentsDialogTriggerButtonTitle) }
    public static var dialogTitle: String { L10n.t(.componentsDialogTitle) }
    public static var dialogMessage: String { L10n.t(.componentsDialogMessage) }
    public static var dialogCheckboxText: String { L10n.t(.componentsDialogCheckboxText) }
    public static var dialogConfirmButton: String { L10n.t(.componentsDialogConfirmButton) }
    public static var dialogCancelButton: String { L10n.t(.cancel) }
    public static var dialogResultPrefix: String { L10n.t(.componentsDialogResultPrefix) }
    public static var dialogResultInitial: String { L10n.t(.componentsDialogResultInitial) }
    public static var dialogResultConfirmed: String { L10n.t(.componentsDialogResultConfirmed) }
    public static var dialogResultCancelled: String { L10n.t(.componentsDialogResultCancelled) }

    // 表单字段展示
    public static var subsectionFormFields: String { L10n.t(.componentsSubsectionFormFields) }
    public static var fieldNormalLabel: String { L10n.t(.componentsFieldNormalLabel) }
    public static let fieldNormalPlaceholder = "https://api.openai.com/v1"
    public static var fieldNormalHelper: String { L10n.t(.componentsFieldNormalHelper) }

    public static var fieldErrorLabel: String { L10n.t(.componentsFieldErrorLabel) }
    public static let fieldErrorInitial = "INVALID_ENV_VAR@NAME"
    public static var fieldErrorMessage: String { L10n.t(.componentsFieldErrorMessage) }

    public static var fieldDisabledLabel: String { L10n.t(.componentsFieldDisabledLabel) }
    public static let fieldDisabledValue = "1024 tokens"
    public static var fieldDisabledHelper: String { L10n.t(.componentsFieldDisabledHelper) }
}

// MARK: - ComponentsPageController

public final class ComponentsPageController: NSViewController {
    private let dialogFeedbackLabel = NSTextField(labelWithString: ComponentsPageStrings.dialogResultInitial)

    public init() {
        super.init(nibName: nil, bundle: nil)
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        self.view = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 900))
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        setupUI()
    }

    private func setupUI() {
        let scrollView = NSScrollView(frame: view.bounds)
        scrollView.autoresizingMask = [.width, .height]
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        let documentView = NSView()
        documentView.translatesAutoresizingMaskIntoConstraints = false

        let mainStack = NSStackView()
        mainStack.orientation = .vertical
        mainStack.alignment = .leading
        mainStack.spacing = 24
        mainStack.translatesAutoresizingMaskIntoConstraints = false

        documentView.addSubview(mainStack)
        scrollView.documentView = documentView

        view.addSubview(scrollView)

        // 标题区
        let headerBox = makeHeaderBox()
        mainStack.addArrangedSubview(headerBox)

        // 色彩规范区
        let colorsSection = makeColorsSection()
        mainStack.addArrangedSubview(colorsSection)

        // 通用组件区
        let componentsSection = makeComponentsSection()
        mainStack.addArrangedSubview(componentsSection)

        NSLayoutConstraint.activate([
            documentView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            documentView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            documentView.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            documentView.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),

            mainStack.topAnchor.constraint(equalTo: documentView.topAnchor, constant: 20),
            mainStack.leadingAnchor.constraint(equalTo: documentView.leadingAnchor, constant: 24),
            mainStack.trailingAnchor.constraint(equalTo: documentView.trailingAnchor, constant: -24),
            mainStack.bottomAnchor.constraint(equalTo: documentView.bottomAnchor, constant: -30),

            headerBox.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            colorsSection.widthAnchor.constraint(equalTo: mainStack.widthAnchor),
            componentsSection.widthAnchor.constraint(equalTo: mainStack.widthAnchor)
        ])
    }

    // MARK: - Section Builders

    private func makeHeaderBox() -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        let titleLabel = NSTextField(labelWithString: ComponentsPageStrings.pageTitle)
        titleLabel.font = NSFont.systemFont(ofSize: 22, weight: .bold)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        let subtitleLabel = NSTextField(labelWithString: ComponentsPageStrings.pageSubtitle)
        subtitleLabel.font = NSFont.systemFont(ofSize: 13, weight: .regular)
        subtitleLabel.textColor = NSColor.secondaryLabelColor
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(titleLabel)
        container.addSubview(subtitleLabel)

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: container.topAnchor),
            titleLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            titleLabel.trailingAnchor.constraint(equalTo: container.trailingAnchor),

            subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 4),
            subtitleLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            subtitleLabel.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            subtitleLabel.bottomAnchor.constraint(equalTo: container.bottomAnchor)
        ])

        return container
    }

    private func makeColorsSection() -> NSView {
        let box = NSBox()
        box.title = ComponentsPageStrings.sectionColorsTitle
        box.titleFont = NSFont.systemFont(ofSize: 15, weight: .bold)
        box.translatesAutoresizingMaskIntoConstraints = false

        let content = NSStackView()
        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 18
        content.translatesAutoresizingMaskIntoConstraints = false

        // 1. 主按钮与品牌色
        let group1 = makeSubheading(ComponentsPageStrings.subsectionButtonsAndBrand)
        let grid1 = makeColorGrid([
            (Theme.Colors.primaryButtonDefault, Theme.Colors.primaryButtonDefaultHex, ComponentsPageStrings.tokenPrimaryButtonDefault),
            (Theme.Colors.primaryButtonHover, Theme.Colors.primaryButtonHoverHex, ComponentsPageStrings.tokenPrimaryButtonHover),
            (Theme.Colors.primaryButtonText, Theme.Colors.primaryButtonTextHex, ComponentsPageStrings.tokenPrimaryButtonText),
            (Theme.Colors.primaryButtonDisabledBg, Theme.Colors.primaryButtonDisabledBgHex, ComponentsPageStrings.tokenPrimaryButtonDisabledBg),
            (Theme.Colors.primaryButtonDisabledText, Theme.Colors.primaryButtonDisabledTextHex, ComponentsPageStrings.tokenPrimaryButtonDisabledText),
            (Theme.Colors.brand, Theme.Colors.brandHex, ComponentsPageStrings.tokenBrand),
            (Theme.Colors.brandLight, Theme.Colors.brandLightHex, ComponentsPageStrings.tokenBrandLight)
        ])

        // 2. 输入框描边与状态
        let group2 = makeSubheading(ComponentsPageStrings.subsectionInputBorderAndStates)
        let grid2 = makeColorGrid([
            (Theme.Colors.inputBorder, Theme.Colors.inputBorderHex, ComponentsPageStrings.tokenInputBorder),
            (Theme.Colors.inputErrorBorder, Theme.Colors.inputErrorBorderHex, ComponentsPageStrings.tokenInputErrorBorder),
            (Theme.Colors.inputErrorText, Theme.Colors.inputErrorTextHex, ComponentsPageStrings.tokenInputErrorText),
            (Theme.Colors.inputDisabledBg, Theme.Colors.inputDisabledBgHex, ComponentsPageStrings.tokenInputDisabledBg),
            (Theme.Colors.inputDisabledText, Theme.Colors.inputDisabledTextHex, ComponentsPageStrings.tokenInputDisabledText)
        ])

        // 3. 四状态颜色 (圆点 / 文字 / 底色)
        let group3 = makeSubheading(ComponentsPageStrings.subsectionStatusTokens)
        let grid3 = makeColorGrid([
            (Theme.Colors.statusNormalDot, Theme.Colors.statusNormalDotHex, ComponentsPageStrings.tokenNormalDot),
            (Theme.Colors.statusNormalText, Theme.Colors.statusNormalTextHex, ComponentsPageStrings.tokenNormalText),
            (Theme.Colors.statusNormalBg, Theme.Colors.statusNormalBgHex, ComponentsPageStrings.tokenNormalBg),

            (Theme.Colors.statusWarningDot, Theme.Colors.statusWarningDotHex, ComponentsPageStrings.tokenWarningDot),
            (Theme.Colors.statusWarningText, Theme.Colors.statusWarningTextHex, ComponentsPageStrings.tokenWarningText),
            (Theme.Colors.statusWarningBg, Theme.Colors.statusWarningBgHex, ComponentsPageStrings.tokenWarningBg),

            (Theme.Colors.statusErrorDot, Theme.Colors.statusErrorDotHex, ComponentsPageStrings.tokenErrorDot),
            (Theme.Colors.statusErrorText, Theme.Colors.statusErrorTextHex, ComponentsPageStrings.tokenErrorText),
            (Theme.Colors.statusErrorBg, Theme.Colors.statusErrorBgHex, ComponentsPageStrings.tokenErrorBg),

            (Theme.Colors.statusInactiveDot, Theme.Colors.statusInactiveDotHex, ComponentsPageStrings.tokenInactiveDot),
            (Theme.Colors.statusInactiveText, Theme.Colors.statusInactiveTextHex, ComponentsPageStrings.tokenInactiveText),
            (Theme.Colors.statusInactiveBg, Theme.Colors.statusInactiveBgHex, ComponentsPageStrings.tokenInactiveBg)
        ])

        content.addArrangedSubview(group1)
        content.addArrangedSubview(grid1)
        content.addArrangedSubview(group2)
        content.addArrangedSubview(grid2)
        content.addArrangedSubview(group3)
        content.addArrangedSubview(grid3)

        installContent(into: box, content: content, inset: 14)

        return box
    }

    private func makeComponentsSection() -> NSView {
        let box = NSBox()
        box.title = ComponentsPageStrings.sectionComponentsTitle
        box.titleFont = NSFont.systemFont(ofSize: 15, weight: .bold)
        box.translatesAutoresizingMaskIntoConstraints = false

        let content = NSStackView()
        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 22
        content.translatesAutoresizingMaskIntoConstraints = false

        // 1. 主按钮展示 (含悬停与禁用)
        let btnHeading = makeSubheading(ComponentsPageStrings.subsectionPrimaryButtons)
        let normalBtn = ThemePrimaryButton(title: ComponentsPageStrings.buttonNormalTitle, target: nil, action: nil)
        let disabledBtn = ThemePrimaryButton(title: ComponentsPageStrings.buttonDisabledTitle, target: nil, action: nil)
        disabledBtn.isEnabled = false

        let btnDesc = NSTextField(labelWithString: ComponentsPageStrings.buttonDescription)
        btnDesc.font = NSFont.systemFont(ofSize: 11)
        btnDesc.textColor = NSColor.secondaryLabelColor

        let btnRow = NSStackView(views: [normalBtn, disabledBtn])
        btnRow.orientation = .horizontal
        btnRow.spacing = 16

        // 2. 状态标签展示 (四种状态)
        let tagHeading = makeSubheading(ComponentsPageStrings.subsectionStatusTags)
        let tagNormal = ThemeStatusTag(status: .normal)
        let tagWarning = ThemeStatusTag(status: .warning)
        let tagError = ThemeStatusTag(status: .error)
        let tagInactive = ThemeStatusTag(status: .inactive)

        let tagRow = NSStackView(views: [tagNormal, tagWarning, tagError, tagInactive])
        tagRow.orientation = .horizontal
        tagRow.spacing = 14

        // 3. 状态横幅展示 (正常 / 注意 / 异常)
        let bannerHeading = makeSubheading(ComponentsPageStrings.subsectionStatusBanners)
        let bannerNormal = ThemeStatusBanner(
            status: .normal,
            title: ComponentsPageStrings.bannerNormalTitle,
            message: ComponentsPageStrings.bannerNormalMsg
        )
        let bannerWarning = ThemeStatusBanner(
            status: .warning,
            title: ComponentsPageStrings.bannerWarningTitle,
            message: ComponentsPageStrings.bannerWarningMsg,
            actionTitle: ComponentsPageStrings.bannerWarningAction,
            actionTarget: self,
            actionSelector: #selector(handleBannerAction)
        )
        let bannerError = ThemeStatusBanner(
            status: .error,
            title: ComponentsPageStrings.bannerErrorTitle,
            message: ComponentsPageStrings.bannerErrorMsg
        )

        let bannerStack = NSStackView(views: [bannerNormal, bannerWarning, bannerError])
        bannerStack.orientation = .vertical
        bannerStack.alignment = .leading
        bannerStack.spacing = 10
        bannerStack.translatesAutoresizingMaskIntoConstraints = false

        // 4. 二次确认弹窗展示
        let dialogHeading = makeSubheading(ComponentsPageStrings.subsectionConfirmDialog)
        let triggerDialogBtn = NSButton(
            title: ComponentsPageStrings.dialogTriggerButtonTitle,
            target: self,
            action: #selector(handleOpenConfirmDialog)
        )
        triggerDialogBtn.bezelStyle = .rounded

        dialogFeedbackLabel.font = NSFont.systemFont(ofSize: 12)
        dialogFeedbackLabel.textColor = NSColor.secondaryLabelColor

        let dialogRow = NSStackView(views: [triggerDialogBtn, dialogFeedbackLabel])
        dialogRow.orientation = .horizontal
        dialogRow.spacing = 14

        // 5. 表单字段展示 (正常 / 错误 / 禁用态)
        let formHeading = makeSubheading(ComponentsPageStrings.subsectionFormFields)

        let normalField = ThemeFormField(
            label: ComponentsPageStrings.fieldNormalLabel,
            placeholder: ComponentsPageStrings.fieldNormalPlaceholder,
            helper: ComponentsPageStrings.fieldNormalHelper
        )
        normalField.setNormal()

        let errorField = ThemeFormField(
            label: ComponentsPageStrings.fieldErrorLabel,
            placeholder: nil,
            helper: nil
        )
        errorField.input.stringValue = ComponentsPageStrings.fieldErrorInitial
        errorField.setError(ComponentsPageStrings.fieldErrorMessage)

        let disabledField = ThemeFormField(
            label: ComponentsPageStrings.fieldDisabledLabel,
            placeholder: nil,
            helper: ComponentsPageStrings.fieldDisabledHelper
        )
        disabledField.input.stringValue = ComponentsPageStrings.fieldDisabledValue
        disabledField.setDisabled(helper: ComponentsPageStrings.fieldDisabledHelper)

        let formStack = NSStackView(views: [normalField, errorField, disabledField])
        formStack.orientation = .vertical
        formStack.alignment = .leading
        formStack.spacing = 14
        formStack.translatesAutoresizingMaskIntoConstraints = false

        // 组装到 content
        content.addArrangedSubview(btnHeading)
        content.addArrangedSubview(btnRow)
        content.addArrangedSubview(btnDesc)

        content.addArrangedSubview(tagHeading)
        content.addArrangedSubview(tagRow)

        content.addArrangedSubview(bannerHeading)
        content.addArrangedSubview(bannerStack)

        content.addArrangedSubview(dialogHeading)
        content.addArrangedSubview(dialogRow)

        content.addArrangedSubview(formHeading)
        content.addArrangedSubview(formStack)

        installContent(into: box, content: content, inset: 14)

        NSLayoutConstraint.activate([
            bannerNormal.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40),
            bannerWarning.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40),
            bannerError.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40),
            normalField.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40),
            errorField.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40),
            disabledField.widthAnchor.constraint(equalTo: box.widthAnchor, constant: -40)
        ])

        return box
    }

    // MARK: - Helpers

    private func installContent(into box: NSBox, content: NSView, inset: CGFloat = 14) {
        guard let host = box.contentView else { return }
        host.subviews.forEach { $0.removeFromSuperview() }
        content.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.leadingAnchor, constant: inset),
            content.trailingAnchor.constraint(equalTo: host.trailingAnchor, constant: -inset),
            content.topAnchor.constraint(equalTo: host.topAnchor, constant: inset),
            content.bottomAnchor.constraint(equalTo: host.bottomAnchor, constant: -inset),
        ])
    }

    private func makeSubheading(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        label.textColor = NSColor.labelColor
        return label
    }

    private func makeColorGrid(_ items: [(color: NSColor, hex: String, desc: String)]) -> NSView {
        let container = NSStackView()
        container.orientation = .vertical
        container.alignment = .leading
        container.spacing = 8
        container.translatesAutoresizingMaskIntoConstraints = false

        for item in items {
            let row = NSStackView()
            row.orientation = .horizontal
            row.alignment = .centerY
            row.spacing = 10

            let swatch = NSView()
            swatch.wantsLayer = true
            swatch.layer?.cornerRadius = 5
            swatch.layer?.borderWidth = 1
            swatch.layer?.borderColor = NSColor.separatorColor.cgColor
            swatch.layer?.backgroundColor = item.color.cgColor
            swatch.translatesAutoresizingMaskIntoConstraints = false

            NSLayoutConstraint.activate([
                swatch.widthAnchor.constraint(equalToConstant: 24),
                swatch.heightAnchor.constraint(equalToConstant: 24)
            ])

            let hexLabel = NSTextField(labelWithString: item.hex)
            hexLabel.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .bold)
            hexLabel.textColor = NSColor.labelColor
            hexLabel.translatesAutoresizingMaskIntoConstraints = false
            hexLabel.widthAnchor.constraint(equalToConstant: 80).isActive = true

            let descLabel = NSTextField(labelWithString: item.desc)
            descLabel.font = NSFont.systemFont(ofSize: 12, weight: .regular)
            descLabel.textColor = NSColor.secondaryLabelColor

            row.addArrangedSubview(swatch)
            row.addArrangedSubview(hexLabel)
            row.addArrangedSubview(descLabel)

            container.addArrangedSubview(row)
        }

        return container
    }

    // MARK: - Actions

    @objc private func handleBannerAction() {
        let alert = NSAlert()
        alert.messageText = L10n.t(.componentsBannerAlertTitle)
        alert.informativeText = L10n.t(.componentsBannerAlertMsg)
        alert.addButton(withTitle: L10n.t(.actionOK))
        alert.runModal()
    }

    @objc private func handleOpenConfirmDialog() {
        let confirmed = ThemeConfirmDialog.show(
            title: ComponentsPageStrings.dialogTitle,
            message: ComponentsPageStrings.dialogMessage,
            checkboxText: ComponentsPageStrings.dialogCheckboxText,
            confirmTitle: ComponentsPageStrings.dialogConfirmButton,
            cancelTitle: ComponentsPageStrings.dialogCancelButton
        )

        dialogFeedbackLabel.stringValue = "\(ComponentsPageStrings.dialogResultPrefix)\(confirmed ? ComponentsPageStrings.dialogResultConfirmed : ComponentsPageStrings.dialogResultCancelled)"
        dialogFeedbackLabel.textColor = confirmed ? Theme.Colors.statusNormalText : Theme.Colors.statusErrorText
    }
}
