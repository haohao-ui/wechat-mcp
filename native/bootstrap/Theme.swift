// Theme.swift: Design tokens and reusable UI components.
// Minimum deployment target: macOS 12.0
import Foundation
import AppKit

// MARK: - Color Hex Conversion

extension NSColor {
    /// Convenience initializer to create an sRGB NSColor from a hex string (e.g. "#12A37A" or "12A37A").
    public convenience init(hex: String) {
        var cleanHex = hex.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        if cleanHex.hasPrefix("#") {
            cleanHex.removeFirst()
        }
        guard cleanHex.count == 6, let rgb = UInt64(cleanHex, radix: 16) else {
            self.init(srgbRed: 0, green: 0, blue: 0, alpha: 1)
            return
        }
        let r = CGFloat((rgb >> 16) & 0xFF) / 255.0
        let g = CGFloat((rgb >> 8) & 0xFF) / 255.0
        let b = CGFloat(rgb & 0xFF) / 255.0
        self.init(srgbRed: r, green: g, blue: b, alpha: 1.0)
    }

    /// Converts this color to its uppercase sRGB hex representation (e.g. "#12A37A").
    public var hexString: String {
        guard let rgbColor = usingColorSpace(.sRGB) else { return "#000000" }
        let r = Int(round(rgbColor.redComponent * 255.0))
        let g = Int(round(rgbColor.greenComponent * 255.0))
        let b = Int(round(rgbColor.blueComponent * 255.0))
        return String(format: "#%02X%02X%02X", r, g, b)
    }
}

// MARK: - Design Tokens

public enum Theme {
    public struct Colors {
        // 主按钮 (Primary Button)
        public static let primaryButtonDefaultHex       = "#0A5B6B"
        public static let primaryButtonHoverHex         = "#084B58"
        public static let primaryButtonTextHex          = "#FFFFFF"
        public static let primaryButtonDisabledBgHex    = "#F4F5F7"
        public static let primaryButtonDisabledTextHex  = "#6B7280"

        public static let primaryButtonDefault      = NSColor(hex: primaryButtonDefaultHex)
        public static let primaryButtonHover        = NSColor(hex: primaryButtonHoverHex)
        public static let primaryButtonText         = NSColor(hex: primaryButtonTextHex)
        public static let primaryButtonDisabledBg   = NSColor(hex: primaryButtonDisabledBgHex)
        public static let primaryButtonDisabledText = NSColor(hex: primaryButtonDisabledTextHex)

        // 品牌色 (Brand Colors)
        public static let brandHex      = "#12A37A"
        public static let brandLightHex = "#E2F5EE"

        public static let brand      = NSColor(hex: brandHex)
        public static let brandLight = NSColor(hex: brandLightHex)

        // 输入框描边与状态 (Input Border & States)
        public static let inputBorderHex        = "#C9CDD4"
        public static let inputErrorBorderHex   = "#E5484D"
        public static let inputErrorTextHex     = "#B73A3E"
        public static let inputDisabledBgHex    = "#F4F5F7"
        public static let inputDisabledTextHex  = "#6B7280"

        public static let inputBorder       = NSColor(hex: inputBorderHex)
        public static let inputErrorBorder  = NSColor(hex: inputErrorBorderHex)
        public static let inputErrorText    = NSColor(hex: inputErrorTextHex)
        public static let inputDisabledBg   = NSColor(hex: inputDisabledBgHex)
        public static let inputDisabledText = NSColor(hex: inputDisabledTextHex)

        // 四种状态颜色 (圆点 / 文字 / 底色)
        // 正常 (Normal)
        public static let statusNormalDotHex    = "#1AAD78"
        public static let statusNormalTextHex   = "#127954"
        public static let statusNormalBgHex     = "#E4F5EF"

        public static let statusNormalDot   = NSColor(hex: statusNormalDotHex)
        public static let statusNormalText  = NSColor(hex: statusNormalTextHex)
        public static let statusNormalBg    = NSColor(hex: statusNormalBgHex)

        // 注意 (Warning)
        public static let statusWarningDotHex   = "#E08E0B"
        public static let statusWarningTextHex  = "#925C07"
        public static let statusWarningBgHex    = "#FBF1E2"

        public static let statusWarningDot  = NSColor(hex: statusWarningDotHex)
        public static let statusWarningText = NSColor(hex: statusWarningTextHex)
        public static let statusWarningBg   = NSColor(hex: statusWarningBgHex)

        // 异常 (Error)
        public static let statusErrorDotHex     = "#E5484D"
        public static let statusErrorTextHex    = "#B73A3E"
        public static let statusErrorBgHex      = "#FCE9EA"

        public static let statusErrorDot    = NSColor(hex: statusErrorDotHex)
        public static let statusErrorText   = NSColor(hex: statusErrorTextHex)
        public static let statusErrorBg     = NSColor(hex: statusErrorBgHex)

        // 未启用 (Inactive)
        public static let statusInactiveDotHex  = "#9AA0A6"
        public static let statusInactiveTextHex = "#6C7074"
        public static let statusInactiveBgHex   = "#F3F4F4"

        public static let statusInactiveDot     = NSColor(hex: statusInactiveDotHex)
        public static let statusInactiveText    = NSColor(hex: statusInactiveTextHex)
        public static let statusInactiveBg      = NSColor(hex: statusInactiveBgHex)
    }
}

// MARK: - Status Tokens Enum

public enum ThemeStatus: String, CaseIterable {
    case normal = "正常"
    case warning = "注意"
    case error = "异常"
    case inactive = "未启用"

    public var title: String {
        switch self {
        case .normal: return L10n.t(.tagNormal)
        case .warning: return L10n.t(.tagAttention)
        case .error: return L10n.t(.tagError)
        case .inactive: return L10n.t(.tagOff)
        }
    }

    public var dotColor: NSColor {
        switch self {
        case .normal:   return Theme.Colors.statusNormalDot
        case .warning:  return Theme.Colors.statusWarningDot
        case .error:    return Theme.Colors.statusErrorDot
        case .inactive: return Theme.Colors.statusInactiveDot
        }
    }

    public var textColor: NSColor {
        switch self {
        case .normal:   return Theme.Colors.statusNormalText
        case .warning:  return Theme.Colors.statusWarningText
        case .error:    return Theme.Colors.statusErrorText
        case .inactive: return Theme.Colors.statusInactiveText
        }
    }

    public var backgroundColor: NSColor {
        switch self {
        case .normal:   return Theme.Colors.statusNormalBg
        case .warning:  return Theme.Colors.statusWarningBg
        case .error:    return Theme.Colors.statusErrorBg
        case .inactive: return Theme.Colors.statusInactiveBg
        }
    }

    public var dotHex: String {
        switch self {
        case .normal:   return Theme.Colors.statusNormalDotHex
        case .warning:  return Theme.Colors.statusWarningDotHex
        case .error:    return Theme.Colors.statusErrorDotHex
        case .inactive: return Theme.Colors.statusInactiveDotHex
        }
    }

    public var textHex: String {
        switch self {
        case .normal:   return Theme.Colors.statusNormalTextHex
        case .warning:  return Theme.Colors.statusWarningTextHex
        case .error:    return Theme.Colors.statusErrorTextHex
        case .inactive: return Theme.Colors.statusInactiveTextHex
        }
    }

    public var backgroundHex: String {
        switch self {
        case .normal:   return Theme.Colors.statusNormalBgHex
        case .warning:  return Theme.Colors.statusWarningBgHex
        case .error:    return Theme.Colors.statusErrorBgHex
        case .inactive: return Theme.Colors.statusInactiveBgHex
        }
    }
}

// MARK: - Reusable UI Components

// 1. 主按钮 (Primary Button)
public final class ThemePrimaryButton: NSButton {
    private var trackingArea: NSTrackingArea?
    private var isHovered: Bool = false

    public override var isEnabled: Bool {
        didSet {
            updateAppearance()
        }
    }

    public override var title: String {
        didSet {
            updateAttributedTitle()
        }
    }

    public init(title: String, target: AnyObject? = nil, action: Selector? = nil) {
        super.init(frame: .zero)
        self.target = target
        self.action = action
        self.title = title
        setup()
    }

    public required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        wantsLayer = true
        isBordered = false
        layer?.cornerRadius = 6
        layer?.masksToBounds = true
        focusRingType = .none
        bezelStyle = .regularSquare
        setButtonType(.momentaryPushIn)
        updateAppearance()
    }

    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
    }

    public override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        guard isEnabled else { return }
        isHovered = true
        updateAppearance()
    }

    public override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        isHovered = false
        updateAppearance()
    }

    private func updateAppearance() {
        if !isEnabled {
            layer?.backgroundColor = Theme.Colors.primaryButtonDisabledBg.cgColor
        } else if isHovered {
            layer?.backgroundColor = Theme.Colors.primaryButtonHover.cgColor
        } else {
            layer?.backgroundColor = Theme.Colors.primaryButtonDefault.cgColor
        }
        updateAttributedTitle()
    }

    private func updateAttributedTitle() {
        let color = isEnabled ? Theme.Colors.primaryButtonText : Theme.Colors.primaryButtonDisabledText
        let font = NSFont.systemFont(ofSize: 13, weight: .medium)
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        let attr = NSAttributedString(string: title, attributes: [
            .foregroundColor: color,
            .font: font,
            .paragraphStyle: paragraph
        ])
        self.attributedTitle = attr
    }

    public override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: max(base.width + 24, 76), height: max(base.height + 10, 32))
    }
}

// 2. 状态标签 (Status Tag)
public final class ThemeStatusTag: NSView {
    public var status: ThemeStatus {
        didSet { updateContent() }
    }
    public var customText: String? {
        didSet { updateContent() }
    }
    public var showsDot: Bool = true {
        didSet {
            dotView.isHidden = !showsDot
            invalidateIntrinsicContentSize()
        }
    }

    private let dotView = NSView()
    private let titleLabel = NSTextField(labelWithString: "")

    public init(status: ThemeStatus, customText: String? = nil, showsDot: Bool = true) {
        self.status = status
        self.customText = customText
        self.showsDot = showsDot
        super.init(frame: .zero)
        setup()
    }

    public required init?(coder: NSCoder) {
        self.status = .normal
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 11
        layer?.masksToBounds = true

        dotView.wantsLayer = true
        dotView.layer?.cornerRadius = 3.5
        dotView.layer?.masksToBounds = true
        dotView.translatesAutoresizingMaskIntoConstraints = false

        titleLabel.isBezeled = false
        titleLabel.isEditable = false
        titleLabel.drawsBackground = false
        titleLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        addSubview(dotView)
        addSubview(titleLabel)

        NSLayoutConstraint.activate([
            dotView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            dotView.centerYAnchor.constraint(equalTo: centerYAnchor),
            dotView.widthAnchor.constraint(equalToConstant: 7),
            dotView.heightAnchor.constraint(equalToConstant: 7),

            titleLabel.leadingAnchor.constraint(equalTo: dotView.trailingAnchor, constant: 6),
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),

            heightAnchor.constraint(equalToConstant: 22)
        ])

        updateContent()
    }

    private func updateContent() {
        layer?.backgroundColor = status.backgroundColor.cgColor
        dotView.layer?.backgroundColor = status.dotColor.cgColor
        titleLabel.textColor = status.textColor
        titleLabel.stringValue = customText ?? status.title
        dotView.isHidden = !showsDot
    }
}

// 3. 状态横幅 (Status Banner)
public final class ThemeStatusBanner: NSView {
    public var status: ThemeStatus {
        didSet { updateAppearance() }
    }

    public var titleText: String {
        didSet { titleLabel.stringValue = titleText }
    }

    public var messageText: String {
        didSet { messageLabel.stringValue = messageText }
    }

    private let indicatorDot = NSView()
    private let titleLabel = NSTextField(wrappingLabelWithString: "")
    private let messageLabel = NSTextField(wrappingLabelWithString: "")
    public private(set) var actionButton: NSButton?

    public init(
        status: ThemeStatus,
        title: String,
        message: String,
        actionTitle: String? = nil,
        actionTarget: AnyObject? = nil,
        actionSelector: Selector? = nil
    ) {
        self.status = status
        self.titleText = title
        self.messageText = message
        if let actionTitle = actionTitle {
            let btn = NSButton(title: actionTitle, target: actionTarget, action: actionSelector)
            btn.bezelStyle = .rounded
            self.actionButton = btn
        } else {
            self.actionButton = nil
        }
        super.init(frame: .zero)
        setup()
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.masksToBounds = true

        indicatorDot.wantsLayer = true
        indicatorDot.layer?.cornerRadius = 4
        indicatorDot.translatesAutoresizingMaskIntoConstraints = false

        titleLabel.font = NSFont.systemFont(ofSize: 13, weight: .bold)
        titleLabel.isEditable = false
        titleLabel.isBezeled = false
        titleLabel.drawsBackground = false
        titleLabel.stringValue = titleText
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        messageLabel.font = NSFont.systemFont(ofSize: 12, weight: .regular)
        messageLabel.isEditable = false
        messageLabel.isBezeled = false
        messageLabel.drawsBackground = false
        messageLabel.stringValue = messageText
        messageLabel.translatesAutoresizingMaskIntoConstraints = false

        let textStack = NSStackView(views: [titleLabel, messageLabel])
        textStack.orientation = .vertical
        textStack.alignment = .leading
        textStack.spacing = 3
        textStack.translatesAutoresizingMaskIntoConstraints = false

        let rowStack = NSStackView()
        rowStack.orientation = .horizontal
        rowStack.alignment = .centerY
        rowStack.spacing = 10
        rowStack.translatesAutoresizingMaskIntoConstraints = false

        rowStack.addArrangedSubview(indicatorDot)
        rowStack.addArrangedSubview(textStack)

        if let btn = actionButton {
            btn.translatesAutoresizingMaskIntoConstraints = false
            rowStack.addArrangedSubview(btn)
        }

        addSubview(rowStack)

        NSLayoutConstraint.activate([
            indicatorDot.widthAnchor.constraint(equalToConstant: 8),
            indicatorDot.heightAnchor.constraint(equalToConstant: 8),

            rowStack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            rowStack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            rowStack.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            rowStack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])

        updateAppearance()
    }

    private func updateAppearance() {
        layer?.backgroundColor = status.backgroundColor.cgColor
        layer?.borderColor = status.dotColor.withAlphaComponent(0.35).cgColor
        indicatorDot.layer?.backgroundColor = status.dotColor.cgColor
        titleLabel.textColor = status.textColor
        messageLabel.textColor = status.textColor
    }
}

// 4. 二次确认弹窗 (Confirm Dialog)
public final class ThemeConfirmDialog: NSObject {
    private let alert: NSAlert
    private let checkbox: NSButton
    private weak var confirmButton: NSButton?

    public init(
        title: String,
        message: String,
        checkboxText: String,
        confirmTitle: String = "确认",
        cancelTitle: String = "取消"
    ) {
        alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = title
        alert.informativeText = message

        let confirmBtn = alert.addButton(withTitle: confirmTitle)
        alert.addButton(withTitle: cancelTitle)

        checkbox = NSButton(checkboxWithTitle: checkboxText, target: nil, action: nil)
        checkbox.state = .off
        checkbox.font = NSFont.systemFont(ofSize: 12)
        checkbox.sizeToFit()

        let container = NSView(frame: NSRect(x: 0, y: 0, width: max(checkbox.bounds.width + 16, 320), height: checkbox.bounds.height + 6))
        checkbox.frame.origin = CGPoint(x: 0, y: 2)
        container.addSubview(checkbox)
        alert.accessoryView = container

        self.confirmButton = confirmBtn
        confirmBtn.isEnabled = false // 初始必须勾选才可确认

        super.init()
        checkbox.target = self
        checkbox.action = #selector(checkboxChanged(_:))
    }

    @objc private func checkboxChanged(_ sender: NSButton) {
        confirmButton?.isEnabled = (sender.state == .on)
    }

    /// 显示模态确认弹窗，只有在用户勾选了复选框并点击确认按钮时才返回 true。
    public func runModal() -> Bool {
        let res = alert.runModal()
        return res == .alertFirstButtonReturn && checkbox.state == .on
    }

    @discardableResult
    public static func show(
        title: String,
        message: String,
        checkboxText: String,
        confirmTitle: String = "确认",
        cancelTitle: String = "取消"
    ) -> Bool {
        let dialog = ThemeConfirmDialog(
            title: title,
            message: message,
            checkboxText: checkboxText,
            confirmTitle: confirmTitle,
            cancelTitle: cancelTitle
        )
        return dialog.runModal()
    }
}

// 5. 自绘描边输入框 (ThemeTextField - 自绘描边与状态切换)
public final class ThemeTextField: NSView {
    public enum State {
        case normal
        case error
        case disabled
    }

    public var state: State = .normal {
        didSet { updateAppearance() }
    }

    public let textField = NSTextField()

    public var stringValue: String {
        get { textField.stringValue }
        set { textField.stringValue = newValue }
    }

    public var placeholderString: String? {
        get { textField.placeholderString }
        set { textField.placeholderString = newValue }
    }

    public var isEditable: Bool {
        get { textField.isEditable }
        set {
            textField.isEditable = newValue
            if !newValue && state != .error {
                state = .disabled
            } else if newValue && state == .disabled {
                state = .normal
            }
        }
    }

    public init(placeholder: String? = nil) {
        super.init(frame: .zero)
        self.placeholderString = placeholder
        setup()
    }

    public required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.borderWidth = 1.0
        layer?.masksToBounds = true

        textField.isBezeled = false
        textField.drawsBackground = false
        textField.focusRingType = .none
        textField.font = NSFont.systemFont(ofSize: 13)
        textField.translatesAutoresizingMaskIntoConstraints = false

        addSubview(textField)

        NSLayoutConstraint.activate([
            textField.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            textField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            textField.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            textField.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32)
        ])

        updateAppearance()
    }

    private func updateAppearance() {
        switch state {
        case .normal:
            layer?.borderColor = Theme.Colors.inputBorder.cgColor
            layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
            textField.textColor = NSColor.controlTextColor
            textField.isEditable = true
        case .error:
            layer?.borderColor = Theme.Colors.inputErrorBorder.cgColor
            layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
            textField.textColor = Theme.Colors.inputErrorText
            textField.isEditable = true
        case .disabled:
            layer?.borderColor = Theme.Colors.inputBorder.cgColor
            layer?.backgroundColor = Theme.Colors.inputDisabledBg.cgColor
            textField.textColor = Theme.Colors.inputDisabledText
            textField.isEditable = false
        }
    }
}

// 6. 表单字段封装 (ThemeFormField - 包含标签、输入框、说明和错误提示)
public final class ThemeFormField: NSView {
    public let label = NSTextField(labelWithString: "")
    public let input: ThemeTextField
    public let helperLabel = NSTextField(wrappingLabelWithString: "")
    public let errorLabel = NSTextField(wrappingLabelWithString: "")

    public init(label title: String, placeholder: String? = nil, helper: String? = nil) {
        self.input = ThemeTextField(placeholder: placeholder)
        super.init(frame: .zero)
        self.label.stringValue = title
        if let helper = helper {
            self.helperLabel.stringValue = helper
        }
        setup()
    }

    public required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func setup() {
        label.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        label.textColor = NSColor.controlTextColor
        label.translatesAutoresizingMaskIntoConstraints = false

        helperLabel.font = NSFont.systemFont(ofSize: 11, weight: .regular)
        helperLabel.textColor = NSColor.secondaryLabelColor
        helperLabel.translatesAutoresizingMaskIntoConstraints = false

        errorLabel.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        errorLabel.textColor = Theme.Colors.inputErrorText
        errorLabel.translatesAutoresizingMaskIntoConstraints = false
        errorLabel.isHidden = true

        input.translatesAutoresizingMaskIntoConstraints = false

        let stack = NSStackView(views: [label, input, errorLabel, helperLabel])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 5
        stack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(stack)

        NSLayoutConstraint.activate([
            input.widthAnchor.constraint(equalTo: stack.widthAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor)
        ])
    }

    public func setNormal(helper: String? = nil) {
        input.state = .normal
        errorLabel.isHidden = true
        if let helper = helper {
            helperLabel.stringValue = helper
        }
        helperLabel.isHidden = helperLabel.stringValue.isEmpty
    }

    public func setError(_ message: String) {
        input.state = .error
        errorLabel.stringValue = message
        errorLabel.isHidden = false
        helperLabel.isHidden = true
    }

    public func setDisabled(helper: String? = nil) {
        input.state = .disabled
        errorLabel.isHidden = true
        if let helper = helper {
            helperLabel.stringValue = helper
        }
        helperLabel.isHidden = helperLabel.stringValue.isEmpty
    }
}
