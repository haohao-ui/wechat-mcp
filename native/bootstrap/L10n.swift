// L10n.swift: 界面文案集中管理（中文 / English / Français / Italiano / Polski）。
//
// 接入方式（其他页面照此迁移）：
//   1. 在 L10nKey 里加一个 case，在 L10n.entry(_:) 的 switch 里给出五种语言的文案
//      （switch 必须穷举，漏翻任何一种语言都会编译失败）。
//   2. 页面里把硬编码字符串换成 L10n.t(.yourKey)；带参数的用 %@ / %d，多个参数用 %1$@ %2$@。
//   3. 不需要自己监听语言切换：切换语言后宿主会重建整个窗口内容（main.swift rebuildShell），
//      页面控制器会重新创建，重新取一次 L10n.t 即可。
//
// 关键用语只在这里定义一次，其他地方一律引用，不要再写字面量：
//   .termPersona（客服角色）、.replyModeBuiltin（内置回复）、.replyModeExternal（外部回复（MCP 接管））、
//   .termSent（已发送）、.sentToday（今日已发送）、.sentNotDelivered（发送不等于送达）。
import Foundation

public enum AppLanguage: String, CaseIterable {
    case zh
    case en
    case fr
    case it
    case pl

    /// 语言自己的名字，不随当前界面语言变化。
    public var displayName: String {
        switch self {
        case .zh: return "中文"
        case .en: return "English"
        case .fr: return "Français"
        case .it: return "Italiano"
        case .pl: return "Polski"
        }
    }
}

public struct L10nEntry {
    public let zh: String
    public let en: String
    public let fr: String
    public let it: String
    public let pl: String

    public func text(_ language: AppLanguage) -> String {
        switch language {
        case .zh: return zh
        case .en: return en
        case .fr: return fr
        case .it: return it
        case .pl: return pl
        }
    }
}

public enum L10nKey: String, CaseIterable {
    case modelPendingTitle, modelDefaultTitle, modelMissingTitle, modelValidationTitle, modelCodexMissing, modelKeyMissing
    case componentsPageSubtitle, componentsSectionColorsTitle, componentsSubsectionButtonsAndBrand, componentsSubsectionInputBorderAndStates, componentsSubsectionStatusTokens, componentsTokenPrimaryButtonDefault, componentsTokenPrimaryButtonHover, componentsTokenPrimaryButtonText, componentsTokenPrimaryButtonDisabledBg, componentsTokenPrimaryButtonDisabledText, componentsTokenBrand, componentsTokenBrandLight, componentsTokenInputBorder, componentsTokenInputErrorBorder, componentsTokenInputErrorText, componentsTokenInputDisabledBg, componentsTokenInputDisabledText, componentsTokenNormalDot, componentsTokenNormalText, componentsTokenNormalBg, componentsTokenWarningDot, componentsTokenWarningText, componentsTokenWarningBg, componentsTokenErrorDot, componentsTokenErrorText, componentsTokenErrorBg, componentsTokenInactiveDot, componentsTokenInactiveText, componentsTokenInactiveBg, componentsSectionComponentsTitle, componentsSubsectionPrimaryButtons, componentsButtonNormalTitle, componentsButtonDisabledTitle, componentsButtonDescription, componentsSubsectionStatusTags, componentsSubsectionStatusBanners, componentsBannerNormalTitle, componentsBannerNormalMsg, componentsBannerWarningTitle, componentsBannerWarningMsg, componentsBannerWarningAction, componentsBannerErrorTitle, componentsBannerErrorMsg, componentsSubsectionConfirmDialog, componentsDialogTriggerButtonTitle, componentsDialogTitle, componentsDialogMessage, componentsDialogCheckboxText, componentsDialogConfirmButton, componentsDialogResultPrefix, componentsDialogResultInitial, componentsDialogResultConfirmed, componentsDialogResultCancelled, componentsSubsectionFormFields, componentsFieldNormalLabel, componentsFieldNormalHelper, componentsFieldErrorLabel, componentsFieldErrorMessage, componentsFieldDisabledLabel, componentsFieldDisabledHelper
    case modelPageScopeNote, modelPageBannerDefaultCodex, modelPageBannerUnset, modelPageBannerPendingRestart, modelPageBannerErrorSummary, modelPageSectionBackend, modelPageBackendCodexDesc, modelPageBackendOpenAIDesc, modelPageSectionConnection, modelPageFieldBaseUrl, modelPageFieldBaseUrlHelper, modelPageFieldModel, modelPageFieldModelPlaceholder, modelPageFieldKeySource, modelPageKeySourceEnv, modelPageKeySourceFile, modelPageKeySourceEnvHelper, modelPageKeySourceFileHelper, modelPageFieldKeyStatus, modelPageKeyStatusEnvNotFound, modelPageKeyNeverExposed, modelPageSectionParameters, modelPageFieldTimeout, modelPageTimeoutHelper, modelPageFieldSupportsImages, modelPageSupportsImagesHelper, modelPageFieldImagesPerCall, modelPageImagesPerCallHelper, modelPageFieldJsonOutput, modelPageJsonOutputHelper, modelPageFieldMaxTokens, modelPageMaxTokensReadOnly, modelPageButtonTest, modelPageButtonDiscard, modelPageTestPassedTitle, modelPageTestPassedNote, modelPageTestFailedTitle, modelPageTestFailedNote, modelPageDialogConfirmTitle, modelPageDialogConfirmBody, modelPageDialogConfirmCheckbox, modelPageDialogConfirmButton, modelPageErrorUrlRequired, modelPageErrorUrlInvalid, modelPageErrorHttpsRequired, modelPageErrorUrlCredentials, modelPageErrorModelRequired, modelPageErrorTimeoutRange, modelPageErrorImagesRange, modelPageErrorEnvRequired, modelPageErrorEnvInvalid, modelPageErrorFileRequired, modelPageErrorFileNotAbsolute, modelPageErrorFilePermission, modelPageErrorFileNotFoundOrAccess, brainPageTabRoleDefinition, brainPageTabKnowledge, brainPageTabTemplates, brainPageTabBindings, brainPageTabHistory, brainPageTabPreview, brainPageModelConfigured, brainPageModelUnconfigured, brainPageGotoModelSettings, brainPageRollbackTitle, brainPageRollbackCheckboxPrompt, brainPageRollbackConfirmButton, brainPageHistoryUnavailable, brainPageLegacyIncompleteDraftNotice, brainPageLegacyIncompleteDraftShort, brainPageMissingFieldIndicator, brainPageDiffSelectHint, brainPageTryItNotice, brainPageSaveDraftFirstNotice, brainPageEmptyHistoryTitle, contactsPageEmptyAll, contactsPageEmptyCategory, contactsPageSelectDetailPrompt, contactsPageSyncStarted, contactsPagePersonaBinding, mcpPageSectionReplyMode, mcpPageSaveSettingsBtn, mcpPageSaveSuccess, mcpPageGoToPersona, mcpPagePointerText, mcpPageRotateTokenTitle, mcpPageRotateTokenInfo, mcpPageOpenApiBusy, mcpPageEmptyEndpoints, mcpPageEmptyTools, logsPageExportLogsTitle, logsPageEmptyLogsTitle, logsPageEmptyFilterTitle, logsPageExportSuccess, logsPageExportFailed, updatePagePageTitle, updatePageSectionRollback, updatePageSectionOperations, updatePageSectionRecovery, updatePageRollbackAppNotice, updatePageNoPreviousVersion, updatePageRollbackCheckboxPrompt, updatePageRollbackConfirmButton, updatePageRollbackSuccess, updatePageRollbackUnavailable, updatePageEmptyCandidate, navComponents
    case legacyActions, legacyConfig, openAttachments, openLogs, copyDiagnostics, allowlist, ignorelist, timeoutAction, skipTurn, defaultPlaceholder, ignorePlaceholder, configUnavailable, confirmBusy, confirmBusyBody, continueAction, actionRequested, actionCancelled, actionFailed, copiedDiagnostics, savingConfig, saveRejected
    // 通用
    case loadMore, unreadCount, recordedOnlyNote
    case appName, quitApp, cancel, loading, valueUnavailable
    // 关键用语（只在这里定义）
    case termPersona, replyModeBuiltin, replyModeExternal, termSent, termAutoReply
    case sentToday, sentNotDelivered, deliveryUnknown
    // 导航
    case navOverview, navSessions, navContacts, navModel, navMcp, navSettings, navUpdate, navLogs
    // 状态标签
    case tagNormal, tagAttention, tagError, tagOff
    // 统一状态源：核心服务卡片 / 横幅 / 需要处理
    case coreServiceTitle
    case cardRunning, cardOffline, cardMissingPermission, cardFolderBlocked, cardModelUnconfigured, cardManuallyPaused
    case bannerOffline, bannerMissingPermission, bannerFolderBlocked, bannerModelUnconfigured, bannerManuallyPaused
    case issuePermissionMissing, issueFolderBlocked
    case permScreen, permAccessibility, permFolders
    case actionRestartCore, actionGrant, actionGotoModel, actionOpenSystemSettings
    case actionResumeAutoReply, actionPauseAutoReply, actionRecheck
    case modelConfigured, modelUnconfigured, modelDefault, modelStateUnknown
    case modelPagePending
    // 总览
    case overviewRunBox, metricListening, metricHandoff, needsActionTitle, needsActionNone
    case checksBox, suggestionSuffix
    case itemPermissions, itemDesktopService, itemBot, itemSessions, itemWechat, itemLayout, itemDisk, itemReplyMode
    case actionRestartDesktopService, actionRestartBot, actionRepositionWechat
    case confirmRestartDesktopTitle, confirmRestartDesktopBody, confirmRestartBotTitle, confirmRestartBotBody, confirmRestart
    // 会话
    case sessionsListBox, sessionsEmpty, kindGroup, kindPrivate
    case modeAuto, modeManual, modeExcluded, modePaused
    case handoffConfirmBody, resumeConfirmBody, handoffChangeFailed
    case actionHandoff, handoffUnsupported, handoffRequested, resumeRequested
    case detailSelectHint, detailUnavailable, detailEmpty, visibleOnlyNote, sessionsApiUnavailable
    case messageFromCustomer, messageFromUs
    case provenanceBox, turnPlaceholder, actionQueryTurn, provenanceHint, turnQuerying, turnNotFound
    case turnDetailTitle, turnContact, turnProvenanceStatus, turnRoleRevision, turnNotRecorded
    case turnModelRef, turnSendKey, turnPhases, turnDecision
    // 会话状态
    case stateHandoff, stateNeedsRecalibration, stateListening, stateReplying
    case stateWaitRowMissing, stateWaitBlocked, stateWaitWindow, stateWindowNotReady
    // 设置
    case settingsPermBox, statusOK, statusMissing, statusBlocked, folderNoProblem
    case settingsLanguageBox, languageLabel, languageNote
    case settingsAccountBox, labelAccount, labelBackend, labelModelName, labelApiKey, modelGlobalNote
    case paceBox, fieldCooldown, fieldMaxReplies, fieldQuiet, fieldTurnMax, fieldContext, fieldAttachment
    case rangeHint, unitSeconds, unitMessages, unitMB
    case manageWindow, settingsMovedNote
    case updateBox, hostVersion, updateIntro, actionGotoUpdate
    case actionSaveConfig, actionRestoreDefaults, savedNeedsRestart, saveFailed, validationError, defaultsLoaded
    case errNotInteger, errWrongType, errOutOfRange
    case externalTimeoutLabel, fallbackToBuiltin
    // 系统检查（StatusModel.evaluate）
    case evPermOK, evPermNone, evPermNoneFix, evPermScreen, evPermScreenFix, evPermAccessibility, evPermAccessibilityFix
    case evFolderBlocked, evFolderFix
    case evDesktopRunning, evDesktopIdle, evDesktopRestarting, evDesktopRestartingFix, evDesktopStopped, evDesktopStoppedFix
    case evBackoffEta
    case evBotConflict, evBotConflictFix, evBotStopping, evBotBackoff, evBotBackoffFix
    case evBotGaveUp, evBotGaveUpFix, evBotConfigError, evBotConfigErrorFix, evBotNotRunning, evBotRestartFix
    case evBotNoHealth, evBotNoHealthFixCore, evBotNoHealthFixPython, evBotStale, evBotStaleFix
    case evBotRecalibrate, evBotRecalibrateFix, evBotPaused, evBotPausedFix, evBotCoreRunning
    case evBotPythonNotLoaded, evBotPythonNotLoadedFix, evBotPythonRunning
    case evSessionsRecalibrate, evSessionsRecalibrateFix, evSessionsHandoff, evSessionsHandoffFix
    case evSessionsWaiting, evSessionsWaitingFix, evSessionsOK
    case evWechatNotRunning, evWechatNotRunningFix, evWechatBlocked, evWechatBlockedFix
    case evWechatSize, evWechatRepositionFix, evWechatReady, evWechatNoWindow, evWechatNoWindowFix
    case evLayoutReady, evLayoutMissing, evLayoutMissingFix
    case evDiskLow, evDiskLowFix, evDiskOK
    case evExternalDisconnected, evExternalConnected, evExternalPending, evExternalFix, evBuiltinRunning
    // 诊断报告
    case diagTitle, diagVersion, diagHost, diagTime
}

public enum L10n {
    public static let defaultsKey = "ui_language"
    public static let didChangeNotification = Notification.Name("WechatHostLanguageDidChange")

    private static var current: AppLanguage = loadPersisted()

    /// 当前界面语言。没有保存过选择时用中文（产品默认语言）。
    public static var language: AppLanguage { current }

    private static func loadPersisted() -> AppLanguage {
        if let raw = UserDefaults.standard.string(forKey: defaultsKey), let lang = AppLanguage(rawValue: raw) {
            return lang
        }
        return .zh
    }

    /// 切换语言。persist=false 只改当前进程（测试用）。
    public static func setLanguage(_ language: AppLanguage, persist: Bool = true) {
        guard language != current else { return }
        current = language
        if persist {
            UserDefaults.standard.set(language.rawValue, forKey: defaultsKey)
        }
        NotificationCenter.default.post(name: didChangeNotification, object: nil)
    }

    public static func t(_ key: L10nKey, _ args: CVarArg...) -> String {
        return format(entry(key).text(current), args)
    }

    public static func text(_ key: L10nKey, in language: AppLanguage, _ args: CVarArg...) -> String {
        return format(entry(key).text(language), args)
    }

    private static func format(_ template: String, _ args: [CVarArg]) -> String {
        if args.isEmpty { return template }
        return String(format: template, arguments: args)
    }

    /// 「自动回复 · 已发送」：由两个关键用语拼出，不单独翻译。
    public static var autoReplySentMark: String {
        return "\(t(.termAutoReply)) · \(t(.termSent))"
    }

    /// 回复方式的两种叫法并列，用于说明文字。
    public static var replyModesPair: String {
        return "\(t(.replyModeBuiltin)) / \(t(.replyModeExternal))"
    }

    private static func e(_ zh: String, _ en: String, _ fr: String, _ it: String, _ pl: String) -> L10nEntry {
        return L10nEntry(zh: zh, en: en, fr: fr, it: it, pl: pl)
    }

    public static func entry(_ key: L10nKey) -> L10nEntry {
        switch key {
        case .modelPendingTitle:
            return e("设置待生效", "Settings pending", "Réglages en attente", "Impostazioni in attesa", "Ustawienia oczekują")
        case .modelDefaultTitle:
            return e("默认接入方式", "Default backend", "Connexion par défaut", "Connessione predefinita", "Domyślne połączenie")
        case .modelMissingTitle:
            return e("模型未配置", "Model not configured", "Modèle non configuré", "Modello non configurato", "Model nieskonfigurowany")
        case .modelValidationTitle:
            return e("连接校验异常", "Connection validation failed", "Validation de connexion échouée", "Validazione connessione fallita", "Weryfikacja połączenia nieudana")
        case .modelCodexMissing:
            return e("未在 PATH 中找到可执行的 codex", "No executable codex found in PATH", "Exécutable codex absent du PATH", "Eseguibile codex non trovato nel PATH", "Nie znaleziono codex w PATH")
        case .modelKeyMissing:
            return e("密钥缺失或读取失败", "Key missing or unreadable", "Clé absente ou illisible", "Chiave assente o illeggibile", "Brak klucza lub nie można go odczytać")

        case .componentsPageSubtitle:
            return e("设计规范、设计令牌与通用组件展示（供验收对照）", "Design tokens and reusable components for acceptance review", "Jetons de conception et composants pour la vérification", "Token di design e componenti per la verifica", "Tokeny projektu i komponenty do odbioru")
        case .componentsSectionColorsTitle:
            return e("色彩规范 (Color Tokens)", "Color tokens", "Jetons de couleur", "Token colore", "Tokeny kolorów")
        case .componentsSubsectionButtonsAndBrand:
            return e("主按钮与品牌色", "Primary button and brand", "Bouton principal et marque", "Pulsante principale e marchio", "Przycisk główny i marka")
        case .componentsSubsectionInputBorderAndStates:
            return e("输入框描边与状态色", "Input borders and states", "Bordures et états des champs", "Bordi e stati dei campi", "Obramowania i stany pól")
        case .componentsSubsectionStatusTokens:
            return e("四种服务状态色 (圆点 / 文字 / 底色)", "Four service states (dot / text / background)", "Quatre états du service (point / texte / fond)", "Quattro stati del servizio (punto / testo / sfondo)", "Cztery stany usługi (kropka / tekst / tło)")
        case .componentsTokenPrimaryButtonDefault:
            return e("主按钮默认背景 (#0A5B6B)", "Primary button background (#0A5B6B)", "Fond du bouton principal (#0A5B6B)", "Sfondo pulsante principale (#0A5B6B)", "Tło przycisku głównego (#0A5B6B)")
        case .componentsTokenPrimaryButtonHover:
            return e("主按钮悬停背景 (#084B58)", "Primary button hover (#084B58)", "Survol du bouton principal (#084B58)", "Passaggio pulsante principale (#084B58)", "Najechanie przycisku głównego (#084B58)")
        case .componentsTokenPrimaryButtonText:
            return e("主按钮文字/白字 (#FFFFFF)", "Primary button text (#FFFFFF)", "Texte du bouton principal (#FFFFFF)", "Testo pulsante principale (#FFFFFF)", "Tekst przycisku głównego (#FFFFFF)")
        case .componentsTokenPrimaryButtonDisabledBg:
            return e("主按钮禁用背景 (#F4F5F7)", "Disabled button background (#F4F5F7)", "Fond du bouton désactivé (#F4F5F7)", "Sfondo pulsante disabilitato (#F4F5F7)", "Tło wyłączonego przycisku (#F4F5F7)")
        case .componentsTokenPrimaryButtonDisabledText:
            return e("主按钮禁用文字 (#6B7280)", "Disabled button text (#6B7280)", "Texte du bouton désactivé (#6B7280)", "Testo pulsante disabilitato (#6B7280)", "Tekst wyłączonego przycisku (#6B7280)")
        case .componentsTokenBrand:
            return e("品牌主色 (#12A37A)", "Brand (#12A37A)", "Marque (#12A37A)", "Marchio (#12A37A)", "Marka (#12A37A)")
        case .componentsTokenBrandLight:
            return e("品牌浅色 (#E2F5EE)", "Light brand (#E2F5EE)", "Marque claire (#E2F5EE)", "Marchio chiaro (#E2F5EE)", "Jasna marka (#E2F5EE)")
        case .componentsTokenInputBorder:
            return e("输入框默认描边 (#C9CDD4)", "Input border (#C9CDD4)", "Bordure du champ (#C9CDD4)", "Bordo campo (#C9CDD4)", "Obramowanie pola (#C9CDD4)")
        case .componentsTokenInputErrorBorder:
            return e("输入框错误态描边 (#E5484D)", "Input error border (#E5484D)", "Bordure de champ en erreur (#E5484D)", "Bordo campo errore (#E5484D)", "Obramowanie błędnego pola (#E5484D)")
        case .componentsTokenInputErrorText:
            return e("输入框错误态文字 (#B73A3E)", "Input error text (#B73A3E)", "Texte de champ en erreur (#B73A3E)", "Testo campo errore (#B73A3E)", "Tekst błędnego pola (#B73A3E)")
        case .componentsTokenInputDisabledBg:
            return e("输入框禁用态背景 (#F4F5F7)", "Disabled input background (#F4F5F7)", "Fond de champ désactivé (#F4F5F7)", "Sfondo campo disabilitato (#F4F5F7)", "Tło wyłączonego pola (#F4F5F7)")
        case .componentsTokenInputDisabledText:
            return e("输入框禁用态文字 (#6B7280)", "Disabled input text (#6B7280)", "Texte de champ désactivé (#6B7280)", "Testo campo disabilitato (#6B7280)", "Tekst wyłączonego pola (#6B7280)")
        case .componentsTokenNormalDot:
            return e("正常状态圆点 (#1AAD78)", "Normal · dot (#1AAD78)", "Normal · point (#1AAD78)", "Normale · punto (#1AAD78)", "Normalny · kropka (#1AAD78)")
        case .componentsTokenNormalText:
            return e("正常状态文字 (#127954)", "Normal · text (#127954)", "Normal · texte (#127954)", "Normale · testo (#127954)", "Normalny · tekst (#127954)")
        case .componentsTokenNormalBg:
            return e("正常状态底色 (#E4F5EF)", "Normal · background (#E4F5EF)", "Normal · fond (#E4F5EF)", "Normale · sfondo (#E4F5EF)", "Normalny · tło (#E4F5EF)")
        case .componentsTokenWarningDot:
            return e("注意状态圆点 (#E08E0B)", "Warning · dot (#E08E0B)", "Attention · point (#E08E0B)", "Attenzione · punto (#E08E0B)", "Ostrzeżenie · kropka (#E08E0B)")
        case .componentsTokenWarningText:
            return e("注意状态文字 (#925C07)", "Warning · text (#925C07)", "Attention · texte (#925C07)", "Attenzione · testo (#925C07)", "Ostrzeżenie · tekst (#925C07)")
        case .componentsTokenWarningBg:
            return e("注意状态底色 (#FBF1E2)", "Warning · background (#FBF1E2)", "Attention · fond (#FBF1E2)", "Attenzione · sfondo (#FBF1E2)", "Ostrzeżenie · tło (#FBF1E2)")
        case .componentsTokenErrorDot:
            return e("异常状态圆点 (#E5484D)", "Error · dot (#E5484D)", "Erreur · point (#E5484D)", "Errore · punto (#E5484D)", "Błąd · kropka (#E5484D)")
        case .componentsTokenErrorText:
            return e("异常状态文字 (#B73A3E)", "Error · text (#B73A3E)", "Erreur · texte (#B73A3E)", "Errore · testo (#B73A3E)", "Błąd · tekst (#B73A3E)")
        case .componentsTokenErrorBg:
            return e("异常状态底色 (#FCE9EA)", "Error · background (#FCE9EA)", "Erreur · fond (#FCE9EA)", "Errore · sfondo (#FCE9EA)", "Błąd · tło (#FCE9EA)")
        case .componentsTokenInactiveDot:
            return e("未启用圆点 (#9AA0A6)", "Inactive · dot (#9AA0A6)", "Inactif · point (#9AA0A6)", "Inattivo · punto (#9AA0A6)", "Nieaktywny · kropka (#9AA0A6)")
        case .componentsTokenInactiveText:
            return e("未启用文字 (#6C7074)", "Inactive · text (#6C7074)", "Inactif · texte (#6C7074)", "Inattivo · testo (#6C7074)", "Nieaktywny · tekst (#6C7074)")
        case .componentsTokenInactiveBg:
            return e("未启用底色 (#F3F4F4)", "Inactive · background (#F3F4F4)", "Inactif · fond (#F3F4F4)", "Inattivo · sfondo (#F3F4F4)", "Nieaktywny · tło (#F3F4F4)")
        case .componentsSectionComponentsTitle:
            return e("通用组件 (Components)", "Reusable components", "Composants réutilisables", "Componenti riutilizzabili", "Komponenty wielokrotnego użytku")
        case .componentsSubsectionPrimaryButtons:
            return e("主按钮 (Primary Button)", "Primary button", "Bouton principal", "Pulsante principale", "Przycisk główny")
        case .componentsButtonNormalTitle:
            return e("保存修改 (悬停体验)", "Save changes (hover)", "Enregistrer (survol)", "Salva modifiche (passaggio)", "Zapisz zmiany (najechanie)")
        case .componentsButtonDisabledTitle:
            return e("保存修改 (禁用状态)", "Save changes (disabled)", "Enregistrer (désactivé)", "Salva modifiche (disabilitato)", "Zapisz zmiany (wyłączone)")
        case .componentsButtonDescription:
            return e("支持悬停 (#084B58) 变色、点击高亮与禁用态自动变灰。", "Hover color #084B58, click highlight and automatic disabled styling.", "Survol #084B58, surbrillance au clic et style désactivé automatique.", "Colore al passaggio #084B58, evidenziazione al clic e stile disabilitato automatico.", "Kolor po najechaniu #084B58, podświetlenie kliknięcia i styl wyłączenia.")
        case .componentsSubsectionStatusTags:
            return e("状态标签 (Status Tag)", "Status tags", "Étiquettes d'état", "Etichette di stato", "Etykiety stanu")
        case .componentsSubsectionStatusBanners:
            return e("状态横幅 (Status Banner)", "Status banners", "Bannières d'état", "Banner di stato", "Banery stanu")
        case .componentsBannerNormalTitle:
            return e("核心服务运行中", "Core service running", "Cœur en cours d'exécution", "Servizio principale attivo", "Rdzeń działa")
        case .componentsBannerNormalMsg:
            return e("微信客服助手正在后台持续监听消息并按规则自动回复。", "The assistant monitors messages and replies automatically according to your rules.", "L'assistant surveille les messages et répond automatiquement selon vos règles.", "L'assistente monitora i messaggi e risponde secondo le regole.", "Asystent monitoruje wiadomości i odpowiada zgodnie z regułami.")
        case .componentsBannerWarningTitle:
            return e("重启后生效提示", "Restart required", "Redémarrage requis", "Riavvio richiesto", "Wymagane ponowne uruchomienie")
        case .componentsBannerWarningMsg:
            return e("已保存的连接设置尚未生效。正在运行的服务仍使用旧设置，重启核心服务后生效。", "Saved settings are pending. The running service uses old settings until restarted.", "Réglages en attente. Le service utilise les anciens réglages jusqu'au redémarrage.", "Impostazioni in attesa. Il servizio usa quelle precedenti fino al riavvio.", "Ustawienia oczekują. Usługa używa starych do ponownego uruchomienia.")
        case .componentsBannerWarningAction:
            return e("重启核心服务", "Restart core service", "Redémarrer le cœur", "Riavvia servizio principale", "Uruchom ponownie rdzeń")
        case .componentsBannerErrorTitle:
            return e("连接校验异常", "Connection validation failed", "Validation de connexion échouée", "Validazione connessione fallita", "Weryfikacja połączenia nieudana")
        case .componentsBannerErrorMsg:
            return e("有 2 项配置需要修改，改好后才能测试连接或保存。", "Fix 2 configuration issues before testing or saving.", "Corrigez 2 problèmes avant de tester ou d'enregistrer.", "Correggi 2 problemi prima di testare o salvare.", "Popraw 2 problemy przed testem lub zapisem.")
        case .componentsSubsectionConfirmDialog:
            return e("带勾选二次确认弹窗 (Confirm Dialog)", "Confirmation dialog with checkbox", "Confirmation avec case à cocher", "Conferma con casella", "Potwierdzenie z polem wyboru")
        case .componentsDialogTriggerButtonTitle:
            return e("打开带勾选二次确认弹窗...", "Open confirmation dialog…", "Ouvrir la confirmation…", "Apri conferma…", "Otwórz potwierdzenie…")
        case .componentsDialogTitle:
            return e("确认修改连接设置？", "Confirm connection changes?", "Confirmer les modifications ?", "Confermare le modifiche?", "Potwierdzić zmiany połączenia?")
        case .componentsDialogMessage:
            return e("自动回复正在运行。你修改了：接口地址、密钥来源。保存后，正在运行的服务仍使用旧设置，重启核心服务后才生效。", "Auto-reply is running. API URL and key source changed. Restart the core service after saving to apply changes.", "La réponse automatique est active. Adresse API et source de clé modifiées. Redémarrez le cœur après l'enregistrement.", "Risposta automatica attiva. URL API e origine chiave modificati. Riavvia il servizio dopo il salvataggio.", "Odpowiedzi automatyczne działają. Zmieniono adres API i źródło klucza. Po zapisie uruchom rdzeń ponownie.")
        case .componentsDialogCheckboxText:
            return e("我知道重启核心服务后才会生效", "I understand that a core service restart is required", "Je comprends qu'un redémarrage du cœur est requis", "Comprendo che è necessario riavviare il servizio", "Rozumiem, że wymagany jest restart rdzenia")
        case .componentsDialogConfirmButton:
            return e("保存并生效", "Save settings", "Enregistrer", "Salva impostazioni", "Zapisz ustawienia")
        case .componentsDialogResultPrefix:
            return e("最近一次交互结果：", "Last interaction:", "Dernière interaction :", "Ultima interazione:", "Ostatnia interakcja:")
        case .componentsDialogResultInitial:
            return e("尚未打开弹窗", "Dialog not opened yet", "Dialogue pas encore ouvert", "Finestra non ancora aperta", "Okno jeszcze nieotwarte")
        case .componentsDialogResultConfirmed:
            return e("用户已勾选确认并点击保存", "Confirmed with checkbox and saved", "Confirmé avec la case et enregistré", "Confermato con casella e salvato", "Potwierdzono polem wyboru i zapisano")
        case .componentsDialogResultCancelled:
            return e("用户取消或未勾选", "Cancelled or checkbox not selected", "Annulé ou case non cochée", "Annullato o casella non selezionata", "Anulowano lub nie zaznaczono pola")
        case .componentsSubsectionFormFields:
            return e("表单字段 (Form Field - 自绘描边)", "Form fields (custom borders)", "Champs (bordures personnalisées)", "Campi (bordi personalizzati)", "Pola (własne obramowania)")
        case .componentsFieldNormalLabel:
            return e("模型接口地址 (正常态)", "Model API URL (normal)", "Adresse API du modèle (normal)", "URL API modello (normale)", "Adres API modelu (normalny)")
        case .componentsFieldNormalHelper:
            return e("支持 HTTPS 或本机地址 localhost / 127.0.0.1 / [::1]", "HTTPS or localhost / 127.0.0.1 / [::1]", "HTTPS ou localhost / 127.0.0.1 / [::1]", "HTTPS o localhost / 127.0.0.1 / [::1]", "HTTPS lub localhost / 127.0.0.1 / [::1]")
        case .componentsFieldErrorLabel:
            return e("API 密钥来源 (错误态)", "API key source (error)", "Source de clé API (erreur)", "Origine chiave API (errore)", "Źródło klucza API (błąd)")
        case .componentsFieldErrorMessage:
            return e("环境变量名不合法，仅允许字母、数字和下划线", "Invalid environment variable name; letters, digits and underscores only", "Nom de variable invalide ; lettres, chiffres et tirets bas uniquement", "Nome variabile non valido; solo lettere, cifre e trattini bassi", "Nieprawidłowa nazwa zmiennej; tylko litery, cyfry i podkreślenia")
        case .componentsFieldDisabledLabel:
            return e("最大输出长度 (禁用态)", "Maximum output length (disabled)", "Longueur maximale (désactivé)", "Lunghezza massima (disabilitato)", "Maksymalna długość (wyłączone)")
        case .componentsFieldDisabledHelper:
            return e("固定参数，由核心引擎严格限制，当前角色只读", "Fixed parameter enforced by the core engine; read-only for this role", "Paramètre fixe imposé par le cœur ; lecture seule pour ce rôle", "Parametro fisso imposto dal motore; sola lettura per questo ruolo", "Stały parametr rdzenia; tylko odczyt dla tej roli")

        case .modelPageScopeNote:
            return e("全局配置 · 所有客服角色共用", "Global settings · shared by all support personas", "Réglages globaux · communs à toutes les personas", "Impostazioni globali · condivise da tutte le persone di assistenza", "Ustawienia globalne · wspólne dla wszystkich person obsługi")
        case .modelPageBannerDefaultCodex:
            return e("未设置兼容接口，当前使用默认接入方式 codex-exec。", "No compatible API configured; using codex-exec.", "Aucune API compatible configurée ; utilisation de codex-exec.", "Nessuna API compatibile configurata; uso di codex-exec.", "Brak zgodnego API; używany jest codex-exec.")
        case .modelPageBannerUnset:
            return e("模型未配置：已选择 openai-compatible，但缺少接口地址、模型名或密钥。自动回复无法生成回复。", "Model unavailable: API URL, model name or key is missing. Auto-reply cannot generate replies.", "Modèle indisponible : adresse API, nom ou clé manquant. Les réponses automatiques ne peuvent pas être générées.", "Modello non disponibile: mancano URL API, nome o chiave. Impossibile generare risposte automatiche.", "Model niedostępny: brak adresu API, nazwy lub klucza. Nie można generować odpowiedzi automatycznych.")
        case .modelPageBannerPendingRestart:
            return e("已保存的连接设置尚未生效，重启核心服务后生效。", "Saved connection settings take effect after restarting the core service.", "Les réglages enregistrés prennent effet après le redémarrage du cœur.", "Le impostazioni salvate hanno effetto dopo il riavvio del servizio principale.", "Zapisane ustawienia zaczną działać po ponownym uruchomieniu rdzenia.")
        case .modelPageBannerErrorSummary:
            return e("有 %d 项配置需要修改，改好后才能测试连接或保存。", "Fix %d configuration issues before testing or saving.", "Corrigez %d problèmes avant de tester ou d'enregistrer.", "Correggi %d problemi prima di testare o salvare.", "Popraw %d problemów przed testem lub zapisem.")
        case .modelPageSectionBackend:
            return e("接入方式", "Backend", "Mode de connexion", "Modalità di connessione", "Sposób połączenia")
        case .modelPageBackendCodexDesc:
            return e("使用本机 Codex 命令行调用模型，不需要填写接口地址、模型名和密钥。", "Use the local Codex CLI. No API URL, model name or key is required.", "Utilisez Codex CLI localement. Aucune adresse API, nom ou clé requis.", "Usa Codex CLI locale. Non servono URL API, nome o chiave.", "Używa lokalnego Codex CLI. Adres API, nazwa i klucz nie są wymagane.")
        case .modelPageBackendOpenAIDesc:
            return e("调用兼容 OpenAI 接口的模型服务，需要填写接口地址、模型名和密钥来源。", "Use an OpenAI-compatible API; provide its URL, model name and key source.", "Utilisez une API compatible OpenAI avec son adresse, le nom du modèle et la source de clé.", "Usa un'API compatibile OpenAI con URL, nome del modello e origine della chiave.", "Używa API zgodnego z OpenAI; podaj adres, nazwę modelu i źródło klucza.")
        case .modelPageSectionConnection:
            return e("连接", "Connection", "Connexion", "Connessione", "Połączenie")
        case .modelPageFieldBaseUrl:
            return e("接口地址", "API URL", "Adresse API", "URL API", "Adres API")
        case .modelPageFieldBaseUrlHelper:
            return e("需 HTTPS；本机地址（localhost / 127.0.0.1 / [::1]）可用 HTTP。不能含用户名、密码或 ? 参数。", "HTTPS required; localhost / 127.0.0.1 / [::1] may use HTTP. No credentials or query parameters.", "HTTPS requis ; localhost / 127.0.0.1 / [::1] autorisent HTTP. Sans identifiants ni paramètres de requête.", "Richiesto HTTPS; localhost / 127.0.0.1 / [::1] consentono HTTP. Senza credenziali o parametri di query.", "Wymagany HTTPS; localhost / 127.0.0.1 / [::1] mogą używać HTTP. Bez danych logowania i parametrów zapytania.")
        case .modelPageFieldModel:
            return e("模型名", "Model name", "Nom du modèle", "Nome del modello", "Nazwa modelu")
        case .modelPageFieldModelPlaceholder:
            return e("例如 gpt-4o-mini", "For example gpt-4o-mini", "Par exemple gpt-4o-mini", "Ad esempio gpt-4o-mini", "Na przykład gpt-4o-mini")
        case .modelPageFieldKeySource:
            return e("密钥来源", "Key source", "Source de la clé", "Origine della chiave", "Źródło klucza")
        case .modelPageKeySourceEnv:
            return e("环境变量", "Environment variable", "Variable d'environnement", "Variabile d'ambiente", "Zmienna środowiskowa")
        case .modelPageKeySourceFile:
            return e("密钥文件", "Key file", "Fichier de clé", "File della chiave", "Plik klucza")
        case .modelPageKeySourceEnvHelper:
            return e("填变量名，不填密钥本身。例如 OPENAI_API_KEY", "Enter the variable name, never the key. Example: OPENAI_API_KEY", "Saisissez le nom de variable, jamais la clé. Exemple : OPENAI_API_KEY", "Inserisci il nome della variabile, mai la chiave. Esempio: OPENAI_API_KEY", "Podaj nazwę zmiennej, nigdy klucz. Przykład: OPENAI_API_KEY")
        case .modelPageKeySourceFileHelper:
            return e("绝对路径；文件权限须为 0600，不能是符号链接。", "Absolute path; permissions must be 0600. Symbolic links are not allowed.", "Chemin absolu ; permissions 0600 requises. Liens symboliques interdits.", "Percorso assoluto; permessi 0600 obbligatori. Link simbolici non consentiti.", "Ścieżka bezwzględna; uprawnienia 0600. Dowiązania symboliczne są niedozwolone.")
        case .modelPageFieldKeyStatus:
            return e("密钥状态", "Key status", "État de la clé", "Stato della chiave", "Stan klucza")
        case .modelPageKeyStatusEnvNotFound:
            return e("（当前环境中未检测到该变量）", "(Variable not found in this environment)", "(Variable absente de cet environnement)", "(Variabile non trovata nell'ambiente)", "(Nie znaleziono zmiennej w środowisku)")
        case .modelPageKeyNeverExposed:
            return e("密钥只显示配置状态或掩码，保存后永远不显示明文，也不写进可见日志。", "Only key status or a mask is shown. Plaintext keys are never displayed or written to visible logs.", "Seul l'état ou un masque est affiché. Les clés ne sont jamais affichées en clair ni inscrites dans les journaux visibles.", "Si mostra solo lo stato o una maschera. Le chiavi non sono mai mostrate in chiaro o nei registri visibili.", "Wyświetlany jest tylko stan lub maska. Klucze nie są pokazywane ani zapisywane jawnie w widocznych dziennikach.")
        case .modelPageSectionParameters:
            return e("参数设置", "Parameters", "Paramètres", "Parametri", "Parametry")
        case .modelPageFieldTimeout:
            return e("超时时间", "Timeout", "Délai d'attente", "Timeout", "Limit czasu")
        case .modelPageTimeoutHelper:
            return e("1–180 秒，默认 60 秒", "1–180 seconds; default 60", "1–180 secondes ; défaut 60", "1–180 secondi; predefinito 60", "1–180 sekund; domyślnie 60")
        case .modelPageFieldSupportsImages:
            return e("支持图片输入", "Image input", "Entrée d'images", "Input immagini", "Obsługa obrazów")
        case .modelPageSupportsImagesHelper:
            return e("模型能读取图片时开启；关闭后只发送文字。", "Enable only for models that read images. Otherwise only text is sent.", "Activez uniquement pour un modèle lisant les images. Sinon, seul le texte est envoyé.", "Abilita solo per modelli che leggono immagini. Altrimenti si invia solo testo.", "Włącz tylko dla modeli odczytujących obrazy. W przeciwnym razie wysyłany jest tylko tekst.")
        case .modelPageFieldImagesPerCall:
            return e("单次最多图片数", "Maximum images per call", "Images maximum par appel", "Immagini massime per chiamata", "Maksymalna liczba obrazów na wywołanie")
        case .modelPageImagesPerCallHelper:
            return e("1–16 张，默认 4 张（仅在开启图片输入时生效）", "1–16 images; default 4 (only when image input is enabled)", "1–16 images ; défaut 4 (si l'entrée d'images est activée)", "1–16 immagini; predefinito 4 (con input immagini abilitato)", "1–16 obrazów; domyślnie 4 (gdy obrazy są włączone)")
        case .modelPageFieldJsonOutput:
            return e("要求 JSON 格式输出", "Require JSON output", "Exiger une sortie JSON", "Richiedi output JSON", "Wymagaj odpowiedzi JSON")
        case .modelPageJsonOutputHelper:
            return e("要求模型只返回 JSON", "Ask the model to return JSON only", "Demander uniquement du JSON", "Richiedi al modello solo JSON", "Model ma zwracać tylko JSON")
        case .modelPageFieldMaxTokens:
            return e("单次生成上限 (max_tokens)", "Output limit (max_tokens)", "Limite de sortie (max_tokens)", "Limite output (max_tokens)", "Limit odpowiedzi (max_tokens)")
        case .modelPageMaxTokensReadOnly:
            return e("1024（由系统固定分配，不可修改）", "1024 (fixed by the system; read-only)", "1024 (fixé par le système ; lecture seule)", "1024 (fisso dal sistema; sola lettura)", "1024 (ustalone przez system; tylko odczyt)")
        case .modelPageButtonTest:
            return e("测试连接", "Test connection", "Tester la connexion", "Prova connessione", "Testuj połączenie")
        case .modelPageButtonDiscard:
            return e("放弃修改", "Discard changes", "Annuler les modifications", "Annulla modifiche", "Odrzuć zmiany")
        case .modelPageTestPassedTitle:
            return e("测试通过：接口与模型列表可用", "Test passed: API and model list available", "Test réussi : API et liste des modèles disponibles", "Test superato: API ed elenco modelli disponibili", "Test udany: API i lista modeli są dostępne")
        case .modelPageTestPassedNote:
            return e("测试通过只说明接口和模型列表可用，不代表对话一定能生成。", "This confirms API and model-list access only; reply generation may still fail.", "Cela confirme uniquement l'accès à l'API et aux modèles ; la génération peut encore échouer.", "Conferma solo l'accesso ad API e modelli; la generazione può ancora fallire.", "Potwierdza tylko dostęp do API i modeli; generowanie odpowiedzi nadal może się nie udać.")
        case .modelPageTestFailedTitle:
            return e("连接测试失败", "Connection test failed", "Échec du test de connexion", "Test connessione fallito", "Test połączenia nieudany")
        case .modelPageTestFailedNote:
            return e("请检查接口地址、模型名或密钥权限是否正确。测试失败不显示密钥或供应商原始报错。", "Check the API URL, model name and key permissions. Keys and raw provider errors are hidden.", "Vérifiez l'adresse API, le modèle et les permissions de clé. Les clés et erreurs brutes sont masquées.", "Verifica URL API, modello e permessi della chiave. Chiavi ed errori originali sono nascosti.", "Sprawdź adres API, model i uprawnienia klucza. Klucze i surowe błędy dostawcy są ukryte.")
        case .modelPageDialogConfirmTitle:
            return e("保存连接设置？", "Save connection settings?", "Enregistrer la connexion ?", "Salvare la connessione?", "Zapisać ustawienia połączenia?")
        case .modelPageDialogConfirmBody:
            return e("自动回复正在运行。你修改了：%@。保存后，正在运行的服务仍使用旧设置，重启核心服务后才生效。", "Auto-reply is running. Changed: %@. The service keeps using old settings until the core service restarts.", "La réponse automatique est active. Modifications : %@. Les anciens réglages restent actifs jusqu'au redémarrage du cœur.", "Risposta automatica attiva. Modifiche: %@. Il servizio usa le vecchie impostazioni fino al riavvio.", "Odpowiedzi automatyczne działają. Zmiany: %@. Stare ustawienia pozostają aktywne do ponownego uruchomienia rdzenia.")
        case .modelPageDialogConfirmCheckbox:
            return e("我知道重启核心服务后才会生效", "I understand that a core service restart is required", "Je comprends qu'un redémarrage du cœur est requis", "Comprendo che è necessario riavviare il servizio principale", "Rozumiem, że wymagane jest ponowne uruchomienie rdzenia")
        case .modelPageDialogConfirmButton:
            return e("保存", "Save", "Enregistrer", "Salva", "Zapisz")
        case .modelPageErrorUrlRequired:
            return e("请填写接口地址", "Enter an API URL", "Saisissez l'adresse API", "Inserisci un URL API", "Podaj adres API")
        case .modelPageErrorUrlInvalid:
            return e("不是有效的网址", "Invalid URL", "Adresse invalide", "URL non valido", "Nieprawidłowy adres")
        case .modelPageErrorHttpsRequired:
            return e("必须使用 HTTPS（只有本机地址可用 HTTP）", "HTTPS required except for local addresses", "HTTPS requis sauf pour les adresses locales", "HTTPS richiesto tranne per indirizzi locali", "HTTPS wymagany poza adresami lokalnymi")
        case .modelPageErrorUrlCredentials:
            return e("地址不能包含用户名、密码或 ? 参数", "URL cannot include credentials or query parameters", "L'adresse ne doit contenir ni identifiants ni paramètres", "L'URL non può contenere credenziali o parametri", "Adres nie może zawierać danych logowania ani parametrów")
        case .modelPageErrorModelRequired:
            return e("请填写模型名", "Enter a model name", "Saisissez le nom du modèle", "Inserisci il nome del modello", "Podaj nazwę modelu")
        case .modelPageErrorTimeoutRange:
            return e("请输入 1–180 之间的整数", "Enter an integer from 1 to 180", "Saisissez un entier de 1 à 180", "Inserisci un intero da 1 a 180", "Podaj liczbę całkowitą od 1 do 180")
        case .modelPageErrorImagesRange:
            return e("请输入 1–16 之间的整数", "Enter an integer from 1 to 16", "Saisissez un entier de 1 à 16", "Inserisci un intero da 1 a 16", "Podaj liczbę całkowitą od 1 do 16")
        case .modelPageErrorEnvRequired:
            return e("请填写环境变量名", "Enter an environment variable name", "Saisissez un nom de variable", "Inserisci un nome di variabile", "Podaj nazwę zmiennej")
        case .modelPageErrorEnvInvalid:
            return e("变量名只能包含字母、数字和下划线，且不能以数字开头", "Use letters, digits and underscores; do not start with a digit", "Lettres, chiffres et tirets bas ; ne commencez pas par un chiffre", "Usa lettere, cifre e trattini bassi; non iniziare con una cifra", "Użyj liter, cyfr i podkreśleń; nie zaczynaj od cyfry")
        case .modelPageErrorFileRequired:
            return e("请填写密钥文件路径", "Enter the key file path", "Saisissez le chemin du fichier de clé", "Inserisci il percorso del file della chiave", "Podaj ścieżkę pliku klucza")
        case .modelPageErrorFileNotAbsolute:
            return e("必须是绝对路径（以 / 开头，不支持 ~）", "Use an absolute path starting with /; ~ is unsupported", "Chemin absolu commençant par / ; ~ non pris en charge", "Percorso assoluto che inizia con /; ~ non supportato", "Ścieżka bezwzględna od /; ~ nie jest obsługiwane")
        case .modelPageErrorFilePermission:
            return e("文件权限须为 0600，且不能是符号链接", "Permissions must be 0600; symbolic links are forbidden", "Permissions 0600 requises ; liens symboliques interdits", "Permessi 0600 richiesti; link simbolici vietati", "Wymagane uprawnienia 0600; dowiązania symboliczne są zabronione")
        case .modelPageErrorFileNotFoundOrAccess:
            return e("密钥文件无法读取或不存在", "Key file unreadable or missing", "Fichier de clé illisible ou absent", "File della chiave illeggibile o assente", "Plik klucza nieczytelny lub nie istnieje")
        case .brainPageTabRoleDefinition:
            return e("角色设定", "Role definition", "Définition du rôle", "Definizione del ruolo", "Definicja roli")
        case .brainPageTabKnowledge:
            return e("知识", "Knowledge", "Connaissances", "Conoscenze", "Wiedza")
        case .brainPageTabTemplates:
            return e("异常回复", "Fallback replies", "Réponses de secours", "Risposte di ripiego", "Odpowiedzi awaryjne")
        case .brainPageTabBindings:
            return e("适用会话", "Conversations", "Conversations", "Conversazioni", "Rozmowy")
        case .brainPageTabHistory:
            return e("历史版本", "Version history", "Historique des versions", "Cronologia versioni", "Historia wersji")
        case .brainPageTabPreview:
            return e("试一试", "Try it", "Essayer", "Prova", "Wypróbuj")
        case .brainPageModelConfigured:
            return e("模型：已配置", "Model: configured", "Modèle : configuré", "Modello: configurato", "Model: skonfigurowano")
        case .brainPageModelUnconfigured:
            return e("模型：未配置", "Model: not configured", "Modèle : non configuré", "Modello: non configurato", "Model: nie skonfigurowano")
        case .brainPageGotoModelSettings:
            return e("配置模型…", "Configure model…", "Configurer le modèle…", "Configura modello…", "Skonfiguruj model…")
        case .brainPageRollbackTitle:
            return e("回滚到历史版本", "Restore a historical version", "Restaurer une ancienne version", "Ripristina versione precedente", "Przywróć wersję historyczną")
        case .brainPageRollbackCheckboxPrompt:
            return e("我已知晓回滚将恢复至选定版本并覆盖当前运行配置", "I understand that restoring this version replaces the active configuration", "Je comprends que la restauration remplace la configuration active", "Comprendo che il ripristino sostituisce la configurazione attiva", "Rozumiem, że przywrócenie zastąpi aktywną konfigurację")
        case .brainPageRollbackConfirmButton:
            return e("确认回滚", "Confirm restore", "Confirmer la restauration", "Conferma ripristino", "Potwierdź przywrócenie")
        case .brainPageHistoryUnavailable:
            return e("版本历史暂不可用", "Version history unavailable", "Historique indisponible", "Cronologia non disponibile", "Historia niedostępna")
        case .brainPageLegacyIncompleteDraftNotice:
            return e("此版本未保存完整草稿，仅显示已保存的内容", "This version has no complete draft; only saved content is shown", "Cette version n'a pas de brouillon complet ; seul le contenu enregistré est affiché", "Questa versione non ha una bozza completa; si mostra solo il contenuto salvato", "Ta wersja nie ma pełnego szkicu; pokazano tylko zapisaną treść")
        case .brainPageLegacyIncompleteDraftShort:
            return e("未保存完整草稿 (仅已存内容)", "Incomplete draft (saved content only)", "Brouillon incomplet (contenu enregistré uniquement)", "Bozza incompleta (solo contenuto salvato)", "Niepełny szkic (tylko zapisana treść)")
        case .brainPageMissingFieldIndicator:
            return e("[缺失]", "[Missing]", "[Manquant]", "[Mancante]", "[Brak]")
        case .brainPageDiffSelectHint:
            return e("请选择两个版本进行对比", "Select two versions to compare", "Sélectionnez deux versions à comparer", "Seleziona due versioni da confrontare", "Wybierz dwie wersje do porównania")
        case .brainPageTryItNotice:
            return e("合成测试仅执行确定性组装预览 (assemble_only)，标明【未调用模型】，不产生费用，绝不发送真实微信消息。", "Deterministic assembly preview only (assemble_only). No model call, no charges and no real WeChat messages.", "Aperçu déterministe uniquement (assemble_only). Aucun appel au modèle, aucun coût ni message WeChat réel.", "Solo anteprima deterministica (assemble_only). Nessuna chiamata al modello, costo o messaggio WeChat reale.", "Tylko deterministyczny podgląd (assemble_only). Bez wywołania modelu, opłat i prawdziwych wiadomości WeChat.")
        case .brainPageSaveDraftFirstNotice:
            return e("请先保存客服角色草稿。请先保存角色草稿，再执行「试一试」。", "Save the persona draft before trying it", "Enregistrez le brouillon avant d'essayer", "Salva la bozza prima di provare", "Zapisz szkic przed próbą")
        case .brainPageEmptyHistoryTitle:
            return e("暂无多版本历史记录", "No version history yet", "Aucun historique pour le moment", "Nessuna cronologia disponibile", "Brak historii wersji")
        case .contactsPageEmptyAll:
            return e("暂无联系人数据，可点击上方【初始化通讯录】或【快速同步】拉取数据", "No contacts yet. Initialize or sync the address book above.", "Aucun contact. Initialisez ou synchronisez le carnet ci-dessus.", "Nessun contatto. Inizializza o sincronizza la rubrica qui sopra.", "Brak kontaktów. Zainicjuj lub zsynchronizuj książkę powyżej.")
        case .contactsPageEmptyCategory:
            return e("当前分类下暂无联系人", "No contacts in this category", "Aucun contact dans cette catégorie", "Nessun contatto in questa categoria", "Brak kontaktów w tej kategorii")
        case .contactsPageSelectDetailPrompt:
            return e("选择左侧联系人查看详细信息", "Select a contact on the left to view details", "Sélectionnez un contact à gauche pour voir les détails", "Seleziona un contatto a sinistra per i dettagli", "Wybierz kontakt po lewej, aby zobaczyć szczegóły")
        case .contactsPageSyncStarted:
            return e("正在启动通讯录同步...", "Starting address book sync…", "Démarrage de la synchronisation…", "Avvio sincronizzazione rubrica…", "Uruchamianie synchronizacji kontaktów…")
        case .contactsPagePersonaBinding:
            return e("绑定客服角色", "Bind support persona", "Associer une persona du support", "Associa persona di assistenza", "Przypisz personę obsługi")
        case .mcpPageSectionReplyMode:
            return e("回复方式与决策", "Reply mode and decisions", "Mode de réponse et décisions", "Modalità risposta e decisioni", "Tryb odpowiedzi i decyzje")
        case .mcpPageSaveSettingsBtn:
            return e("保存回复设置", "Save reply settings", "Enregistrer les réponses", "Salva impostazioni risposte", "Zapisz ustawienia odpowiedzi")
        case .mcpPageSaveSuccess:
            return e("回复设置已保存", "Reply settings saved", "Réglages de réponse enregistrés", "Impostazioni risposte salvate", "Ustawienia odpowiedzi zapisane")
        case .mcpPageGoToPersona:
            return e("前往客服角色页面", "Open support persona", "Ouvrir la persona du support", "Apri persona di assistenza", "Otwórz personę obsługi")
        case .mcpPagePointerText:
            return e("角色定义、知识与话术请前往【客服角色】页面管理；本页面仅管理回复模式、外部超时与对外接口连接。", "Manage roles, knowledge and templates on the support persona page. This page manages reply mode, external timeouts and API connections.", "Gérez rôles, connaissances et modèles sur la page de persona. Cette page gère le mode, les délais et les connexions API.", "Gestisci ruoli, conoscenze e modelli nella pagina persona. Qui si gestiscono modalità, timeout e connessioni API.", "Zarządzaj rolami, wiedzą i szablonami na stronie persony. Ta strona zarządza trybem, limitami czasu i połączeniami API.")
        case .mcpPageRotateTokenTitle:
            return e("重新生成对外接口访问令牌？", "Regenerate the API access token?", "Régénérer le jeton d'accès API ?", "Rigenerare il token API?", "Wygenerować ponownie token API?")
        case .mcpPageRotateTokenInfo:
            return e("重新生成后将立即生效，当前连接的外部服务将会断开。确定重新生成吗？", "This takes effect immediately and disconnects external services. Continue?", "L'effet est immédiat et déconnecte les services externes. Continuer ?", "Ha effetto immediato e disconnette i servizi esterni. Continuare?", "Zmiana jest natychmiastowa i rozłączy usługi zewnętrzne. Kontynuować?")
        case .mcpPageOpenApiBusy:
            return e("已有客户端连接中", "A client is already connected", "Un client est déjà connecté", "Un client è già connesso", "Klient jest już połączony")
        case .mcpPageEmptyEndpoints:
            return e("暂无可用服务端点", "No service endpoints available", "Aucun point de terminaison disponible", "Nessun endpoint disponibile", "Brak dostępnych punktów końcowych")
        case .mcpPageEmptyTools:
            return e("暂无可用 MCP 工具", "No MCP tools available", "Aucun outil MCP disponible", "Nessuno strumento MCP disponibile", "Brak dostępnych narzędzi MCP")
        case .logsPageExportLogsTitle:
            return e("导出日志...", "Export logs…", "Exporter les journaux…", "Esporta registri…", "Eksportuj dzienniki…")
        case .logsPageEmptyLogsTitle:
            return e("当前暂无日志记录", "No log entries yet", "Aucune entrée de journal", "Nessuna voce nel registro", "Brak wpisów w dzienniku")
        case .logsPageEmptyFilterTitle:
            return e("无匹配日志，请尝试更换筛选条件", "No matching logs; try another filter", "Aucun résultat ; essayez un autre filtre", "Nessun risultato; prova un altro filtro", "Brak wyników; spróbuj innego filtra")
        case .logsPageExportSuccess:
            return e("日志已导出", "Logs exported", "Journaux exportés", "Registri esportati", "Dzienniki wyeksportowane")
        case .logsPageExportFailed:
            return e("导出日志失败", "Log export failed", "Échec de l'export des journaux", "Esportazione registri fallita", "Eksport dzienników nieudany")
        case .updatePagePageTitle:
            return e("应用版本更新", "Application updates", "Mises à jour de l'application", "Aggiornamenti applicazione", "Aktualizacje aplikacji")
        case .updatePageSectionRollback:
            return e("回滚到上一版本", "Restore previous version", "Restaurer la version précédente", "Ripristina versione precedente", "Przywróć poprzednią wersję")
        case .updatePageSectionOperations:
            return e("操作与维护", "Operations and maintenance", "Opérations et maintenance", "Operazioni e manutenzione", "Operacje i konserwacja")
        case .updatePageSectionRecovery:
            return e("故障恢复指引", "Recovery guide", "Guide de récupération", "Guida al ripristino", "Instrukcja odzyskiwania")
        case .updatePageRollbackAppNotice:
            return e("换回上一个已安装的版本", "Replace with the previously installed version", "Remplacer par la version précédemment installée", "Sostituisci con la versione installata precedente", "Zastąp poprzednio zainstalowaną wersją")
        case .updatePageNoPreviousVersion:
            return e("无可回滚的上一版本", "No previous version available", "Aucune version précédente disponible", "Nessuna versione precedente disponibile", "Brak poprzedniej wersji")
        case .updatePageRollbackCheckboxPrompt:
            return e("我已知晓将换回上一个已安装的版本并覆盖当前程序", "I understand that this replaces the application with the previously installed version", "Je comprends que cela remplace l'application par la version précédente", "Comprendo che sostituisce l'applicazione con la versione precedente", "Rozumiem, że zastąpi to aplikację poprzednią wersją")
        case .updatePageRollbackConfirmButton:
            return e("确认回滚", "Confirm rollback", "Confirmer le retour arrière", "Conferma ripristino", "Potwierdź przywrócenie")
        case .updatePageRollbackSuccess:
            return e("已成功换回上一个已安装的版本，需要重启应用以完成替换", "Previous version restored. Restart the application to finish.", "Version précédente restaurée. Redémarrez l'application pour terminer.", "Versione precedente ripristinata. Riavvia l'applicazione per completare.", "Przywrócono poprzednią wersję. Uruchom aplikację ponownie, aby zakończyć.")
        case .updatePageRollbackUnavailable:
            return e("回滚功能暂不可用", "Rollback unavailable", "Retour arrière indisponible", "Ripristino non disponibile", "Przywrócenie niedostępne")
        case .updatePageEmptyCandidate:
            return e("当前已是最新版本，未检测到可用更新", "Already up to date; no update found", "À jour ; aucune mise à jour trouvée", "Già aggiornato; nessun aggiornamento trovato", "Aktualna wersja; brak aktualizacji")
        case .navComponents:
            return e("组件与配色", "Components and colors", "Composants et couleurs", "Componenti e colori", "Komponenty i kolory")

        case .legacyActions:
            return e("一键操作", "Actions", "Actions", "Azioni", "Działania")
        case .legacyConfig:
            return e("常用配置", "Common settings", "Réglages courants", "Impostazioni comuni", "Ustawienia ogólne")
        case .openAttachments:
            return e("打开附件文件夹", "Open attachments folder", "Ouvrir le dossier des pièces jointes", "Apri cartella allegati", "Otwórz folder załączników")
        case .openLogs:
            return e("打开日志文件夹", "Open logs folder", "Ouvrir les journaux", "Apri cartella registri", "Otwórz folder dzienników")
        case .copyDiagnostics:
            return e("复制诊断信息", "Copy diagnostics", "Copier le diagnostic", "Copia diagnostica", "Kopiuj diagnostykę")
        case .allowlist:
            return e("允许名单 (allowlist):", "Allowlist:", "Liste autorisée :", "Lista consentiti:", "Lista dozwolonych:")
        case .ignorelist:
            return e("忽略名单 (ignorelist):", "Ignore list:", "Liste ignorée :", "Lista ignorati:", "Lista ignorowanych:")
        case .timeoutAction:
            return e("超时后:", "On timeout:", "À expiration :", "Alla scadenza:", "Po przekroczeniu czasu:")
        case .skipTurn:
            return e("这一轮不回复", "Skip this turn", "Ignorer ce tour", "Salta questo turno", "Pomiń tę turę")
        case .defaultPlaceholder:
            return e("留空使用默认值 %@", "Leave blank for default %@", "Vide pour la valeur par défaut %@", "Vuoto per il valore predefinito %@", "Puste dla wartości domyślnej %@")
        case .ignorePlaceholder:
            return e("每行一个：名称, private 或 group", "One per line: name, private or group", "Un par ligne : nom, private ou group", "Uno per riga: nome, private o group", "Jeden na wiersz: nazwa, private lub group")
        case .configUnavailable:
            return e("无法读取配置文件，无法保存。", "Cannot read the configuration file; saving unavailable.", "Configuration illisible ; enregistrement indisponible.", "Configurazione non leggibile; salvataggio non disponibile.", "Nie można odczytać konfiguracji ani jej zapisać.")
        case .confirmBusy:
            return e("机器人正在处理消息", "The bot is processing messages", "Le bot traite des messages", "Il bot sta elaborando messaggi", "Bot przetwarza wiadomości")
        case .confirmBusyBody:
            return e("执行「%@」可能打断当前处理，是否继续？", "%@ may interrupt processing. Continue?", "%@ peut interrompre le traitement. Continuer ?", "%@ può interrompere l’elaborazione. Continuare?", "%@ może przerwać przetwarzanie. Kontynuować?")
        case .continueAction:
            return e("继续%@", "Continue %@", "Continuer %@", "Continua %@", "Kontynuuj %@")
        case .actionRequested:
            return e("已请求执行操作，请检查最新状态。", "Action requested. Check the latest status.", "Action demandée. Vérifiez l’état actuel.", "Azione richiesta. Controlla lo stato attuale.", "Zażądano działania. Sprawdź aktualny stan.")
        case .actionCancelled:
            return e("操作已取消", "Action cancelled", "Action annulée", "Azione annullata", "Działanie anulowane")
        case .actionFailed:
            return e("操作未完成，请检查配置和服务状态。", "Action did not complete. Check configuration and service status.", "Action non terminée. Vérifiez la configuration et le service.", "Azione non completata. Controlla configurazione e servizio.", "Nie ukończono działania. Sprawdź konfigurację i usługę.")
        case .copiedDiagnostics:
            return e("诊断信息已复制到剪贴板", "Diagnostics copied to clipboard", "Diagnostic copié", "Diagnostica copiata", "Skopiowano diagnostykę")
        case .savingConfig:
            return e("正在保存并校验配置…", "Saving and validating settings…", "Enregistrement et validation…", "Salvataggio e convalida…", "Zapisywanie i sprawdzanie…")
        case .saveRejected:
            return e("配置保存被拒绝", "Settings were not saved", "Réglages non enregistrés", "Impostazioni non salvate", "Nie zapisano ustawień")
        case .loadMore:
            return e("加载更多", "Load more", "Charger plus", "Carica altro", "Wczytaj więcej")
        case .unreadCount:
            return e("未读 %d", "%d unread", "%d non lus", "%d non letti", "%d nieprzeczytanych")
        case .recordedOnlyNote:
            return e("仅显示已记录的消息；发送不等于送达。", "Only recorded messages; sent does not mean delivered.", "Messages enregistrés uniquement ; envoyé ne signifie pas remis.", "Solo messaggi registrati; inviato non significa consegnato.", "Tylko zapisane wiadomości; wysłano nie oznacza dostarczono.")
        // MARK: 通用
        case .appName:
            return e("微信客服助手", "WeChat Support Assistant", "Assistant de support WeChat", "Assistente di supporto WeChat", "Asystent obsługi WeChat")
        case .quitApp:
            return e("退出微信客服助手", "Quit WeChat Support Assistant", "Quitter l'Assistant de support WeChat", "Esci da Assistente di supporto WeChat", "Zakończ Asystenta obsługi WeChat")
        case .cancel:
            return e("取消", "Cancel", "Annuler", "Annulla", "Anuluj")
        case .loading:
            return e("正在加载…", "Loading…", "Chargement…", "Caricamento…", "Wczytywanie…")
        case .valueUnavailable:
            return e("暂无数据", "No data yet", "Pas encore de données", "Nessun dato", "Brak danych")

        // MARK: 关键用语
        case .termPersona:
            return e("客服角色", "Support persona", "Persona du support", "Persona dell'assistenza", "Persona obsługi klienta")
        case .replyModeBuiltin:
            return e("内置回复", "Built-in reply", "Réponse intégrée", "Risposta integrata", "Odpowiedź wbudowana")
        case .replyModeExternal:
            return e("外部回复（MCP 接管）", "External reply (MCP takeover)", "Réponse externe (prise en charge MCP)", "Risposta esterna (gestita da MCP)", "Odpowiedź zewnętrzna (przejęcie przez MCP)")
        case .termSent:
            return e("已发送", "Sent", "Envoyé", "Inviato", "Wysłano")
        case .termAutoReply:
            return e("自动回复", "Auto-reply", "Réponse automatique", "Risposta automatica", "Automatyczna odpowiedź")
        case .sentToday:
            return e("今日已发送", "Sent today", "Envoyés aujourd'hui", "Inviati oggi", "Wysłane dziś")
        case .sentNotDelivered:
            return e("发送不等于送达", "Sent does not mean delivered", "« Envoyé » ne veut pas dire « remis »", "Inviato non significa consegnato", "Wysłano nie oznacza dostarczono")
        case .deliveryUnknown:
            return e("送达未知", "Delivery unknown", "Remise inconnue", "Consegna sconosciuta", "Dostarczenie nieznane")

        // MARK: 导航
        case .navOverview:
            return e("总览", "Overview", "Vue d'ensemble", "Panoramica", "Przegląd")
        case .navSessions:
            return e("会话", "Conversations", "Conversations", "Conversazioni", "Rozmowy")
        case .navContacts:
            return e("联系人", "Contacts", "Contacts", "Contatti", "Kontakty")
        case .navModel:
            return e("模型", "Model", "Modèle", "Modello", "Model")
        case .navMcp:
            return e("MCP 接入", "MCP access", "Accès MCP", "Accesso MCP", "Dostęp MCP")
        case .navSettings:
            return e("设置", "Settings", "Réglages", "Impostazioni", "Ustawienia")
        case .navUpdate:
            return e("应用更新", "App updates", "Mises à jour de l'application", "Aggiornamenti dell'app", "Aktualizacje aplikacji")
        case .navLogs:
            return e("日志", "Logs", "Journaux", "Registri", "Dzienniki")

        // MARK: 状态标签
        case .tagNormal:
            return e("正常", "Normal", "Normal", "Normale", "Prawidłowy")
        case .tagAttention:
            return e("注意", "Attention", "Attention", "Attenzione", "Uwaga")
        case .tagError:
            return e("异常", "Error", "Erreur", "Errore", "Błąd")
        case .tagOff:
            return e("未启用", "Not enabled", "Non activé", "Non attivo", "Nieaktywne")

        // MARK: 统一状态源
        case .coreServiceTitle:
            return e("核心服务", "Core service", "Service principal", "Servizio principale", "Usługa główna")
        case .cardRunning:
            return e("运行中 · 自动回复已开启", "Running · auto-reply on", "En service · réponse automatique active", "In esecuzione · risposta automatica attiva", "Działa · automatyczne odpowiedzi włączone")
        case .cardOffline:
            return e("离线 · 自动回复已暂停", "Offline · auto-reply paused", "Hors ligne · réponse automatique en pause", "Offline · risposta automatica in pausa", "Offline · automatyczne odpowiedzi wstrzymane")
        case .cardMissingPermission:
            return e("缺少%@权限 · 自动回复已暂停", "Missing permission: %@ · auto-reply paused", "Autorisation manquante : %@ · réponse automatique en pause", "Autorizzazione mancante: %@ · risposta automatica in pausa", "Brak uprawnienia: %@ · automatyczne odpowiedzi wstrzymane")
        case .cardFolderBlocked:
            return e("附件文件夹未授权 · 图片和文件无法保存", "Attachment folder not authorized · images and files cannot be saved", "Dossier des pièces jointes non autorisé · images et fichiers non enregistrés", "Cartella allegati non autorizzata · immagini e file non salvati", "Brak dostępu do folderu załączników · obrazy i pliki nie są zapisywane")
        case .cardModelUnconfigured:
            return e("模型未配置 · 自动回复已暂停", "Model not configured · auto-reply paused", "Modèle non configuré · réponse automatique en pause", "Modello non configurato · risposta automatica in pausa", "Model nieskonfigurowany · automatyczne odpowiedzi wstrzymane")
        case .cardManuallyPaused:
            return e("自动回复已手动暂停", "Auto-reply paused manually", "Réponse automatique mise en pause manuellement", "Risposta automatica messa in pausa manualmente", "Automatyczne odpowiedzi wstrzymane ręcznie")
        case .bannerOffline:
            return e("核心服务离线，自动回复已暂停。新消息不会自动发送。", "The core service is offline and auto-reply is paused. New messages will not be answered automatically.", "Le service principal est hors ligne et la réponse automatique est en pause. Aucune réponse ne sera envoyée automatiquement.", "Il servizio principale è offline e la risposta automatica è in pausa. I nuovi messaggi non riceveranno risposta automatica.", "Usługa główna jest offline, a automatyczne odpowiedzi są wstrzymane. Nowe wiadomości nie otrzymają automatycznej odpowiedzi.")
        case .bannerMissingPermission:
            return e("缺少%@权限，自动回复已暂停。", "Missing permission: %@. Auto-reply is paused.", "Autorisation manquante : %@. La réponse automatique est en pause.", "Autorizzazione mancante: %@. La risposta automatica è in pausa.", "Brak uprawnienia: %@. Automatyczne odpowiedzi są wstrzymane.")
        case .bannerFolderBlocked:
            return e("附件保存所在文件夹未授权（%@），图片和文件无法保存。", "The attachment folder is not authorized (%@). Images and files cannot be saved.", "Le dossier des pièces jointes n'est pas autorisé (%@). Les images et fichiers ne peuvent pas être enregistrés.", "La cartella degli allegati non è autorizzata (%@). Immagini e file non possono essere salvati.", "Brak dostępu do folderu załączników (%@). Obrazy i pliki nie mogą zostać zapisane.")
        case .bannerModelUnconfigured:
            return e("模型未配置，自动回复无法生成回复。", "The model is not configured, so auto-reply cannot generate replies.", "Le modèle n'est pas configuré : la réponse automatique ne peut pas générer de réponses.", "Il modello non è configurato: la risposta automatica non può generare risposte.", "Model nie jest skonfigurowany, więc automatyczne odpowiedzi nie mogą być generowane.")
        case .bannerManuallyPaused:
            return e("自动回复已手动暂停。", "Auto-reply was paused manually.", "La réponse automatique a été mise en pause manuellement.", "La risposta automatica è stata messa in pausa manualmente.", "Automatyczne odpowiedzi zostały wstrzymane ręcznie.")
        case .issuePermissionMissing:
            return e("%@：缺失", "%@: missing", "%@ : manquante", "%@: mancante", "%@: brak")
        case .issueFolderBlocked:
            return e("%@：受阻", "%@: blocked", "%@ : bloqué", "%@: bloccato", "%@: zablokowany")
        case .permScreen:
            return e("屏幕录制", "Screen recording", "Enregistrement de l'écran", "Registrazione schermo", "Nagrywanie ekranu")
        case .permAccessibility:
            return e("辅助功能", "Accessibility", "Accessibilité", "Accessibilità", "Dostępność")
        case .permFolders:
            return e("文件夹访问", "Folder access", "Accès aux dossiers", "Accesso alle cartelle", "Dostęp do folderów")
        case .actionRestartCore:
            return e("重启核心服务", "Restart core service", "Redémarrer le service principal", "Riavvia il servizio principale", "Uruchom ponownie usługę główną")
        case .actionGrant:
            return e("前往授权", "Grant access", "Autoriser", "Autorizza", "Przyznaj dostęp")
        case .actionGotoModel:
            return e("前往「模型」", "Go to Model", "Ouvrir « Modèle »", "Vai a «Modello»", "Przejdź do „Model”")
        case .actionOpenSystemSettings:
            return e("打开系统设置", "Open System Settings", "Ouvrir les Réglages Système", "Apri Impostazioni di Sistema", "Otwórz Ustawienia systemowe")
        case .actionResumeAutoReply:
            return e("恢复自动回复", "Resume auto-reply", "Reprendre la réponse automatique", "Riprendi risposta automatica", "Wznów automatyczne odpowiedzi")
        case .actionPauseAutoReply:
            return e("暂停自动回复", "Pause auto-reply", "Suspendre la réponse automatique", "Sospendi risposta automatica", "Wstrzymaj automatyczne odpowiedzi")
        case .actionRecheck:
            return e("重新检查", "Check again", "Vérifier à nouveau", "Controlla di nuovo", "Sprawdź ponownie")
        case .modelConfigured:
            return e("已配置", "Configured", "Configuré", "Configurato", "Skonfigurowano")
        case .modelUnconfigured:
            return e("未配置", "Not configured", "Non configuré", "Non configurato", "Nie skonfigurowano")
        case .modelDefault:
            return e("默认", "Default", "Par défaut", "Predefinito", "Domyślny")
        case .modelStateUnknown:
            return e("状态未知", "Status unknown", "État inconnu", "Stato sconosciuto", "Stan nieznany")
        case .modelPagePending:
            return e("「模型」页正在接入，接入后在这里设置全局模型参数。", "The Model page is being connected. Global model settings will appear here.", "La page Modèle est en cours d'intégration. Les réglages globaux du modèle apparaîtront ici.", "La pagina Modello è in fase di integrazione. Le impostazioni globali del modello appariranno qui.", "Strona Model jest w trakcie podłączania. Globalne ustawienia modelu pojawią się tutaj.")

        // MARK: 总览
        case .overviewRunBox:
            return e("运行概览", "At a glance", "En un coup d'œil", "In sintesi", "W skrócie")
        case .metricListening:
            return e("监听会话", "Watched conversations", "Conversations surveillées", "Conversazioni monitorate", "Obserwowane rozmowy")
        case .metricHandoff:
            return e("待人工", "Waiting for a person", "En attente d'un humain", "In attesa di una persona", "Czeka na człowieka")
        case .needsActionTitle:
            return e("需要处理", "Needs attention", "À traiter", "Da gestire", "Wymaga działania")
        case .needsActionNone:
            return e("没有需要处理的事项。", "Nothing needs your attention.", "Rien à traiter.", "Niente da gestire.", "Nic nie wymaga działania.")
        case .checksBox:
            return e("系统检查与诊断", "System checks", "Vérifications du système", "Controlli di sistema", "Kontrole systemu")
        case .suggestionSuffix:
            return e("（建议: %@）", " (Suggestion: %@)", " (Conseil : %@)", " (Suggerimento: %@)", " (Sugestia: %@)")
        case .itemPermissions:
            return e("权限", "Permissions", "Autorisations", "Autorizzazioni", "Uprawnienia")
        case .itemDesktopService:
            return e("桌面服务", "Desktop service", "Service de bureau", "Servizio desktop", "Usługa pulpitu")
        case .itemBot:
            return e("机器人", "Bot", "Bot", "Bot", "Bot")
        case .itemSessions:
            return e("会话", "Conversations", "Conversations", "Conversazioni", "Rozmowy")
        case .itemWechat:
            return e("微信", "WeChat", "WeChat", "WeChat", "WeChat")
        case .itemLayout:
            return e("布局", "Layout", "Disposition", "Layout", "Układ")
        case .itemDisk:
            return e("磁盘", "Disk", "Disque", "Disco", "Dysk")
        case .itemReplyMode:
            return e("回复方式", "Reply mode", "Mode de réponse", "Modalità di risposta", "Tryb odpowiedzi")
        case .actionRestartDesktopService:
            return e("重启桌面服务", "Restart desktop service", "Redémarrer le service de bureau", "Riavvia il servizio desktop", "Uruchom ponownie usługę pulpitu")
        case .actionRestartBot:
            return e("重启机器人", "Restart bot", "Redémarrer le bot", "Riavvia il bot", "Uruchom ponownie bota")
        case .actionRepositionWechat:
            return e("放回标准位置", "Reset window position", "Replacer la fenêtre", "Ripristina posizione finestra", "Przywróć pozycję okna")
        case .confirmRestartDesktopTitle:
            return e("确认重启桌面服务？", "Restart the desktop service?", "Redémarrer le service de bureau ?", "Riavviare il servizio desktop?", "Uruchomić ponownie usługę pulpitu?")
        case .confirmRestartDesktopBody:
            return e("重启期间可能会短暂中断正在进行的屏幕与微信交互。", "Screen and WeChat interaction in progress may be interrupted briefly during the restart.", "Les interactions en cours avec l'écran et WeChat peuvent être brièvement interrompues pendant le redémarrage.", "Durante il riavvio le interazioni in corso con lo schermo e WeChat possono interrompersi brevemente.", "Podczas ponownego uruchamiania trwające interakcje z ekranem i WeChat mogą zostać na chwilę przerwane.")
        case .confirmRestartBotTitle:
            return e("确认重启机器人？", "Restart the bot?", "Redémarrer le bot ?", "Riavviare il bot?", "Uruchomić ponownie bota?")
        case .confirmRestartBotBody:
            return e("重启机器人将重新初始化核心进程及所有监听任务。", "Restarting the bot re-initializes the core process and all watch tasks.", "Le redémarrage du bot réinitialise le processus principal et toutes les tâches de surveillance.", "Il riavvio del bot reinizializza il processo principale e tutte le attività di monitoraggio.", "Ponowne uruchomienie bota inicjuje od nowa proces główny i wszystkie zadania obserwacji.")
        case .confirmRestart:
            return e("确认重启", "Restart", "Redémarrer", "Riavvia", "Uruchom ponownie")

        // MARK: 会话
        case .sessionsListBox:
            return e("会话列表", "Conversation list", "Liste des conversations", "Elenco conversazioni", "Lista rozmów")
        case .sessionsEmpty:
            return e("暂无活跃会话", "No active conversations", "Aucune conversation active", "Nessuna conversazione attiva", "Brak aktywnych rozmów")
        case .kindGroup:
            return e("群聊", "Group", "Groupe", "Gruppo", "Grupa")
        case .kindPrivate:
            return e("私聊", "Private", "Privé", "Privata", "Prywatna")
        case .modeAuto:
            return e("自动回复中", "Auto-replying", "Réponse auto active", "Risposta auto attiva", "Auto-odpowiedzi")
        case .modeManual:
            return e("人工处理", "Handled by a person", "Traitée par un humain", "Gestita da una persona", "Obsługuje człowiek")
        case .modeExcluded:
            return e("已排除", "Excluded", "Exclue", "Esclusa", "Wykluczona")
        case .modePaused:
            return e("已暂停", "Paused", "En pause", "In pausa", "Wstrzymane")
        case .actionHandoff:
            return e("转人工", "Hand over to a person", "Passer à un humain", "Passa a una persona", "Przekaż człowiekowi")
        case .handoffConfirmBody:
            return e("将 %@ 转人工？此会话会停止自动回复，直到手动恢复。已经发起的发送无法撤回。", "Hand %@ over to a person? Auto-replies stop until resumed. A send already started cannot be recalled.", "Passer %@ à un humain ? Les réponses automatiques restent arrêtées jusqu’à la reprise. Un envoi déjà lancé ne peut être annulé.", "Passare %@ a una persona? Le risposte automatiche restano ferme fino alla ripresa. Un invio già avviato non può essere annullato.", "Przekazać %@ człowiekowi? Automatyczne odpowiedzi zostaną zatrzymane do wznowienia. Rozpoczętej wysyłki nie można cofnąć.")
        case .resumeConfirmBody:
            return e("为 %@ 恢复自动回复？仍需核心服务在线且满足会话规则。", "Resume auto-replies for %@? The core must be online and conversation rules still apply.", "Reprendre les réponses automatiques pour %@ ? Le service doit être en ligne et les règles restent applicables.", "Riprendere le risposte automatiche per %@? Il servizio deve essere online e le regole restano valide.", "Wznowić automatyczne odpowiedzi dla %@? Usługa musi być online, a reguły rozmowy nadal obowiązują.")
        case .handoffChangeFailed:
            return e("未能确认会话状态是否已更改。请刷新查看后再试。", "The conversation change could not be confirmed. Refresh before trying again.", "Le changement n’a pas pu être confirmé. Actualisez avant de réessayer.", "La modifica non è stata confermata. Aggiorna prima di riprovare.", "Nie można potwierdzić zmiany. Odśwież przed ponowną próbą.")
        case .handoffUnsupported:
            return e("当前核心服务不支持从界面转人工，这个会话没有变化。", "The current core service cannot hand a conversation over from this screen. Nothing was changed.", "Le service principal actuel ne permet pas de passer la main depuis cet écran. Rien n'a été modifié.", "Il servizio principale attuale non consente il passaggio a una persona da questa schermata. Nulla è stato modificato.", "Bieżąca usługa główna nie pozwala przekazać rozmowy z tego ekranu. Nic nie zostało zmienione.")
        case .handoffRequested:
            return e("已请求转人工：%@", "Hand-over requested: %@", "Passage à un humain demandé : %@", "Passaggio a una persona richiesto: %@", "Poproszono o przekazanie człowiekowi: %@")
        case .resumeRequested:
            return e("已请求恢复自动回复：%@", "Auto-reply resume requested: %@", "Reprise de la réponse automatique demandée : %@", "Ripresa della risposta automatica richiesta: %@", "Poproszono o wznowienie automatycznych odpowiedzi: %@")
        case .detailSelectHint:
            return e("选择会话查看消息。", "Select a conversation to see its messages.", "Sélectionnez une conversation pour voir ses messages.", "Seleziona una conversazione per vederne i messaggi.", "Wybierz rozmowę, aby zobaczyć wiadomości.")
        case .detailUnavailable:
            return e("暂时拿不到这个会话的消息内容。", "Messages for this conversation are not available right now.", "Les messages de cette conversation ne sont pas disponibles pour le moment.", "I messaggi di questa conversazione non sono disponibili al momento.", "Wiadomości tej rozmowy są teraz niedostępne.")
        case .detailEmpty:
            return e("这个会话还没有可显示的消息。", "This conversation has no messages to show yet.", "Cette conversation n'a pas encore de messages à afficher.", "Questa conversazione non ha ancora messaggi da mostrare.", "Ta rozmowa nie ma jeszcze wiadomości do wyświetlenia.")
        case .visibleOnlyNote:
            return e("仅显示可见消息", "Only visible messages are shown", "Seuls les messages visibles sont affichés", "Sono mostrati solo i messaggi visibili", "Pokazywane są tylko widoczne wiadomości")
        case .sessionsApiUnavailable:
            return e("会话数据接口暂不可用，下面只显示监听状态。", "Conversation data is not available right now. Only watch status is shown below.", "Les données des conversations sont indisponibles pour le moment. Seul l'état de surveillance est affiché ci-dessous.", "I dati delle conversazioni non sono disponibili al momento. Sotto è mostrato solo lo stato di monitoraggio.", "Dane rozmów są teraz niedostępne. Poniżej widać tylko stan obserwacji.")
        case .messageFromCustomer:
            return e("客户", "Customer", "Client", "Cliente", "Klient")
        case .messageFromUs:
            return e("我方", "Us", "Nous", "Noi", "My")
        case .provenanceBox:
            return e("对话轮次追溯", "Turn provenance", "Traçabilité des tours", "Tracciabilità dei turni", "Śledzenie tur")
        case .turnPlaceholder:
            return e("输入 turn_id 查询详细追溯", "Enter a turn_id to look up its provenance", "Saisissez un turn_id pour consulter sa traçabilité", "Inserisci un turn_id per consultarne la tracciabilità", "Wpisz turn_id, aby sprawdzić jego pochodzenie")
        case .actionQueryTurn:
            return e("查询轮次", "Look up turn", "Rechercher le tour", "Cerca turno", "Wyszukaj turę")
        case .provenanceHint:
            return e("暂无选中轮次。请在上方输入框手动输入有效 turn_id 并点击查询，以查看详细快照、角色修订与发送状态。", "No turn selected. Enter a valid turn_id above and look it up to see its snapshot, persona revision and send status.", "Aucun tour sélectionné. Saisissez un turn_id valide ci-dessus puis lancez la recherche pour voir l'instantané, la révision du persona et l'état d'envoi.", "Nessun turno selezionato. Inserisci sopra un turn_id valido e avvia la ricerca per vedere l'istantanea, la revisione della persona e lo stato di invio.", "Nie wybrano tury. Wpisz powyżej prawidłowy turn_id i wyszukaj, aby zobaczyć migawkę, wersję persony i stan wysyłki.")
        case .turnQuerying:
            return e("正在查询轮次 [%@] 的追溯详情...", "Looking up provenance for turn [%@]...", "Recherche de la traçabilité du tour [%@]...", "Ricerca della tracciabilità del turno [%@]...", "Wyszukiwanie pochodzenia tury [%@]...")
        case .turnNotFound:
            return e("未找到轮次追溯（该轮次 ID 不存在，或是没有追溯记录的旧轮次）。", "No provenance found (the turn ID does not exist, or it is an older turn without provenance).", "Aucune traçabilité trouvée (cet identifiant de tour n'existe pas, ou c'est un ancien tour sans traçabilité).", "Nessuna tracciabilità trovata (l'ID del turno non esiste oppure è un turno precedente senza tracciabilità).", "Nie znaleziono pochodzenia (identyfikator tury nie istnieje albo to starsza tura bez zapisu pochodzenia).")
        case .turnDetailTitle:
            return e("【轮次追溯详情】", "[Turn provenance]", "[Traçabilité du tour]", "[Tracciabilità del turno]", "[Pochodzenie tury]")
        case .turnContact:
            return e("联系人", "Contact", "Contact", "Contatto", "Kontakt")
        case .turnProvenanceStatus:
            return e("追溯记录状态", "Provenance status", "État de la traçabilité", "Stato della tracciabilità", "Stan zapisu pochodzenia")
        case .turnRoleRevision:
            return e("角色修订", "Persona revision", "Révision du persona", "Revisione della persona", "Wersja persony")
        case .turnNotRecorded:
            return e("未记录", "Not recorded", "Non enregistré", "Non registrato", "Nie zapisano")
        case .turnModelRef:
            return e("模型引用", "Model reference", "Référence du modèle", "Riferimento del modello", "Odwołanie do modelu")
        case .turnSendKey:
            return e("幂等键", "Idempotency key", "Clé d'idempotence", "Chiave di idempotenza", "Klucz idempotencji")
        case .turnPhases:
            return e("执行状态阶段", "Execution phases", "Phases d'exécution", "Fasi di esecuzione", "Etapy wykonania")
        case .turnDecision:
            return e("回复决策: %1$@ | 回复: %2$@", "Reply decision: %1$@ | Reply: %2$@", "Décision de réponse : %1$@ | Réponse : %2$@", "Decisione di risposta: %1$@ | Risposta: %2$@", "Decyzja o odpowiedzi: %1$@ | Odpowiedź: %2$@")

        // MARK: 会话状态
        case .stateHandoff:
            return e("已转人工", "Handed over to a person", "Passée à un humain", "Passata a una persona", "Przekazano człowiekowi")
        case .stateNeedsRecalibration:
            return e("需要重新校准", "Needs recalibration", "Recalibrage nécessaire", "Ricalibrazione necessaria", "Wymaga ponownej kalibracji")
        case .stateListening:
            return e("监听中", "Watching", "Surveillance en cours", "In monitoraggio", "Obserwowanie")
        case .stateReplying:
            return e("正在回复", "Replying", "Réponse en cours", "Risposta in corso", "Odpowiadanie")
        case .stateWaitRowMissing:
            return e("等待对方新消息（会话不在列表可见范围）", "Waiting for a new message (the conversation is outside the visible list)", "En attente d'un nouveau message (la conversation n'est pas dans la partie visible de la liste)", "In attesa di un nuovo messaggio (la conversazione non è nella parte visibile dell'elenco)", "Oczekiwanie na nową wiadomość (rozmowa jest poza widoczną częścią listy)")
        case .stateWaitBlocked:
            return e("等待中（有别的窗口挡住）", "Waiting (another window is in the way)", "En attente (une autre fenêtre gêne)", "In attesa (un'altra finestra è davanti)", "Oczekiwanie (inne okno zasłania)")
        case .stateWaitWindow:
            return e("等待微信窗口中（%@）", "Waiting for the WeChat window (%@)", "En attente de la fenêtre WeChat (%@)", "In attesa della finestra di WeChat (%@)", "Oczekiwanie na okno WeChat (%@)")
        case .stateWindowNotReady:
            return e("窗口未就绪", "window not ready", "fenêtre non prête", "finestra non pronta", "okno nie jest gotowe")

        // MARK: 设置
        case .settingsPermBox:
            return e("权限与连接", "Permissions & connection", "Autorisations et connexion", "Autorizzazioni e connessione", "Uprawnienia i połączenie")
        case .statusOK:
            return e("正常", "OK", "OK", "OK", "OK")
        case .statusMissing:
            return e("缺失", "Missing", "Manquante", "Mancante", "Brak")
        case .statusBlocked:
            return e("受阻", "Blocked", "Bloqué", "Bloccato", "Zablokowany")
        case .folderNoProblem:
            return e("核心服务没有报告附件文件夹受阻。", "The core service has not reported a blocked attachment folder.", "Le service principal n'a signalé aucun dossier de pièces jointes bloqué.", "Il servizio principale non ha segnalato cartelle degli allegati bloccate.", "Usługa główna nie zgłosiła zablokowanego folderu załączników.")
        case .settingsLanguageBox:
            return e("语言", "Language", "Langue", "Lingua", "Język")
        case .languageLabel:
            return e("界面语言:", "Interface language:", "Langue de l'interface :", "Lingua dell'interfaccia:", "Język interfejsu:")
        case .languageNote:
            return e("切换后立即生效，下次打开仍使用这个语言。", "Applies immediately and is remembered next time.", "S'applique immédiatement et sera conservée à la prochaine ouverture.", "Si applica subito e viene ricordata alla prossima apertura.", "Działa od razu i zostanie zapamiętany przy następnym uruchomieniu.")
        case .settingsAccountBox:
            return e("账号与模型", "Account & model", "Compte et modèle", "Account e modello", "Konto i model")
        case .labelAccount:
            return e("微信账号:", "WeChat account:", "Compte WeChat :", "Account WeChat:", "Konto WeChat:")
        case .labelBackend:
            return e("模型接口:", "Model backend:", "Backend du modèle :", "Backend del modello:", "Backend modelu:")
        case .labelModelName:
            return e("模型名称:", "Model name:", "Nom du modèle :", "Nome del modello:", "Nazwa modelu:")
        case .labelApiKey:
            return e("API 密钥:", "API key:", "Clé API :", "Chiave API:", "Klucz API:")
        case .modelGlobalNote:
            return e("模型是全局设置，在「模型」页修改。", "The model is a global setting. Change it on the Model page.", "Le modèle est un réglage global. Modifiez-le sur la page Modèle.", "Il modello è un'impostazione globale. Modificalo nella pagina Modello.", "Model jest ustawieniem globalnym. Zmień go na stronie Model.")
        case .paceBox:
            return e("回复节奏与运行参数", "Reply pacing & runtime", "Rythme des réponses et exécution", "Ritmo delle risposte ed esecuzione", "Tempo odpowiedzi i działanie")
        case .fieldCooldown:
            return e("冷却时间", "Cooldown", "Délai de repos", "Tempo di attesa", "Czas odstępu")
        case .fieldMaxReplies:
            return e("每小时回复上限", "Replies per hour limit", "Limite de réponses par heure", "Limite di risposte all'ora", "Limit odpowiedzi na godzinę")
        case .fieldQuiet:
            return e("静默等待时间", "Quiet wait", "Attente de silence", "Attesa di silenzio", "Oczekiwanie na ciszę")
        case .fieldTurnMax:
            return e("一轮最长耗时", "Longest turn", "Durée maximale d'un tour", "Durata massima di un turno", "Maksymalny czas tury")
        case .fieldContext:
            return e("上下文消息数", "Context messages", "Messages de contexte", "Messaggi di contesto", "Wiadomości kontekstu")
        case .fieldAttachment:
            return e("附件大小上限", "Attachment size limit", "Taille maximale des pièces jointes", "Dimensione massima degli allegati", "Limit rozmiaru załącznika")
        case .rangeHint:
            return e("%1$d–%2$d %3$@（默认 %4$d）", "%1$d–%2$d %3$@ (default %4$d)", "%1$d–%2$d %3$@ (par défaut %4$d)", "%1$d–%2$d %3$@ (predefinito %4$d)", "%1$d–%2$d %3$@ (domyślnie %4$d)")
        case .unitSeconds:
            return e("秒", "s", "s", "s", "s")
        case .unitMessages:
            return e("条", "messages", "messages", "messaggi", "wiadomości")
        case .unitMB:
            return e("MB", "MB", "Mo", "MB", "MB")
        case .manageWindow:
            return e("自动管理微信主窗口 (调整至 880x640)", "Manage the WeChat main window automatically (resize to 880x640)", "Gérer automatiquement la fenêtre principale de WeChat (redimensionner en 880x640)", "Gestisci automaticamente la finestra principale di WeChat (ridimensiona a 880x640)", "Automatycznie zarządzaj głównym oknem WeChat (rozmiar 880x640)")
        case .settingsMovedNote:
            return e("注：允许名单与忽略名单在「联系人」页设置；回复方式（%@）在「MCP 接入」页设置。", "Note: the allowlist and ignore list are on the Contacts page; the reply mode (%@) is on the MCP access page.", "Remarque : la liste autorisée et la liste ignorée se trouvent sur la page Contacts ; le mode de réponse (%@) se règle sur la page Accès MCP.", "Nota: la lista consentiti e la lista ignorati sono nella pagina Contatti; la modalità di risposta (%@) si imposta nella pagina Accesso MCP.", "Uwaga: lista dozwolonych i lista ignorowanych są na stronie Kontakty; tryb odpowiedzi (%@) ustawia się na stronie Dostęp MCP.")
        case .updateBox:
            return e("应用版本与更新", "App version & updates", "Version et mises à jour", "Versione e aggiornamenti", "Wersja i aktualizacje")
        case .hostVersion:
            return e("当前宿主版本:", "Current host version:", "Version actuelle de l'hôte :", "Versione attuale dell'host:", "Bieżąca wersja hosta:")
        case .updateIntro:
            return e("应用版本更新（含微信 Bot 核心与辅助服务）已集中在「应用更新」页面管理，支持发布者验签、排空延期与本机重签名。", "App updates (including the WeChat bot core and helper services) are managed on the App updates page, with publisher signature checks, drain-and-defer and local re-signing.", "Les mises à jour (y compris le cœur du bot WeChat et les services auxiliaires) se gèrent sur la page Mises à jour de l'application : vérification de la signature de l'éditeur, report après vidage et re-signature locale.", "Gli aggiornamenti (inclusi il core del bot WeChat e i servizi ausiliari) si gestiscono nella pagina Aggiornamenti dell'app: verifica della firma dell'editore, rinvio dopo lo svuotamento e nuova firma locale.", "Aktualizacje (w tym rdzeń bota WeChat i usługi pomocnicze) są obsługiwane na stronie Aktualizacje aplikacji: weryfikacja podpisu wydawcy, odroczenie po opróżnieniu kolejki i lokalne ponowne podpisanie.")
        case .actionGotoUpdate:
            return e("前往应用更新页...", "Go to App updates...", "Ouvrir les mises à jour...", "Vai agli aggiornamenti...", "Przejdź do aktualizacji...")
        case .actionSaveConfig:
            return e("保存配置", "Save settings", "Enregistrer les réglages", "Salva impostazioni", "Zapisz ustawienia")
        case .actionRestoreDefaults:
            return e("恢复默认值", "Restore defaults", "Rétablir les valeurs par défaut", "Ripristina predefiniti", "Przywróć domyślne")
        case .savedNeedsRestart:
            return e("已保存到配置文件。重启核心服务后生效。", "Saved to the configuration file. It takes effect after the core service restarts.", "Enregistré dans le fichier de configuration. Prise d'effet après le redémarrage du service principal.", "Salvato nel file di configurazione. Ha effetto dopo il riavvio del servizio principale.", "Zapisano w pliku konfiguracji. Zacznie działać po ponownym uruchomieniu usługi głównej.")
        case .saveFailed:
            return e("保存失败: %@", "Save failed: %@", "Échec de l'enregistrement : %@", "Salvataggio non riuscito: %@", "Zapis nie powiódł się: %@")
        case .validationError:
            return e("校验错误: %@", "Validation error: %@", "Erreur de validation : %@", "Errore di convalida: %@", "Błąd walidacji: %@")
        case .defaultsLoaded:
            return e("已载入默认值，点击「保存配置」后才会保存。", "Defaults loaded. They are saved only after you click Save settings.", "Valeurs par défaut chargées. Elles ne sont enregistrées qu'après avoir cliqué sur Enregistrer les réglages.", "Valori predefiniti caricati. Vengono salvati solo dopo aver fatto clic su Salva impostazioni.", "Wczytano wartości domyślne. Zostaną zapisane dopiero po kliknięciu Zapisz ustawienia.")
        case .errNotInteger:
            return e("%1$@ (%2$@) 必须为有效整数", "%1$@ (%2$@) must be a whole number", "%1$@ (%2$@) doit être un nombre entier", "%1$@ (%2$@) deve essere un numero intero", "%1$@ (%2$@) musi być liczbą całkowitą")
        case .errWrongType:
            return e("%1$@ (%2$@) 类型不正确", "%1$@ (%2$@) has the wrong type", "%1$@ (%2$@) n'a pas le bon type", "%1$@ (%2$@) ha un tipo non valido", "%1$@ (%2$@) ma nieprawidłowy typ")
        case .errOutOfRange:
            return e("%1$@ (%2$@) 超出范围 (%3$d-%4$d %5$@)", "%1$@ (%2$@) is out of range (%3$d-%4$d %5$@)", "%1$@ (%2$@) est hors plage (%3$d-%4$d %5$@)", "%1$@ (%2$@) è fuori intervallo (%3$d-%4$d %5$@)", "%1$@ (%2$@) jest poza zakresem (%3$d-%4$d %5$@)")
        case .externalTimeoutLabel:
            return e("%@超时 (秒):", "%@ timeout (s):", "Délai de %@ (s) :", "Timeout di %@ (s):", "Limit czasu – %@ (s):")
        case .fallbackToBuiltin:
            return e("改用%@", "Switch to %@", "Passer à %@", "Passa a %@", "Przełącz na: %@")

        // MARK: 系统检查
        case .evPermOK:
            return e("屏幕录制与辅助功能已授权", "Screen recording and Accessibility are authorized", "Enregistrement de l'écran et Accessibilité autorisés", "Registrazione schermo e Accessibilità autorizzate", "Nagrywanie ekranu i Dostępność są autoryzowane")
        case .evPermNone:
            return e("屏幕录制与辅助功能未授权", "Screen recording and Accessibility are not authorized", "Enregistrement de l'écran et Accessibilité non autorisés", "Registrazione schermo e Accessibilità non autorizzate", "Nagrywanie ekranu i Dostępność nie są autoryzowane")
        case .evPermNoneFix:
            return e("请在「系统设置 → 隐私与安全性」中授予屏幕录制和辅助功能权限", "Grant Screen Recording and Accessibility in System Settings → Privacy & Security", "Autorisez Enregistrement de l'écran et Accessibilité dans Réglages Système → Confidentialité et sécurité", "Autorizza Registrazione schermo e Accessibilità in Impostazioni di Sistema → Privacy e sicurezza", "Przyznaj Nagrywanie ekranu i Dostępność w Ustawienia systemowe → Prywatność i ochrona")
        case .evPermScreen:
            return e("屏幕录制未授权", "Screen recording is not authorized", "Enregistrement de l'écran non autorisé", "Registrazione schermo non autorizzata", "Nagrywanie ekranu nie jest autoryzowane")
        case .evPermScreenFix:
            return e("请在「系统设置 → 隐私与安全性」中授予屏幕录制权限", "Grant Screen Recording in System Settings → Privacy & Security", "Autorisez Enregistrement de l'écran dans Réglages Système → Confidentialité et sécurité", "Autorizza Registrazione schermo in Impostazioni di Sistema → Privacy e sicurezza", "Przyznaj Nagrywanie ekranu w Ustawienia systemowe → Prywatność i ochrona")
        case .evPermAccessibility:
            return e("辅助功能未授权", "Accessibility is not authorized", "Accessibilité non autorisée", "Accessibilità non autorizzata", "Dostępność nie jest autoryzowana")
        case .evPermAccessibilityFix:
            return e("请在「系统设置 → 隐私与安全性」中授予辅助功能权限", "Grant Accessibility in System Settings → Privacy & Security", "Autorisez Accessibilité dans Réglages Système → Confidentialité et sécurité", "Autorizza Accessibilità in Impostazioni di Sistema → Privacy e sicurezza", "Przyznaj Dostępność w Ustawienia systemowe → Prywatność i ochrona")
        case .evFolderBlocked:
            return e("附件保存所在文件夹未授权（%@），图片和文件无法保存", "The attachment folder is not authorized (%@); images and files cannot be saved", "Le dossier des pièces jointes n'est pas autorisé (%@) ; images et fichiers ne peuvent pas être enregistrés", "La cartella degli allegati non è autorizzata (%@); immagini e file non possono essere salvati", "Brak dostępu do folderu załączników (%@); obrazy i pliki nie mogą zostać zapisane")
        case .evFolderFix:
            return e("请在「系统设置 → 隐私与安全性 → 文件和文件夹」中为 微信客服助手 打开 桌面、文稿、下载 文件夹的访问", "In System Settings → Privacy & Security → Files and Folders, allow WeChat Support Assistant to access Desktop, Documents and Downloads", "Dans Réglages Système → Confidentialité et sécurité → Fichiers et dossiers, autorisez l'Assistant de support WeChat à accéder à Bureau, Documents et Téléchargements", "In Impostazioni di Sistema → Privacy e sicurezza → File e cartelle, consenti ad Assistente di supporto WeChat l'accesso a Scrivania, Documenti e Download", "W Ustawienia systemowe → Prywatność i ochrona → Pliki i foldery zezwól Asystentowi obsługi WeChat na dostęp do folderów Biurko, Dokumenty i Pobrane")
        case .evDesktopRunning:
            return e("桌面服务运行中%@", "Desktop service is running%@", "Service de bureau en cours d'exécution%@", "Servizio desktop in esecuzione%@", "Usługa pulpitu działa%@")
        case .evDesktopIdle:
            return e("桌面服务空闲，等待连接", "Desktop service is idle, waiting for a connection", "Service de bureau inactif, en attente de connexion", "Servizio desktop inattivo, in attesa di connessione", "Usługa pulpitu bezczynna, czeka na połączenie")
        case .evDesktopRestarting:
            return e("桌面服务正在退避重试", "Desktop service is backing off before retrying", "Le service de bureau patiente avant de réessayer", "Il servizio desktop attende prima di riprovare", "Usługa pulpitu czeka przed ponowną próbą")
        case .evDesktopRestartingFix:
            return e("等待自动重试或点击重启服务", "Wait for the automatic retry or restart the service", "Attendez la nouvelle tentative automatique ou redémarrez le service", "Attendi il nuovo tentativo automatico o riavvia il servizio", "Poczekaj na automatyczną ponowną próbę lub uruchom usługę ponownie")
        case .evDesktopStopped:
            return e("桌面服务未运行", "Desktop service is not running", "Le service de bureau n'est pas en cours d'exécution", "Il servizio desktop non è in esecuzione", "Usługa pulpitu nie działa")
        case .evDesktopStoppedFix:
            return e("点击「重启桌面服务」", "Click Restart desktop service", "Cliquez sur Redémarrer le service de bureau", "Fai clic su Riavvia il servizio desktop", "Kliknij Uruchom ponownie usługę pulpitu")
        case .evBackoffEta:
            return e("退避重试中 (预计 %d 重试)", "Backing off (next retry at %d)", "En attente (prochaine tentative à %d)", "In attesa (prossimo tentativo a %d)", "Oczekiwanie (następna próba o %d)")
        case .evBotConflict:
            return e("互斥冲突：Rust 核心已启用，但检测到 Python 机器人任务已加载", "Conflict: the Rust core is enabled but the Python bot job is also loaded", "Conflit : le cœur Rust est activé mais la tâche du bot Python est aussi chargée", "Conflitto: il core Rust è abilitato ma è caricato anche il job del bot Python", "Konflikt: rdzeń Rust jest włączony, ale załadowano też zadanie bota Python")
        case .evBotConflictFix:
            return e("请先卸载 Python launchd 任务: launchctl bootout gui/$UID/local.wechat-rpa.bot", "Unload the Python launchd job first: launchctl bootout gui/$UID/local.wechat-rpa.bot", "Déchargez d'abord la tâche launchd Python : launchctl bootout gui/$UID/local.wechat-rpa.bot", "Scarica prima il job launchd Python: launchctl bootout gui/$UID/local.wechat-rpa.bot", "Najpierw wyładuj zadanie launchd Python: launchctl bootout gui/$UID/local.wechat-rpa.bot")
        case .evBotStopping:
            return e("正在停止 Rust 核心（等待当前一轮完成）", "Stopping the Rust core (waiting for the current turn to finish)", "Arrêt du cœur Rust (attente de la fin du tour en cours)", "Arresto del core Rust (in attesa della fine del turno corrente)", "Zatrzymywanie rdzenia Rust (oczekiwanie na koniec bieżącej tury)")
        case .evBotBackoff:
            return e("Rust 核心正在退避重试中", "The Rust core is backing off before retrying", "Le cœur Rust patiente avant de réessayer", "Il core Rust attende prima di riprovare", "Rdzeń Rust czeka przed ponowną próbą")
        case .evBotBackoffFix:
            return e("等待自动重试或排查配置", "Wait for the automatic retry or check the configuration", "Attendez la nouvelle tentative automatique ou vérifiez la configuration", "Attendi il nuovo tentativo automatico o controlla la configurazione", "Poczekaj na automatyczną ponowną próbę lub sprawdź konfigurację")
        case .evBotGaveUp:
            return e("Rust 核心连续崩溃达到上限，已停止自动重启", "The Rust core crashed too many times in a row; automatic restart has stopped", "Le cœur Rust a planté trop de fois de suite ; le redémarrage automatique est arrêté", "Il core Rust si è arrestato troppe volte di seguito; il riavvio automatico è stato interrotto", "Rdzeń Rust zbyt wiele razy z rzędu uległ awarii; automatyczne uruchamianie zostało wstrzymane")
        case .evBotGaveUpFix:
            return e("排查错误后点击「重启机器人」", "Fix the error, then click Restart bot", "Corrigez l'erreur puis cliquez sur Redémarrer le bot", "Correggi l'errore, poi fai clic su Riavvia il bot", "Usuń błąd, a potem kliknij Uruchom ponownie bota")
        case .evBotConfigError:
            return e("Rust 核心配置错误 (退出码 2)，已停止自动重启", "Rust core configuration error (exit code 2); automatic restart has stopped", "Erreur de configuration du cœur Rust (code de sortie 2) ; le redémarrage automatique est arrêté", "Errore di configurazione del core Rust (codice di uscita 2); il riavvio automatico è stato interrotto", "Błąd konfiguracji rdzenia Rust (kod wyjścia 2); automatyczne uruchamianie zostało wstrzymane")
        case .evBotConfigErrorFix:
            return e("检查配置文件 ~/.WeChatRPABot/config.json", "Check the configuration file ~/.WeChatRPABot/config.json", "Vérifiez le fichier de configuration ~/.WeChatRPABot/config.json", "Controlla il file di configurazione ~/.WeChatRPABot/config.json", "Sprawdź plik konfiguracji ~/.WeChatRPABot/config.json")
        case .evBotNotRunning:
            return e("Rust 核心未运行", "The Rust core is not running", "Le cœur Rust n'est pas en cours d'exécution", "Il core Rust non è in esecuzione", "Rdzeń Rust nie działa")
        case .evBotRestartFix:
            return e("点击「重启机器人」", "Click Restart bot", "Cliquez sur Redémarrer le bot", "Fai clic su Riavvia il bot", "Kliknij Uruchom ponownie bota")
        case .evBotNoHealth:
            return e("未检测到机器人健康状态 (health.json 缺失)%@", "No bot health status found (health.json is missing)%@", "Aucun état de santé du bot détecté (health.json manquant)%@", "Nessuno stato di salute del bot rilevato (health.json mancante)%@", "Nie wykryto stanu bota (brak health.json)%@")
        case .evBotNoHealthFixCore:
            return e("等待核心写入状态或启动机器人服务", "Wait for the core to write its status, or start the bot service", "Attendez que le cœur écrive son état, ou démarrez le service du bot", "Attendi che il core scriva lo stato, oppure avvia il servizio del bot", "Poczekaj, aż rdzeń zapisze stan, lub uruchom usługę bota")
        case .evBotNoHealthFixPython:
            return e("启动机器人服务", "Start the bot service", "Démarrez le service du bot", "Avvia il servizio del bot", "Uruchom usługę bota")
        case .evBotStale:
            return e("机器人健康状态超过 120 秒未更新%@", "Bot health status has not been updated for more than 120 seconds%@", "L'état de santé du bot n'a pas été mis à jour depuis plus de 120 secondes%@", "Lo stato di salute del bot non è aggiornato da più di 120 secondi%@", "Stan bota nie był aktualizowany od ponad 120 sekund%@")
        case .evBotStaleFix:
            return e("检查机器人进程或点击「重启机器人」", "Check the bot process or click Restart bot", "Vérifiez le processus du bot ou cliquez sur Redémarrer le bot", "Controlla il processo del bot o fai clic su Riavvia il bot", "Sprawdź proces bota lub kliknij Uruchom ponownie bota")
        case .evBotRecalibrate:
            return e("机器人需要重新校准 (needs_recalibration)%@", "The bot needs recalibration (needs_recalibration)%@", "Le bot doit être recalibré (needs_recalibration)%@", "Il bot deve essere ricalibrato (needs_recalibration)%@", "Bot wymaga ponownej kalibracji (needs_recalibration)%@")
        case .evBotRecalibrateFix:
            return e("运行 control calibrate 重新定位", "Run control calibrate to locate the interface again", "Exécutez control calibrate pour relocaliser l'interface", "Esegui control calibrate per individuare di nuovo l'interfaccia", "Uruchom control calibrate, aby ponownie zlokalizować interfejs")
        case .evBotPaused:
            return e("机器人已暂停%@", "The bot is paused%@", "Le bot est en pause%@", "Il bot è in pausa%@", "Bot jest wstrzymany%@")
        case .evBotPausedFix:
            return e("点击「恢复自动回复」", "Click Resume auto-reply", "Cliquez sur Reprendre la réponse automatique", "Fai clic su Riprendi risposta automatica", "Kliknij Wznów automatyczne odpowiedzi")
        case .evBotCoreRunning:
            return e("Rust 核心运行中%1$@ (%2$@)", "The Rust core is running%1$@ (%2$@)", "Le cœur Rust est en cours d'exécution%1$@ (%2$@)", "Il core Rust è in esecuzione%1$@ (%2$@)", "Rdzeń Rust działa%1$@ (%2$@)")
        case .evBotPythonNotLoaded:
            return e("Python 模式：机器人服务未加载 (launchd 缺失)", "Python mode: the bot service is not loaded (launchd job missing)", "Mode Python : le service du bot n'est pas chargé (tâche launchd absente)", "Modalità Python: il servizio del bot non è caricato (job launchd assente)", "Tryb Python: usługa bota nie jest załadowana (brak zadania launchd)")
        case .evBotPythonNotLoadedFix:
            return e("检查并加载 local.wechat-rpa.bot", "Check and load local.wechat-rpa.bot", "Vérifiez et chargez local.wechat-rpa.bot", "Controlla e carica local.wechat-rpa.bot", "Sprawdź i załaduj local.wechat-rpa.bot")
        case .evBotPythonRunning:
            return e("Python 模式：机器人正常运行中 (%@)", "Python mode: the bot is running normally (%@)", "Mode Python : le bot fonctionne normalement (%@)", "Modalità Python: il bot funziona normalmente (%@)", "Tryb Python: bot działa prawidłowo (%@)")
        case .evSessionsRecalibrate:
            return e("会话【%@】需要重新校准", "Conversation [%@] needs recalibration", "La conversation [%@] doit être recalibrée", "La conversazione [%@] deve essere ricalibrata", "Rozmowa [%@] wymaga ponownej kalibracji")
        case .evSessionsRecalibrateFix:
            return e("运行校准生成新地图", "Run calibration to generate a new map", "Lancez le calibrage pour générer une nouvelle carte", "Esegui la calibrazione per generare una nuova mappa", "Uruchom kalibrację, aby utworzyć nową mapę")
        case .evSessionsHandoff:
            return e("会话【%@】已转人工", "Conversation [%@] was handed over to a person", "La conversation [%@] a été passée à un humain", "La conversazione [%@] è stata passata a una persona", "Rozmowa [%@] została przekazana człowiekowi")
        case .evSessionsHandoffFix:
            return e("点击「恢复自动回复」或处理会话", "Click Resume auto-reply, or handle the conversation", "Cliquez sur Reprendre la réponse automatique, ou traitez la conversation", "Fai clic su Riprendi risposta automatica, oppure gestisci la conversazione", "Kliknij Wznów automatyczne odpowiedzi lub obsłuż rozmowę")
        case .evSessionsWaiting:
            return e("会话【%1$@】处于 %2$@", "Conversation [%1$@]: %2$@", "Conversation [%1$@] : %2$@", "Conversazione [%1$@]: %2$@", "Rozmowa [%1$@]: %2$@")
        case .evSessionsWaitingFix:
            return e("检查微信窗口是否可见且未被遮挡", "Check that the WeChat window is visible and not covered", "Vérifiez que la fenêtre WeChat est visible et non masquée", "Controlla che la finestra di WeChat sia visibile e non coperta", "Sprawdź, czy okno WeChat jest widoczne i niezasłonięte")
        case .evSessionsOK:
            return e("所有会话状态正常 (%d 个)", "All conversations are fine (%d)", "Toutes les conversations vont bien (%d)", "Tutte le conversazioni sono a posto (%d)", "Wszystkie rozmowy w porządku (%d)")
        case .evWechatNotRunning:
            return e("微信进程未运行", "WeChat is not running", "WeChat n'est pas lancé", "WeChat non è in esecuzione", "WeChat nie jest uruchomiony")
        case .evWechatNotRunningFix:
            return e("请打开微信客户端", "Open the WeChat app", "Ouvrez l'application WeChat", "Apri l'app WeChat", "Otwórz aplikację WeChat")
        case .evWechatBlocked:
            return e("有其他微信窗口挡在主窗口前面 (%@)", "Another WeChat window is in front of the main window (%@)", "Une autre fenêtre WeChat se trouve devant la fenêtre principale (%@)", "Un'altra finestra di WeChat è davanti alla finestra principale (%@)", "Inne okno WeChat zasłania okno główne (%@)")
        case .evWechatBlockedFix:
            return e("关闭遮挡窗口，或点击「微信窗口放回标准位置」", "Close the window in the way, or click Reset window position", "Fermez la fenêtre gênante, ou cliquez sur Replacer la fenêtre", "Chiudi la finestra davanti, oppure fai clic su Ripristina posizione finestra", "Zamknij zasłaniające okno lub kliknij Przywróć pozycję okna")
        case .evWechatSize:
            return e("微信主窗口尺寸 (%1$dx%2$d) 与标准 (880x640) 不符", "The WeChat main window size (%1$dx%2$d) differs from the standard (880x640)", "La taille de la fenêtre principale de WeChat (%1$dx%2$d) diffère du standard (880x640)", "La dimensione della finestra principale di WeChat (%1$dx%2$d) è diversa dallo standard (880x640)", "Rozmiar głównego okna WeChat (%1$dx%2$d) różni się od standardowego (880x640)")
        case .evWechatRepositionFix:
            return e("点击「微信窗口放回标准位置」", "Click Reset window position", "Cliquez sur Replacer la fenêtre", "Fai clic su Ripristina posizione finestra", "Kliknij Przywróć pozycję okna")
        case .evWechatReady:
            return e("微信主窗口就绪且无遮挡", "The WeChat main window is ready and not covered", "La fenêtre principale de WeChat est prête et non masquée", "La finestra principale di WeChat è pronta e non coperta", "Główne okno WeChat jest gotowe i niezasłonięte")
        case .evWechatNoWindow:
            return e("未检测到微信主窗口", "No WeChat main window found", "Fenêtre principale de WeChat introuvable", "Finestra principale di WeChat non trovata", "Nie znaleziono głównego okna WeChat")
        case .evWechatNoWindowFix:
            return e("打开微信主窗口并点击「微信窗口放回标准位置」", "Open the WeChat main window and click Reset window position", "Ouvrez la fenêtre principale de WeChat et cliquez sur Replacer la fenêtre", "Apri la finestra principale di WeChat e fai clic su Ripristina posizione finestra", "Otwórz główne okno WeChat i kliknij Przywróć pozycję okna")
        case .evLayoutReady:
            return e("当前版本布局地图就绪%@", "The layout map for this version is ready%@", "La carte de disposition de cette version est prête%@", "La mappa del layout per questa versione è pronta%@", "Mapa układu dla tej wersji jest gotowa%@")
        case .evLayoutMissing:
            return e("当前微信版本没有布局地图", "There is no layout map for this WeChat version", "Aucune carte de disposition pour cette version de WeChat", "Nessuna mappa del layout per questa versione di WeChat", "Brak mapy układu dla tej wersji WeChat")
        case .evLayoutMissingFix:
            return e("运行校准以生成布局地图", "Run calibration to generate the layout map", "Lancez le calibrage pour générer la carte de disposition", "Esegui la calibrazione per generare la mappa del layout", "Uruchom kalibrację, aby utworzyć mapę układu")
        case .evDiskLow:
            return e("磁盘剩余空间不足 5GB (剩余 %@)", "Less than 5 GB of disk space left (%@ free)", "Moins de 5 Go d'espace disque disponible (%@ libres)", "Meno di 5 GB di spazio su disco (%@ liberi)", "Mniej niż 5 GB wolnego miejsca na dysku (wolne: %@)")
        case .evDiskLowFix:
            return e("清理附件或日志以释放空间", "Clear attachments or logs to free space", "Supprimez des pièces jointes ou des journaux pour libérer de l'espace", "Elimina allegati o registri per liberare spazio", "Usuń załączniki lub dzienniki, aby zwolnić miejsce")
        case .evDiskOK:
            return e("磁盘空间充足 (剩余 %@)", "Enough disk space (%@ free)", "Espace disque suffisant (%@ libres)", "Spazio su disco sufficiente (%@ liberi)", "Wystarczająco miejsca na dysku (wolne: %@)")
        case .evExternalDisconnected:
            return e("%@未连接", "%@: not connected", "%@ : non connectée", "%@: non connessa", "%@: brak połączenia")
        case .evExternalConnected:
            return e("%@已连接", "%@: connected", "%@ : connectée", "%@: connessa", "%@: połączono")
        case .evExternalPending:
            return e(" (待处理 %d 轮)", " (%d turns pending)", " (%d tours en attente)", " (%d turni in attesa)", " (oczekujące tury: %d)")
        case .evExternalFix:
            return e("启动外部回复客户端以恢复回复", "Start the external reply client to resume replies", "Démarrez le client de réponse externe pour reprendre les réponses", "Avvia il client di risposta esterna per riprendere le risposte", "Uruchom zewnętrznego klienta odpowiedzi, aby wznowić odpowiedzi")
        case .evBuiltinRunning:
            return e("%@正常运行中", "%@: running normally", "%@ : fonctionne normalement", "%@: funziona normalmente", "%@: działa prawidłowo")

        // MARK: 诊断报告
        case .diagTitle:
            return e("=== 微信客服助手 诊断报告 ===", "=== WeChat Support Assistant diagnostics ===", "=== Diagnostic de l'Assistant de support WeChat ===", "=== Diagnostica di Assistente di supporto WeChat ===", "=== Diagnostyka Asystenta obsługi WeChat ===")
        case .diagVersion:
            return e("版本", "Version", "Version", "Versione", "Wersja")
        case .diagHost:
            return e("宿主环境", "Host", "Hôte", "Host", "Host")
        case .diagTime:
            return e("生成时间", "Generated at", "Généré le", "Generato il", "Wygenerowano")
        }
    }
}
