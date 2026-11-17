// ComponentsPage.swift: Design tokens and UI components showcase page for issue-2/theme-tokens.
// Minimum deployment target: macOS 12.0
import Foundation
import AppKit

// MARK: - Localized Strings Constants (To be migrated to L10n.swift by T4)
// All strings displayed on this page are centralized here for easy localization migration.

public enum ComponentsPageStrings {
    // 页面标题与导言
    public static let pageTitle = "组件与配色"
    public static let pageSubtitle = "设计规范、设计令牌与通用组件展示（供验收对照）"

    // 色彩规范板块
    public static let sectionColorsTitle = "色彩规范 (Color Tokens)"
    public static let subsectionButtonsAndBrand = "主按钮与品牌色"
    public static let subsectionInputBorderAndStates = "输入框描边与状态色"
    public static let subsectionStatusTokens = "四种服务状态色 (圆点 / 文字 / 底色)"

    // 按钮与品牌色用途说明
    public static let tokenPrimaryButtonDefault = "主按钮默认背景 (#0A5B6B)"
    public static let tokenPrimaryButtonHover = "主按钮悬停背景 (#084B58)"
    public static let tokenPrimaryButtonText = "主按钮文字/白字 (#FFFFFF)"
    public static let tokenPrimaryButtonDisabledBg = "主按钮禁用背景 (#F4F5F7)"
    public static let tokenPrimaryButtonDisabledText = "主按钮禁用文字 (#6B7280)"
    public static let tokenBrand = "品牌主色 (#12A37A)"
    public static let tokenBrandLight = "品牌浅色 (#E2F5EE)"

    // 输入框状态用途说明
    public static let tokenInputBorder = "输入框默认描边 (#C9CDD4)"
    public static let tokenInputErrorBorder = "输入框错误态描边 (#E5484D)"
    public static let tokenInputErrorText = "输入框错误态文字 (#B73A3E)"
    public static let tokenInputDisabledBg = "输入框禁用态背景 (#F4F5F7)"
    public static let tokenInputDisabledText = "输入框禁用态文字 (#6B7280)"

    // 四状态用途说明
    public static let tokenNormalDot = "正常状态圆点 (#1AAD78)"
    public static let tokenNormalText = "正常状态文字 (#127954)"
    public static let tokenNormalBg = "正常状态底色 (#E4F5EF)"

    public static let tokenWarningDot = "注意状态圆点 (#E08E0B)"
    public static let tokenWarningText = "注意状态文字 (#925C07)"
    public static let tokenWarningBg = "注意状态底色 (#FBF1E2)"

    public static let tokenErrorDot = "异常状态圆点 (#E5484D)"
    public static let tokenErrorText = "异常状态文字 (#B73A3E)"
    public static let tokenErrorBg = "异常状态底色 (#FCE9EA)"

    public static let tokenInactiveDot = "未启用圆点 (#9AA0A6)"
    public static let tokenInactiveText = "未启用文字 (#6C7074)"
    public static let tokenInactiveBg = "未启用底色 (#F3F4F4)"

    // 通用组件板块
    public static let sectionComponentsTitle = "通用组件 (Components)"

    // 主按钮展示
    public static let subsectionPrimaryButtons = "主按钮 (Primary Button)"
    public static let buttonNormalTitle = "保存修改 (悬停体验)"
    public static let buttonDisabledTitle = "保存修改 (禁用状态)"
    public static let buttonDescription = "支持悬停 (#084B58) 变色、点击高亮与禁用态自动变灰。"

    // 状态标签展示
    public static let subsectionStatusTags = "状态标签 (Status Tag)"
    public static let tagNormalTitle = "正常"
    public static let tagWarningTitle = "注意"
    public static let tagErrorTitle = "异常"
    public static let tagInactiveTitle = "未启用"

    // 状态横幅展示
    public static let subsectionStatusBanners = "状态横幅 (Status Banner)"
    public static let bannerNormalTitle = "核心服务运行中"
    public static let bannerNormalMsg = "微信客服助手正在后台持续监听消息并按规则自动回复。"
    public static let bannerWarningTitle = "重启后生效提示"
    public static let bannerWarningMsg = "已保存的连接设置尚未生效。正在运行的服务仍使用旧设置，重启核心服务后生效。"
    public static let bannerWarningAction = "重启核心服务"
    public static let bannerErrorTitle = "连接校验异常"
    public static let bannerErrorMsg = "有 2 项配置需要修改，改好后才能测试连接或保存。"

    // 二次确认弹窗展示
    public static let subsectionConfirmDialog = "带勾选二次确认弹窗 (Confirm Dialog)"
    public static let dialogTriggerButtonTitle = "打开带勾选二次确认弹窗..."
    public static let dialogTitle = "确认修改连接设置？"
    public static let dialogMessage = "自动回复正在运行。你修改了：接口地址、密钥来源。保存后，正在运行的服务仍使用旧设置，重启核心服务后才生效。"
    public static let dialogCheckboxText = "我知道重启核心服务后才会生效"
    public static let dialogConfirmButton = "保存并生效"
    public static let dialogCancelButton = "取消"
    public static let dialogResultPrefix = "最近一次交互结果："
    public static let dialogResultInitial = "尚未打开弹窗"
    public static let dialogResultConfirmed = "用户已勾选确认并点击保存"
    public static let dialogResultCancelled = "用户取消或未勾选"

    // 表单字段展示
    public static let subsectionFormFields = "表单字段 (Form Field - 自绘描边)"
    public static let fieldNormalLabel = "模型接口地址 (正常态)"
    public static let fieldNormalPlaceholder = "https://api.openai.com/v1"
    public static let fieldNormalHelper = "支持 HTTPS 或本机地址 localhost / 127.0.0.1 / [::1]"

    public static let fieldErrorLabel = "API 密钥来源 (错误态)"
    public static let fieldErrorInitial = "INVALID_ENV_VAR@NAME"
    public static let fieldErrorMessage = "环境变量名不合法，仅允许字母、数字和下划线"

    public static let fieldDisabledLabel = "最大输出长度 (禁用态)"
    public static let fieldDisabledValue = "1024 tokens"
    public static let fieldDisabledHelper = "固定参数，由核心引擎严格限制，当前角色只读"
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
        alert.messageText = "横幅操作触发"
        alert.informativeText = "成功响应横幅右侧按钮动作。"
        alert.addButton(withTitle: "好的")
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
