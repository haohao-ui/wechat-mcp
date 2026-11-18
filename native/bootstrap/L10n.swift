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
    // 设计版页面迁移（自动生成，勿手改 key 名）
    case uiBrain001, uiBrain002, uiBrain003, uiBrain004, uiBrain005, uiBrain006, uiBrain007, uiBrain008, uiBrain009, uiBrain010, uiBrain011, uiBrain012, uiBrain013, uiBrain014, uiBrain015, uiBrain016, uiBrain017, uiBrain018, uiBrain019, uiBrain020, uiBrain021, uiBrain022, uiBrain023, uiBrain024, uiBrain025, uiBrain026, uiBrain027, uiBrain028, uiBrain029, uiBrain030, uiBrain031, uiBrain032, uiBrain033, uiBrain034, uiBrain035, uiBrain036, uiBrain037, uiBrain038, uiBrain039, uiBrain040, uiBrain041, uiBrain042, uiBrain043, uiBrain044, uiBrain045, uiBrain046, uiBrain047, uiBrain048, uiBrain049, uiBrain050, uiBrain051, uiBrain052, uiBrain053, uiBrain054, uiBrain055, uiBrain056, uiBrain057, uiBrain058, uiBrain059, uiBrain060, uiBrain061, uiBrain062, uiBrain063, uiBrain064, uiBrain065, uiBrain066, uiBrain067, uiBrain068, uiBrain069, uiBrain070, uiBrain071, uiBrain072, uiBrain073, uiBrain074, uiBrain075, uiBrain076, uiBrain077, uiBrain078, uiBrain079, uiBrain080, uiBrain081, uiBrain082, uiBrain083, uiBrain084, uiBrain085, uiBrain086, uiBrain087, uiBrain088, uiBrain089, uiBrain090, uiBrain091, uiBrain092, uiBrain093, uiBrain094, uiBrain095, uiBrain096, uiBrain097, uiBrain098, uiBrain099, uiBrain100, uiBrain101, uiBrain102, uiBrain103, uiBrain104, uiBrain105, uiBrain106, uiBrain107, uiBrain108, uiBrain109, uiBrain110, uiBrain111, uiBrain112, uiMcp113, uiMcp114, uiMcp115, uiMcp116, uiMcp117, uiMcp118, uiMcp119, uiMcp120, uiMcp121, uiMcp122, uiMcp123, uiMcp124, uiMcp125, uiMcp126, uiMcp127, uiMcp128, uiMcp129, uiMcp130, uiMcp131, uiMcp132, uiMcp133, uiMcp134, uiMcp135, uiUpdate136, uiUpdate137, uiUpdate138, uiUpdate139, uiUpdate140, uiUpdate141, uiUpdate142, uiUpdate143, uiUpdate144, uiUpdate145, uiUpdate146, uiUpdate147, uiUpdate148, uiUpdate149, uiUpdate150, uiUpdate151, uiUpdate152, uiUpdate153, uiUpdate154, uiUpdate155, uiUpdate156, uiUpdate157, uiUpdate158, uiUpdate159, uiUpdate160, uiUpdate161, uiUpdate162, uiUpdate163, uiUpdate164, uiUpdate165, uiUpdate166, uiUpdate167, uiUpdate168, uiUpdate169, uiUpdate170, uiUpdate171, uiUpdate172, uiUpdate173, uiUpdate174, uiUpdate175, uiUpdate176, uiUpdate177, uiUpdate178, uiUpdate179, uiUpdate180, uiUpdate181, uiUpdate182, uiUpdate183, uiUpdate184, uiUpdate185, uiUpdate186, uiUpdate187, uiUpdate188, uiUpdate189, uiUpdate190, uiUpdate191, uiUpdate192, uiUpdate193, uiUpdate194, uiUpdate195, uiUpdate196, uiUpdate197, uiUpdate198, uiUpdate199, uiUpdate200, uiUpdate201, uiUpdate202, uiUpdate203, uiUpdate204, uiUpdate205, uiUpdate206, uiUpdate207, uiUpdate208, uiUpdate209, uiUpdate210, uiUpdate211, uiUpdate212, uiUpdate213, uiUpdate214, uiUpdate215, uiUpdate216, uiUpdate217, uiUpdate218, uiUpdate219, uiUpdate220
    // 收尾：嵌套插值与多行文案
    case uiResidualUnknownReason, uiResidualUnknown, uiResidualBuiltinBrain, uiResidualYes, uiResidualNo, uiResidualHistoryUnavailableFmt, uiResidualAccountFmt, uiResidualReplyModeFmt, uiResidualModelCalledFmt, uiResidualBudgetFmt, uiResidualNoCompiled, uiResidualMcpTestFailedFmt, uiResidualConnectFailed, uiResidualMcpConfigExample, uiResidualUpdateRecoveryGuide
    // 设计版页面迁移（自动生成，勿手改 key 名）
    case uiOverview001, uiOverview002, uiOverview003, uiOverview004, uiOverview005, uiOverview006, uiOverview007, uiOverview008, uiSessions009, uiSessions010, uiSessions011, uiSessions012, uiSessions013, uiSessions014, uiSessions015, uiSessions016, uiSessions017, uiSessions018, uiSessions019, uiSessions020, uiSettings021, uiSettings022, uiSettings023, uiSettings024, uiSettings025, uiSettings026, uiSettings027, uiSettings028, uiSettings029, uiSettings030, uiSettings031, uiSettings032, uiSettings033, uiSettings034, uiSettings035, uiSettings036, uiSettings037, uiSettings038, uiSettings039, uiSettings040, uiSettings041, uiSettings042, uiSettings043, uiSettings044, uiSettings045, uiSettings046, uiShell047, uiShell048, uiStatusPanel049, uiStatusPanel050, uiStatusPanel051, uiStatusPanel052, uiStatusPanel053, uiStatusPanel054, uiStatusPanel055, uiStatusPanel056, uiStatusPanel057, uiStatusPanel058, uiStatusPanel059, uiStatusPanel060, uiStatusPanel061, uiStatusPanel062, uiStatusPanel063, uiStatusPanel064, uiStatusPanel065, uiStatusPanel066, uiStatusPanel067, uiStatusPanel068, uiStatusPanel069, uiStatusPanel070, uiStatusPanel071, uiStatusPanel072, uiStatusPanel073, uiStatusPanel074, uiStatusPanel075, uiStatusPanel076, uiStatusPanel077, uiStatusPanel078, uiStatusPanel079, uiStatusPanel080, uiStatusPanel081, uiStatusPanel082, uiStatusPanel083, uiStatusPanel084, uiStatusPanel085, uiStatusPanel086, uiStatusPanel087, uiStatusPanel088, uiStatusPanel089, uiStatusPanel090, uiStatusPanel091, uiStatusPanel092, uiStatusPanel093, uiStatusPanel094, uiStatusPanel095, uiStatusPanel096, uiStatusPanel097, uiConfigForm098, uiConfigForm099, uiConfigForm100, uiConfigForm101, uiConfigForm102, uiConfigForm103, uiConfigForm104, uiConfigForm105, uiConfigForm106, uiConfigForm107, uiConfigForm108, uiConfigForm109, uiConfigForm110, uiConfigForm111, uiConfigForm112, uiConfigForm113, uiConfigForm114, uiConfigForm115, uiConfigForm116, uiConfigForm117, uiConfigForm118, uiConfigForm119, uiConfigForm120, uiConfigForm121, uiConfigForm122, uiConfigForm123, uiConfigForm124, uiConfigForm125
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

    case actionConfirm
    case actionCopy
    case actionDelete
    case actionFailedTitle
    case actionOK
    case actionPause
    case actionRefresh
    case actionResume
    case brainActionActivateConfig
    case brainActionArchiveRole
    case brainActionCopyRole
    case brainActionExitCodeFmt
    case brainActionInitRole
    case brainActionInvalidGenerationFmt
    case brainActionRollbackConfig
    case brainActionSaveBindings
    case brainActionSaveRole
    case brainActivateRoleConfirmTitleFmt
    case brainActivateRoleDialogBodyDefault
    case brainActivateRoleDialogBodyExisting
    case brainActivateRoleDialogBodyNewDefault
    case brainActivateRoleDialogTitleFmt
    case brainActivatingRole
    case brainActivationScopeMismatch
    case brainActivationScopeMismatchErr
    case brainActivationUnconfirmedFmt
    case brainAddOverride
    case brainAddOverrideErrContact
    case brainAddOverrideErrExistsFmt
    case brainAddOverrideErrGeneral
    case brainAddOverrideErrMustAllowlist
    case brainAddOverrideErrRole
    case brainAddOverrideFailed
    case brainAddOverrideSelectContact
    case brainAddOverrideSelectRole
    case brainAllowlistContactLabel
    case brainAllowlistUnavailableFmt
    case brainAssemblePreview
    case brainBindRoleLabel
    case brainBindingsDraftSavedSuccess
    case brainBindingsIntro
    case brainBindingsSavedSuccess
    case brainBindingsSectionDesc
    case brainBtnActivateConfig
    case brainBtnAddOverride
    case brainBtnConfirmActivate
    case brainBtnCopyRole
    case brainBtnDeleteOverride
    case brainBtnDeleteRole
    case brainBtnDiff
    case brainBtnImportKnowledge
    case brainBtnModelAndConn
    case brainBtnNewRole
    case brainBtnRollbackConfig
    case brainBtnRollbackSelected
    case brainBtnRollbackToSelectedVer
    case brainBtnRunPreview
    case brainBtnSaveBindingsDraft
    case brainBtnSaveDraft
    case brainBtnSetDefaultRole
    case brainBtnTryIt
    case brainBtnTryItPreview
    case brainBtnUpdateOverride
    case brainCasBindingConflictFmt
    case brainCasBindingsConflictFmt
    case brainCasRoleConflictFmt
    case brainChatKindGroupOption
    case brainChatKindPrivateOption
    case brainColBoundRole
    case brainColContactAllowlist
    case brainColCreatedAt
    case brainColDefaultRevId
    case brainColGen
    case brainColKind
    case brainColRev
    case brainColRole
    case brainColStatus
    case brainConfirmActivate
    case brainConstraintsBoxNotice
    case brainConstraintsBoxTitle
    case brainCopySuffix
    case brainCostNotePreviewOnly
    case brainCostNoteUncalled
    case brainCurrentAccountFmt
    case brainCurrentAccountNone
    case brainDefaultDraftReady
    case brainDefaultRoleDraftReady
    case brainDefaultRoleDraftSelectedFmt
    case brainDefaultRoleLabel
    case brainDefaultRolePriorityNotice
    case brainDefaultRoleScopeNotice
    case brainDefaultRoleSelectedFmt
    case brainDefaultTemplateMissing
    case brainDeleteDraftSuccess
    case brainDeleteOverrideErrSelect
    case brainDeleteOverrideSelectRule
    case brainDeleteRoleActivePrompt
    case brainDeleteRoleConfirmTitleFmt
    case brainDeleteRoleDialogInfoSaved
    case brainDeleteRoleDialogInfoUnsaved
    case brainDeleteRoleDialogTitleFmt
    case brainDeleteRoleDraftPrompt
    case brainDeleteRoleFailedFmt
    case brainDeleteRoleSuccess
    case brainDeleteSelectedOverride
    case brainDeliveryStatusFmt
    case brainDeliveryUnknown
    case brainDeliveryUnknownValue
    case brainDiffBoxTitle
    case brainDiffBudgetChangeFmt
    case brainDiffBudgetChangedFmt
    case brainDiffGoalsBothMissing
    case brainDiffGoalsChangeFmt
    case brainDiffGoalsChangedFmt
    case brainDiffGoalsNoChangeFmt
    case brainDiffHeaderFmt
    case brainDiffIncompleteDraftNoticeAFmt
    case brainDiffIncompleteDraftNoticeBFmt
    case brainDiffKnowledgeBothMissing
    case brainDiffKnowledgeChangeFmt
    case brainDiffKnowledgeChangedFmt
    case brainDiffKnowledgeNoChangeFmt
    case brainDiffLabelVerA
    case brainDiffLabelVerB
    case brainDiffLeftFmt
    case brainDiffLegacyVerAFmt
    case brainDiffLegacyVerBFmt
    case brainDiffNameChangedFmt
    case brainDiffNameNoChangeFmt
    case brainDiffPersonaBothMissing
    case brainDiffPersonaHeader
    case brainDiffPersonaNoChangeFmt
    case brainDiffPersonaTitle
    case brainDiffPersonaUnchangedFmt
    case brainDiffRightFmt
    case brainDiffRoleNameChangeFmt
    case brainDiffRoleNameNoChangeFmt
    case brainDiffRulesBothMissing
    case brainDiffRulesChangeFmt
    case brainDiffRulesChangedFmt
    case brainDiffRulesNoChangeFmt
    case brainDiffSavedIndicator
    case brainDiffTemplatesChangeFmt
    case brainDiffTemplatesChangedFmt
    case brainDiffToneChangeFmt
    case brainDiffToneChangedFmt
    case brainDiffToneNoChangeFmt
    case brainDoubleClickToEdit
    case brainDraftLoadedNotice
    case brainDraftLoadedStatus
    case brainDraftRunningStatusPlaceholder
    case brainDraftSavedNotice
    case brainEnterMockMessagePrompt
    case brainErrActionFailedFmt
    case brainErrActionInvalidDataFmt
    case brainErrFetchHistoryFailed
    case brainErrFetchRoleFailed
    case brainErrFetchRolesFailed
    case brainErrFetchTurnFailed
    case brainErrInvalidUtf8
    case brainErrSaveKnowledgeFailed
    case brainErrSaveKnowledgeInvalidResp
    case brainErrTurnNotFound
    case brainErrorUnknown
    case brainFetchConfigFailed
    case brainFetchHistoryFailed
    case brainFetchRoleFailed
    case brainFetchTurnDetailFailed
    case brainGenActiveNoDraft
    case brainGenActiveShort
    case brainGenActiveWithDraft
    case brainGenCurrentActive
    case brainGenCurrentActiveNoDraft
    case brainGenCurrentActiveWithDraft
    case brainGenCurrentSuffix
    case brainGenInitial
    case brainGenItemTitleFmt
    case brainGenOrdinalFmt
    case brainGenSavedDraft
    case brainGenerationFmt
    case brainGoalsFieldHelper
    case brainGoalsFieldLabel
    case brainGoalsPlaceholder
    case brainHistoryIntro
    case brainHistoryNoticeCompleteFmt
    case brainHistoryNoticeIncompleteFmt
    case brainHistorySectionIntro
    case brainHistoryTableBoxTitle
    case brainImportAndBindKnowledge
    case brainImportKnowledgeEmptyErr
    case brainImportNewKnowledgeBtn
    case brainInitDefaultRole
    case brainKnowledgeAddBoxTitle
    case brainKnowledgeContentField
    case brainKnowledgeContentLabel
    case brainKnowledgeIntro
    case brainKnowledgeNone
    case brainKnowledgeRefListLabel
    case brainKnowledgeSavedSuccess
    case brainKnowledgeSectionDesc
    case brainKnowledgeSourceField
    case brainKnowledgeSourceLabel
    case brainKnowledgeSourcePlaceholder
    case brainKnowledgeTitleContentRequired
    case brainKnowledgeTitleField
    case brainLabelAllowlistContact
    case brainLabelBindPersona
    case brainLoadConfigFailedFmt
    case brainLoadRoleDraftFailedFmt
    case brainLoadRolesFailedFmt
    case brainMigratedFromLegacy
    case brainModelAndConnection
    case brainModelLoading
    case brainModelStatusPending
    case brainNewAssistantDefaultGoal
    case brainNewAssistantDefaultName
    case brainNewAssistantDefaultPersona
    case brainNewAssistantDefaultTone
    case brainNewRoleDefaultName
    case brainNewRoleDefaultPersona
    case brainNewRoleDefaultTone
    case brainNo
    case brainNoAllowlistContactsNotice
    case brainNoKnowledgeReferenced
    case brainNoOverrides
    case brainNoOverridesDetail
    case brainNoOverridesSummary
    case brainNotReady
    case brainOverrideAlreadyExistsFmt
    case brainOverrideDeletedDraftFmt
    case brainOverrideDeletedFromDraftFmt
    case brainOverrideDraftAddedFmt
    case brainOverrideDraftUpdatedFmt
    case brainOverrideItemLineFmt
    case brainOverrideLineFmt
    case brainOverrideMustFromAllowlist
    case brainOverrideNotFoundForUpdate
    case brainOverridesBoxTitle
    case brainPersonaFieldLabel
    case brainPersonaSectionLabel
    case brainPhaseEchoObserved
    case brainPhaseModelGenerated
    case brainPhaseSelfEcho
    case brainPhaseSendTriggered
    case brainPleaseSelectRolePrompt
    case brainPreviewAssembling
    case brainPreviewFailed
    case brainPreviewGroupKind
    case brainPreviewInputPrompt
    case brainPreviewMockMessageLabel
    case brainPreviewPrivateKind
    case brainPreviewResultTitle
    case brainProvenanceComplete
    case brainProvenanceFull
    case brainProvenanceLegacyNotRecorded
    case brainProvenanceLegacyUnrecorded
    case brainReadDefaultTemplateFailedFmt
    case brainReferencedKnowledgeListTitle
    case brainReplyModeFmt
    case brainReplyModeLoading
    case brainReplyModePending
    case brainReplyModeUnset
    case brainRestoreFailedFmt
    case brainRestoredToGenerationFmt
    case brainRestoringVersion
    case brainRevisionStatusFmt
    case brainRevisionStatusLineFmt
    case brainRevisionStatusPlaceholder
    case brainRoleActivatedFmt
    case brainRoleActivatedSuccessFmt
    case brainRoleCannotDeleteBound
    case brainRoleColTitle
    case brainRoleCopiedNotice
    case brainRoleCopyNameFmt
    case brainRoleDeletedSuccess
    case brainRoleDraftSavedPrompt
    case brainRoleListTitle
    case brainRoleNameFieldLabel
    case brainRoleNotEnabled
    case brainRoleStateActiveFmt
    case brainRoleStateDefaultPrefix
    case brainRoleStateNewDraftRunningFmt
    case brainRoleStateSavedNotActive
    case brainRoleStateUnsavedDraft
    case brainRoleTableTooltip
    case brainRoleUnsavedModifiedNotice
    case brainRollbackAlreadyCurrent
    case brainRollbackBoxTitle
    case brainRollbackCancelledCheckbox
    case brainRollbackCancelledNotChecked
    case brainRollbackDialogBodyFmt
    case brainRollbackDialogDraftStatusFmt
    case brainRollbackDialogIntro
    case brainRollbackDialogRunConfigFmt
    case brainRollbackDialogTitleFmt
    case brainRollbackDraftIncomplete
    case brainRollbackDraftSavedFull
    case brainRollbackFailedFmt
    case brainRollbackLegacySpecialNotice
    case brainRollbackSelectGenPrompt
    case brainRollbackSuccessFmt
    case brainRollbackTargetGenLabel
    case brainRollbackVersion
    case brainRollbackingGenFmt
    case brainRollingBack
    case brainRollingBackToGenFmt
    case brainRulesFieldHelper
    case brainRulesFieldLabel
    case brainRulesPlaceholder
    case brainRunningAssemblePreview
    case brainSaveBindingsDraft
    case brainSaveCurrentBeforeDefault
    case brainSaveCurrentBeforeNew
    case brainSaveDraft
    case brainSaveDraftBeforeSetDefault
    case brainSaveKnowledgeFailed
    case brainSaveKnowledgeFailedFmt
    case brainSaveKnowledgeInvalidResp
    case brainSavingBindingsDraft
    case brainSavingDraft
    case brainSavingKnowledge
    case brainSavingKnowledgeToCore
    case brainSavingRoleDraft
    case brainSelectOrAddRolePrompt
    case brainSelectTargetGenPrompt
    case brainSelectedDefaultRoleFmt
    case brainSimulatedUserMsgLabel
    case brainStateDefaultPrefix
    case brainStateEnabledFmt
    case brainStateNewDraftRunningFmt
    case brainStateSavedNotEnabled
    case brainStateUnsavedDraft
    case brainStatusSaved
    case brainStatusUnknown
    case brainSystemConstraintsNote
    case brainSystemConstraintsTitle
    case brainTableHistoryBoxTitle
    case brainTargetGenAlreadyCurrent
    case brainTargetGenLabel
    case brainTemplateGreeting
    case brainTemplateGreetingLabel
    case brainTemplateInsufficient
    case brainTemplateInsufficientLabel
    case brainTemplateTooLarge
    case brainTemplateTooLargeLabel
    case brainTemplateUnsupported
    case brainTemplateUnsupportedLabel
    case brainTemplatesIntro
    case brainTemplatesSectionDesc
    case brainToneFieldLabel
    case brainTryItBudgetFmt
    case brainTryItConstraintsFmt
    case brainTryItCostFreeNotice
    case brainTryItCostIncurredFmt
    case brainTryItFailedFmt
    case brainTryItHeader
    case brainTryItResultTitle
    case brainTryItSnapshotShaFmt
    case brainTryItStatusFmt
    case brainTurnNotFound
    case brainUnsavedBeforeActivatePrompt
    case brainUnsavedChangesBlockActivation
    case brainUnsavedChangesNotice
    case brainUnsavedPromptSaveFirst
    case brainUnsavedRoleDeleted
    case brainUnsavedRoleNotice
    case brainUpdateOverrideErrContact
    case brainUpdateOverrideErrNotFound
    case brainUpdateOverrideErrRole
    case brainUpdateOverrideSelectContact
    case brainUpdateOverrideSelectRole
    case brainUpdateSelectedOverride
    case brainVerDraftSuffix
    case brainVerRunningSuffix
    case brainVersionALabel
    case brainVersionBLabel
    case brainYes
    case componentsBannerAlertMsg
    case componentsBannerAlertTitle
    case contactsActionAddAutoReply
    case contactsActionIgnore
    case contactsActionImmutable
    case contactsActionSetIgnore
    case contactsActionUnignore
    case contactsCategoryGroup
    case contactsCategoryIgnored
    case contactsCategoryOfficial
    case contactsCategoryPersonal
    case contactsCategoryWork
    case contactsChangeHandling
    case contactsColHandling
    case contactsColMsgs
    case contactsColName
    case contactsColWechat
    case contactsDetailAddedOn
    case contactsDetailBoxTitle
    case contactsDetailEnterprise
    case contactsDetailHandling
    case contactsDetailRegion
    case contactsDetailRemark
    case contactsDetailSource
    case contactsDetailTags
    case contactsDetailWechat
    case contactsErrorOfficialNoAutoReply
    case contactsHandlingIgnored
    case contactsHandlingNeverReply
    case contactsHandlingRecordOnly
    case contactsInitSync
    case contactsInitSyncDialogConfirm
    case contactsInitSyncDialogMsg
    case contactsInitSyncDialogTitle
    case contactsNoSelected
    case contactsOffline
    case contactsQuickSync
    case contactsReviewNoticeTitle
    case contactsReviewPrompt
    case contactsSyncCompleted
    case contactsSyncIncomplete
    case contactsSyncLoading
    case contactsSyncNever
    case contactsTabGroupCount
    case contactsTabIgnoredCount
    case contactsTabOfficialCount
    case contactsTabPersonalCount
    case contactsTabWorkCount
    case contactsWechatUnread
    case contactsWechatWorkNone
    case logsClear
    case logsLevelAll
    case logsLevelErrorOnly
    case logsLevelLabel
    case logsLevelWarnAndAbove
    case logsSearchPlaceholder
    case logsSourceLabel
    case mcpBoxConfigExample
    case mcpControlOffline
    case mcpCopiedOpenApiConfig
    case mcpCopiedSsh
    case mcpCopiedSshPlaceholder
    case mcpCopyOpenApiBtn
    case mcpCopyOpsSshBtn
    case mcpEndpointChatHistory
    case mcpEndpointExternalService
    case mcpEndpointOpenApi
    case mcpEndpointToolService
    case mcpErrorQueueInt
    case mcpErrorTimeoutInt
    case mcpFetchTokenFailed
    case mcpFieldQueueMinutes
    case mcpFieldQueueMinutesHelper
    case mcpFieldTimeoutAction
    case mcpFieldTimeoutActionHelper
    case mcpFieldTimeoutHelper
    case mcpFieldTimeoutSeconds
    case mcpGroupDesktop
    case mcpGroupOpenApi
    case mcpGroupOps
    case mcpOpenApiCheckFailed
    case mcpOpenApiConnectedCount
    case mcpOpenApiDisabled
    case mcpOpenApiDisabledNotice
    case mcpOpenApiEnabled
    case mcpOpenApiUrl
    case mcpPlaceholderLocalAddress
    case mcpPlaceholderToken
    case mcpPlaceholderUser
    case mcpReadingToken
    case mcpRegenerate
    case mcpRegenerateTokenFailed
    case mcpRegeneratingToken
    case mcpSaveFailed
    case mcpSectionEndpoints
    case mcpSectionOpenApi
    case mcpSectionTools
    case mcpStatusClosed
    case mcpStatusNotStarted
    case mcpStatusReady
    case mcpTestConnComplete
    case mcpTestConnection
    case mcpTestFailed
    case mcpTestingConnection
    case mcpTestingOpenApi
    case mcpTokenCopied
    case mcpTokenMasked
    case mcpTokenMaskedWithTail
    case mcpTokenRegeneratedSuccess
    case mcpValidationError
    case statusChecking
    case statusNotReady
    case statusQuerying
    case statusUnknown
    case statusUnverified
    case updateBtnDeferDisabled
    case updateBtnDownloadDisabled
    case updateBtnInstallDisabled
    case updateCandidateAlreadyLatest
    case updateCandidateAvailableVerified
    case updateCandidateFoundUnverified
    case updateCandidateIncompatibleEnv
    case updateCandidateNoVersion
    case updateCandidateNotCheckedYet
    case updateCandidateSourceUnconfigured
    case updateCannotInstallOnHost
    case updateCannotRollbackFmt
    case updateCertNotFound
    case updateCheckComplete
    case updateCheckFailedFmt
    case updateCheckingStatus
    case updateCompatArchVerified
    case updateCompatCurrentOk
    case updateCompatIncompatible
    case updateCompatRequirementsNotMet
    case updateCompatSystemCompatible
    case updateCompatSystemIncompatible
    case updateCompatibilityDefaultReason
    case updateConsistencyConsistent
    case updateConsistencyInconsistent
    case updateCoreCompiledVersion
    case updateCoreVersionUnknown
    case updateDrainWaitingNotice
    case updateErrCommandFailedFmt
    case updateErrNetworkOfflineFmt
    case updateErrParseFmt
    case updateFeatureDisabledNotice
    case updateHighWatermarkNotice
    case updateIntroDesc
    case updateKeepOldRecord
    case updateKeychainLocked
    case updateKeychainReady
    case updateLabelCompatibility
    case updateLabelConsistency
    case updateLabelCoreVersion
    case updateLabelCurrentSuite
    case updateLabelHostArch
    case updateLabelInstallSeq
    case updateLabelLocalSign
    case updateLabelPackageSize
    case updateLabelPublisherVerify
    case updateLabelReleaseNotes
    case updateLabelRollbackEligibility
    case updateLabelRollbackVersion
    case updateLabelTargetVersion
    case updateLocalRunning
    case updateNoCandidateNotes
    case updateNoDetailedNotes
    case updateNoRollbackDetected
    case updateNotesNone
    case updateNullValue
    case updateOldDataSourceUntrusted
    case updatePermissionsNotice
    case updatePipelineSteps
    case updatePublisherApplied
    case updatePublisherFailedCompat
    case updatePublisherNoRootKey
    case updatePublisherSignatureVerified
    case updatePublisherUnverifiedNoKey
    case updateReasonBackupCorrupt
    case updateReasonCannot
    case updateReasonCheckpointRequired
    case updateReasonInstallRootUnconfigured
    case updateReasonNoMatching
    case updateReasonNotSupported
    case updateReasonPrevCorrupt
    case updateReasonRecoveryRequired
    case updateReasonRollbackStateInvalid
    case updateReasonSafe
    case updateReasonSchemaChanged
    case updateReasonStopCoreFirst
    case updateRecentCommitted
    case updateRecordApplied
    case updateRecoveryGuideHeader
    case updateRecoveryGuideItem1
    case updateRecoveryGuideItem2
    case updateRecoveryGuideItem3
    case updateRecoveryGuideItem4
    case updateRecoveryNeededFmt
    case updateRecoveryNormal
    case updateRecoveryReasonFmt
    case updateRecoveryReportTitle
    case updateRecoveryStageFmt
    case updateRecoveryTxFmt
    case updateRefreshingStatus
    case updateReleaseDescUnknown
    case updateReleaseNotesEmpty
    case updateReleaseNotesUnconfigured
    case updateRollbackCancelledCheckbox
    case updateRollbackConfirmBody
    case updateRollbackConfirmTitleFmt
    case updateRollbackExitCodeFmt
    case updateRollbackFailedFmt
    case updateRollbackNoOutput
    case updateRollbackNotice
    case updateRollbackRecordedSafe
    case updateRollbackTimeout
    case updateRollbackingFmt
    case updateSectionCandidate
    case updateSectionCurrentEnv
    case updateSectionProgress
    case updateSeqUnknown
    case updateSigningForbidden
    case updateSigningForbiddenNoSource
    case updateSigningNeedVerify
    case updateSigningUnavailableFmt
    case updateSourceNone
    case updateSourceSignedPackage
    case updateSourceUnknown
    case updateSourceUnknownFmt
    case updateStageExecuting
    case updateStageExternalTxFmt
    case updateStageIdle
    case updateStateInstalled
    case updateStateNone
    case updateStateUnknown
    case updateStateUnknownFmt
    case updateStateVerified
    case updateStatusFetchNoticeFmt
    case updateStatusRefreshFailedFmt
    case updateStatusRefreshed
    case updateSupervisorSuppressed
    case updateSupervisorUnsuppressed
    case updateUntrustedStatusSourceFmt
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
        // MARK: 设计版页面迁移（自动生成）
        case .uiBrain001:
            return e("获取大脑配置失败", "Failed to load Brain configuration", "Échec du chargement de la configuration Brain", "Caricamento della configurazione Brain non riuscito", "Nie udało się wczytać konfiguracji Brain")
        case .uiBrain002:
            return e("未记录操作", "No operation recorded", "Aucune opération enregistrée", "Nessuna operazione registrata", "Nie zapisano operacji")
        case .uiBrain003:
            return e("保存角色草稿", "Save role draft", "Enregistrer le brouillon du rôle", "Salva bozza del ruolo", "Zapisz szkic roli")
        case .uiBrain004:
            return e("保存知识", "Save knowledge", "Enregistrer les connaissances", "Salva conoscenza", "Zapisz wiedzę")
        case .uiBrain005:
            return e("版本回退", "Revert version", "Revenir à une version", "Ripristina versione", "Przywróć wersję")
        case .uiBrain006:
            return e("…（共 %@ 字）", "…(%@ characters in total)", "…(%@ caractères au total)", "…(%@ caratteri in totale)", "…(łącznie %@ znaków)")
        case .uiBrain007:
            return e("无", "None", "Aucun", "Nessuno", "Brak")
        case .uiBrain008:
            return e("（%@ 项）", "(%@ items)", "(%@ éléments)", "(%@ elementi)", "(%@ pozycji)")
        case .uiBrain009:
            return e("欢迎语", "Welcome message", "Message d’accueil", "Messaggio di benvenuto", "Wiadomość powitalna")
        case .uiBrain010:
            return e("信息不足", "Insufficient information", "Informations insuffisantes", "Informazioni insufficienti", "Niewystarczające informacje")
        case .uiBrain011:
            return e("缺项标签", "Missing item tag", "Étiquette d’élément manquant", "Etichetta elemento mancante", "Etykieta brakującego elementu")
        case .uiBrain012:
            return e("多条标签", "Multi-item tag", "Étiquette multi-éléments", "Etichetta multi-elemento", "Etykieta wielu elementów")
        case .uiBrain013:
            return e("不可读原因 %@ 项", "%@ unreadable reasons", "%@ raisons illisibles", "%@ motivi illeggibili", "%@ nieczytelnych powodów")
        case .uiBrain014:
            return e("未设置自定义话术", "No custom script set", "Aucun script personnalisé défini", "Nessun copione personalizzato impostato", "Nie ustawiono własnego skryptu")
        case .uiBrain015:
            return e("%@：双方均未返回内容（%@）", "%@: neither side returned content (%@)", "%@ : aucun des deux côtés n’a renvoyé de contenu (%@)", "%@: nessuno dei due lati ha restituito contenuti (%@)", "%@: żadna strona nie zwróciła treści (%@)")
        case .uiBrain016:
            return e("%@：无变化（%@）", "%@: no changes (%@)", "%@ : aucun changement (%@)", "%@: nessuna modifica (%@)", "%@: brak zmian (%@)")
        case .uiBrain017:
            return e("版本 %@：%@", "Version %@: %@", "Version %@ : %@", "Versione %@: %@", "Wersja %@: %@")
        case .uiBrain018:
            return e("  角色=%@ · 创建时间=%@ · 草稿=%@ · 运行中=%@", "  role=%@ · created=%@ · draft=%@ · running=%@", "  rôle=%@ · créé le=%@ · brouillon=%@ · en service=%@", "  ruolo=%@ · creato=%@ · bozza=%@ · in esecuzione=%@", "  rola=%@ · utworzono=%@ · szkic=%@ · działająca=%@")
        case .uiBrain019:
            return e("【完整性说明】", "[Integrity notes]", "[Note d’intégrité]", "[Nota di integrità]", "[Informacje o integralności]")
        case .uiBrain020:
            return e("• 版本 A（%@）后端未返回完整草稿内容，A 侧字段按「%@」处理，不补全、不编造。", "• Version A (%@): the backend did not return the full draft content; side A fields are treated as “%@” — not filled in or invented.", "• Version A (%@) : le backend n’a pas renvoyé le brouillon complet ; les champs du côté A sont traités comme « %@ », sans complétion ni invention.", "• Versione A (%@): il backend non ha restituito la bozza completa; i campi del lato A sono trattati come «%@», senza completamenti né invenzioni.", "• Wersja A (%@): backend nie zwrócił pełnej treści szkicu; pola strony A są traktowane jako „%@” — bez uzupełniania i bez wymyślania.")
        case .uiBrain021:
            return e("• 版本 B（%@）后端未返回完整草稿内容，B 侧字段按「%@」处理，不补全、不编造。", "• Version B (%@): the backend did not return the full draft content; side B fields are treated as “%@” — not filled in or invented.", "• Version B (%@) : le backend n’a pas renvoyé le brouillon complet ; les champs du côté B sont traités comme « %@ », sans complétion ni invention.", "• Versione B (%@): il backend non ha restituito la bozza completa; i campi del lato B sono trattati come «%@», senza completamenti né invenzioni.", "• Wersja B (%@): backend nie zwrócił pełnej treści szkicu; pola strony B są traktowane jako „%@” — bez uzupełniania i bez wymyślania.")
        case .uiBrain022:
            return e("【版本 %@ 已保存内容】%@", "[Saved content of version %@] %@", "[Contenu enregistré de la version %@] %@", "[Contenuto salvato della versione %@] %@", "[Zapisana treść wersji %@] %@")
        case .uiBrain023:
            return e("• 语气风格：%@", "• Tone and style: %@", "• Ton et style : %@", "• Tono e stile: %@", "• Ton i styl: %@")
        case .uiBrain024:
            return e("• 身份与业务范围：%@", "• Identity and business scope: %@", "• Identité et périmètre d’activité : %@", "• Identità e ambito di attività: %@", "• Tożsamość i zakres działalności: %@")
        case .uiBrain025:
            return e("• 核心目标：%@", "• Core goals: %@", "• Objectifs principaux : %@", "• Obiettivi principali: %@", "• Cele główne: %@")
        case .uiBrain026:
            return e("• 业务规则：%@", "• Business rules: %@", "• Règles métier : %@", "• Regole di business: %@", "• Reguły biznesowe: %@")
        case .uiBrain027:
            return e("• 引用知识：%@", "• Referenced knowledge: %@", "• Connaissances référencées : %@", "• Conoscenze di riferimento: %@", "• Wiedza referencyjna: %@")
        case .uiBrain028:
            return e("• 异常回复模板：%@", "• Fallback reply template: %@", "• Modèle de réponse de repli : %@", "• Modello di risposta di riserva: %@", "• Szablon odpowiedzi awaryjnej: %@")
        case .uiBrain029:
            return e("• 上下文预算：%@", "• Context budget: %@", "• Budget de contexte : %@", "• Budget di contesto: %@", "• Budżet kontekstu: %@")
        case .uiBrain030:
            return e("字段级差异不可用：需要两个版本都返回完整草稿内容；本次只展示上方的版本元信息与可用一侧的已保存内容。", "Field-level diff unavailable: both versions must return full draft content. Only the version metadata above and the saved content from the available side are shown.", "Comparaison champ par champ indisponible : les deux versions doivent renvoyer le brouillon complet. Seules les métadonnées de version ci-dessus et le contenu enregistré du côté disponible sont affichés.", "Differenze a livello di campo non disponibili: entrambe le versioni devono restituire la bozza completa. Vengono mostrati solo i metadati delle versioni sopra e il contenuto salvato del lato disponibile.", "Różnice na poziomie pól niedostępne: obie wersje muszą zwrócić pełną treść szkicu. Wyświetlane są tylko metadane wersji powyżej oraz zapisana treść dostępnej strony.")
        case .uiBrain031:
            return e("名称", "Name", "Nom", "Nome", "Nazwa")
        case .uiBrain032:
            return e("语气风格", "Tone and style", "Ton et style", "Tono e stile", "Ton i styl")
        case .uiBrain033:
            return e("身份与业务范围", "Identity and business scope", "Identité et périmètre d’activité", "Identità e ambito di attività", "Tożsamość i zakres działalności")
        case .uiBrain034:
            return e("核心目标", "Core goals", "Objectifs principaux", "Obiettivi principali", "Cele główne")
        case .uiBrain035:
            return e("业务规则", "Business rules", "Règles métier", "Regole di business", "Reguły biznesowe")
        case .uiBrain036:
            return e("引用知识", "Referenced knowledge", "Connaissances référencées", "Conoscenze di riferimento", "Wiedza referencyjna")
        case .uiBrain037:
            return e("异常回复模板", "Fallback reply template", "Modèle de réponse de repli", "Modello di risposta di riserva", "Szablon odpowiedzi awaryjnej")
        case .uiBrain038:
            return e("上下文预算", "Context budget", "Budget de contexte", "Budget di contesto", "Budżet kontekstu")
        case .uiBrain039:
            return e("父版本", "Parent version", "Version parente", "Versione padre", "Wersja nadrzędna")
        case .uiBrain040:
            return e("内容摘要 sha256", "Content digest sha256", "Empreinte du contenu sha256", "Digest del contenuto sha256", "Skrót treści sha256")
        case .uiBrain041:
            return e("初始化大脑", "Initialize Brain", "Initialiser Brain", "Inizializza Brain", "Zainicjuj Brain")
        case .uiBrain042:
            return e("读取版本历史超时", "Timed out reading version history", "Délai dépassé lors de la lecture de l’historique des versions", "Timeout durante la lettura dello storico versioni", "Przekroczono limit czasu odczytu historii wersji")
        case .uiBrain043:
            return e("读取版本历史失败 (exit %@)", "Failed to read version history (exit %@)", "Échec de la lecture de l’historique des versions (exit %@)", "Lettura dello storico versioni non riuscita (exit %@)", "Nie udało się odczytać historii wersji (exit %@)")
        case .uiBrain044:
            return e("版本历史：尚未加载", "Version history: not loaded yet", "Historique des versions : pas encore chargé", "Storico versioni: non ancora caricato", "Historia wersji: jeszcze nie wczytano")
        case .uiBrain045:
            return e("真实模型调用：状态待加载", "Real model calls: status not loaded yet", "Appels réels au modèle : statut en attente de chargement", "Chiamate reali al modello: stato in attesa di caricamento", "Rzeczywiste wywołania modelu: stan oczekuje na wczytanie")
        case .uiBrain046:
            return e("欢迎语:", "Welcome message:", "Message d’accueil :", "Messaggio di benvenuto:", "Wiadomość powitalna:")
        case .uiBrain047:
            return e("信息不足:", "Insufficient information:", "Informations insuffisantes :", "Informazioni insufficienti:", "Niewystarczające informacje:")
        case .uiBrain048:
            return e("格式不支持:", "Unsupported format:", "Format non pris en charge :", "Formato non supportato:", "Nieobsługiwany format:")
        case .uiBrain049:
            return e("文件超限:", "File too large:", "Fichier trop volumineux :", "File troppo grande:", "Plik przekracza limit:")
        case .uiBrain050:
            return e("绑定大脑角色:", "Bind Brain role", "Lier un rôle Brain", "Associa ruolo Brain", "Powiąż rolę Brain")
        case .uiBrain051:
            return e("版本管理：保存草稿不影响运行版本；启用配置仅对新轮次生效。支持并发修改检测 (CAS) 与原子激活/回退。", "Version management: saving a draft does not affect the running version; enabling a configuration applies only to new turns. Concurrent-modification detection (CAS) and atomic activation/revert are supported.", "Gestion des versions : l’enregistrement d’un brouillon n’affecte pas la version en service ; l’activation d’une configuration ne s’applique qu’aux nouveaux tours. Détection des modifications concurrentes (CAS) et activation/retour atomiques pris en charge.", "Gestione versioni: salvare una bozza non influisce sulla versione in esecuzione; l’attivazione di una configurazione vale solo per i nuovi turni. Sono supportati il rilevamento delle modifiche concorrenti (CAS) e l’attivazione/ripristino atomici.", "Zarządzanie wersjami: zapisanie szkicu nie wpływa na działającą wersję; włączenie konfiguracji dotyczy tylko nowych tur. Obsługiwane są wykrywanie równoczesnych zmian (CAS) oraz atomowa aktywacja/przywrócenie.")
        case .uiBrain052:
            return e("历史版本：只读列出后端保存的代次与版本，可对比任意两个版本的差异，并可带二次确认回滚到选定代次。旧代次没有完整草稿时只展示后端已保存内容，不补全、不编造。", "Version history: a read-only list of the generations and versions saved by the backend; compare any two versions and roll back to a selected generation after confirmation. When an older generation has no full draft, only the content saved by the backend is shown — nothing is filled in or invented.", "Historique des versions : liste en lecture seule des générations et versions enregistrées par le backend ; comparez deux versions et revenez à une génération choisie après confirmation. Si une ancienne génération n’a pas de brouillon complet, seul le contenu enregistré par le backend est affiché, sans complétion ni invention.", "Storico versioni: elenco in sola lettura delle generazioni e versioni salvate dal backend; confronta due versioni e ripristina una generazione scelta dopo conferma. Se una generazione precedente non ha una bozza completa, viene mostrato solo il contenuto salvato dal backend, senza completamenti né invenzioni.", "Historia wersji: lista tylko do odczytu generacji i wersji zapisanych przez backend; porównaj dowolne dwie wersje i po potwierdzeniu przywróć wybraną generację. Gdy starsza generacja nie ma pełnego szkicu, wyświetlana jest tylko treść zapisana przez backend — bez uzupełniania i wymyślania.")
        case .uiBrain053:
            return e("版本与代次规则说明", "Version and generation rules", "Règles des versions et générations", "Regole di versioni e generazioni", "Zasady wersji i generacji")
        case .uiBrain054:
            return e("• 草稿与运行分离：保存草稿仅存入本地草稿区，不影响当前服务对外响应。", "• Drafts separated from running config: saving a draft only stores it in the local draft area and does not affect the service’s current responses.", "• Brouillons séparés de l’exécution : l’enregistrement d’un brouillon le stocke uniquement dans la zone de brouillon locale et n’affecte pas les réponses actuelles du service.", "• Bozze separate dall’esecuzione: salvare una bozza la memorizza solo nell’area locale delle bozze e non influisce sulle risposte attuali del servizio.", "• Szkice oddzielone od działania: zapisanie szkicu trafia wyłącznie do lokalnego obszaru szkiców i nie wpływa na bieżące odpowiedzi usługi.")
        case .uiBrain055:
            return e("• 原子激活：点击「启用配置」时将草稿生成新版本并原子推进激活代次。", "• Atomic activation: clicking “Enable configuration” turns the draft into a new version and atomically advances the active generation.", "• Activation atomique : cliquer sur « Activer la configuration » transforme le brouillon en nouvelle version et avance la génération active de façon atomique.", "• Attivazione atomica: facendo clic su «Attiva configurazione» la bozza diventa una nuova versione e la generazione attiva avanza in modo atomico.", "• Aktywacja atomowa: kliknięcie „Włącz konfigurację” zamienia szkic w nową wersję i atomowo przesuwa aktywną generację.")
        case .uiBrain056:
            return e("• 安全回退：点击「回退版本」将配置无损切换回上一稳定激活代次。", "• Safe revert: clicking “Revert version” switches the configuration losslessly back to the previous stable active generation.", "• Retour sûr : cliquer sur « Revenir à la version » rebascule la configuration sans perte vers la génération active stable précédente.", "• Ripristino sicuro: facendo clic su «Ripristina versione» la configurazione torna senza perdite alla precedente generazione attiva stabile.", "• Bezpieczne przywrócenie: kliknięcie „Przywróć wersję” bezstratnie przełącza konfigurację na poprzednią stabilną aktywną generację.")
        case .uiBrain057:
            return e("操作", "Operation", "Opération", "Operazione", "Operacja")
        case .uiBrain058:
            return e("默认版本ID", "Default version ID", "ID de version par défaut", "ID versione predefinito", "Domyślny ID wersji")
        case .uiBrain059:
            return e("回滚到历史版本 (二次确认)", "Roll back to a historical version (confirm)", "Revenir à une version historique (confirmation)", "Ripristina una versione storica (conferma)", "Przywróć wersję historyczną (potwierdzenie)")
        case .uiBrain060:
            return e("试一试：输入一段客户问题，查看大脑如何组装上下文与预览回复。合成测试仅执行确定性组装预览 (assemble_only)，标明【未调用模型】，不产生费用，绝不发送真实微信消息。", "Try it: enter a customer question to see how the Brain assembles context and previews a reply. The synthetic test only runs a deterministic assembly preview (assemble_only), is labelled [no model call], incurs no cost and never sends real WeChat messages.", "Essai : saisissez une question client pour voir comment le Brain assemble le contexte et prévisualise une réponse. Le test synthétique exécute uniquement un aperçu d’assemblage déterministe (assemble_only), est marqué [aucun appel au modèle], ne coûte rien et n’envoie jamais de vrais messages WeChat.", "Prova: inserisci una domanda di un cliente per vedere come il Brain assembla il contesto e genera l’anteprima di una risposta. Il test sintetico esegue solo un’anteprima di assemblaggio deterministica (assemble_only), è contrassegnato [nessuna chiamata al modello], non comporta costi e non invia mai messaggi WeChat reali.", "Wypróbuj: wpisz pytanie klienta, aby zobaczyć, jak Brain składa kontekst i generuje podgląd odpowiedzi. Test syntetyczny wykonuje wyłącznie deterministyczny podgląd składania (assemble_only), jest oznaczony [bez wywołania modelu], nie generuje kosztów i nigdy nie wysyła prawdziwych wiadomości WeChat.")
        case .uiBrain061:
            return e("例如：你们支持七天无理由退货吗？", "Example: do you accept seven-day returns with no questions asked?", "Exemple : acceptez-vous les retours sous sept jours sans justification ?", "Esempio: accettate resi entro sette giorni senza motivo?", "Przykład: czy obsługujecie zwroty w ciągu siedmiu dni bez podania przyczyny?")
        case .uiBrain062:
            return e("客户问题:", "Customer question:", "Question du client :", "Domanda del cliente:", "Pytanie klienta:")
        case .uiBrain063:
            return e("试一试结果（预览回复 + 组装预览快照与决策）:", "Try-it result (preview reply + assembly-preview snapshot and decision):", "Résultat de l’essai (réponse prévisualisée + instantané d’aperçu d’assemblage et décision) :", "Risultato della prova (risposta di anteprima + snapshot dell’anteprima di assemblaggio e decisione):", "Wynik próby (podgląd odpowiedzi + migawka podglądu składania i decyzja):")
        case .uiBrain064:
            return e("加载大脑配置失败: %@", "Failed to load Brain configuration: %@", "Échec du chargement de la configuration Brain : %@", "Caricamento della configurazione Brain non riuscito: %@", "Nie udało się wczytać konfiguracji Brain: %@")
        case .uiBrain065:
            return e("版本历史：正在加载…", "Version history: loading…", "Historique des versions : chargement…", "Storico versioni: caricamento…", "Historia wersji: wczytywanie…")
        case .uiBrain066:
            return e("暂无版本历史记录", "No version history yet", "Aucun historique de versions", "Nessuno storico versioni", "Brak historii wersji")
        case .uiBrain067:
            return e("版本历史：当前 %@ · 代次记录 %@ 条 · 版本 %@ 个", "Version history: current version %@ · %@ generation records · %@ versions", "Historique des versions : version actuelle %@ · %@ enregistrements de générations · %@ versions", "Storico versioni: versione attuale %@ · %@ record di generazioni · %@ versioni", "Historia wersji: bieżąca wersja %@ · rekordy generacji: %@ · wersje: %@")
        case .uiBrain068:
            return e("\n后端历史口径：%@（只保留已生效激活、已提交写入与可达版本；更早的编辑历史未保留）", "\nBackend history scope: %@ (only applied activations, committed writes and reachable versions are kept; earlier edit history is not retained)", "\nPérimètre de l’historique backend : %@ (seules les activations appliquées, les écritures validées et les versions accessibles sont conservées ; l’historique des modifications antérieures n’est pas retenu)", "\nAmbito dello storico del backend: %@ (vengono conservate solo attivazioni applicate, scritture confermate e versioni raggiungibili; lo storico delle modifiche precedenti non è mantenuto)", "\nZakres historii backendu: %@ (zapisywane są tylko zastosowane aktywacje, zatwierdzone zapisy i osiągalne wersje; wcześniejsza historia edycji nie jest zachowywana)")
        case .uiBrain069:
            return e("请选择两个版本后点击「对比差异」。", "Select two versions, then click “Compare differences”.", "Sélectionnez deux versions, puis cliquez sur « Comparer les différences ».", "Seleziona due versioni, poi fai clic su «Confronta differenze».", "Wybierz dwie wersje, a następnie kliknij „Porównaj różnice”.")
        case .uiBrain070:
            return e("当前没有可对比的版本记录。", "There are no version records to compare.", "Aucun enregistrement de version à comparer pour le moment.", "Al momento non ci sono record di versione da confrontare.", "Obecnie brak rekordów wersji do porównania.")
        case .uiBrain071:
            return e("内置大脑", "Built-in Brain", "Brain intégré", "Brain integrato", "Wbudowany Brain")
        case .uiBrain072:
            return e("未知（未返回模型摘要）", "Unknown (no model summary returned)", "Inconnu (aucun résumé du modèle renvoyé)", "Sconosciuto (nessun riepilogo del modello restituito)", "Nieznane (nie zwrócono podsumowania modelu)")
        case .uiBrain073:
            return e("回复方式：%@ · 真实模型调用：当前构建仅支持 assemble_only 确定性组装预览（不调用模型、不产生费用），因此不会生成真实回复文本。", "Reply mode: %@ · Real model calls: this build only supports assemble_only deterministic assembly previews (no model call, no cost), so no real reply text is generated.", "Mode de réponse : %@ · Appels réels au modèle : cette version ne prend en charge que les aperçus d’assemblage déterministes assemble_only (aucun appel au modèle, aucun coût), aucun texte de réponse réel n’est donc généré.", "Modalità di risposta: %@ · Chiamate reali al modello: questa build supporta solo anteprime di assemblaggio deterministiche assemble_only (nessuna chiamata al modello, nessun costo), quindi non viene generato alcun testo di risposta reale.", "Tryb odpowiedzi: %@ · Rzeczywiste wywołania modelu: ta kompilacja obsługuje tylko deterministyczne podglądy składania assemble_only (bez wywołania modelu, bez kosztów), więc nie powstaje prawdziwy tekst odpowiedzi.")
        case .uiBrain074:
            return e("• 联系人: %@ [%@] ➔ %@ (%@) [版本: %@]", "• Contact: %@ [%@] ➔ %@ (%@) [version: %@]", "• Contact : %@ [%@] ➔ %@ (%@) [version : %@]", "• Contatto: %@ [%@] ➔ %@ (%@) [versione: %@]", "• Kontakt: %@ [%@] ➔ %@ (%@) [wersja: %@]")
        case .uiBrain075:
            return e("角色: %@ · 草稿: %@ · 运行: %@", "Role: %@ · Draft: %@ · Running: %@", "Rôle : %@ · Brouillon : %@ · En service : %@", "Ruolo: %@ · Bozza: %@ · In esecuzione: %@", "Rola: %@ · Szkic: %@ · Działająca: %@")
        case .uiBrain076:
            return e("删除失败：%@", "Delete failed: %@", "Échec de la suppression : %@", "Eliminazione non riuscita: %@", "Usuwanie nie powiodło się: %@")
        case .uiBrain077:
            return e("角色「%@」已启用（%@），适用范围已核对。", "Role “%@” is enabled (%@) and its scope has been verified.", "Le rôle « %@ » est activé (%@) et son périmètre a été vérifié.", "Il ruolo «%@» è attivato (%@) e l’ambito è stato verificato.", "Rola „%@” jest włączona (%@), a jej zakres został sprawdzony.")
        case .uiBrain078:
            return e("启用未确认：%@。请重新读取配置核对；已保存的绑定草稿会保留。", "Activation not confirmed: %@. Reload the configuration to verify; the saved binding draft will be kept.", "Activation non confirmée : %@. Rechargez la configuration pour vérifier ; le brouillon de liaison enregistré sera conservé.", "Attivazione non confermata: %@. Ricarica la configurazione per verificare; la bozza di associazione salvata verrà mantenuta.", "Aktywacja niepotwierdzona: %@. Wczytaj ponownie konfigurację, aby to sprawdzić; zapisany szkic powiązania zostanie zachowany.")
        case .uiBrain079:
            return e("回退失败：尚未加载大脑配置", "Revert failed: Brain configuration not loaded yet", "Échec du retour : la configuration Brain n’est pas encore chargée", "Ripristino non riuscito: configurazione Brain non ancora caricata", "Przywrócenie nie powiodło się: konfiguracja Brain nie została jeszcze wczytana")
        case .uiBrain080:
            return e("回滚大脑配置到第 %@ 代？", "Roll the Brain configuration back to generation %@?", "Revenir à la génération %@ de la configuration Brain ?", "Ripristinare la configurazione Brain alla generazione %@?", "Przywrócić konfigurację Brain do generacji %@?")
        case .uiBrain081:
            return e("回滚将把账号配置原子切换到第 %@ 代的已保存状态；当前未保存的草稿将丢失。仅影响新轮次。\n", "The rollback will atomically switch the account configuration to the state saved in generation %@; unsaved drafts will be lost. Only new turns are affected.\n", "Le retour basculera atomiquement la configuration du compte vers l’état enregistré de la génération %@ ; les brouillons non enregistrés seront perdus. Seuls les nouveaux tours sont concernés.\n", "Il ripristino passerà in modo atomico la configurazione dell’account allo stato salvato della generazione %@; le bozze non salvate andranno perse. Influisce solo sui nuovi turni.\n", "Przywrócenie atomowo przełączy konfigurację konta na stan zapisany w generacji %@; niezapisane szkice zostaną utracone. Dotyczy to tylko nowych tur.\n")
        case .uiBrain082:
            return e("未保存完整草稿（仅显示后端已保存内容，不补全）", "No full draft saved (showing only backend-saved content, nothing filled in)", "Aucun brouillon complet enregistré (seul le contenu enregistré par le backend est affiché, sans complétion)", "Nessuna bozza completa salvata (viene mostrato solo il contenuto salvato dal backend, senza completamenti)", "Brak zapisanego pełnego szkicu (wyświetlana jest tylko treść zapisana przez backend, bez uzupełniania)")
        case .uiBrain083:
            return e("该代次不是激活快照，仅记录写入结果", "This generation is not an activation snapshot; only write results are recorded", "Cette génération n’est pas un instantané d’activation ; seuls les résultats d’écriture sont enregistrés", "Questa generazione non è uno snapshot di attivazione; sono registrati solo i risultati di scrittura", "Ta generacja nie jest migawką aktywacji; zapisywane są tylko wyniki zapisu")
        case .uiBrain084:
            return e("有激活快照", "Has activation snapshot", "Instantané d’activation présent", "Snapshot di attivazione presente", "Zawiera migawkę aktywacji")
        case .uiBrain085:
            return e("\n• 目标代次：第 %@ 代 · %@ · 操作=%@", "\n• Target generation: generation %@ · %@ · operation=%@", "\n• Génération cible : génération %@ · %@ · opération=%@", "\n• Generazione di destinazione: generazione %@ · %@ · operazione=%@", "\n• Generacja docelowa: generacja %@ · %@ · operacja=%@")
        case .uiBrain086:
            return e("\n• 运行配置：默认角色=%@，版本=%@，覆盖规则=%@ 条（%@）", "\n• Running configuration: default role=%@, version=%@, override rules=%@ (%@)", "\n• Configuration en service : rôle par défaut=%@, version=%@, règles de remplacement=%@ (%@)", "\n• Configurazione in esecuzione: ruolo predefinito=%@, versione=%@, regole di override=%@ (%@)", "\n• Konfiguracja działająca: rola domyślna=%@, wersja=%@, reguły nadpisań=%@ (%@)")
        case .uiBrain087:
            return e("\n• 草稿状态：%@", "\n• Draft status: %@", "\n• État du brouillon : %@", "\n• Stato della bozza: %@", "\n• Stan szkicu: %@")
        case .uiBrain088:
            return e("\n• 后端未记录该代次的详细快照；回滚只恢复该代次实际保存的配置。", "\n• The backend recorded no detailed snapshot for this generation; the rollback restores only the configuration actually saved for it.", "\n• Le backend n’a enregistré aucun instantané détaillé pour cette génération ; le retour ne restaure que la configuration réellement enregistrée pour celle-ci.", "\n• Il backend non ha registrato uno snapshot dettagliato per questa generazione; il ripristino riporta solo la configurazione effettivamente salvata per essa.", "\n• Backend nie zapisał szczegółowej migawki tej generacji; przywrócenie odtwarza tylko konfigurację faktycznie dla niej zapisaną.")
        case .uiBrain089:
            return e("已取消回滚", "Rollback canceled", "Retour annulé", "Ripristino annullato", "Przywracanie anulowane")
        case .uiBrain090:
            return e("已回退至代次 %@", "Reverted to generation %@", "Retour à la génération %@", "Ripristinata la generazione %@", "Przywrócono generację %@")
        case .uiBrain091:
            return e("（该代次未保存完整草稿，仅恢复后端已保存内容）", "(No full draft saved for this generation; only backend-saved content was restored)", "(Aucun brouillon complet enregistré pour cette génération ; seul le contenu enregistré par le backend a été restauré)", "(Nessuna bozza completa salvata per questa generazione; è stato ripristinato solo il contenuto salvato dal backend)", "(Dla tej generacji nie zapisano pełnego szkicu; przywrócono tylko treść zapisaną przez backend)")
        case .uiBrain092:
            return e("请先保存角色草稿，再执行「试一试」。", "Save the role draft first, then run “Try it”.", "Enregistrez d’abord le brouillon du rôle, puis lancez « Essai ».", "Salva prima la bozza del ruolo, poi esegui «Prova».", "Najpierw zapisz szkic roli, a potem uruchom „Wypróbuj”.")
        case .uiBrain093:
            return e("请先选择一个角色，再执行「试一试」。", "Select a role first, then run “Try it”.", "Sélectionnez d’abord un rôle, puis lancez « Essai ».", "Seleziona prima un ruolo, poi esegui «Prova».", "Najpierw wybierz rolę, a potem uruchom „Wypróbuj”.")
        case .uiBrain094:
            return e("请输入客户问题正文", "Enter the customer question text", "Saisissez le texte de la question du client", "Inserisci il testo della domanda del cliente", "Wpisz treść pytania klienta")
        case .uiBrain095:
            return e("试一试失败: %@\n", "Try-it failed: %@\n", "Échec de l’essai : %@\n", "Prova non riuscita: %@\n", "Próba nie powiodła się: %@\n")
        case .uiBrain096:
            return e("真实模型调用不可用：当前构建只提供 assemble_only 确定性组装预览，不会生成真实回复。\n", "Real model calls unavailable: this build only provides assemble_only deterministic assembly previews and will not generate a real reply.\n", "Appels réels au modèle indisponibles : cette version ne propose que des aperçus d’assemblage déterministes assemble_only et ne génère aucune réponse réelle.\n", "Chiamate reali al modello non disponibili: questa build offre solo anteprime di assemblaggio deterministiche assemble_only e non genera risposte reali.\n", "Rzeczywiste wywołania modelu niedostępne: ta kompilacja oferuje tylko deterministyczne podglądy składania assemble_only i nie generuje prawdziwej odpowiedzi.\n")
        case .uiBrain097:
            return e("【试一试结果】\n", "[Try-it result]\n", "[Résultat de l’essai]\n", "[Risultato della prova]\n", "[Wynik próby]\n")
        case .uiBrain098:
            return e("客户问题: %@\n", "Customer question: %@\n", "Question du client : %@\n", "Domanda del cliente: %@\n", "Pytanie klienta: %@\n")
        case .uiBrain099:
            return e("费用说明: %@\n", "Cost note: %@\n", "Note sur les coûts : %@\n", "Nota sui costi: %@\n", "Informacje o kosztach: %@\n")
        case .uiBrain100:
            return e("\n【预览回复】\n", "\n[Preview reply]\n", "\n[Réponse prévisualisée]\n", "\n[Risposta di anteprima]\n", "\n[Podgląd odpowiedzi]\n")
        case .uiBrain101:
            return e("后端返回了决策，但未包含 reply 字段；不编造回复内容。\n", "The backend returned a decision but no reply field; no reply content will be invented.\n", "Le backend a renvoyé une décision sans champ reply ; aucun contenu de réponse ne sera inventé.\n", "Il backend ha restituito una decisione ma nessun campo reply; nessun contenuto di risposta verrà inventato.\n", "Backend zwrócił decyzję, ale bez pola reply; treść odpowiedzi nie będzie wymyślana.\n")
        case .uiBrain102:
            return e("不可用：本次为确定性组装预览 (assemble_only)，未调用真实模型，无法生成回复文本。\n", "Unavailable: this is a deterministic assembly preview (assemble_only); no real model was called, so no reply text can be generated.\n", "Indisponible : il s’agit d’un aperçu d’assemblage déterministe (assemble_only) ; aucun modèle réel n’a été appelé, aucun texte de réponse ne peut être généré.\n", "Non disponibile: questa è un’anteprima di assemblaggio deterministica (assemble_only); non è stato chiamato alcun modello reale, quindi non è possibile generare testo di risposta.\n", "Niedostępne: to deterministyczny podgląd składania (assemble_only); nie wywołano rzeczywistego modelu, więc nie można wygenerować tekstu odpowiedzi.\n")
        case .uiBrain103:
            return e("\n【系统约束】\n", "\n[System constraints]\n", "\n[Contraintes système]\n", "\n[Vincoli di sistema]\n", "\n[Ograniczenia systemu]\n")
        case .uiBrain104:
            return e("后端未返回独立系统约束字段（组装内容中未解析到 system_contract）；不做推测。\n", "The backend returned no separate system-constraint field (no system_contract parsed from the assembled content); nothing is inferred.\n", "Le backend n’a renvoyé aucun champ de contraintes système distinct (aucun system_contract analysé dans le contenu assemblé) ; aucune supposition n’est faite.\n", "Il backend non ha restituito un campo separato per i vincoli di sistema (nessun system_contract rilevato nel contenuto assemblato); non vengono fatte supposizioni.\n", "Backend nie zwrócił osobnego pola ograniczeń systemowych (w złożonej treści nie odnaleziono system_contract); niczego nie zakładamy.\n")
        case .uiBrain105:
            return e("\n【组装内容 (compiled)】\n", "\n[Assembled content (compiled)]\n", "\n[Contenu assemblé (compiled)]\n", "\n[Contenuto assemblato (compiled)]\n", "\n[Złożona treść (compiled)]\n")
        case .uiBrain106:
            return e("未保存完整草稿", "No full draft saved", "Aucun brouillon complet enregistré", "Nessuna bozza completa salvata", "Brak zapisanego pełnego szkicu")
        case .uiBrain107:
            return e("第 %@ 代 · %@ · 操作=%@", "Generation %@ · %@ · operation=%@", "Génération %@ · %@ · opération=%@", "Generazione %@ · %@ · operazione=%@", "Generacja %@ · %@ · operacja=%@")
        case .uiBrain108:
            return e("写入角色: %@ · 写入版本: %@", "Written role: %@ · written version: %@", "Rôle écrit : %@ · version écrite : %@", "Ruolo scritto: %@ · versione scritta: %@", "Zapisana rola: %@ · zapisana wersja: %@")
        case .uiBrain109:
            return e("运行/草稿配置: 后端未记录该代次的绑定快照（仅写入结果）", "Running/draft configuration: the backend recorded no binding snapshot for this generation (write results only)", "Configuration en service/brouillon : le backend n’a enregistré aucun instantané de liaison pour cette génération (résultats d’écriture uniquement)", "Configurazione in esecuzione/bozza: il backend non ha registrato uno snapshot di associazione per questa generazione (solo risultati di scrittura)", "Konfiguracja działająca/szkic: backend nie zapisał migawki powiązania dla tej generacji (tylko wyniki zapisu)")
        case .uiBrain110:
            return e("运行配置: 默认角色=%@ · 版本=%@ · 覆盖规则=%@ 条", "Running configuration: default role=%@ · version=%@ · %@ override rules", "Configuration en service : rôle par défaut=%@ · version=%@ · %@ règles de remplacement", "Configurazione in esecuzione: ruolo predefinito=%@ · versione=%@ · regole di override: %@", "Konfiguracja działająca: rola domyślna=%@ · wersja=%@ · reguły nadpisań: %@")
        case .uiBrain111:
            return e("草稿状态: 已保存完整草稿", "Draft status: full draft saved", "État du brouillon : brouillon complet enregistré", "Stato della bozza: bozza completa salvata", "Stan szkicu: zapisano pełny szkic")
        case .uiBrain112:
            return e("草稿状态: 该代次未保存完整草稿，仅显示后端已保存内容（不补全、不编造）", "Draft status: no full draft saved for this generation; showing only backend-saved content (nothing filled in or invented)", "État du brouillon : aucun brouillon complet enregistré pour cette génération ; seul le contenu enregistré par le backend est affiché (sans complétion ni invention)", "Stato della bozza: nessuna bozza completa salvata per questa generazione; viene mostrato solo il contenuto salvato dal backend (senza completamenti né invenzioni)", "Stan szkicu: dla tej generacji nie zapisano pełnego szkicu; wyświetlana jest tylko treść zapisana przez backend (bez uzupełniania i wymyślania)")
        case .uiMcp113:
            return e("无效 UTF8 数据", "Invalid UTF8 data", "Données UTF8 non valides", "Dati UTF8 non validi", "Nieprawidłowe dane UTF8")
        case .uiMcp114:
            return e("角色定义、知识与话术请前往【客服角色】页面管理；本页面仅管理回复模式、外部超时与对外接口连接。", "Manage role definitions, knowledge and scripts on the [Brain] page; this page only manages the Brain mode, external timeouts and the external interface connection.", "La définition des rôles, les connaissances et les scripts se gèrent sur la page [Brain] ; cette page ne gère que le mode Brain, les délais d’attente externes et la connexion de l’interface externe.", "Definizioni dei ruoli, conoscenze e copioni si gestiscono nella pagina [Brain]; questa pagina gestisce solo la modalità Brain, i timeout esterni e la connessione dell’interfaccia esterna.", "Definicje ról, wiedzę i skrypty konfiguruje się na stronie [Brain]; ta strona zarządza tylko trybem Brain, limitami czasu zewnętrznego i połączeniem interfejsu zewnętrznego.")
        case .uiMcp115:
            return e("内置大脑 (调用配置好的大模型生成回复)", "Built-in Brain (calls the configured large model to generate replies)", "Brain intégré (appelle le grand modèle configuré pour générer les réponses)", "Brain integrato (chiama il modello di grandi dimensioni configurato per generare risposte)", "Wbudowany Brain (wywołuje skonfigurowany duży model, aby generować odpowiedzi)")
        case .uiMcp116:
            return e("外部大脑 (MCP 协议接管消息轮询与回复决策)", "External Brain (the MCP protocol takes over message polling and reply decisions)", "Brain externe (le protocole MCP prend en charge l’interrogation des messages et les décisions de réponse)", "Brain esterno (il protocollo MCP gestisce il polling dei messaggi e le decisioni di risposta)", "Zewnętrzny Brain (protokół MCP przejmuje odpytywanie wiadomości i decyzje o odpowiedzi)")
        case .uiMcp117:
            return e("保存大脑设置", "Save Brain settings", "Enregistrer les réglages Brain", "Salva impostazioni Brain", "Zapisz ustawienia Brain")
        case .uiMcp118:
            return e("启用对外接口（外部大脑通过 HTTP 接入）", "Enable external interface (external Brain connects over HTTP)", "Activer l’interface externe (le Brain externe se connecte via HTTP)", "Abilita interfaccia esterna (il Brain esterno si collega tramite HTTP)", "Włącz interfejs zewnętrzny (zewnętrzny Brain łączy się przez HTTP)")
        case .uiMcp119:
            return e("回复决策大脑", "Reply decision Brain", "Brain décisionnel des réponses", "Brain per le decisioni di risposta", "Brain decydujący o odpowiedziach")
        case .uiMcp120:
            return e("等待回复:", "Reply wait:", "Attente de réponse :", "Attesa risposta:", "Oczekiwanie na odpowiedź:")
        case .uiMcp121:
            return e("超时后动作:", "Action on timeout:", "Action après expiration :", "Azione allo scadere:", "Działanie po przekroczeniu czasu:")
        case .uiMcp122:
            return e("最长排队:", "Max queue wait:", "Attente max. en file :", "Attesa massima in coda:", "Maks. czas w kolejce:")
        case .uiMcp123:
            return e("前往大脑页面", "Go to Brain page", "Aller à la page Brain", "Vai alla pagina Brain", "Przejdź do strony Brain")
        case .uiMcp124:
            return e("Bearer <令牌>", "Bearer <token>", "Bearer <token>", "Bearer <token>", "Bearer <token>")
        case .uiMcp125:
            return e("开关状态: 已开启", "Switch: on", "Interrupteur : activé", "Interruttore: attivo", "Przełącznik: włączony")
        case .uiMcp126:
            return e("开关状态: 已关闭", "Switch: off", "Interrupteur : désactivé", "Interruttore: disattivo", "Przełącznik: wyłączony")
        case .uiMcp127:
            return e("接口地址: http://%@:%@/mcp", "Endpoint: http://%@:%@/mcp", "Adresse de l’interface : http://%@:%@/mcp", "Indirizzo dell’interfaccia: http://%@:%@/mcp", "Adres interfejsu: http://%@:%@/mcp")
        case .uiMcp128:
            return e("大脑设置已保存", "Brain settings saved", "Réglages Brain enregistrés", "Impostazioni Brain salvate", "Ustawienia Brain zapisane")
        case .uiMcp129:
            return e("已开启，服务重启后生效（约数秒）", "Enabled; takes effect after the service restarts (a few seconds)", "Activé ; prend effet après le redémarrage du service (quelques secondes)", "Attivato; ha effetto dopo il riavvio del servizio (pochi secondi)", "Włączono; zacznie działać po ponownym uruchomieniu usługi (kilka sekund)")
        case .uiMcp130:
            return e("已关闭，服务重启后生效（约数秒）", "Disabled; takes effect after the service restarts (a few seconds)", "Désactivé ; prend effet après le redémarrage du service (quelques secondes)", "Disattivato; ha effetto dopo il riavvio del servizio (pochi secondi)", "Wyłączono; zacznie działać po ponownym uruchomieniu usługi (kilka sekund)")
        case .uiMcp131:
            return e("重新生成对外接口令牌", "Regenerate external interface token", "Régénérer le token de l’interface externe", "Rigenera il token dell’interfaccia esterna", "Wygeneruj ponownie token interfejsu zewnętrznego")
        case .uiMcp132:
            return e("重新生成后将立即生效，当前连接的大脑将会断开。确定重新生成吗？", "Regenerating takes effect immediately and disconnects the currently connected Brain. Regenerate now?", "La régénération prend effet immédiatement et déconnecte le Brain actuellement connecté. Régénérer maintenant ?", "La rigenerazione ha effetto immediato e disconnette il Brain attualmente connesso. Rigenerare ora?", "Ponowne wygenerowanie zadziała natychmiast i rozłączy obecnie podłączony Brain. Wygenerować ponownie?")
        case .uiMcp133:
            return e("连接正常 (可用工具: %@ 个)", "Connected (available tools: %@)", "Connexion correcte (outils disponibles : %@)", "Connessione attiva (strumenti disponibili: %@)", "Połączenie działa (dostępne narzędzia: %@)")
        case .uiMcp134:
            return e("已有大脑连接中", "A Brain is already connected", "Un Brain est déjà connecté", "Un Brain è già connesso", "Brain jest już podłączony")
        case .uiMcp135:
            return e("外部大脑接口", "External Brain interface", "Interface du Brain externe", "Interfaccia del Brain esterno", "Interfejs zewnętrznego Brain")
        case .uiUpdate136:
            return e("更新器输出或管道超出限制", "Updater output or pipe exceeded the limit", "La sortie ou le pipe du programme de mise à jour a dépassé la limite", "L’output o la pipe del programma di aggiornamento hanno superato il limite", "Wynik lub potok aktualizatora przekroczył limit")
        case .uiUpdate137:
            return e("%@ / %@ (不可信)", "%@ / %@ (untrusted)", "%@ / %@ (non fiable)", "%@ / %@ (non attendibile)", "%@ / %@ (niezaufane)")
        case .uiUpdate138:
            return e("%@ (防降级高水位，非安装版本)", "%@ (anti-downgrade high-water mark, not the installed version)", "%@ (seuil haut anti-rétrogradation, pas la version installée)", "%@ (limite anti-degradazione, non la versione installata)", "%@ (znacznik antydegradacji, nie zainstalowana wersja)")
        case .uiUpdate139:
            return e("命令执行错误 [%@]: %@", "Command execution error [%@]: %@", "Erreur d’exécution de la commande [%@] : %@", "Errore di esecuzione del comando [%@]: %@", "Błąd wykonania polecenia [%@]: %@")
        case .uiUpdate140:
            return e("回滚到上一版本…", "Roll back to the previous version…", "Revenir à la version précédente…", "Ripristino alla versione precedente…", "Przywracanie poprzedniej wersji…")
        case .uiUpdate141:
            return e("整包换回上一个已安装的应用程序，执行前需要二次确认", "Swap the whole bundle back to the previously installed application; a confirmation is required before running", "Remplacer l’intégralité du paquet par l’application précédemment installée ; une confirmation est requise avant l’exécution", "Sostituisce l’intero bundle con l’applicazione installata in precedenza; è necessaria una conferma prima dell’esecuzione", "Przywraca cały pakiet do poprzednio zainstalowanej aplikacji; przed wykonaniem wymagane jest potwierdzenie")
        case .uiUpdate142:
            return e("查询中…", "Checking…", "Vérification…", "Verifica in corso…", "Sprawdzanie…")
        case .uiUpdate143:
            return e("备份记录时间：", "Backup record time:", "Date d’enregistrement de la sauvegarde :", "Ora del record di backup:", "Czas zapisu kopii zapasowej:")
        case .uiUpdate144:
            return e("当前不可回滚：%@", "Rollback unavailable: %@", "Retour indisponible : %@", "Ripristino non disponibile: %@", "Przywracanie niedostępne: %@")
        case .uiUpdate145:
            return e("回滚未完成 (退出码 %@)", "Rollback did not complete (exit code %@)", "Le retour ne s’est pas terminé (code de sortie %@)", "Il ripristino non è stato completato (codice di uscita %@)", "Przywracanie nie zostało ukończone (kod wyjścia %@)")
        case .uiUpdate146:
            return e("无法将字符串转为 UTF-8 字节", "Cannot convert string to UTF-8 bytes", "Impossible de convertir la chaîne en octets UTF-8", "Impossibile convertire la stringa in byte UTF-8", "Nie można przekonwertować ciągu na bajty UTF-8")
        case .uiUpdate147:
            return e("未知阶段", "Unknown stage", "Étape inconnue", "Fase sconosciuta", "Nieznany etap")
        case .uiUpdate148:
            return e("无事务ID", "No transaction ID", "Aucun ID de transaction", "Nessun ID transazione", "Brak ID transakcji")
        case .uiUpdate149:
            return e("可信更新器不可用", "Trusted updater unavailable", "Programme de mise à jour de confiance indisponible", "Programma di aggiornamento attendibile non disponibile", "Zaufany aktualizator niedostępny")
        case .uiUpdate150:
            return e("更新清单尚未验证或更新器暂不支持下载", "The update manifest is not verified yet, or the updater does not support downloads", "Le manifeste de mise à jour n’est pas encore vérifié, ou le programme de mise à jour ne prend pas en charge le téléchargement", "Il manifest di aggiornamento non è ancora verificato oppure il programma di aggiornamento non supporta i download", "Manifest aktualizacji nie został jeszcze zweryfikowany albo aktualizator nie obsługuje pobierania")
        case .uiUpdate151:
            return e("更新缓存已满，请清理未使用候选后重试", "The update cache is full; clean up unused candidates and try again", "Le cache de mise à jour est plein ; supprimez les candidats inutilisés et réessayez", "La cache degli aggiornamenti è piena; rimuovi i candidati inutilizzati e riprova", "Pamięć podręczna aktualizacji jest pełna; usuń nieużywane kandydaty i spróbuj ponownie")
        case .uiUpdate152:
            return e("已有更新包正在下载，请稍后重试", "An update package is already downloading; try again later", "Un paquet de mise à jour est déjà en cours de téléchargement ; réessayez plus tard", "È già in corso il download di un pacchetto di aggiornamento; riprova più tardi", "Trwa już pobieranie pakietu aktualizacji; spróbuj ponownie później")
        case .uiUpdate153:
            return e("下载或校验未完成，请重试", "Download or verification did not complete; please try again", "Le téléchargement ou la vérification ne s’est pas terminé ; veuillez réessayer", "Download o verifica non completati; riprova", "Pobieranie lub weryfikacja nie zostały ukończone; spróbuj ponownie")
        case .uiUpdate154:
            return e("更新源或下载结果已变化，请重新检查更新", "The update source or download result has changed; check for updates again", "La source de mise à jour ou le résultat du téléchargement a changé ; vérifiez à nouveau les mises à jour", "L’origine degli aggiornamenti o il risultato del download è cambiato; controlla di nuovo gli aggiornamenti", "Źródło aktualizacji lub wynik pobierania uległy zmianie; sprawdź aktualizacje ponownie")
        case .uiUpdate155:
            return e("加载中...", "Loading...", "Chargement...", "Caricamento...", "Wczytywanie...")
        case .uiUpdate156:
            return e("未校验 (未知)", "Not verified (unknown)", "Non vérifié (inconnu)", "Non verificato (sconosciuto)", "Niezweryfikowane (nieznane)")
        case .uiUpdate157:
            return e("尚未检查更新。点击「检查更新」读取授权清单状态；未配置更新源时不会联网或下载。", "Updates have not been checked yet. Click “Check for updates” to read the authorized manifest status; without a configured update source, nothing connects to the network or downloads.", "Les mises à jour n’ont pas encore été vérifiées. Cliquez sur « Rechercher des mises à jour » pour lire l’état du manifeste autorisé ; sans source de mise à jour configurée, aucun accès réseau ni téléchargement n’a lieu.", "Gli aggiornamenti non sono ancora stati controllati. Fai clic su «Verifica aggiornamenti» per leggere lo stato del manifest autorizzato; senza un’origine degli aggiornamenti configurata non vengono effettuati accessi alla rete né download.", "Nie sprawdzono jeszcze aktualizacji. Kliknij „Sprawdź aktualizacje”, aby odczytać stan autoryzowanego manifestu; bez skonfigurowanego źródła aktualizacji nie ma połączenia sieciowego ani pobierania.")
        case .uiUpdate158:
            return e("只读", "Read-only", "Lecture seule", "Sola lettura", "Tylko odczyt")
        case .uiUpdate159:
            return e("只读监控中", "Read-only monitoring", "Surveillance en lecture seule", "Monitoraggio in sola lettura", "Monitorowanie tylko do odczytu")
        case .uiUpdate160:
            return e("检查更新", "Check for updates", "Rechercher des mises à jour", "Verifica aggiornamenti", "Sprawdź aktualizacje")
        case .uiUpdate161:
            return e("刷新状态", "Refresh status", "Actualiser l’état", "Aggiorna stato", "Odśwież stan")
        case .uiUpdate162:
            return e("下载更新", "Download update", "Télécharger la mise à jour", "Scarica aggiornamento", "Pobierz aktualizację")
        case .uiUpdate163:
            return e("安装并重启", "Install and restart", "Installer et redémarrer", "Installa e riavvia", "Zainstaluj i uruchom ponownie")
        case .uiUpdate164:
            return e("延期更新", "Defer update", "Reporter la mise à jour", "Rinvia aggiornamento", "Odrocz aktualizację")
        case .uiUpdate165:
            return e("受控写入（当前未启用）：下载、安装、延期由真实更新事务控制，界面不会模拟成功。", "Controlled writes (currently disabled): download, install and defer are driven by real update transactions; the interface will not simulate success.", "Écritures contrôlées (actuellement désactivées) : le téléchargement, l’installation et le report sont pilotés par de vraies transactions de mise à jour ; l’interface ne simule pas de réussite.", "Scritture controllate (attualmente disattivate): download, installazione e rinvio sono gestiti da transazioni di aggiornamento reali; l’interfaccia non simula il successo.", "Zapisy kontrolowane (obecnie wyłączone): pobieranie, instalacja i odroczenie są sterowane rzeczywistymi transakcjami aktualizacji; interfejs nie symuluje sukcesu.")
        case .uiUpdate166:
            return e("当前处于只读状态查看阶段；真实下载、安装、排空延期与恢复写入均已安全禁用；整包回滚由回滚卡片单独控制。", "Currently in a read-only viewing phase; real downloads, installs, draining deferrals and restore writes are safely disabled; full-bundle rollback is controlled separately by the rollback card.", "Phase d’observation en lecture seule ; les téléchargements réels, installations, reports en cours de vidage et écritures de restauration sont désactivés en toute sécurité ; le retour de l’intégralité du paquet est contrôlé séparément par la carte de retour.", "Fase di visualizzazione in sola lettura; download, installazioni, rinvii in svuotamento e scritture di ripristino reali sono disattivati in sicurezza; il ripristino dell’intero bundle è controllato separatamente dalla scheda di ripristino.", "Trwa etap przeglądania tylko do odczytu; rzeczywiste pobierania, instalacje, odroczenia w trakcie opróżniania i zapisy przywracania są bezpiecznie wyłączone; przywracanie całego pakietu jest sterowane osobno przez kartę przywracania.")
        case .uiUpdate167:
            return e("显示技术详情", "Show technical details", "Afficher les détails techniques", "Mostra dettagli tecnici", "Pokaż szczegóły techniczne")
        case .uiUpdate168:
            return e("版本与更新", "Version and updates", "Version et mises à jour", "Versione e aggiornamenti", "Wersja i aktualizacje")
        case .uiUpdate169:
            return e("发布时间", "Release date", "Date de publication", "Data di pubblicazione", "Data publikacji")
        case .uiUpdate170:
            return e("适用环境", "Target environment", "Environnement cible", "Ambiente di destinazione", "Środowisko docelowe")
        case .uiUpdate171:
            return e("当前仅支持检查状态，此操作尚未启用", "Only status checking is supported right now; this action is not enabled yet", "Seule la vérification de l’état est prise en charge pour le moment ; cette action n’est pas encore activée", "Al momento è supportato solo il controllo dello stato; questa azione non è ancora attiva", "Obecnie obsługiwane jest tylko sprawdzanie stanu; ta czynność nie jest jeszcze włączona")
        case .uiUpdate172:
            return e("需要人工恢复", "Manual recovery required", "Récupération manuelle requise", "Ripristino manuale necessario", "Wymagane ręczne odzyskanie")
        case .uiUpdate173:
            return e("核心编译版本", "Core build version", "Version de compilation du cœur", "Versione di build del core", "Wersja kompilacji rdzenia")
        case .uiUpdate174:
            return e("整套安装序列", "Full install sequence", "Séquence d’installation complète", "Sequenza di installazione completa", "Pełna sekwencja instalacji")
        case .uiUpdate175:
            return e("防降级高水位", "Anti-downgrade high-water mark", "Seuil haut anti-rétrogradation", "Limite anti-degradazione", "Znacznik antydegradacji")
        case .uiUpdate176:
            return e("发布状态与来源", "Release status and source", "État et source de la publication", "Stato e origine della release", "Stan i źródło wydania")
        case .uiUpdate177:
            return e("宿主运行架构", "Host architecture", "Architecture de l’hôte", "Architettura host", "Architektura hosta")
        case .uiUpdate178:
            return e("组件一致性", "Component consistency", "Cohérence des composants", "Coerenza dei componenti", "Spójność komponentów")
        case .uiUpdate179:
            return e("维护抑制状态", "Maintenance hold state", "État de blocage pour maintenance", "Stato di blocco per manutenzione", "Stan wstrzymania konserwacyjnego")
        case .uiUpdate180:
            return e("更新包大小", "Update package size", "Taille du paquet de mise à jour", "Dimensione del pacchetto di aggiornamento", "Rozmiar pakietu aktualizacji")
        case .uiUpdate181:
            return e("发布者验证", "Publisher verification", "Vérification de l’éditeur", "Verifica dell’editore", "Weryfikacja wydawcy")
        case .uiUpdate182:
            return e("本机重签名", "Local re-signing", "Resignature locale", "Firma locale", "Lokalne ponowne podpisanie")
        case .uiUpdate183:
            return e("组件版本不一致", "Component versions do not match", "Les versions des composants ne correspondent pas", "Le versioni dei componenti non corrispondono", "Wersje komponentów nie są zgodne")
        case .uiUpdate184:
            return e("响应未提供发布者验签成功证明", "The response provided no proof of successful publisher signature verification", "La réponse ne fournissait aucune preuve de vérification réussie de la signature de l’éditeur", "La risposta non forniva alcuna prova di verifica riuscita della firma dell’editore", "Odpowiedź nie zawierała dowodu pomyślnej weryfikacji podpisu wydawcy")
        case .uiUpdate185:
            return e("此版本已有匹配的安装记录", "A matching install record already exists for this version", "Un enregistrement d’installation correspondant existe déjà pour cette version", "Esiste già un record di installazione corrispondente per questa versione", "Dla tej wersji istnieje już pasujący zapis instalacji")
        case .uiUpdate186:
            return e("清单签名与安装记录已核对", "Manifest signature and install record verified", "Signature du manifeste et enregistrement d’installation vérifiés", "Firma del manifest e record di installazione verificati", "Podpis manifestu i zapis instalacji zostały zweryfikowane")
        case .uiUpdate187:
            return e("运行状态需单独核验", "Running state must be verified separately", "L’état d’exécution doit être vérifié séparément", "Lo stato di esecuzione va verificato separatamente", "Stan działania wymaga osobnej weryfikacji")
        case .uiUpdate188:
            return e("清单与当前环境兼容", "Manifest is compatible with the current environment", "Le manifeste est compatible avec l’environnement actuel", "Il manifest è compatibile con l’ambiente attuale", "Manifest jest zgodny z bieżącym środowiskiem")
        case .uiUpdate189:
            return e("更新检查未通过", "Update check failed", "La recherche de mise à jour a échoué", "Verifica aggiornamenti non riuscita", "Sprawdzanie aktualizacji nie powiodło się")
        case .uiUpdate190:
            return e("签名或环境检查未通过", "Signature or environment check failed", "La vérification de la signature ou de l’environnement a échoué", "Verifica della firma o dell’ambiente non riuscita", "Weryfikacja podpisu lub środowiska nie powiodła się")
        case .uiUpdate191:
            return e("发现新版本，清单签名已验证", "New version found; manifest signature verified", "Nouvelle version détectée ; signature du manifeste vérifiée", "Nuova versione trovata; firma del manifest verificata", "Znaleziono nową wersję; podpis manifestu zweryfikowany")
        case .uiUpdate192:
            return e("清单与当前系统兼容，安装包待校验", "Manifest is compatible with the current system; the installer still needs verification", "Le manifeste est compatible avec le système actuel ; le paquet d’installation reste à vérifier", "Il manifest è compatibile con il sistema attuale; il pacchetto di installazione deve ancora essere verificato", "Manifest jest zgodny z bieżącym systemem; pakiet instalacyjny czeka na weryfikację")
        case .uiUpdate193:
            return e("安装包与本机签名尚未校验", "The installer and local signature have not been verified yet", "Le paquet d’installation et la signature locale n’ont pas encore été vérifiés", "Il pacchetto di installazione e la firma locale non sono ancora stati verificati", "Pakiet instalacyjny i podpis lokalny nie zostały jeszcze zweryfikowane")
        case .uiUpdate194:
            return e("下载中…", "Downloading…", "Téléchargement…", "Download in corso…", "Pobieranie…")
        case .uiUpdate195:
            return e("已下载", "Downloaded", "Téléchargé", "Scaricato", "Pobrano")
        case .uiUpdate196:
            return e("校验本地更新包", "Verify local update package", "Vérifier le paquet de mise à jour local", "Verifica il pacchetto di aggiornamento locale", "Zweryfikuj lokalny pakiet aktualizacji")
        case .uiUpdate197:
            return e("更新包已下载，安装前将继续验证", "The update package is downloaded; verification continues before installation", "Le paquet de mise à jour est téléchargé ; la vérification se poursuit avant l’installation", "Il pacchetto di aggiornamento è stato scaricato; la verifica proseguirà prima dell’installazione", "Pakiet aktualizacji został pobrany; weryfikacja będzie kontynuowana przed instalacją")
        case .uiUpdate198:
            return e("安装包摘要已验证，尚未解包与本机签名", "The installer digest is verified; it has not been unpacked or locally signed yet", "L’empreinte du paquet d’installation est vérifiée ; il n’a pas encore été décompressé ni signé localement", "Il digest del pacchetto di installazione è verificato; non è ancora stato scompattato né firmato localmente", "Skrót pakietu instalacyjnego został zweryfikowany; nie został jeszcze rozpakowany ani podpisany lokalnie")
        case .uiUpdate199:
            return e("本机有候选缓存，需重新校验更新包", "A candidate is cached locally; the update package must be verified again", "Un candidat est en cache local ; le paquet de mise à jour doit être vérifié à nouveau", "Un candidato è in cache locale; il pacchetto di aggiornamento deve essere verificato di nuovo", "Kandydat jest w lokalnej pamięci podręcznej; pakiet aktualizacji wymaga ponownej weryfikacji")
        case .uiUpdate200:
            return e("缓存尚未重新校验，不代表已安装", "The cache has not been re-verified; this does not mean it is installed", "Le cache n’a pas été vérifié à nouveau ; cela ne signifie pas qu’il est installé", "La cache non è stata verificata di nuovo; ciò non significa che sia installato", "Pamięć podręczna nie została ponownie zweryfikowana; nie oznacza to, że jest zainstalowana")
        case .uiUpdate201:
            return e("下载到本机私有缓存，不会停止业务或安装", "Downloads to a local private cache; does not stop operations or install anything", "Télécharge dans un cache privé local ; n’arrête pas l’activité et n’installe rien", "Scarica in una cache privata locale; non interrompe l’attività né installa nulla", "Pobiera do lokalnej prywatnej pamięci podręcznej; nie wstrzymuje działania ani nic nie instaluje")
        case .uiUpdate202:
            return e("当前更新器未报告可用的下载能力", "The updater does not currently report download capability", "Le programme de mise à jour ne signale actuellement aucune capacité de téléchargement", "Il programma di aggiornamento attualmente non segnala capacità di download", "Aktualizator nie zgłasza obecnie możliwości pobierania")
        case .uiUpdate203:
            return e("可下载", "Download available", "Téléchargement possible", "Download disponibile", "Pobieranie dostępne")
        case .uiUpdate204:
            return e("支持已验证清单的受限下载；安装、延期和恢复尚未启用。", "Restricted downloads of verified manifests are supported; install, defer and restore are not enabled yet.", "Les téléchargements restreints de manifestes vérifiés sont pris en charge ; l’installation, le report et la restauration ne sont pas encore activés.", "Sono supportati download limitati di manifest verificati; installazione, rinvio e ripristino non sono ancora attivi.", "Obsługiwane jest ograniczone pobieranie zweryfikowanych manifestów; instalacja, odroczenie i przywracanie nie są jeszcze włączone.")
        case .uiUpdate205:
            return e("仅检查", "Check only", "Vérification uniquement", "Solo verifica", "Tylko sprawdzanie")
        case .uiUpdate206:
            return e("下载需由更新器报告支持；安装、延期和恢复尚未启用。", "Downloads require the updater to report support; install, defer and restore are not enabled yet.", "Le téléchargement nécessite que le programme de mise à jour signale sa prise en charge ; l’installation, le report et la restauration ne sont pas encore activés.", "Il download richiede che il programma di aggiornamento ne segnali il supporto; installazione, rinvio e ripristino non sono ancora attivi.", "Pobieranie wymaga zgłoszenia obsługi przez aktualizator; instalacja, odroczenie i przywracanie nie są jeszcze włączone.")
        case .uiUpdate207:
            return e("尚未配置授权更新清单；当前不会联网、下载或安装。", "No authorized update manifest is configured; nothing connects to the network, downloads or installs.", "Aucun manifeste de mise à jour autorisé n’est configuré ; aucun accès réseau, téléchargement ou installation n’a lieu.", "Nessun manifest di aggiornamento autorizzato è configurato; non vengono effettuati accessi alla rete, download o installazioni.", "Nie skonfigurowano autoryzowanego manifestu aktualizacji; nie ma połączenia sieciowego, pobierania ani instalacji.")
        case .uiUpdate208:
            return e("尚未配置授权更新源；当前仅展示本地状态，不会联网、下载或安装。", "No authorized update source is configured; only local status is shown and nothing connects to the network, downloads or installs.", "Aucune source de mise à jour autorisée n’est configurée ; seul l’état local est affiché, sans accès réseau, téléchargement ni installation.", "Nessuna origine di aggiornamento autorizzata è configurata; viene mostrato solo lo stato locale e non si effettuano accessi alla rete, download o installazioni.", "Nie skonfigurowano autoryzowanego źródła aktualizacji; wyświetlany jest tylko stan lokalny, bez połączenia sieciowego, pobierania i instalacji.")
        case .uiUpdate209:
            return e("空闲 (只读状态)", "Idle (read-only state)", "Inactif (état en lecture seule)", "Inattivo (stato di sola lettura)", "Bezczynny (stan tylko do odczytu)")
        case .uiUpdate210:
            return e("状态: %@ (外部事务)", "Status: %@ (external transaction)", "État : %@ (transaction externe)", "Stato: %@ (transazione esterna)", "Stan: %@ (transakcja zewnętrzna)")
        case .uiUpdate211:
            return e("外部更新事务处于排空等待态；当前界面为只读，延期请由管理员命令行协调。", "An external update transaction is waiting to drain; this interface is read-only — coordinate deferrals via the administrator command line.", "Une transaction de mise à jour externe attend d’être vidée ; cette interface est en lecture seule — coordonnez les reports via la ligne de commande administrateur.", "Una transazione di aggiornamento esterna è in attesa di svuotamento; questa interfaccia è in sola lettura — coordina i rinvii tramite la riga di comando dell’amministratore.", "Zewnętrzna transakcja aktualizacji oczekuje na opróżnienie; ten interfejs jest tylko do odczytu — odroczenia koordynuj przez wiersz poleceń administratora.")
        case .uiUpdate212:
            return e("正在查询更新状态 (只读查询)...", "Checking update status (read-only query)...", "Interrogation de l’état des mises à jour (requête en lecture seule)...", "Interrogazione dello stato degli aggiornamenti (query in sola lettura)...", "Sprawdzanie stanu aktualizacji (zapytanie tylko do odczytu)...")
        case .uiUpdate213:
            return e("检查未通过，请查看提示。", "The check failed; see the message.", "La vérification a échoué ; consultez le message.", "La verifica non è riuscita; consulta il messaggio.", "Sprawdzanie nie powiodło się; zobacz komunikat.")
        case .uiUpdate214:
            return e("清单检查完成，本次未下载或安装。", "Manifest check complete; nothing was downloaded or installed.", "Vérification du manifeste terminée ; aucun téléchargement ni installation n’a eu lieu.", "Controllo del manifest completato; non è stato scaricato né installato nulla.", "Sprawdzanie manifestu zakończone; nic nie pobrano ani nie zainstalowano.")
        case .uiUpdate215:
            return e("正在下载并校验更新包，不改变业务运行状态。", "Downloading and verifying the update package; the running service state is unchanged.", "Téléchargement et vérification du paquet de mise à jour ; l’état d’activité reste inchangé.", "Download e verifica del pacchetto di aggiornamento in corso; lo stato operativo resta invariato.", "Pobieranie i weryfikacja pakietu aktualizacji; stan działania usługi pozostaje bez zmian.")
        case .uiUpdate216:
            return e("更新包已下载并校验，尚未安装。", "The update package has been downloaded and verified; it is not installed yet.", "Le paquet de mise à jour a été téléchargé et vérifié ; il n’est pas encore installé.", "Il pacchetto di aggiornamento è stato scaricato e verificato; non è ancora installato.", "Pakiet aktualizacji został pobrany i zweryfikowany; nie jest jeszcze zainstalowany.")
        case .uiUpdate217:
            return e("下载结果与当前清单不同，请重新检查更新。", "The download result differs from the current manifest; check for updates again.", "Le résultat du téléchargement diffère du manifeste actuel ; vérifiez à nouveau les mises à jour.", "Il risultato del download differisce dal manifest attuale; controlla di nuovo gli aggiornamenti.", "Wynik pobierania różni się od bieżącego manifestu; sprawdź aktualizacje ponownie.")
        case .uiUpdate218:
            return e("下载未完成：%@", "Download did not complete: %@", "Téléchargement non terminé : %@", "Download non completato: %@", "Pobieranie nie zostało ukończone: %@")
        case .uiUpdate219:
            return e("操作未启用: 当前阶段安装/延期/恢复入口尚未启用 (unsupported)，不进行任何写入或业务状态修改。", "Action not enabled: install/defer/restore entry points are not enabled at this stage (unsupported); no writes or operational state changes are performed.", "Action non activée : les entrées installation/report/restauration ne sont pas encore activées à ce stade (unsupported) ; aucune écriture ni modification de l’état d’activité n’est effectuée.", "Azione non attiva: le voci installa/rinvia/ripristina non sono abilitate in questa fase (unsupported); non viene eseguita alcuna scrittura né modifica dello stato operativo.", "Czynność niewłączona: opcje instalacji/odroczenia/przywracania nie są na tym etapie włączone (unsupported); nie wykonuje się żadnych zapisów ani zmian stanu działania.")
        case .uiUpdate220:
            return e("状态已刷新 (只读)。", "Status refreshed (read-only).", "État actualisé (lecture seule).", "Stato aggiornato (sola lettura).", "Stan odświeżony (tylko odczyt).")

        // MARK: 收尾：嵌套插值与多行文案
        case .uiResidualUnknownReason:
            return e("未知原因", "unknown reason", "raison inconnue", "motivo sconosciuto", "nieznany powód")
        case .uiResidualUnknown:
            return e("未知", "unknown", "inconnu", "sconosciuto", "nieznane")
        case .uiResidualBuiltinBrain:
            return e("内置大脑", "built-in brain", "cerveau intégré", "cervello integrato", "wbudowany mózg")
        case .uiResidualYes:
            return e("是", "yes", "oui", "sì", "tak")
        case .uiResidualNo:
            return e("否", "no", "non", "no", "nie")
        case .uiResidualHistoryUnavailableFmt:
            return e("版本历史暂不可用：%@", "Version history is unavailable: %@", "Historique des versions indisponible : %@", "Cronologia versioni non disponibile: %@", "Historia wersji niedostępna: %@")
        case .uiResidualAccountFmt:
            return e("当前账号: %@", "Current account: %@", "Compte actuel : %@", "Account attuale: %@", "Bieżące konto: %@")
        case .uiResidualReplyModeFmt:
            return e("回复方式：%@", "Reply mode: %@", "Mode de réponse : %@", "Modalità di risposta: %@", "Tryb odpowiedzi: %@")
        case .uiResidualModelCalledFmt:
            return e("是否调用模型: %@\\n", "Model called: %@\\n", "Modèle appelé : %@\\n", "Modello chiamato: %@\\n", "Wywołano model: %@\\n")
        case .uiResidualBudgetFmt:
            return e("字节预算: 原始=%@, 纳入=%@, 是否截断=%@\\n", "Byte budget: original=%@, included=%@, truncated=%@\\n", "Budget d'octets : original=%@, inclus=%@, tronqué=%@\\n", "Budget in byte: originale=%@, incluso=%@, troncato=%@\\n", "Budżet bajtów: oryginał=%@, uwzględniono=%@, obcięto=%@\\n")
        case .uiResidualNoCompiled:
            return e("（后端未返回组装内容）", "(backend returned no assembled content)", "(le backend n'a renvoyé aucun contenu assemblé)", "(il backend non ha restituito contenuti assemblati)", "(backend nie zwrócił złożonej treści)")
        case .uiResidualMcpTestFailedFmt:
            return e("测试失败: %@", "Test failed: %@", "Échec du test : %@", "Test non riuscito: %@", "Test nie powiódł się: %@")
        case .uiResidualConnectFailed:
            return e("连接失败", "Connection failed", "Échec de la connexion", "Connessione non riuscita", "Połączenie nie powiodło się")
        case .uiResidualMcpConfigExample:
            return e("1. 运维 SSH 入口 (终端 / SSH 转发):\\nssh <用户>@<本机地址> nc -U ~/.wechat-rpa/worker.sock\\n\\n2. 对外接口配置 (Claude Desktop / Cursor 等 HTTP MCP 客户端):\\n{\\n  \"mcpServers\": {\\n    \"wechat-brain\": {\\n      \"url\": \"http://<bind>:<port>/mcp\",\\n      \"headers\": {\\n        \"Authorization\": \"Bearer <token>\"\\n      }\\n    }\\n  }\\n}", "1. Ops SSH entry (terminal / SSH forwarding):\\nssh <user>@<host> nc -U ~/.wechat-rpa/worker.sock\\n\\n2. External API config (HTTP MCP clients such as Claude Desktop / Cursor):\\n{\\n  \"mcpServers\": {\\n    \"wechat-brain\": {\\n      \"url\": \"http://<bind>:<port>/mcp\",\\n      \"headers\": {\\n        \"Authorization\": \"Bearer <token>\"\\n      }\\n    }\\n  }\\n}", "1. Accès SSH d'exploitation (terminal / redirection SSH) :\\nssh <utilisateur>@<hôte> nc -U ~/.wechat-rpa/worker.sock\\n\\n2. Configuration de l'API externe (clients HTTP MCP comme Claude Desktop / Cursor) :\\n{\\n  \"mcpServers\": {\\n    \"wechat-brain\": {\\n      \"url\": \"http://<bind>:<port>/mcp\",\\n      \"headers\": {\\n        \"Authorization\": \"Bearer <token>\"\\n      }\\n    }\\n  }\\n}", "1. Accesso SSH operativo (terminale / inoltro SSH):\\nssh <utente>@<host> nc -U ~/.wechat-rpa/worker.sock\\n\\n2. Configurazione API esterna (client HTTP MCP come Claude Desktop / Cursor):\\n{\\n  \"mcpServers\": {\\n    \"wechat-brain\": {\\n      \"url\": \"http://<bind>:<port>/mcp\",\\n      \"headers\": {\\n        \"Authorization\": \"Bearer <token>\"\\n      }\\n    }\\n  }\\n}", "1. Dostęp SSH operacyjny (terminal / przekierowanie SSH):\\nssh <użytkownik>@<host> nc -U ~/.wechat-rpa/worker.sock\\n\\n2. Konfiguracja zewnętrznego API (klienci HTTP MCP, np. Claude Desktop / Cursor):\\n{\\n  \"mcpServers\": {\\n    \"wechat-brain\": {\\n      \"url\": \"http://<bind>:<port>/mcp\",\\n      \"headers\": {\\n        \"Authorization\": \"Bearer <token>\"\\n      }\\n    }\\n  }\\n}")
        case .uiResidualUpdateRecoveryGuide:
            return e("【更新异常中断，需要人工恢复】\\n- 事务 ID: %@\\n- 中断阶段: %@\\n- 错误原因: %@\\n\\n只读说明与受控恢复指引:\\n1. 维护标志文件已记录；实际核心进程存活状态与业务数据完整性未知。\\n2. 当前界面处于严格只读受控阶段，禁止通过界面清除维护标志或进行破坏性写入。\\n3. 真实版本回退能力当前处于 unsupported 禁用状态，不可直接回退；需要专门受控恢复。\\n4. 请核对 ~/.wechat-rpa/logs/core.log 与 host.log 获取详细故障上下文并联系管理员。", "[Update interrupted abnormally; manual recovery is required]\\n- Transaction ID: %@\\n- Interrupted stage: %@\\n- Error: %@\\n\\nRead-only notes and controlled recovery steps:\\n1. A maintenance flag file has been recorded; the actual core process liveness and business-data integrity are unknown.\\n2. This screen is strictly read-only; clearing the maintenance flag or performing destructive writes here is not allowed.\\n3. Real version rollback is currently unsupported and disabled; a controlled recovery is required.\\n4. Check ~/.wechat-rpa/logs/core.log and host.log for details and contact an administrator.", "«\\n[Mise à jour interrompue anormalement, une récupération manuelle est nécessaire]\\n- ID de transaction : %@\\n- Étape interrompue : %@\\n- Erreur : %@\\n\\nNotes en lecture seule et étapes de récupération contrôlée :\\n1. Un fichier indicateur de maintenance a été enregistré ; l'état réel du processus cœur et l'intégrité des données métier sont inconnus.\\n2. Cette interface est strictement en lecture seule ; effacer l'indicateur ou effectuer des écritures destructrices ici est interdit.\\n3. Le retour à une version réelle est actuellement désactivé (unsupported) ; une récupération contrôlée est nécessaire.\\n4. Consultez ~/.wechat-rpa/logs/core.log et host.log pour le contexte et contactez un administrateur.\\n»", "[Aggiornamento interrotto in modo anomalo, è necessaria una ripristino manuale]\\n- ID transazione: %@\\n- Fase interrotta: %@\\n- Errore: %@\\n\\nNote in sola lettura e passi di ripristino controllato:\\n1. È stato registrato un file di flag di manutenzione; lo stato reale del processo core e l’integrità dei dati aziendali sono sconosciuti.\\n2. Questa schermata è strettamente in sola lettura; cancellare il flag o eseguire scritture distruttive qui non è consentito.\\n3. Il ripristino reale della versione è attualmente disabilitato (unsupported); è necessario un ripristino controllato.\\n4. Controlla ~/.wechat-rpa/logs/core.log e host.log per i dettagli e contatta un amministratore.", "[Aktualizacja przerwana nieprawidłowo, wymagane jest ręczne przywrócenie]\\n- ID transakcji: %@\\n- Przerwany etap: %@\\n- Błąd: %@\\n\\nUwagi tylko do odczytu i kroki kontrolowanego przywracania:\\n1. Zapisano plik znacznika konserwacji; rzeczywisty stan procesu rdzenia i integralność danych biznesowych są nieznane.\\n2. Ten ekran jest ściśle tylko do odczytu; czyszczenie znacznika lub destrukcyjne zapisy nie są tu dozwolone.\\n3. Rzeczywiste przywracanie wersji jest obecnie wyłączone (unsupported); wymagane jest kontrolowane przywrócenie.\\n4. Sprawdź ~/.wechat-rpa/logs/core.log i host.log, aby uzyskać szczegóły, i skontaktuj się z administratorem.")

        // MARK: 设计版页面迁移（自动生成）
        case .uiOverview001:
            return e("今日已发送: —", "Sent today: —", "Envoyés aujourd'hui : —", "Inviati oggi: —", "Wysłano dzisiaj: —")
        case .uiOverview002:
            return e("监听会话: -", "Monitored sessions: -", "Sessions surveillées : -", "Sessioni monitorate: -", "Monitorowane sesje: -")
        case .uiOverview003:
            return e("待人工: -", "Awaiting human: -", "En attente d'un humain : -", "In attesa di un operatore: -", "Oczekują na obsługę: -")
        case .uiOverview004:
            return e("今日已发送: ", "Sent today: ", "Envoyés aujourd'hui : ", "Inviati oggi: ", "Wysłano dzisiaj: ")
        case .uiOverview005:
            return e("监听会话: %@", "Watching: %@", "En surveillance : %@", "In ascolto: %@", "Obserwowane: %@")
        case .uiOverview006:
            return e("待人工: %@", "Awaiting human: %@", "En attente d'un humain : %@", "In attesa di un operatore: %@", "Oczekują na obsługę: %@")
        case .uiOverview007:
            return e("%@ （建议: %@）", "%@ (suggested: %@)", "%@ (suggestion : %@)", "%@ (suggerimento: %@)", "%@ (sugestia: %@)")
        case .uiOverview008:
            return e("MCP/大脑", "MCP/Brain", "MCP/Brain", "MCP/Brain", "MCP/Brain")
        case .uiSessions009:
            return e("会话监听列表", "Session monitoring list", "Liste des sessions surveillées", "Elenco delle sessioni monitorate", "Lista monitorowanych sesji")
        case .uiSessions010:
            return e("对话轮次追溯 (Provenance)", "Turn provenance", "Traçabilité des tours (Provenance)", "Tracciabilità dei turni (Provenance)", "Pochodzenie tur (Provenance)")
        case .uiSessions011:
            return e("停止自动回复，把这个会话交给人工处理", "Stop auto-reply and hand this conversation to a person", "Arrêter la réponse automatique et confier cette conversation à un humain", "Interrompi la risposta automatica e affida questa conversazione a una persona", "Zatrzymaj automatyczne odpowiedzi i przekaż tę rozmowę człowiekowi")
        case .uiSessions012:
            return e("未找到轮次追溯 (该轮次 ID 不存在或为旧版无追溯轮次): %@", "Turn provenance not found (unknown turn ID or a legacy turn without provenance): %@", "Traçabilité du tour introuvable (ID inconnu ou tour hérité sans traçabilité) : %@", "Tracciamento del turno non trovato (ID sconosciuto o turno legacy senza tracciamento): %@", "Nie znaleziono śladu tury (nieznany ID lub starsza tura bez śladu): %@")
        case .uiSessions013:
            return e("【轮次追溯详情】\n", "[Turn provenance details]\n", "[Détails de la traçabilité du tour]\n", "[Dettagli di tracciabilità del turno]\n", "[Szczegóły pochodzenia tury]\n")
        case .uiSessions014:
            return e("把这个会话转人工？", "Hand this conversation to a person?", "Confier cette conversation à un humain ?", "Affidare questa conversazione a una persona?", "Przekazać tę rozmowę człowiekowi?")
        case .uiSessions015:
            return e("转人工后「%@」将停止自动回复，由人工接手。这项操作会影响真实会话。", "After handing over, “%@” stops auto-reply and a person takes over. This affects a real conversation.", "Après le transfert, « %@ » n'aura plus de réponse automatique et un humain prendra le relais. Cela concerne une vraie conversation.", "Dopo il passaggio, «%@» non avrà più risposta automatica e subentrerà una persona. L'operazione riguarda una conversazione reale.", "Po przekazaniu „%@” przestanie odpowiadać automatycznie, a rozmowę przejmie człowiek. Ta operacja dotyczy prawdziwej rozmowy.")
        case .uiSessions016:
            return e("正在把「%@」转人工…", "Handing “%@” to a person…", "Transfert de « %@ » vers un humain…", "Passaggio di «%@» a una persona…", "Przekazywanie „%@” człowiekowi…")
        case .uiSessions017:
            return e("历史库里找不到会话「%@」，无法定位稳定会话 ID", "Session “%@” not found in the history database; cannot resolve a stable session ID", "Conversation « %@ » introuvable dans l'historique ; impossible de résoudre un ID stable", "Conversazione «%@» non trovata nella cronologia; impossibile risolvere un ID stabile", "Nie znaleziono sesji „%@” w historii; nie można ustalić stabilnego ID")
        case .uiSessions018:
            return e("存在 %@ 个同名会话「%@」，不能按名字转人工", "%@ sessions are named “%@”; cannot hand over by name", "%@ conversations portent le nom « %@ » ; transfert impossible par nom", "%@ conversazioni si chiamano «%@»; impossibile passare per nome", "Istnieje %@ sesji o nazwie „%@”; nie można przekazać według nazwy")
        case .uiSessions019:
            return e("已把「%@」转人工，自动回复已停止", "“%@” handed to a person; auto-reply stopped", "« %@ » transférée à un humain ; réponse automatique arrêtée", "«%@» passata a una persona; risposta automatica interrotta", "Przekazano „%@” człowiekowi; automatyczne odpowiedzi zatrzymane")
        case .uiSessions020:
            return e("转人工失败：%@", "Handover failed: %@", "Échec du transfert : %@", "Passaggio non riuscito: %@", "Przekazanie nie powiodło się: %@")
        case .uiSettings021:
            return e("%@ (%@) 必须为有效整数", "%@ (%@) must be a valid integer", "%@ (%@) doit être un entier valide", "%@ (%@) deve essere un numero intero valido", "%@ (%@) musi być poprawną liczbą całkowitą")
        case .uiSettings022:
            return e("%@ (%@) 类型不正确", "%@ (%@) has the wrong type", "%@ (%@) a un type incorrect", "%@ (%@) ha un tipo errato", "%@ (%@) ma nieprawidłowy typ")
        case .uiSettings023:
            return e("%@ (%@) 超出范围 (%@-%@ %@)", "%@ (%@) is out of range (%@-%@ %@)", "%@ (%@) est hors limites (%@-%@ %@)", "%@ (%@) è fuori intervallo (%@-%@ %@)", "%@ (%@) jest poza zakresem (%@-%@ %@)")
        case .uiSettings024:
            return e("屏幕录制权限（微信窗口识别）", "Screen Recording permission (WeChat window detection)", "Autorisation Enregistrement de l'écran (détection des fenêtres WeChat)", "Autorizzazione Registrazione schermo (rilevamento delle finestre WeChat)", "Uprawnienie Nagrywanie ekranu (wykrywanie okien WeChat)")
        case .uiSettings025:
            return e("辅助功能权限（窗口控制与点击）", "Accessibility permission (window control and clicks)", "Autorisation Accessibilité (contrôle des fenêtres et clics)", "Autorizzazione Accessibilità (controllo finestre e clic)", "Uprawnienie Dostępność (sterowanie oknami i kliknięcia)")
        case .uiSettings026:
            return e("桌面文件夹（附件与临时文件）", "Desktop folder (attachments and temp files)", "Dossier Bureau (pièces jointes et fichiers temporaires)", "Cartella Scrivania (allegati e file temporanei)", "Folder Pulpit (załączniki i pliki tymczasowe)")
        case .uiSettings027:
            return e("文稿文件夹（附件读写）", "Documents folder (attachment read/write)", "Dossier Documents (lecture/écriture des pièces jointes)", "Cartella Documenti (lettura/scrittura allegati)", "Folder Dokumenty (odczyt i zapis załączników)")
        case .uiSettings028:
            return e("下载文件夹（附件读写）", "Downloads folder (attachment read/write)", "Dossier Téléchargements (lecture/écriture des pièces jointes)", "Cartella Download (lettura/scrittura allegati)", "Folder Pobrane (odczyt i zapis załączników)")
        case .uiSettings029:
            return e("请求权限", "Request permission", "Demander l'autorisation", "Richiedi autorizzazione", "Poproś o uprawnienie")
        case .uiSettings030:
            return e("重新检查权限", "Recheck permissions", "Vérifier à nouveau les autorisations", "Ricontrolla le autorizzazioni", "Sprawdź uprawnienia ponownie")
        case .uiSettings031:
            return e("冷却时间:", "Cooldown:", "Délai de recharge :", "Intervallo di attesa:", "Czas odnowienia:")
        case .uiSettings032:
            return e("0–300 秒 (默认 15)", "0–300 s (default 15)", "0–300 s (par défaut 15)", "0–300 s (predefinito 15)", "0–300 s (domyślnie 15)")
        case .uiSettings033:
            return e("每小时上限:", "Max per hour:", "Maximum par heure :", "Massimo all'ora:", "Maks. na godzinę:")
        case .uiSettings034:
            return e("1–100 条 (默认 20)", "1–100 messages (default 20)", "1–100 messages (par défaut 20)", "1–100 messaggi (predefinito 20)", "1–100 wiadomości (domyślnie 20)")
        case .uiSettings035:
            return e("静默等待:", "Silence wait:", "Attente de silence :", "Attesa del silenzio:", "Oczekiwanie na ciszę:")
        case .uiSettings036:
            return e("0–120 秒 (默认 10)", "0–120 s (default 10)", "0–120 s (par défaut 10)", "0–120 s (predefinito 10)", "0–120 s (domyślnie 10)")
        case .uiSettings037:
            return e("单轮最长:", "Max per turn:", "Maximum par tour :", "Massimo per turno:", "Maks. na turę:")
        case .uiSettings038:
            return e("10–600 秒 (默认 60)", "10–600 s (default 60)", "10–600 s (par défaut 60)", "10–600 s (predefinito 60)", "10–600 s (domyślnie 60)")
        case .uiSettings039:
            return e("上下文条数:", "Context messages:", "Messages de contexte :", "Messaggi di contesto:", "Liczba komunikatów kontekstu:")
        case .uiSettings040:
            return e("0–100 条 (默认 5)", "0–100 messages (default 5)", "0–100 messages (par défaut 5)", "0–100 messaggi (predefinito 5)", "0–100 wiadomości (domyślnie 5)")
        case .uiSettings041:
            return e("附件上限:", "Attachment limit:", "Limite des pièces jointes :", "Limite allegati:", "Limit załączników:")
        case .uiSettings042:
            return e("1–200 MB (默认 50)", "1–200 MB (default 50)", "1–200 Mo (par défaut 50)", "1–200 MB (predefinito 50)", "1–200 MB (domyślnie 50)")
        case .uiSettings043:
            return e("注：允许名单与忽略名单已移至「联系人」页；外部大脑配置已移至「MCP」页。", "Note: the allow and ignore lists moved to the Contacts page; external Brain settings moved to the MCP page.", "Remarque : les listes d'autorisation et d'ignorés ont été déplacées vers la page Contacts ; la configuration du Brain externe se trouve désormais sur la page MCP.", "Nota: gli elenchi di autorizzazione e di esclusione sono stati spostati nella pagina Contatti; la configurazione del Brain esterno è ora nella pagina MCP.", "Uwaga: listy dozwolonych i ignorowanych przeniesiono na stronę Kontakty; konfiguracja zewnętrznego Brain jest teraz na stronie MCP.")
        case .uiSettings044:
            return e("已授权", "Authorized", "Autorisé", "Autorizzato", "Autoryzowano")
        case .uiSettings045:
            return e("配置保存成功", "Configuration saved", "Configuration enregistrée", "Configurazione salvata", "Konfiguracja zapisana")
        case .uiSettings046:
            return e("已载入默认值，点击「保存配置」后生效", "Defaults loaded; click \"Save configuration\" to apply", "Valeurs par défaut chargées ; cliquez sur « Enregistrer la configuration » pour appliquer", "Valori predefiniti caricati; fai clic su \"Salva configurazione\" per applicarli", "Wczytano wartości domyślne; kliknij „Zapisz konfigurację”, aby je zastosować")
        case .uiShell047:
            return e("页面导航", "Page navigation", "Navigation des pages", "Navigazione pagine", "Nawigacja stron")
        case .uiShell048:
            return e("当前页面", "Current page", "Page actuelle", "Pagina corrente", "Bieżąca strona")
        case .uiStatusPanel049:
            return e("开发构建", "Development build", "Version de développement", "Build di sviluppo", "Kompilacja deweloperska")
        case .uiStatusPanel050:
            return e("微信客服助手 — 系统状态 (版本: %@)", "WeChat Support Assistant — System status (version: %@)", "Assistant de support WeChat — État du système (version : %@)", "Assistente di supporto WeChat — Stato del sistema (versione: %@)", "Asystent obsługi WeChat — Stan systemu (wersja: %@)")
        case .uiStatusPanel051:
            return e("权限状态", "Permission status", "État des autorisations", "Stato delle autorizzazioni", "Stan uprawnień")
        case .uiStatusPanel052:
            return e("机器人核心", "Bot core", "Cœur du bot", "Core del bot", "Rdzeń bota")
        case .uiStatusPanel053:
            return e("各会话状态", "Per-session status", "État de chaque session", "Stato per sessione", "Stan poszczególnych sesji")
        case .uiStatusPanel054:
            return e("微信客户端", "WeChat client", "Client WeChat", "Client WeChat", "Klient WeChat")
        case .uiStatusPanel055:
            return e("布局地图", "Layout map", "Carte de disposition", "Mappa del layout", "Mapa układu")
        case .uiStatusPanel056:
            return e("磁盘空间", "Disk space", "Espace disque", "Spazio su disco", "Miejsce na dysku")
        case .uiStatusPanel057:
            return e("大脑状态", "Brain status", "État du Brain", "Stato del Brain", "Stan Brain")
        case .uiStatusPanel058:
            return e("正在检查...", "Checking...", "Vérification...", "Verifica in corso...", "Sprawdzanie...")
        case .uiStatusPanel059:
            return e("立即刷新", "Refresh now", "Actualiser maintenant", "Aggiorna ora", "Odśwież teraz")
        case .uiStatusPanel060:
            return e("暂停机器人", "Pause bot", "Mettre le bot en pause", "Metti in pausa il bot", "Wstrzymaj bota")
        case .uiStatusPanel061:
            return e("账号:", "Account:", "Compte :", "Account:", "Konto:")
        case .uiStatusPanel062:
            return e("模型:", "Model:", "Modèle :", "Modello:", "Model:")
        case .uiStatusPanel063:
            return e("密钥:", "API key:", "Clé API :", "Chiave API:", "Klucz API:")
        case .uiStatusPanel064:
            return e("附件上限 MB (1~200):", "Attachment limit MB (1–200):", "Limite des pièces jointes Mo (1–200) :", "Limite allegati MB (1–200):", "Limit załączników MB (1–200):")
        case .uiStatusPanel065:
            return e("每小时回复上限:", "Max replies per hour:", "Réponses max. par heure :", "Risposte massime all'ora:", "Maks. odpowiedzi na godzinę:")
        case .uiStatusPanel066:
            return e("冷却时间 (秒):", "Cooldown (seconds):", "Délai de recharge (secondes) :", "Intervallo di attesa (secondi):", "Czas odnowienia (sekundy):")
        case .uiStatusPanel067:
            return e("静默等待 (秒):", "Silence wait (seconds):", "Attente de silence (secondes) :", "Attesa del silenzio (secondi):", "Oczekiwanie na ciszę (sekundy):")
        case .uiStatusPanel068:
            return e("轮次最长时间 (秒):", "Max turn duration (seconds):", "Durée max. d'un tour (secondes) :", "Durata massima del turno (secondi):", "Maks. czas tury (sekundy):")
        case .uiStatusPanel069:
            return e("上下文消息条数:", "Context message count:", "Nombre de messages de contexte :", "Numero di messaggi di contesto:", "Liczba komunikatów kontekstu:")
        case .uiStatusPanel070:
            return e("回复由:", "Replies by:", "Réponses générées par :", "Risposte fornite da:", "Odpowiada:")
        case .uiStatusPanel071:
            return e("外部大脑超时 (秒):", "External Brain timeout (seconds):", "Délai d'attente du Brain externe (secondes) :", "Timeout del Brain esterno (secondi):", "Limit czasu zewnętrznego Brain (sekundy):")
        case .uiStatusPanel072:
            return e("外部大脑 (MCP next_turn / reply)", "External Brain (MCP next_turn / reply)", "Brain externe (MCP next_turn / reply)", "Brain esterno (MCP next_turn / reply)", "Zewnętrzny Brain (MCP next_turn / reply)")
        case .uiStatusPanel073:
            return e("改用内置大脑", "Use built-in Brain instead", "Utiliser le Brain intégré", "Usa il Brain integrato", "Użyj wbudowanego Brain")
        case .uiStatusPanel074:
            return e("留空使用默认值 120", "Leave empty to use the default of 120", "Laisser vide pour utiliser la valeur par défaut 120", "Lascia vuoto per usare il valore predefinito 120", "Pozostaw puste, aby użyć domyślnej wartości 120")
        case .uiStatusPanel075:
            return e(" (建议: %@)", " (suggested: %@)", " (suggestion : %@)", " (suggerimento: %@)", " (sugestia: %@)")
        case .uiStatusPanel076:
            return e("恢复机器人", "Resume bot", "Reprendre le bot", "Riprendi il bot", "Wznów bota")
        case .uiStatusPanel077:
            return e("退避重试中 (预计 %@ 重试)", "Back-off retry (next retry %@)", "Nouvelle tentative différée (prochaine : %@)", "Nuovo tentativo in back-off (prossimo: %@)", "Ponowna próba z opóźnieniem (następna: %@)")
        case .uiStatusPanel078:
            return e("未找到机器人配置文件（%@），无法保存", "Bot config file not found (%@); cannot save", "Fichier de configuration du bot introuvable (%@) ; enregistrement impossible", "File di configurazione del bot non trovato (%@); impossibile salvare", "Nie znaleziono pliku konfiguracji bota (%@); nie można zapisać")
        case .uiStatusPanel079:
            return e("机器人正在发送回复", "The bot is sending a reply", "Le bot envoie une réponse", "Il bot sta inviando una risposta", "Bot wysyła odpowiedź")
        case .uiStatusPanel080:
            return e("机器人当前正在处理并发送消息，执行「%@」可能会打断发送。确定要继续吗？", "The bot is currently processing and sending messages; “%@” may interrupt sending. Continue?", "Le bot traite et envoie des messages ; « %@ » peut interrompre l'envoi. Continuer ?", "Il bot sta elaborando e inviando messaggi; «%@» potrebbe interrompere l'invio. Continuare?", "Bot właśnie przetwarza i wysyła wiadomości; „%@” może przerwać wysyłkę. Kontynuować?")
        case .uiStatusPanel081:
            return e("正在重启桌面服务...", "Restarting the desktop service...", "Redémarrage du service de bureau...", "Riavvio del servizio desktop...", "Ponowne uruchamianie usługi pulpitu...")
        case .uiStatusPanel082:
            return e("桌面服务已触发重启", "Desktop service restart triggered", "Redémarrage du service de bureau déclenché", "Riavvio del servizio desktop avviato", "Uruchomiono ponowne uruchomienie usługi pulpitu")
        case .uiStatusPanel083:
            return e("正在重启机器人...", "Restarting the bot...", "Redémarrage du bot...", "Riavvio del bot...", "Ponowne uruchamianie bota...")
        case .uiStatusPanel084:
            return e("已触发 Rust 核心重启", "Rust core restart triggered", "Redémarrage du cœur Rust déclenché", "Riavvio del core Rust avviato", "Uruchomiono ponowne uruchomienie rdzenia Rust")
        case .uiStatusPanel085:
            return e("已触发机器人重启", "Bot restart triggered", "Redémarrage du bot déclenché", "Riavvio del bot avviato", "Uruchomiono ponowne uruchomienie bota")
        case .uiStatusPanel086:
            return e("重启机器人失败: %@", "Restarting the bot failed: %@", "Échec du redémarrage du bot : %@", "Riavvio del bot non riuscito: %@", "Ponowne uruchomienie bota nie powiodło się: %@")
        case .uiStatusPanel087:
            return e("机器人已恢复运行", "Bot resumed", "Bot repris", "Bot ripreso", "Bot wznowiony")
        case .uiStatusPanel088:
            return e("机器人已暂停", "Bot paused", "Bot en pause", "Bot in pausa", "Bot wstrzymany")
        case .uiStatusPanel089:
            return e("操作失败: %@", "Operation failed: %@", "Échec de l'opération : %@", "Operazione non riuscita: %@", "Operacja nie powiodła się: %@")
        case .uiStatusPanel090:
            return e("正在恢复自动回复：%@", "Resuming auto-reply: %@", "Reprise de la réponse automatique : %@", "Ripresa della risposta automatica: %@", "Wznawianie automatycznych odpowiedzi: %@")
        case .uiStatusPanel091:
            return e("已恢复自动回复", "Auto-replies resumed", "Réponses automatiques reprises", "Risposte automatiche riprese", "Wznowiono automatyczne odpowiedzi")
        case .uiStatusPanel092:
            return e("恢复失败：%@", "Resume failed: %@", "Échec de la reprise : %@", "Ripresa non riuscita: %@", "Wznowienie nie powiodło się: %@")
        case .uiStatusPanel093:
            return e("微信窗口已放回标准位置 (880x640)", "WeChat window restored to its standard position (880x640)", "Fenêtre WeChat replacée à sa position standard (880x640)", "Finestra WeChat riportata nella posizione standard (880x640)", "Okno WeChat przywrócone do standardowej pozycji (880x640)")
        case .uiStatusPanel094:
            return e("正在保存并校验配置...", "Saving and validating the configuration...", "Enregistrement et vérification de la configuration...", "Salvataggio e verifica della configurazione...", "Zapisywanie i weryfikacja konfiguracji...")
        case .uiStatusPanel095:
            return e("配置路径无效", "Invalid configuration path", "Chemin de configuration invalide", "Percorso della configurazione non valido", "Nieprawidłowa ścieżka konfiguracji")
        case .uiStatusPanel096:
            return e("表单格式错误: %@", "Form format error: %@", "Erreur de format du formulaire : %@", "Errore di formato del modulo: %@", "Błąd formatu formularza: %@")
        case .uiStatusPanel097:
            return e("配置保存成功，机器人已重启生效", "Configuration saved; the bot restarted to apply it", "Configuration enregistrée ; le bot a redémarré pour l'appliquer", "Configurazione salvata; il bot si è riavviato per applicarla", "Konfiguracja zapisana; bot uruchomił się ponownie, aby ją zastosować")
        case .uiConfigForm098:
            return e("超时必须为 1–180 之间的整数", "Timeout must be an integer between 1 and 180", "Le délai doit être un nombre entier compris entre 1 et 180", "Il timeout deve essere un numero intero tra 1 e 180", "Limit czasu musi być liczbą całkowitą z zakresu 1–180")
        case .uiConfigForm099:
            return e("图片数必须为 1–16 之间的整数", "Image count must be an integer between 1 and 16", "Le nombre d'images doit être un nombre entier compris entre 1 et 16", "Il numero di immagini deve essere un numero intero tra 1 e 16", "Liczba obrazów musi być liczbą całkowitą z zakresu 1–16")
        case .uiConfigForm100:
            return e("请选择有效的接入方式", "Choose a valid access method", "Sélectionnez un mode de connexion valide", "Seleziona un metodo di accesso valido", "Wybierz prawidłowy sposób połączenia")
        case .uiConfigForm101:
            return e("接口地址必须使用 HTTPS（本机可用 HTTP），且不能包含凭据、查询或片段", "The endpoint must use HTTPS (HTTP is allowed on the local machine) and must not contain credentials, a query, or a fragment", "L'adresse de l'interface doit utiliser HTTPS (HTTP autorisé en local) et ne peut contenir ni identifiants, ni requête, ni fragment", "L'indirizzo dell'interfaccia deve usare HTTPS (HTTP è consentito in locale) e non può contenere credenziali, query o frammenti", "Adres interfejsu musi używać HTTPS (lokalnie dozwolone jest HTTP) i nie może zawierać danych logowania, zapytania ani fragmentu")
        case .uiConfigForm102:
            return e("请填写有效的环境变量名", "Enter a valid environment variable name", "Saisissez un nom de variable d'environnement valide", "Inserisci un nome di variabile d'ambiente valido", "Podaj prawidłową nazwę zmiennej środowiskowej")
        case .uiConfigForm103:
            return e("密钥文件必须为当前用户所有的绝对路径普通文件、权限 0600、非符号链接且内容有效", "The key file must be an absolute-path regular file owned by the current user, with mode 0600, not a symlink, and valid contents", "Le fichier de clé doit être un fichier ordinaire en chemin absolu appartenant à l'utilisateur actuel, avec les droits 0600, sans lien symbolique et au contenu valide", "Il file della chiave deve essere un file regolare con percorso assoluto di proprietà dell'utente corrente, con permessi 0600, non un collegamento simbolico e con contenuto valido", "Plik klucza musi być zwykłym plikiem o ścieżce bezwzględnej, należącym do bieżącego użytkownika, z uprawnieniami 0600, bez dowiązania symbolicznego i z prawidłową zawartością")
        case .uiConfigForm104:
            return e("请选择有效的密钥来源", "Choose a valid key source", "Sélectionnez une source de clé valide", "Seleziona un'origine valida per la chiave", "Wybierz prawidłowe źródło klucza")
        case .uiConfigForm105:
            return e("%@ 不能为空，必须为整数", "%@ cannot be empty; an integer is required", "%@ ne peut pas être vide ; un entier est requis", "%@ non può essere vuoto; è richiesto un numero intero", "%@ nie może być puste; wymagana jest liczba całkowita")
        case .uiConfigForm106:
            return e("%@ 必须为整数，不能包含小数或非数字字符", "%@ must be an integer without decimals or non-numeric characters", "%@ doit être un entier, sans décimales ni caractères non numériques", "%@ deve essere un numero intero, senza decimali o caratteri non numerici", "%@ musi być liczbą całkowitą, bez ułamków i znaków nienumerycznych")
        case .uiConfigForm107:
            return e("%@ 必须为整数", "%@ must be an integer", "%@ doit être un entier", "%@ deve essere un numero intero", "%@ musi być liczbą całkowitą")
        case .uiConfigForm108:
            return e("brain_timeout_seconds 必须为整数", "brain_timeout_seconds must be an integer", "brain_timeout_seconds doit être un nombre entier", "brain_timeout_seconds deve essere un numero intero", "brain_timeout_seconds musi być liczbą całkowitą")
        case .uiConfigForm109:
            return e("brain.external.timeout_seconds 必须在 1 到 3600 秒之间", "brain.external.timeout_seconds must be between 1 and 3600 seconds", "brain.external.timeout_seconds doit être compris entre 1 et 3600 secondes", "brain.external.timeout_seconds deve essere compreso tra 1 e 3600 secondi", "brain.external.timeout_seconds musi mieścić się w zakresie od 1 do 3600 sekund")
        case .uiConfigForm110:
            return e("brain.external.timeout_action 必须是 skip、builtin 或 handoff", "brain.external.timeout_action must be skip, builtin, or handoff", "brain.external.timeout_action doit être skip, builtin ou handoff", "brain.external.timeout_action deve essere skip, builtin o handoff", "brain.external.timeout_action musi mieć wartość skip, builtin lub handoff")
        case .uiConfigForm111:
            return e("brain_queue_minutes 必须为整数", "brain_queue_minutes must be an integer", "brain_queue_minutes doit être un nombre entier", "brain_queue_minutes deve essere un numero intero", "brain_queue_minutes musi być liczbą całkowitą")
        case .uiConfigForm112:
            return e("brain.external.queue_minutes 必须在 1 到 1440 分钟之间", "brain.external.queue_minutes must be between 1 and 1440 minutes", "brain.external.queue_minutes doit être compris entre 1 et 1440 minutes", "brain.external.queue_minutes deve essere compreso tra 1 e 1440 minuti", "brain.external.queue_minutes musi mieścić się w zakresie od 1 do 1440 minut")
        case .uiConfigForm113:
            return e("brain.mode 必须是 builtin 或 external", "brain.mode must be builtin or external", "brain.mode doit être builtin ou external", "brain.mode deve essere builtin o external", "brain.mode musi mieć wartość builtin lub external")
        case .uiConfigForm114:
            return e("brain.mode 缺失", "brain.mode is missing", "brain.mode est absent", "Manca brain.mode", "Brak brain.mode")
        case .uiConfigForm115:
            return e("brain.mode=builtin 时不允许包含其他字段", "No other fields are allowed when brain.mode=builtin", "Aucun autre champ n'est autorisé lorsque brain.mode=builtin", "Nessun altro campo è consentito con brain.mode=builtin", "Przy brain.mode=builtin nie są dozwolone inne pola")
        case .uiConfigForm116:
            return e("brain 包含未知字段: %@", "brain contains unknown fields: %@", "brain contient des champs inconnus : %@", "brain contiene campi sconosciuti: %@", "brain zawiera nieznane pola: %@")
        case .uiConfigForm117:
            return e("brain.external 包含未知字段: %@", "brain.external contains unknown fields: %@", "brain.external contient des champs inconnus : %@", "brain.external contiene campi sconosciuti: %@", "brain.external zawiera nieznane pola: %@")
        case .uiConfigForm118:
            return e("MODEL_FORM_INVALID: 模型配置格式无效", "MODEL_FORM_INVALID: invalid model configuration format", "MODEL_FORM_INVALID : format de configuration du modèle invalide", "MODEL_FORM_INVALID: formato della configurazione del modello non valido", "MODEL_FORM_INVALID: nieprawidłowy format konfiguracji modelu")
        case .uiConfigForm119:
            return e("配置校验失败", "Configuration validation failed", "Échec de la validation de la configuration", "Convalida della configurazione non riuscita", "Walidacja konfiguracji nie powiodła się")
        case .uiConfigForm120:
            return e("配置文件备份失败: %@，已中止保存", "Backing up the config failed: %@; save aborted", "La sauvegarde de la configuration a échoué : %@ ; enregistrement annulé", "Backup della configurazione non riuscito: %@; salvataggio annullato", "Kopia zapasowa konfiguracji nie powiodła się: %@; zapis przerwany")
        case .uiConfigForm121:
            return e("CONFIG_CHECK_TIMEOUT: 配置校验超时", "CONFIG_CHECK_TIMEOUT: configuration check timed out", "CONFIG_CHECK_TIMEOUT : délai dépassé pour la vérification de la configuration", "CONFIG_CHECK_TIMEOUT: verifica della configurazione scaduta", "CONFIG_CHECK_TIMEOUT: przekroczono czas weryfikacji konfiguracji")
        case .uiConfigForm122:
            return e("CONFIG_CHECK_FAILED: 配置校验失败，请检查配置", "CONFIG_CHECK_FAILED: configuration check failed; review your configuration", "CONFIG_CHECK_FAILED : échec de la vérification de la configuration ; vérifiez votre configuration", "CONFIG_CHECK_FAILED: verifica della configurazione non riuscita; controlla la configurazione", "CONFIG_CHECK_FAILED: weryfikacja konfiguracji nie powiodła się; sprawdź konfigurację")
        case .uiConfigForm123:
            return e("config-check 超时", "config-check timed out", "config-check : délai dépassé", "config-check scaduto", "config-check: przekroczono czas")
        case .uiConfigForm124:
            return e("config-check 校验失败 (exit %@)", "config-check validation failed (exit %@)", "Échec de la validation config-check (exit %@)", "Validazione config-check non riuscita (exit %@)", "Walidacja config-check nie powiodła się (exit %@)")
        case .uiConfigForm125:
            return e("无法执行 config-check (exit %@)", "Could not run config-check (exit %@)", "Impossible d'exécuter config-check (exit %@)", "Impossibile eseguire config-check (exit %@)", "Nie można uruchomić config-check (exit %@)")

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

        case .actionConfirm:
            return e("确定", "Confirm", "Confirmer", "Conferma", "Potwierdź")
        case .actionCopy:
            return e("复制", "Copy", "Copier", "Copia", "Kopiuj")
        case .actionDelete:
            return e("删除", "Delete", "Supprimer", "Elimina", "Usuń")
        case .actionFailedTitle:
            return e("操作失败", "Action failed", "Action échouée", "Azione fallita", "Działanie nieudane")
        case .actionOK:
            return e("好的", "OK", "OK", "OK", "OK")
        case .actionPause:
            return e("暂停", "Pause", "Pause", "Pausa", "Wstrzymaj")
        case .actionRefresh:
            return e("刷新", "Refresh", "Actualiser", "Aggiorna", "Odśwież")
        case .actionResume:
            return e("继续", "Resume", "Reprendre", "Riprendi", "Wznów")
        case .brainActionActivateConfig:
            return e("启用配置", "Activate configuration", "Activer la configuration", "Attiva configurazione", "Aktywuj konfigurację")
        case .brainActionArchiveRole:
            return e("归档角色", "Archive role", "Archiver le rôle", "Archivia ruolo", "Archiwizuj rolę")
        case .brainActionCopyRole:
            return e("复制角色", "Copy role", "Copier le rôle", "Copia ruolo", "Kopiuj rolę")
        case .brainActionExitCodeFmt:
            return e("%1$@失败 (exit %2$d)", "%1$@ failed (exit %2$d)", "%1$@ échoué (sortie %2$d)", "%1$@ fallito (uscita %2$d)", "%1$@ nieudane (wyjście %2$d)")
        case .brainActionInitRole:
            return e("初始化客服角色", "Initialize customer service role", "Initialiser le rôle de service client", "Inizializza ruolo servizio clienti", "Inicjalizuj rolę obsługi klienta")
        case .brainActionInvalidGenerationFmt:
            return e("%@返回无效数据，无法获取 generation", "%@ returned invalid data; unable to get generation", "%@ a renvoyé des données invalides ; génération introuvable", "%@ ha restituito dati non validi; generazione introvabile", "%@ zwróciło nieprawidłowe dane; nie można pobrać generacji")
        case .brainActionRollbackConfig:
            return e("回退配置", "Rollback configuration", "Rétablir la configuration", "Ripristina configurazione", "Wycofaj konfigurację")
        case .brainActionSaveBindings:
            return e("保存绑定", "Save bindings", "Enregistrer les liaisons", "Salva associazioni", "Zapisz powiązania")
        case .brainActionSaveRole:
            return e("保存角色", "Save role", "Enregistrer le rôle", "Salva ruolo", "Zapisz rolę")
        case .brainActivateRoleConfirmTitleFmt:
            return e("启用角色「%@」？", "Activate role \"%@\"?", "Activer le rôle « %@ » ?", "Attivare il ruolo \"%@\"?", "Aktywować rolę „%@”?")
        case .brainActivateRoleDialogBodyDefault:
            return e("将把该角色设为账号默认角色，并启用整份会话配置。白名单内没有专属角色的会话将使用它；已有专属角色保持不变。仅影响新轮次。", "Will set this role as default and activate configuration. Whitelisted sessions without dedicated role will use it. Affects new turns only.", "Définit ce rôle par défaut et active la configuration pour les sessions sans rôle dédié. N'affecte que les nouveaux tours.", "Imposterà questo ruolo come predefinito e attiverà la configurazione. Ha effetto solo sui nuovi turni.", "Ustawi tę rolę jako domyślną i aktywuje konfigurację dla sesji bez dedykowanej roli. Dotyczy tylko nowych tur.")
        case .brainActivateRoleDialogBodyExisting:
            return e("将更新该角色已有的默认或专属绑定到当前已保存版本，并启用整份会话配置。其他角色绑定保持原样。仅影响新轮次。", "Updates existing default or override bindings to the saved version and activates the configuration. Affects new turns only.", "Met à jour les liaisons par défaut ou dédiées vers la version enregistrée et active la configuration. Affecte uniquement les nouveaux tours.", "Aggiorna le associazioni predefinite o dedicate alla versione salvata e attiva la configurazione. Ha effetto solo sui nuovi turni.", "Aktualizuje przypisania domyślne lub dedykowane do zapisanej wersji i aktywuje konfigurację. Wpływa tylko na nowe tury.")
        case .brainActivateRoleDialogBodyNewDefault:
            return e("将把该角色设为账号默认角色，并启用整份会话配置。白名单内没有专属角色的会话将使用它；已有专属角色保持不变。仅影响新轮次。", "Sets this role as account default and activates configuration. Chats without overrides will use it. Affects new turns only.", "Définit ce rôle par défaut et active la configuration. Utilisé pour les chats sans dérogation. Nouveaux tours uniquement.", "Imposta come predefinito per l'account e attiva la configurazione. Usato per chat senza eccezioni. Solo nuovi turni.", "Ustawia tę rolę jako domyślną i aktywuje konfigurację. Używana dla czatów bez wyjątków. Tylko nowe tury.")
        case .brainActivateRoleDialogTitleFmt:
            return e("启用角色「%@」？", "Activate role '%@'?", "Activer le rôle « %@ » ?", "Attivare il ruolo '%@'?", "Aktywować rolę „%@”?")
        case .brainActivatingRole:
            return e("正在保存适用范围并启用角色...", "Saving scope and activating role...", "Enregistrement de la portée et activation du rôle...", "Salvataggio dell'ambito e attivazione del ruolo...", "Zapisywanie zakresu i aktywowanie roli...")
        case .brainActivationScopeMismatch:
            return e("启用结果与预期范围不一致，请重新读取配置确认。", "Activation scope mismatch; reload configuration to verify.", "Résultat d'activation discordant ; rechargez la configuration pour vérifier.", "Ambito di attivazione discordante; ricarica la configurazione per verificare.", "Niezgodność zakresu aktywacji; załaduj konfigurację ponownie, aby zweryfikować.")
        case .brainActivationScopeMismatchErr:
            return e("启用结果与预期范围不一致，请重新读取配置确认。", "Activation result does not match expected scope; please reload config to verify.", "Le résultat de l'activation ne correspond pas à la portée attendue ; rechargez la configuration.", "Il risultato dell'attivazione non corrisponde all'ambito previsto; ricarica la configurazione.", "Wynik aktywacji nie jest zgodny z oczekiwanym zakresem; załaduj ponownie konfigurację.")
        case .brainActivationUnconfirmedFmt:
            return e("启用未确认：%1$@。请重新读取配置核对；已保存的绑定草稿会保留。", "Activation unconfirmed: %1$@. Reload configuration to check; saved draft bindings remain.", "Activation non confirmée : %1$@. Rechargez pour vérifier ; les liaisons enregistrées sont conservées.", "Attivazione non confermata: %1$@. Ricarica per verificare; le bozze salvate restano.", "Aktywacja niepotwierdzona: %1$@. Załaduj ponownie, aby sprawdzić; zapisany szkic pozostaje.")
        case .brainAddOverride:
            return e("添加覆盖", "Add override", "Ajouter une dérogation", "Aggiungi eccezione", "Dodaj wyjątek")
        case .brainAddOverrideErrContact:
            return e("添加失败：请先选择合法的白名单联系人", "Add failed: Please select a valid allowlist contact first", "Échec de l'ajout : Veuillez d'abord sélectionner un contact valide", "Aggiunta non riuscita: Seleziona prima un contatto valido della whitelist", "Dodawanie nie powiodło się: Wybierz najpierw prawidłowy kontakt z białej listy")
        case .brainAddOverrideErrExistsFmt:
            return e("添加失败：联系人「%@」已存在覆盖规则，可直接编辑", "Add failed: Override rule already exists for \"%@\", edit directly", "Échec de l'ajout : Une dérogation existe déjà pour « %@ », modifiez-la directement", "Aggiunta non riuscita: Esiste già una regola per \"%@\", modificala direttamente", "Dodawanie nie powiodło się: Reguła wyjątku już istnieje dla „%@”, edytuj bezpośrednio")
        case .brainAddOverrideErrGeneral:
            return e("添加覆盖失败", "Failed to add override", "Échec de l'ajout de dérogation", "Aggiunta eccezione non riuscita", "Nie udało się dodać wyjątku")
        case .brainAddOverrideErrMustAllowlist:
            return e("添加失败：联系人必须源自真实白名单，禁止凭姓名构造ID", "Add failed: Contact must originate from real allowlist; synthesized IDs not allowed", "Échec de l'ajout : Le contact doit provenir de la liste blanche réelle ; ID synthétisés interdits", "Aggiunta non riuscita: Il contatto deve provenire dalla whitelist reale; ID sintetizzati vietati", "Dodawanie nie powiodło się: Kontakt musi pochodzić z rzeczywistej białej listy; syntetyczne ID zabronione")
        case .brainAddOverrideErrRole:
            return e("添加失败：请选择有效的角色", "Add failed: Please select a valid role", "Échec de l'ajout : Veuillez sélectionner un rôle valide", "Aggiunta non riuscita: Seleziona un ruolo valido", "Dodawanie nie powiodło się: Wybierz prawidłową rolę")
        case .brainAddOverrideFailed:
            return e("添加覆盖失败", "Failed to add override", "Échec de l'ajout de la dérogation", "Aggiunta eccezione fallita", "Dodawanie wyjątku nieudane")
        case .brainAddOverrideSelectContact:
            return e("添加失败：请先选择合法的白名单联系人", "Add failed: select a valid allowlist contact", "Échec de l'ajout : sélectionnez un contact autorisé valide", "Aggiunta fallita: seleziona un contatto consentito valido", "Dodawanie nieudane: wybierz prawidłowy kontakt z białej listy")
        case .brainAddOverrideSelectRole:
            return e("添加失败：请选择有效的角色", "Add failed: select a valid role", "Échec de l'ajout : sélectionnez un rôle valide", "Aggiunta fallita: seleziona un ruolo valido", "Dodawanie nieudane: wybierz prawidłową rolę")
        case .brainAllowlistContactLabel:
            return e("白名单联系人:", "Allowlist Contact:", "Contact liste blanche :", "Contatto whitelist:", "Kontakt z białej listy:")
        case .brainAllowlistUnavailableFmt:
            return e("白名单联系人来源不可用 [%@]，已禁用新增与编辑", "Allowlist contacts source unavailable [%@]; adding and editing disabled", "Source de contacts liste blanche indisponible [%@] ; ajout et modification désactivés", "Fonte contatti whitelist non disponibile [%@]; aggiunta e modifica disabilitate", "Źródło kontaktów białej listy niedostępne [%@]; dodawanie i edycja wyłączone")
        case .brainAssemblePreview:
            return e("组装预览", "Assemble preview", "Aperçu d'assemblage", "Anteprima assemblaggio", "Podgląd złożenia")
        case .brainBindRoleLabel:
            return e("绑定客服角色:", "Bind Customer Service Role:", "Lier au rôle de service :", "Associa ruolo assistenza:", "Powiąż rolę obsługi:")
        case .brainBindingsDraftSavedSuccess:
            return e("绑定草稿保存成功！新配置将在启用后作用于新轮次。", "Bindings draft saved. Changes take effect on new turns after activation.", "Brouillon enregistré. Les modifications prendront effet après activation.", "Bozza salvata. Le modifiche avranno effetto dopo l'attivazione.", "Szkic przypisań zapisany. Zmiany zaczną działać po aktywacji w nowych turach.")
        case .brainBindingsIntro:
            return e("账号与联系人绑定。联系人唯一来源为白名单，本处仅引用不复制联系人清单；修改仅暂存为草稿，点击「保存绑定草稿」通过 CAS 校验后落盘；启用配置后在下一轮对话生效。", "Account and contact bindings. Source of truth is allowlist; changes saved as draft, committed via CAS on 'Save Bindings Draft', taking effect next turn after activation.", "Liaisons compte et contacts. Source unique : liste blanche ; modifications en brouillon validées par CAS, effectives au prochain tour après activation.", "Associazioni account e contatti. Unica fonte è la whitelist; modifiche in bozza salvate con CAS, attive dal turno successivo dopo l'abilitazione.", "Powiązania kont i kontaktów. Jedynym źródłem jest biała lista; zmiany w szkicu zapisywane przez CAS, aktywne w kolejnej turze po włączeniu.")
        case .brainBindingsSavedSuccess:
            return e("绑定草稿保存成功！新配置将在启用后作用于新轮次。", "Bindings draft saved successfully! New config takes effect on new turns after activation.", "Brouillon de liaisons enregistré avec succès ! Effectif lors des nouveaux tours après activation.", "Bozza associazioni salvata con successo! La nuova configurazione sarà attiva dai nuovi turni.", "Szkic powiązań zapisany pomyślnie! Nowa konfiguracja zacznie obowiązywać po włączeniu.")
        case .brainBindingsSectionDesc:
            return e("账号与联系人绑定。联系人唯一来源为白名单，本处仅引用不复制联系人清单；修改仅暂存为草稿，点击「保存绑定草稿」通过 CAS 校验后落盘；启用配置后在下一轮对话生效。", "Account and contact bindings. Contacts are referenced strictly from the allowlist. Edits are staged as drafts until saved via CAS. Changes take effect on the next conversation turn after activation.", "Liaisons compte et contacts. Les contacts proviennent de la liste autorisée. Enregistrez via CAS pour appliquer au prochain tour.", "Associazioni account e contatti. I contatti derivano dalla lista consentiti. Salva con CAS per attivare al turno successivo.", "Przypisania konta i kontaktów. Kontakty pochodzą wyłącznie z białej listy. Zapisz przez CAS, aby zastosować w następnej turze.")
        case .brainBtnActivateConfig:
            return e("启用配置", "Activate Configuration", "Activer la configuration", "Attiva configurazione", "Aktywuj konfigurację")
        case .brainBtnAddOverride:
            return e("添加覆盖", "Add Override", "Ajouter une dérogation", "Aggiungi eccezione", "Dodaj wyjątek")
        case .brainBtnConfirmActivate:
            return e("确认启用", "Confirm Activation", "Confirmer l'activation", "Conferma attivazione", "Potwierdź aktywację")
        case .brainBtnCopyRole:
            return e("复制角色", "Copy role", "Copier le rôle", "Copia ruolo", "Kopiuj rolę")
        case .brainBtnDeleteOverride:
            return e("删除选定覆盖", "Delete Selected Override", "Supprimer la dérogation", "Elimina eccezione selezionata", "Usuń wybrany wyjątek")
        case .brainBtnDeleteRole:
            return e("删除角色", "Delete Role", "Supprimer le rôle", "Elimina ruolo", "Usuń rolę")
        case .brainBtnDiff:
            return e("对比差异", "Compare Diffs", "Comparer les versions", "Confronta differenze", "Porównaj różnice")
        case .brainBtnImportKnowledge:
            return e("导入并关联至角色", "Import & Attach to Role", "Importer et lier au rôle", "Importa e associa al ruolo", "Importuj i połącz z rolą")
        case .brainBtnModelAndConn:
            return e("模型与连接…", "Model & Connection…", "Modèle et connexion…", "Modello e connessione…", "Model i połączenie…")
        case .brainBtnNewRole:
            return e("新增角色", "New Role", "Nouveau rôle", "Nuovo ruolo", "Nowa rola")
        case .brainBtnRollbackConfig:
            return e("回退版本", "Rollback Version", "Rétablir la version", "Ripristina versione", "Wycofaj wersję")
        case .brainBtnRollbackSelected:
            return e("回滚至选定版本…", "Restore selected version…", "Restaurer la version sélectionnée…", "Ripristina versione selezionata…", "Przywróć wybraną wersję…")
        case .brainBtnRollbackToSelectedVer:
            return e("回滚至选定版本…", "Rollback to Selected Version…", "Rétablir la version sélectionnée…", "Ripristina alla versione selezionata…", "Wycofaj do wybranej wersji…")
        case .brainBtnRunPreview:
            return e("组装预览", "Assemble Preview", "Aperçu d'assemblage", "Anteprima assemblaggio", "Podgląd złożenia")
        case .brainBtnSaveBindingsDraft:
            return e("保存绑定草稿", "Save Bindings Draft", "Enregistrer le brouillon de liaisons", "Salva bozza associazioni", "Zapisz szkic powiązań")
        case .brainBtnSaveDraft:
            return e("保存草稿", "Save Draft", "Enregistrer le brouillon", "Salva bozza", "Zapisz szkic")
        case .brainBtnSetDefaultRole:
            return e("设为默认角色…", "Set as Default Role…", "Définir comme rôle par défaut…", "Imposta come ruolo predefinito…", "Ustaw jako rolę domyślną…")
        case .brainBtnTryIt:
            return e("试一试 (组装预览)", "Try it (assemble preview)", "Essayer (aperçu d'assemblage)", "Prova (anteprima)", "Wypróbuj (podgląd złożenia)")
        case .brainBtnTryItPreview:
            return e("试一试 (组装预览)", "Try It (Assemble Preview)", "Essayer (Aperçu d'assemblage)", "Prova (Anteprima assemblaggio)", "Wypróbuj (Podgląd złożenia)")
        case .brainBtnUpdateOverride:
            return e("更新选定覆盖", "Update Selected Override", "Mettre à jour la dérogation", "Aggiorna eccezione selezionata", "Zaktualizuj wybrany wyjątek")
        case .brainCasBindingConflictFmt:
            return e("保存绑定冲突 [%1$d]：%2$@。已保留您当前未提交的绑定草稿与表单，请稍后重试。", "Binding conflict [%1$d]: %2$@. Uncommitted bindings and forms were preserved; try again later.", "Conflit de liaisons [%1$d] : %2$@. Les liaisons et formulaires non validés ont été conservés ; réessayez plus tard.", "Conflitto associazioni [%1$d]: %2$@. Le associazioni e i moduli non inviati sono stati mantenuti; riprova più tardi.", "Konflikt przypisań [%1$d]: %2$@. Niezatwierdzone przypisania i formularze zostały zachowane; spróbuj ponownie później.")
        case .brainCasBindingsConflictFmt:
            return e("保存绑定冲突 [%@]：%@。已保留您当前未提交的绑定草稿与表单，请稍后重试。", "Binding save conflict [%@]: %@. Unsubmitted binding draft and forms preserved; please retry later.", "Conflit d'enregistrement de liaison [%@] : %@. Brouillon non envoyé conservé ; veuillez réessayer plus tard.", "Conflitto di salvataggio associazione [%@]: %@. Bozza non inviata conservata; riprova più tardi.", "Konflikt zapisu powiązania [%@]: %@. Niezatwierdzony szkic zachowany; spróbuj ponownie później.")
        case .brainCasRoleConflictFmt:
            return e("并发修改冲突: %@。已保留您当前编辑的草稿，请核对最新版本后再次保存。", "Conflict: %@. Your unsaved draft was preserved; check the latest version and save again.", "Conflit : %@. Votre brouillon a été conservé ; vérifiez la version récente et enregistrez à nouveau.", "Conflitto: %@. La bozza non salvata è stata mantenuta; verifica la versione recente e salva di nuovo.", "Konflikt: %@. Twój niezapisany szkic został zachowany; sprawdź najnowszą wersję i zapisz ponownie.")
        case .brainChatKindGroupOption:
            return e("群聊 (group)", "Group chat (group)", "Chat de groupe (group)", "Chat di gruppo (group)", "Czat grupowy (group)")
        case .brainChatKindPrivateOption:
            return e("私聊 (private)", "Private chat (private)", "Chat privé (private)", "Chat privata (private)", "Czat prywatny (private)")
        case .brainColBoundRole:
            return e("绑定角色", "Bound Role", "Rôle associé", "Ruolo associato", "Powiązana rola")
        case .brainColContactAllowlist:
            return e("联系人/群 (白名单)", "Contact/Group (Allowlist)", "Contact/Groupe (Liste blanche)", "Contatto/Gruppo (Whitelist)", "Kontakt/Grupa (Biała lista)")
        case .brainColCreatedAt:
            return e("生成时间", "Created at", "Généré le", "Data creazione", "Czas utworzenia")
        case .brainColDefaultRevId:
            return e("默认版本 ID", "Default Revision ID", "ID de révision par défaut", "ID revisione predefinito", "ID domyślnej wersji")
        case .brainColGen:
            return e("代次", "Gen", "Gén", "Gen", "Gen")
        case .brainColKind:
            return e("类型", "Type", "Type", "Tipo", "Typ")
        case .brainColRev:
            return e("版本", "Revision", "Révision", "Revisione", "Wersja")
        case .brainColRole:
            return e("角色", "Role", "Rôle", "Ruolo", "Rola")
        case .brainColStatus:
            return e("状态", "Status", "État", "Stato", "Status")
        case .brainConfirmActivate:
            return e("确认启用", "Confirm activation", "Confirmer l'activation", "Conferma attivazione", "Potwierdź aktywację")
        case .brainConstraintsBoxNotice:
            return e("不可虚构事实、语音转写以本轮转写为准、未识别项点名并依提示回复。本约束由代码自动拼接在角色人设之后，不可在界面删改。", "Do not hallucinate facts; voice transcription takes precedence; name unrecognised items and reply per guidance. Appended automatically by code, read-only.", "Ne pas inventer de faits ; la transcription vocale prévaut ; nommer les éléments non reconnus. Joint automatiquement par le code, en lecture seule.", "Non inventare fatti; la trascrizione vocale ha la precedenza; indicare elementi non riconosciuti. Aggiunto automaticamente dal codice, in sola lettura.", "Nie zmyślaj faktów; transkrypcja głosowa ma pierwszeństwo; wymieniaj nierozpoznane elementy. Dołączane automatycznie, tylko do odczytu.")
        case .brainConstraintsBoxTitle:
            return e("系统能力约束 (只读)", "System Constraints (Read-only)", "Contraintes système (Lecture seule)", "Vincoli di sistema (Sola lettura)", "Ograniczenia systemowe (Tylko do odczytu)")
        case .brainCopySuffix:
            return e(" (副本)", " (Copy)", " (Copie)", " (Copia)", " (Kopia)")
        case .brainCostNotePreviewOnly:
            return e("未调用模型，仅组装预览", "Deterministic assembly preview only; no model call", "Aperçu d'assemblage uniquement ; aucun appel au modèle", "Solo anteprima deterministica; nessuna chiamata al modello", "Tylko podgląd złożenia; bez wywołania modelu")
        case .brainCostNoteUncalled:
            return e("未调用模型，仅组装预览", "Model not invoked; deterministic assembly preview only", "Modèle non invoqué ; aperçu d'assemblage déterministe uniquement", "Modello non invocato; solo anteprima di assemblaggio deterministica", "Model nie został wywołany; tylko deterministyczny podgląd złożenia")
        case .brainCurrentAccountFmt:
            return e("当前账号: %@", "Current account: %@", "Compte actuel : %@", "Account corrente: %@", "Bieżące konto: %@")
        case .brainCurrentAccountNone:
            return e("当前账号: -", "Account: -", "Compte : -", "Account: -", "Konto: -")
        case .brainDefaultDraftReady:
            return e("默认角色草稿已准备好，可直接编辑并保存。", "Default role draft ready; edit and save.", "Brouillon de rôle par défaut prêt ; modifiez et enregistrez.", "Bozza del ruolo predefinito pronta; modifica e salva.", "Szkic roli domyślnej jest gotowy; edytuj i zapisz.")
        case .brainDefaultRoleDraftReady:
            return e("默认角色草稿已准备好，可直接编辑并保存。", "Default role draft ready; edit directly and save.", "Brouillon de rôle par défaut prêt ; modifiez directement et enregistrez.", "Bozza ruolo predefinito pronta; modifica e salva.", "Szkic roli domyślnej jest gotowy; edytuj bezpośrednio i zapisz.")
        case .brainDefaultRoleDraftSelectedFmt:
            return e("默认角色草稿已选择为「%@」，待点击「保存绑定草稿」", "Default role draft selected as \"%@\", pending 'Save Bindings Draft'", "Brouillon du rôle par défaut défini sur « %@ », en attente d'enregistrement", "Bozza ruolo predefinito impostata su \"%@\", in attesa di salvataggio", "Wybrano szkic roli domyślnej jako „%@”, oczekuje na zapisanie szkicu powiązań")
        case .brainDefaultRoleLabel:
            return e("账号默认角色:", "Account Default Role:", "Rôle par défaut du compte :", "Ruolo predefinito dell'account:", "Domyślna rola konta:")
        case .brainDefaultRolePriorityNotice:
            return e("默认角色用于白名单内未指定专属角色的会话；专属角色保持优先。", "Default role is used for allowlist sessions without dedicated role; dedicated roles take precedence.", "Le rôle par défaut est utilisé pour les sessions sans rôle dédié ; les rôles dédiés restent prioritaires.", "Il ruolo predefinito è usato per le sessioni senza ruolo dedicato; i ruoli dedicati hanno la priorità.", "Rola domyślna jest używana dla sesji bez dedykowanej roli; dedykowane role mają pierwszeństwo.")
        case .brainDefaultRoleScopeNotice:
            return e("默认角色用于白名单内未指定专属角色的会话；专属角色保持优先。", "Default role is used for allowlist conversations without an override; overrides take precedence.", "Le rôle par défaut s'applique aux conversations sans dérogation ; les rôles dédiés sont prioritaires.", "Il ruolo predefinito si applica alle conversazioni senza eccezioni; i ruoli dedicati hanno priorità.", "Rola domyślna jest używana dla rozmów bez wyjątku; role dedykowane mają pierwszeństwo.")
        case .brainDefaultRoleSelectedFmt:
            return e("已选择「%@」为默认角色。请保存绑定草稿，再启用配置。", "Selected '%@' as default role. Save bindings draft before activating.", "« %@ » sélectionné par défaut. Enregistrez les liaisons avant d'activer.", "Selezionato '%@' come predefinito. Salva le associazioni prima di attivare.", "Wybrano „%@” jako rolę domyślną. Zapisz szkic przypisań przed aktywacją.")
        case .brainDefaultTemplateMissing:
            return e("默认角色模板缺失，请重新安装完整应用。", "Default role template missing; please reinstall application.", "Modèle de rôle par défaut manquant ; veuillez réinstaller l'application.", "Modello di ruolo predefinito mancante; reinstalla l'applicazione.", "Brak szablonu roli domyślnej; zainstaluj ponownie aplikację.")
        case .brainDeleteDraftSuccess:
            return e("未保存角色已删除", "Unsaved role deleted", "Rôle non enregistré supprimé", "Ruolo non salvato eliminato", "Niezapisana rola została usunięta")
        case .brainDeleteOverrideErrSelect:
            return e("删除失败：请在表格中或下拉框中选择要删除的覆盖规则", "Delete failed: Please select an override rule to delete in the table or dropdown", "Échec de suppression : Veuillez sélectionner une dérogation à supprimer dans le tableau", "Eliminazione non riuscita: Seleziona una regola da eliminare nella tabella o nel menu", "Usuwanie nie powiodło się: Wybierz regułę wyjątku do usunięcia w tabeli lub menu")
        case .brainDeleteOverrideSelectRule:
            return e("删除失败：请在表格中或下拉框中选择要删除的覆盖规则", "Delete failed: select an override to delete", "Échec de la suppression : sélectionnez une dérogation", "Eliminazione fallita: seleziona un'eccezione da eliminare", "Usuwanie nieudane: wybierz wyjątek do usunięcia")
        case .brainDeleteRoleActivePrompt:
            return e("删除后将从角色列表移除，历史对话和版本记录会保留。", "It will be removed from the list; historical chats and version records will be kept.", "Il sera retiré de la liste ; les conversations et versions historiques seront conservées.", "Verrà rimosso dall'elenco; conversazioni e versioni storiche saranno mantenute.", "Zostanie usunięta z listy; historia rozmów i wersje zostaną zachowane.")
        case .brainDeleteRoleConfirmTitleFmt:
            return e("删除角色「%@」？", "Delete role \"%@\"?", "Supprimer le rôle « %@ » ?", "Eliminare il ruolo \"%@\"?", "Usunąć rolę „%@”?")
        case .brainDeleteRoleDialogInfoSaved:
            return e("删除后将从角色列表移除，历史对话和版本记录会保留。", "After deletion, it is removed from the role list; history and version records are preserved.", "Après suppression, il est retiré de la liste ; l'historique et les versions sont conservés.", "Dopo l'eliminazione verrà rimosso dall'elenco; cronologia e versioni vengono conservate.", "Po usunięciu rola zniknie z listy; historia i wersje zostaną zachowane.")
        case .brainDeleteRoleDialogInfoUnsaved:
            return e("该角色尚未保存，删除后将丢弃这份草稿。", "This role has not been saved; deleting will discard this draft.", "Ce rôle n'a pas été enregistré ; la suppression supprimera ce brouillon.", "Questo ruolo non è stato salvato; l'eliminazione scarterà questa bozza.", "Ta rola nie została zapisana; usunięcie odrzuci ten szkic.")
        case .brainDeleteRoleDialogTitleFmt:
            return e("删除角色「%@」？", "Delete role '%@'?", "Supprimer le rôle « %@ » ?", "Eliminare il ruolo '%@'?", "Usunąć rolę „%@”?")
        case .brainDeleteRoleDraftPrompt:
            return e("该角色尚未保存，删除后将丢弃这份草稿。", "This role has not been saved; deleting will discard this draft.", "Ce rôle n'a pas été enregistré ; la suppression supprimera ce brouillon.", "Questo ruolo non è stato salvato; l'eliminazione scarterà questa bozza.", "Ta rola nie została zapisana; usunięcie odrzuci ten szkic.")
        case .brainDeleteRoleFailedFmt:
            return e("删除失败：%1$@", "Deletion failed: %1$@", "Échec de la suppression : %1$@", "Eliminazione non riuscita: %1$@", "Usuwanie nieudane: %1$@")
        case .brainDeleteRoleSuccess:
            return e("角色已删除", "Role deleted", "Rôle supprimé", "Ruolo eliminato", "Rola została usunięta")
        case .brainDeleteSelectedOverride:
            return e("删除选定覆盖", "Delete selected override", "Supprimer la dérogation", "Elimina eccezione selezionata", "Usuń zaznaczony wyjątek")
        case .brainDeliveryStatusFmt:
            return e("送达状态: %@", "Delivery status: %@", "État de remise : %@", "Stato di consegna: %@", "Status doręczenia: %@")
        case .brainDeliveryUnknown:
            return e("送达状态: 未知 (unknown)", "Delivery status: unknown", "État de livraison : inconnu", "Stato consegna: sconosciuto", "Stan doręczenia: nieznany")
        case .brainDeliveryUnknownValue:
            return e("未知 (unknown)", "Unknown (unknown)", "Inconnu (unknown)", "Sconosciuto (unknown)", "Nieznany (unknown)")
        case .brainDiffBoxTitle:
            return e("两版本差异对比", "Two-Version Diff Comparison", "Comparaison des différences entre deux versions", "Confronto differenze tra due versioni", "Porównanie różnic między dwoma wersjami")
        case .brainDiffBudgetChangeFmt:
            return e("• 上下文预算: %1$@ -> %2$@", "• Context budget: %1$@ -> %2$@", "• Budget de contexte : %1$@ -> %2$@", "• Budget di contesto: %1$@ -> %2$@", "• Budżet kontekstu: %1$@ -> %2$@")
        case .brainDiffBudgetChangedFmt:
            return e("• 上下文预算: %@ -> %@", "• Context budget: %@ -> %@", "• Budget de contexte : %@ -> %@", "• Budget di contesto: %@ -> %@", "• Budżet kontekstu: %@ -> %@")
        case .brainDiffGoalsBothMissing:
            return e("• 核心目标: 双方均缺失 [缺失]", "• Goals: Both missing [Missing]", "• Objectifs principaux : Manquant des deux côtés [Manquant]", "• Obiettivi principali: Mancante da entrambi i lati [Mancante]", "• Cele główne: Brak po obu stronach [Brak]")
        case .brainDiffGoalsChangeFmt:
            return e("• 核心目标: %1$@ -> %2$@", "• Goals: %1$@ -> %2$@", "• Objectifs : %1$@ -> %2$@", "• Obiettivi: %1$@ -> %2$@", "• Cele: %1$@ -> %2$@")
        case .brainDiffGoalsChangedFmt:
            return e("• 核心目标: %@ -> %@", "• Goals: %@ -> %@", "• Objectifs principaux : %@ -> %@", "• Obiettivi principali: %@ -> %@", "• Cele główne: %@ -> %@")
        case .brainDiffGoalsNoChangeFmt:
            return e("• 核心目标: 无变化 %@", "• Goals: unchanged %@", "• Objectifs : inchangés %@", "• Obiettivi: invariati %@", "• Cele: bez zmian %@")
        case .brainDiffHeaderFmt:
            return e("【版本对比】%1$@ (左) vs %2$@ (右)", "[Version Diff] %1$@ (left) vs %2$@ (right)", "[Diff de versions] %1$@ (gauche) vs %2$@ (droite)", "[Diff versioni] %1$@ (sinistra) vs %2$@ (destra)", "[Porównanie wersji] %1$@ (lewa) vs %2$@ (prawa)")
        case .brainDiffIncompleteDraftNoticeAFmt:
            return e("※ 版本 A (%@): 此版本未保存完整草稿，仅显示已保存的内容。", "※ Version A (%@): Incomplete draft; only saved content is shown.", "※ Version A (%@) : Brouillon incomplet ; seul le contenu enregistré est affiché.", "※ Versione A (%@): Bozza incompleta; solo il contenuto salvato viene mostrato.", "※ Wersja A (%@): Niepełny szkic; pokazano tylko zapisaną treść.")
        case .brainDiffIncompleteDraftNoticeBFmt:
            return e("※ 版本 B (%@): 此版本未保存完整草稿，仅显示已保存的内容。", "※ Version B (%@): Incomplete draft; only saved content is shown.", "※ Version B (%@) : Brouillon incomplet ; seul le contenu enregistré est affiché.", "※ Versione B (%@): Bozza incompleta; solo il contenu salvato viene mostrato.", "※ Wersja B (%@): Niepełny szkic; pokazano tylko zapisaną treść.")
        case .brainDiffKnowledgeBothMissing:
            return e("• 知识库引用: 双方均缺失 [缺失]", "• Knowledge refs: Both missing [Missing]", "• Références de connaissances : Manquant des deux côtés [Manquant]", "• Riferimenti di conoscenza: Mancante da entrambi i lati [Mancante]", "• Referencje bazy wiedzy: Brak po obu stronach [Brak]")
        case .brainDiffKnowledgeChangeFmt:
            return e("• 知识库引用: %1$@ -> %2$@", "• Knowledge refs: %1$@ -> %2$@", "• Références de connaissances : %1$@ -> %2$@", "• Riferimenti di conoscenza: %1$@ -> %2$@", "• Odwołania do wiedzy: %1$@ -> %2$@")
        case .brainDiffKnowledgeChangedFmt:
            return e("• 知识库引用: %@ -> %@", "• Knowledge refs: %@ -> %@", "• Références de connaissances : %@ -> %@", "• Riferimenti di conoscenza: %@ -> %@", "• Referencje bazy wiedzy: %@ -> %@")
        case .brainDiffKnowledgeNoChangeFmt:
            return e("• 知识库引用: 无变化 (%d 篇)", "• Knowledge refs: unchanged (%d documents)", "• Références de connaissances : inchangées (%d documents)", "• Riferimenti di conoscenza: invariati (%d documenti)", "• Odwołania do wiedzy: bez zmian (%d dokumentów)")
        case .brainDiffLabelVerA:
            return e("版本 A:", "Version A:", "Version A :", "Versione A:", "Wersja A:")
        case .brainDiffLabelVerB:
            return e("版本 B:", "Version B:", "Version B :", "Versione B:", "Wersja B:")
        case .brainDiffLeftFmt:
            return e("  [- 左]: %@", "  [- Left]: %@", "  [- Gauche] : %@", "  [- Sinistra]: %@", "  [- Lewa]: %@")
        case .brainDiffLegacyVerAFmt:
            return e("※ 版本 A (%@): 此版本未保存完整草稿，仅显示已保存的内容。", "※ Version A (%@): Incomplete draft saved; only saved content is shown.", "※ Version A (%@) : Brouillon incomplet ; seul le contenu enregistré est affiché.", "※ Versione A (%@): Bozza incompleta; viene mostrato solo il contenuto salvato.", "※ Wersja A (%@): Niepełny szkic; pokazano tylko zapisaną treść.")
        case .brainDiffLegacyVerBFmt:
            return e("※ 版本 B (%@): 此版本未保存完整草稿，仅显示已保存的内容。", "※ Version B (%@): Incomplete draft saved; only saved content is shown.", "※ Version B (%@) : Brouillon incomplet ; seul le contenu enregistré est affiché.", "※ Versione B (%@): Bozza incompleta; viene mostrato solo il contenuto salvato.", "※ Wersja B (%@): Niepełny szkic; pokazano tylko zapisaną treść.")
        case .brainDiffNameChangedFmt:
            return e("• 角色名称: [%@] -> [%@]", "• Role name: [%@] -> [%@]", "• Nom du rôle : [%@] -> [%@]", "• Nome ruolo: [%@] -> [%@]", "• Nazwa roli: [%@] -> [%@]")
        case .brainDiffNameNoChangeFmt:
            return e("• 角色名称: 无变化 [%@]", "• Role name: No change [%@]", "• Nom du rôle : Aucun changement [%@]", "• Nome ruolo: Nessuna modifica [%@]", "• Nazwa roli: Bez zmian [%@]")
        case .brainDiffPersonaBothMissing:
            return e("• 人设设定 (Persona): 双方均缺失 [缺失]", "• Persona: Both missing [Missing]", "• Persona : Manquant des deux côtés [Manquant]", "• Persona: Mancante da entrambi i lati [Mancante]", "• Persona: Brak po obu stronach [Brak]")
        case .brainDiffPersonaHeader:
            return e("• 人设设定 (Persona):", "• Persona:", "• Persona :", "• Persona:", "• Persona:")
        case .brainDiffPersonaNoChangeFmt:
            return e("• 人设设定 (Persona): 无变化 (%d 字符)", "• Persona: No change (%d characters)", "• Persona : Aucun changement (%d caractères)", "• Persona: Nessuna modifica (%d caratteri)", "• Persona: Bez zmian (%d znaków)")
        case .brainDiffPersonaTitle:
            return e("• 人设设定 (Persona):", "• Persona:", "• Persona :", "• Persona:", "• Persona:")
        case .brainDiffPersonaUnchangedFmt:
            return e("• 人设设定 (Persona): 无变化 (%d 字符)", "• Persona: unchanged (%d characters)", "• Persona : inchangée (%d caractères)", "• Persona: invariata (%d caratteri)", "• Persona: bez zmian (%d znaków)")
        case .brainDiffRightFmt:
            return e("  [+ 右]: %@", "  [+ Right]: %@", "  [+ Droite] : %@", "  [+ Destra]: %@", "  [+ Prawa]: %@")
        case .brainDiffRoleNameChangeFmt:
            return e("• 角色名称: [%1$@] -> [%2$@]", "• Role name: [%1$@] -> [%2$@]", "• Nom du rôle : [%1$@] -> [%2$@]", "• Nome ruolo: [%1$@] -> [%2$@]", "• Nazwa roli: [%1$@] -> [%2$@]")
        case .brainDiffRoleNameNoChangeFmt:
            return e("• 角色名称: 无变化 [%@]", "• Role name: unchanged [%@]", "• Nom du rôle : inchangé [%@]", "• Nome ruolo: invariato [%@]", "• Nazwa roli: bez zmian [%@]")
        case .brainDiffRulesBothMissing:
            return e("• 业务规则: 双方均缺失 [缺失]", "• Business rules: Both missing [Missing]", "• Règles métier : Manquant des deux côtés [Manquant]", "• Regole aziendali: Mancante da entrambi i lati [Mancante]", "• Reguły biznesowe: Brak po obu stronach [Brak]")
        case .brainDiffRulesChangeFmt:
            return e("• 业务规则: %1$@ -> %2$@", "• Rules: %1$@ -> %2$@", "• Règles : %1$@ -> %2$@", "• Regole: %1$@ -> %2$@", "• Reguły: %1$@ -> %2$@")
        case .brainDiffRulesChangedFmt:
            return e("• 业务规则: %@ -> %@", "• Business rules: %@ -> %@", "• Règles métier : %@ -> %@", "• Regole aziendali: %@ -> %@", "• Reguły biznesowe: %@ -> %@")
        case .brainDiffRulesNoChangeFmt:
            return e("• 业务规则: 无变化 %@", "• Business rules: No change %@", "• Règles métier : Aucun changement %@", "• Regole aziendali: Nessuna modifica %@", "• Reguły biznesowe: Bez zmian %@")
        case .brainDiffSavedIndicator:
            return e("已保存", "Saved", "Enregistré", "Salvato", "Zapisano")
        case .brainDiffTemplatesChangeFmt:
            return e("• 异常回复模板: %1$@ -> %2$@", "• Fallback templates: %1$@ -> %2$@", "• Modèles de secours : %1$@ -> %2$@", "• Modelli di riserva: %1$@ -> %2$@", "• Szablony awaryjne: %1$@ -> %2$@")
        case .brainDiffTemplatesChangedFmt:
            return e("• 异常回复模板: %@ -> %@", "• Fallback templates: %@ -> %@", "• Modèles de secours : %@ -> %@", "• Modelli di riserva: %@ -> %@", "• Szablony awaryjne: %@ -> %@")
        case .brainDiffToneChangeFmt:
            return e("• 角色语气: [%1$@] -> [%2$@]", "• Role tone: [%1$@] -> [%2$@]", "• Ton du rôle : [%1$@] -> [%2$@]", "• Tono ruolo: [%1$@] -> [%2$@]", "• Ton roli: [%1$@] -> [%2$@]")
        case .brainDiffToneChangedFmt:
            return e("• 角色语气: [%@] -> [%@]", "• Role tone: [%@] -> [%@]", "• Ton du rôle : [%@] -> [%@]", "• Tono ruolo: [%@] -> [%@]", "• Ton roli: [%@] -> [%@]")
        case .brainDiffToneNoChangeFmt:
            return e("• 角色语气: 无变化 [%@]", "• Role tone: unchanged [%@]", "• Ton du rôle : inchangé [%@]", "• Tono ruolo: invariato [%@]", "• Ton roli: bez zmian [%@]")
        case .brainDoubleClickToEdit:
            return e("双击角色名编辑", "Double-click role name to edit", "Double-cliquez pour modifier", "Fai doppio clic per modificare", "Kliknij dwukrotnie nazwę roli, aby edytować")
        case .brainDraftLoadedNotice:
            return e("草稿已加载，可编辑后保存或合成预览", "Draft loaded; edit and save or run assemble preview", "Brouillon chargé ; modifiez et enregistrez ou prévisualisez", "Bozza caricata; modifica e salva o esegui l'anteprima", "Szkic załadowany; edytuj i zapisz lub uruchom podgląd")
        case .brainDraftLoadedStatus:
            return e("草稿已加载，可编辑后保存或合成预览", "Draft loaded; edit and save, or synthesize preview", "Brouillon chargé ; modifiez et enregistrez, ou générez un aperçu", "Bozza caricata; modifica e salva o genera anteprima", "Szkic załadowany; edytuj i zapisz lub wygeneruj podgląd")
        case .brainDraftRunningStatusPlaceholder:
            return e("草稿: - · 运行: -", "Draft: - · Running: -", "Brouillon : - · Actif : -", "Bozza: - · Attivo: -", "Szkic: - · Aktywny: -")
        case .brainDraftSavedNotice:
            return e("草稿已保存。请在「适用会话」设置回复范围，再启用配置。", "Draft saved. Configure conversation scope in 'Conversations' before activating.", "Brouillon enregistré. Définissez le périmètre dans « Conversations » avant d'activer.", "Bozza salvata. Configura l'ambito in 'Conversazioni' prima di attivare.", "Szkic zapisany. Skonfiguruj zakres w „Rozmowach” przed aktywacją.")
        case .brainEnterMockMessagePrompt:
            return e("请输入模拟消息正文", "Enter a test message", "Saisissez un message de test", "Inserisci un messaggio di prova", "Wpisz wiadomość testową")
        case .brainErrActionFailedFmt:
            return e("%@失败 (exit %@)", "%@ failed (exit %@)", "Échec de %@ (sortie %@)", "%@ non riuscito (uscita %@)", "Niepowodzenie %@ (kod wyjścia %@)")
        case .brainErrActionInvalidDataFmt:
            return e("%@返回无效数据，无法获取 generation", "%@ returned invalid data, unable to obtain generation", "%@ a renvoyé des données invalides, impossible d'obtenir la génération", "%@ ha restituito dati non validi, impossibile ottenere la generazione", "%@ zwróciło nieprawidłowe dane, nie można uzyskać generacji")
        case .brainErrFetchHistoryFailed:
            return e("获取历史版本失败", "Failed to fetch history versions", "Échec de récupération de l'historique des versions", "Recupero cronologia versioni non riuscito", "Nie udało się pobrać historii wersji")
        case .brainErrFetchRoleFailed:
            return e("获取角色失败", "Failed to fetch role", "Échec de récupération du rôle", "Recupero del ruolo non riuscito", "Nie udało się pobrać roli")
        case .brainErrFetchRolesFailed:
            return e("获取客服角色配置失败", "Failed to fetch customer service roles configuration", "Échec de récupération de la configuration des rôles", "Recupero della configurazione dei ruoli non riuscito", "Nie udało się pobrać konfiguracji ról obsługi klienta")
        case .brainErrFetchTurnFailed:
            return e("获取轮次详情失败", "Failed to fetch turn details", "Échec de récupération des détails du tour", "Recupero dettagli turno non riuscito", "Nie udało się pobrać szczegółów tury")
        case .brainErrInvalidUtf8:
            return e("无效 UTF8 字符串", "Invalid UTF8 string", "Chaîne UTF-8 invalide", "Stringa UTF-8 non valida", "Nieprawidłowy ciąg znaków UTF-8")
        case .brainErrSaveKnowledgeFailed:
            return e("保存知识失败", "Failed to save knowledge", "Échec de l'enregistrement des connaissances", "Salvataggio conoscenza non riuscito", "Nie udało się zapisać wiedzy")
        case .brainErrSaveKnowledgeInvalidResp:
            return e("保存知识返回无效响应", "Invalid response returned while saving knowledge", "Réponse invalide renvoyée lors de l'enregistrement des connaissances", "Risposta non valida restituita durante il salvataggio della conoscenza", "Nieprawidłowa odpowiedź podczas zapisywania wiedzy")
        case .brainErrTurnNotFound:
            return e("轮次不存在", "Turn does not exist", "Le tour n'existe pas", "Il turno non esiste", "Tura nie istnieje")
        case .brainErrorUnknown:
            return e("未知错误", "Unknown error", "Erreur inconnue", "Errore sconosciuto", "Nieznany błąd")
        case .brainFetchConfigFailed:
            return e("获取客服角色配置失败", "Failed to fetch support persona configuration", "Échec de l'obtention de la configuration de la persona", "Recupero configurazione persona fallito", "Pobieranie konfiguracji persony nieudane")
        case .brainFetchHistoryFailed:
            return e("获取历史版本失败", "Failed to fetch version history", "Échec de récupération de l'historique", "Recupero cronologia versioni fallito", "Pobieranie historii wersji nieudane")
        case .brainFetchRoleFailed:
            return e("获取角色失败", "Failed to fetch role", "Échec de récupération du rôle", "Recupero ruolo fallito", "Pobieranie roli nieudane")
        case .brainFetchTurnDetailFailed:
            return e("获取轮次详情失败", "Failed to fetch turn details", "Échec de récupération des détails du tour", "Recupero dettagli turno fallito", "Pobieranie szczegółów tury nieudane")
        case .brainGenActiveNoDraft:
            return e("当前激活 (未存草稿)", "Active (no draft)", "Actif (sans brouillon)", "Attivo (senza bozza)", "Aktywny (bez szkicu)")
        case .brainGenActiveShort:
            return e("当前激活", "Active", "Actif", "Attivo", "Aktywny")
        case .brainGenActiveWithDraft:
            return e("当前激活 (含草稿)", "Active (with draft)", "Actif (avec brouillon)", "Attivo (con bozza)", "Aktywny (ze szkicem)")
        case .brainGenCurrentActive:
            return e("当前激活", "Current active", "Actif actuel", "Attivo corrente", "Bieżący aktywny")
        case .brainGenCurrentActiveNoDraft:
            return e("当前激活 (未存草稿)", "Current active (no draft)", "Actif actuel (sans brouillon)", "Attivo corrente (senza bozza)", "Bieżący aktywny (bez szkicu)")
        case .brainGenCurrentActiveWithDraft:
            return e("当前激活 (含草稿)", "Current active (with draft)", "Actif actuel (avec brouillon)", "Attivo corrente (con bozza)", "Bieżący aktywny (ze szkicem)")
        case .brainGenCurrentSuffix:
            return e(" (当前)", " (current)", " (actuel)", " (corrente)", " (bieżąca)")
        case .brainGenInitial:
            return e("初始代次", "Initial Gen", "Génération initiale", "Generazione iniziale", "Początkowa generacja")
        case .brainGenItemTitleFmt:
            return e("第 %1$@ 代%2$@", "Gen %1$@%2$@", "Gén %1$@%2$@", "Gen %1$@%2$@", "Gen %1$@%2$@")
        case .brainGenOrdinalFmt:
            return e("第 %@ 代", "Gen %@", "Gén %@", "Gen %@", "Gen %@")
        case .brainGenSavedDraft:
            return e("已存草稿", "Saved draft", "Brouillon enregistré", "Bozza salvata", "Zapisany szkic")
        case .brainGenerationFmt:
            return e("第 %llu 代", "Generation %llu", "Génération %llu", "Generazione %llu", "Generacja %llu")
        case .brainGoalsFieldHelper:
            return e("用逗号分隔多个目标", "Separate multiple goals with commas", "Séparez les objectifs par des virgules", "Separa gli obiettivi con virgole", "Rozdziel wiele celów przecinkami")
        case .brainGoalsFieldLabel:
            return e("核心目标:", "Key Goals:", "Objectifs clés :", "Obiettivi principali:", "Główne cele:")
        case .brainGoalsPlaceholder:
            return e("用逗号分隔多个目标", "Separate multiple goals with commas", "Séparez les objectifs par des virgules", "Separa gli obiettivi con virgole", "Oddziel cele przecinkami")
        case .brainHistoryIntro:
            return e("历史版本管理：可查看历次保存与启用的版本、对比任意两版本差异，并支持安全回滚至稳定代次。", "Version history management: view past saved and activated generations, diff any two revisions, and safely rollback to stable versions.", "Gestion de l'historique : affichez les générations enregistrées et actives, comparez deux révisions et rétablissez une version stable en toute sécurité.", "Gestione cronologia versioni: visualizza le generazioni passate, confronta due revisioni e ripristina versioni stabili in sicurezza.", "Zarządzanie historią wersji: przeglądaj zapisane i aktywne generacje, porównuj dwie wersje i bezpiecznie wycofuj do stabilnej generacji.")
        case .brainHistoryNoticeCompleteFmt:
            return e("第 %1$llu 代：已保存完整草稿与配置。\n• 默认角色=%2$@, 版本=%3$@, 覆盖规则=%4$d条", "Generation %1$llu: Complete draft and configuration saved.\n• Default role=%2$@, version=%3$@, overrides=%4$d", "Génération %1$llu : Brouillon complet et configuration enregistrés.\n• Rôle par défaut=%2$@, version=%3$@, dérogations=%4$d", "Generazione %1$llu: Bozza completa e configurazione salvate.\n• Ruolo predefinito=%2$@, versione=%3$@, eccezioni=%4$d", "Generacja %1$llu: Zapisano pełny szkic i konfigurację.\n• Rola domyślna=%2$@, wersja=%3$@, wyjątki=%4$d")
        case .brainHistoryNoticeIncompleteFmt:
            return e("第 %1$@ 代：%2$@。\n• 已保存运行配置: 默认角色=%3$@, 版本=%4$@, 覆盖规则=%5$@条\n• 草稿状态: %2$@ (%6$@)", "Gen %1$@: %2$@.\n• Saved active config: Default role=%3$@, revision=%4$@, overrides=%5$@\n• Draft status: %2$@ (%6$@)", "Gén %1$@ : %2$@.\n• Configuration enregistrée : Rôle=%3$@, révision=%4$@, dérogations=%5$@\n• Brouillon : %2$@ (%6$@)", "Gen %1$@: %2$@.\n• Config salvata: Ruolo=%3$@, revisione=%4$@, eccezioni=%5$@\n• Bozza: %2$@ (%6$@)", "Gen %1$@: %2$@.\n• Zapisana konfiguracja: Rola=%3$@, wersja=%4$@, wyjątki=%5$@\n• Szkic: %2$@ (%6$@)")
        case .brainHistorySectionIntro:
            return e("历史版本管理：可查看历次保存与启用的版本、对比任意两版本差异，并支持安全回滚至稳定代次。", "Version history: view past versions, compare diffs, and safely roll back to a stable generation.", "Historique des versions : affichez les versions, comparez et restaurez en sécurité.", "Cronologia versioni: visualizza le versioni, confronta le differenze e ripristina in sicurezza.", "Historia wersji: przeglądaj wersje, porównuj różnice i bezpiecznie przywracaj.")
        case .brainHistoryTableBoxTitle:
            return e("版本与代次历史", "Generations and version history", "Historique des versions et générations", "Cronologia versioni e generazioni", "Historia wersji i generacji")
        case .brainImportAndBindKnowledge:
            return e("导入并关联至角色", "Import and associate with role", "Importer et associer au rôle", "Importa e associa al ruolo", "Importuj i przypisz do roli")
        case .brainImportKnowledgeEmptyErr:
            return e("导入知识失败：标题与正文不能为空", "Import failed: Title and content cannot be empty", "Échec de l'importation : Le titre et le contenu ne peuvent pas être vides", "Importazione non riuscita: Titolo e contenuto non possono essere vuoti", "Importowanie nie powiodło się: Tytuł i treść nie mogą być puste")
        case .brainImportNewKnowledgeBtn:
            return e("导入/新建知识", "Import / New knowledge", "Importer / Créer une connaissance", "Importa / Crea conoscenza", "Importuj / Nowa wiedza")
        case .brainInitDefaultRole:
            return e("初始化客服角色", "Initialize support persona", "Initialiser la persona du support", "Inizializza persona di assistenza", "Zainicjuj personę obsługi")
        case .brainKnowledgeAddBoxTitle:
            return e("导入/新建知识", "Import / Create Knowledge", "Importer / Créer des connaissances", "Importa / Crea conoscenza", "Importuj / Utwórz wiedzę")
        case .brainKnowledgeContentField:
            return e("知识正文 (Markdown):", "Content (Markdown):", "Contenu (Markdown) :", "Contenuto (Markdown):", "Treść (Markdown):")
        case .brainKnowledgeContentLabel:
            return e("知识正文 (Markdown):", "Knowledge Content (Markdown):", "Contenu de la connaissance (Markdown) :", "Contenuto conoscenza (Markdown):", "Treść wiedzy (Markdown):")
        case .brainKnowledgeIntro:
            return e("业务知识 (Markdown/纯文本)。通过核心 brain-save-knowledge 控制接口保存，自动计算 SHA256 摘要并固定版本。", "Business knowledge (Markdown/Plain text). Saved via core brain-save-knowledge control API, automatically hashing SHA256 and pinning revision.", "Connaissances métier (Markdown/Texte brut). Enregistré via l'API de contrôle brain-save-knowledge avec empreinte SHA256 et version figée.", "Conoscenza aziendale (Markdown/Testo). Salvato tramite API brain-save-knowledge, calcolando SHA256 e bloccando la revisione.", "Wiedza biznesowa (Markdown/Zwykły tekst). Zapisywana przez API brain-save-knowledge z automatycznym haszem SHA256 i przypiętą wersją.")
        case .brainKnowledgeNone:
            return e("当前角色未引用任何知识", "Current role references no knowledge", "Aucune connaissance référencée pour ce rôle", "Nessuna conoscenza associata a questo ruolo", "Bieżąca rola nie odwołuje się do żadnej wiedzy")
        case .brainKnowledgeRefListLabel:
            return e("当前角色引用的知识列表:", "Knowledge referenced by current role:", "Connaissances référencées par le rôle :", "Conoscenze associate al ruolo corrente:", "Wiedza powiązana z bieżącą rolą:")
        case .brainKnowledgeSavedSuccess:
            return e("知识已成功保存至核心并关联到当前草稿！", "Knowledge saved to core and linked to current draft!", "Connaissances enregistrées et liées au brouillon actuel !", "Conoscenze salvate e collegate alla bozza attuale!", "Wiedza zapisana w rdzeniu i powiązana z bieżącym szkicem!")
        case .brainKnowledgeSectionDesc:
            return e("业务知识 (Markdown/纯文本)。通过核心 brain-save-knowledge 控制接口保存，自动计算 SHA256 摘要并固定版本。", "Business knowledge (Markdown/plain text). Saved via brain-save-knowledge with automatic SHA256 hashing and version pinning.", "Connaissances (Markdown/texte brut). Enregistrées via brain-save-knowledge avec empreinte SHA256 et version figée.", "Conoscenze (Markdown/testo semplice). Salvate tramite brain-save-knowledge con hash SHA256 e versione bloccata.", "Wiedza biznesowa (Markdown/zwykły tekst). Zapisywana przez brain-save-knowledge z automatycznym skrótem SHA256 i wersjonowaniem.")
        case .brainKnowledgeSourceField:
            return e("来源标识:", "Source:", "Source :", "Origine:", "Źródło:")
        case .brainKnowledgeSourceLabel:
            return e("来源标识:", "Source Identifier:", "Identifiant de la source :", "Identificatore sorgente:", "Identyfikator źródła:")
        case .brainKnowledgeSourcePlaceholder:
            return e("例如 faq.md 或 manual.txt", "e.g. faq.md or manual.txt", "par ex. faq.md ou manuel.txt", "es. faq.md o manuale.txt", "np. faq.md lub instrukcja.txt")
        case .brainKnowledgeTitleContentRequired:
            return e("导入知识失败：标题与正文不能为空", "Import knowledge failed: title and content cannot be empty", "Échec : le titre et le contenu ne peuvent pas être vides", "Importazione fallita: titolo e contenuto non possono essere vuoti", "Importowanie nieudane: tytuł i treść nie mogą być puste")
        case .brainKnowledgeTitleField:
            return e("知识标题:", "Title:", "Titre :", "Titolo:", "Tytuł:")
        case .brainLabelAllowlistContact:
            return e("白名单联系人:", "Allowlist contact:", "Contact autorisé :", "Contatto consentito:", "Kontakt z białej listy:")
        case .brainLabelBindPersona:
            return e("绑定客服角色:", "Bind support persona:", "Associer la persona :", "Associa persona:", "Przypisz personę obsługi:")
        case .brainLoadConfigFailedFmt:
            return e("加载客服角色配置失败: %@", "Failed to load support persona configuration: %@", "Échec du chargement de la configuration : %@", "Caricamento configurazione persona fallito: %@", "Ładowanie konfiguracji persony nieudane: %@")
        case .brainLoadRoleDraftFailedFmt:
            return e("读取角色草稿失败: %@", "Failed to load role draft: %@", "Échec de lecture du brouillon de rôle : %@", "Lettura bozza ruolo fallita: %@", "Odczyt szkicu roli nieudany: %@")
        case .brainLoadRolesFailedFmt:
            return e("加载客服角色配置失败: %@", "Failed to load customer service roles configuration: %@", "Échec du chargement de la configuration des rôles : %@", "Caricamento configurazione ruoli non riuscito: %@", "Nie udało się załadować konfiguracji ról obsługi: %@")
        case .brainMigratedFromLegacy:
            return e("从旧版迁移", "Migrated from legacy", "Migré depuis l'ancienne version", "Migrato dalla versione precedente", "Zmigrowano ze starszej wersji")
        case .brainModelAndConnection:
            return e("模型与连接…", "Model and connection…", "Modèle et connexion…", "Modello e connessione…", "Model i połączenie…")
        case .brainModelLoading:
            return e("模型：待加载", "Model: loading…", "Modèle : chargement…", "Modello: caricamento…", "Model: wczytywanie…")
        case .brainModelStatusPending:
            return e("模型：待加载", "Model: Loading...", "Modèle : Chargement...", "Modello: In caricamento...", "Model: Ładowanie...")
        case .brainNewAssistantDefaultGoal:
            return e("解答用户咨询", "Answer customer inquiries", "Répondre aux demandes des clients", "Rispondere alle richieste dei clienti", "Odpowiadaj na zapytania klientów")
        case .brainNewAssistantDefaultName:
            return e("新助手", "New Assistant", "Nouvel Assistant", "Nuovo Assistente", "Nowy Asystent")
        case .brainNewAssistantDefaultPersona:
            return e("您是一位专业友善的客服助手。", "You are a professional and friendly customer support assistant.", "Vous êtes un assistant de support client professionnel et amical.", "Sei un assistente di supporto clienti professionale e cordiale.", "Jesteś profesjonalnym i przyjaznym asystentem obsługi klienta.")
        case .brainNewAssistantDefaultTone:
            return e("温和自然", "Warm and natural", "Chaleureux et naturel", "Caloroso e naturale", "Ciepły i naturalny")
        case .brainNewRoleDefaultName:
            return e("新助手", "New Assistant", "Nouvel assistant", "Nuovo assistente", "Nowy asystent")
        case .brainNewRoleDefaultPersona:
            return e("您是一位专业友善的客服助手。", "You are a professional and friendly customer service assistant.", "Vous êtes un assistant de service client professionnel et aimable.", "Sei un assistente del servizio clienti professionale e cordiale.", "Jesteś profesjonalnym i życzliwym asystentem obsługi klienta.")
        case .brainNewRoleDefaultTone:
            return e("温和自然", "Warm and natural", "Chaleureux et naturel", "Caloroso e naturale", "Ciepły i naturalny")
        case .brainNo:
            return e("否", "No", "Non", "No", "Nie")
        case .brainNoAllowlistContactsNotice:
            return e("当前白名单无可用联系人，新增覆盖前请先确认白名单配置与通讯录同步", "No allowlist contacts available; verify allowlist configuration and contact sync before adding overrides", "Aucun contact autorisé ; vérifiez la configuration et la synchronisation avant d'ajouter une dérogation", "Nessun contatto consentito; verifica la configurazione e la sincronizzazione prima di aggiungere eccezioni", "Brak kontaktów na białej liście; sprawdź konfigurację i synchronizację przed dodaniem wyjątków")
        case .brainNoKnowledgeReferenced:
            return e("当前角色未引用任何知识", "No knowledge referenced by this role", "Aucune connaissance référencée par ce rôle", "Nessuna conoscenza associata a questo ruolo", "Ta rola nie odwołuje się do żadnej wiedzy")
        case .brainNoOverrides:
            return e("暂无特定联系人覆盖", "No specific contact overrides", "Aucune dérogation de contact spécifique", "Nessuna eccezione per contatti specifici", "Brak wyjątków dla określonych kontaktów")
        case .brainNoOverridesDetail:
            return e("暂无特定联系人覆盖（所有联系人均使用默认角色）", "No specific contact overrides (all contacts use default role)", "Aucune dérogation spécifique (tous les contacts utilisent le rôle par défaut)", "Nessuna eccezione specifica (tutti i contatti usano il ruolo predefinito)", "Brak wyjątków dla kontaktów (wszystkie kontakty używają roli domyślnej)")
        case .brainNoOverridesSummary:
            return e("暂无特定联系人覆盖（所有联系人均使用默认角色）", "No contact overrides (all contacts use default role)", "Aucune dérogation de contact (tous les contacts utilisent le rôle par défaut)", "Nessuna eccezione per contatti (tutti i contatti usano il ruolo predefinito)", "Brak wyjątków dla kontaktów (wszystkie kontakty używają roli domyślnej)")
        case .brainNotReady:
            return e("未就绪", "Not ready", "Non prêt", "Non pronto", "Niegotowe")
        case .brainOverrideAlreadyExistsFmt:
            return e("添加失败：联系人「%@」已存在覆盖规则，可直接编辑", "Add failed: override already exists for '%@'; edit it directly", "Échec : la dérogation existe déjà pour « %@ » ; modifiez-la directement", "Aggiunta fallita: esiste già un'eccezione per '%@'; modificala direttamente", "Dodawanie nieudane: wyjątek dla „%@” już istnieje; edytuj go bezpośrednio")
        case .brainOverrideDeletedDraftFmt:
            return e("已从草稿中删除覆盖规则 [%@]，待点击「保存绑定草稿」", "Override rule [%@] deleted from draft, pending 'Save Bindings Draft'", "Dérogation [%@] supprimée du brouillon, en attente d'enregistrement", "Regola [%@] eliminata dalla bozza, in attesa di salvataggio", "Usunięto regułę wyjątku [%@] ze szkicu, oczekuje na zapisanie szkicu")
        case .brainOverrideDeletedFromDraftFmt:
            return e("已从草稿中删除覆盖规则 [%@]，待点击「保存绑定草稿」", "Removed override [%@] from draft; click 'Save bindings draft' to commit", "Dérogation [%@] retirée du brouillon ; enregistrez pour valider", "Rimossa eccezione [%@] dalla bozza; salva per confermare", "Usunięto wyjątek [%@] ze szkicu; kliknij „Zapisz szkic przypisań”")
        case .brainOverrideDraftAddedFmt:
            return e("已添加针对「%@」的覆盖规则草稿，待保存", "Added override draft for '%@'; save to commit", "Dérogation ajoutée pour « %@ » ; enregistrez pour valider", "Aggiunta bozza di eccezione per '%@'; salva per confermare", "Dodano szkic wyjątku dla „%@”; zapisz, aby zatwierdzić")
        case .brainOverrideDraftUpdatedFmt:
            return e("已更新针对「%@」的覆盖规则草稿，待保存", "Updated override draft for '%@'; save to commit", "Dérogation mise à jour pour « %@ » ; enregistrez pour valider", "Aggiornata bozza di eccezione per '%@'; salva per confermare", "Zaktualizowano szkic wyjątku dla „%@”; zapisz, aby zatwierdzić")
        case .brainOverrideItemLineFmt:
            return e("• 联系人: %1$@ [%2$@] ➔ %3$@ (%4$@) [版本: %5$@]", "• Contact: %1$@ [%2$@] ➔ %3$@ (%4$@) [Revision: %5$@]", "• Contact : %1$@ [%2$@] ➔ %3$@ (%4$@) [Révision : %5$@]", "• Contatto: %1$@ [%2$@] ➔ %3$@ (%4$@) [Revisione: %5$@]", "• Kontakt: %1$@ [%2$@] ➔ %3$@ (%4$@) [Wersja: %5$@]")
        case .brainOverrideLineFmt:
            return e("• 联系人: %1$@ [%2$@] ➔ %3$@ (%4$@) [版本: %5$@]", "• Contact: %1$@ [%2$@] ➔ %3$@ (%4$@) [rev: %5$@]", "• Contact : %1$@ [%2$@] ➔ %3$@ (%4$@) [rév : %5$@]", "• Contatto: %1$@ [%2$@] ➔ %3$@ (%4$@) [rev: %5$@]", "• Kontakt: %1$@ [%2$@] ➔ %3$@ (%4$@) [wersja: %5$@]")
        case .brainOverrideMustFromAllowlist:
            return e("添加失败：联系人必须源自真实白名单，禁止凭姓名构造ID", "Add failed: contacts must originate from the authoritative allowlist; synthetic IDs forbidden", "Échec : les contacts doivent provenir de la liste autorisée ; identifiants synthétiques interdits", "Aggiunta fallita: i contatti devono provenire dalla lista consentiti; ID sintetici vietati", "Dodawanie nieudane: kontakty muszą pochodzić z autorytatywnej białej listy; syntetyczne ID zabronione")
        case .brainOverrideNotFoundForUpdate:
            return e("更新失败：未找到对应的覆盖规则，请先点击「添加覆盖」", "Update failed: override not found; click 'Add override' first", "Échec : dérogation introuvable ; cliquez d'abord sur « Ajouter une dérogation »", "Aggiornamento fallito: eccezione non trovata; clicca prima su 'Aggiungi eccezione'", "Aktualizacja nieudana: nie znaleziono wyjątku; najpierw kliknij „Dodaj wyjątek”")
        case .brainOverridesBoxTitle:
            return e("特定联系人 / 群覆盖规则 (白名单唯一来源)", "Specific Contact / Group Overrides (Allowlist Source)", "Dérogations pour contacts / groupes (Source liste blanche)", "Eccezioni contatti / gruppi specifici (Fonte whitelist)", "Wyjątki dla kontaktów / grup (Wyłączne źródło biała lista)")
        case .brainPersonaFieldLabel:
            return e("身份与业务范围：", "Persona & scope:", "Identité et périmètre :", "Identità e ambito:", "Tożsamość i zakres:")
        case .brainPersonaSectionLabel:
            return e("身份与业务范围：", "Identity & Business Scope:", "Identité et portée métier :", "Identità e ambito aziendale:", "Tożsamość i zakres biznesowy:")
        case .brainPhaseEchoObserved:
            return e("己方回显", "Self echo observed", "Écho propre observé", "Rilevato eco proprio", "Zaobserwowano własne echo")
        case .brainPhaseModelGenerated:
            return e("模型已生成", "Model generated", "Modèle généré", "Modello generato", "Model wygenerowany")
        case .brainPhaseSelfEcho:
            return e("己方回显", "Self echo", "Écho local", "Eco locale", "Własne echo")
        case .brainPhaseSendTriggered:
            return e("发送已触发", "Send triggered", "Envoi déclenché", "Invio attivato", "Wysyłanie wyzwolone")
        case .brainPleaseSelectRolePrompt:
            return e("请选择角色，或点击「新增角色」", "Please select a role, or click 'New Role'", "Veuillez sélectionner un rôle, ou cliquez sur « Nouveau rôle »", "Seleziona un ruolo o fai clic su 'Nuovo ruolo'", "Wybierz rolę lub kliknij „Nowa rola”")
        case .brainPreviewAssembling:
            return e("正在执行组装预览 (assemble_only)...", "Executing assembly preview (assemble_only)...", "Exécution de l'aperçu d'assemblage (assemble_only)...", "Esecuzione anteprima di assemblaggio (assemble_only)...", "Wykonywanie podglądu złożenia (assemble_only)...")
        case .brainPreviewFailed:
            return e("组装预览失败", "Assembly preview failed", "Échec de l'aperçu d'assemblage", "Anteprima assemblaggio fallita", "Podgląd złożenia nieudany")
        case .brainPreviewGroupKind:
            return e("群聊 (group)", "Group (group)", "Groupe (group)", "Gruppo (group)", "Grupa (group)")
        case .brainPreviewInputPrompt:
            return e("请输入模拟消息正文", "Please enter simulated message content", "Veuillez saisir le contenu du message simulé", "Inserisci il contenuto del messaggio simulato", "Wprowadź treść symulowanej wiadomości")
        case .brainPreviewMockMessageLabel:
            return e("模拟客户消息:", "Mock customer message:", "Message client simulé :", "Messaggio cliente simulato:", "Symulowana wiadomość klienta:")
        case .brainPreviewPrivateKind:
            return e("私聊 (private)", "Private (private)", "Privé (private)", "Privato (private)", "Prywatny (private)")
        case .brainPreviewResultTitle:
            return e("试一试决策快照与预览:", "Try It snapshot & preview:", "Instantané de test et aperçu :", "Istantanea di prova e anteprima:", "Migawka testu i podgląd:")
        case .brainProvenanceComplete:
            return e("包含完整轮次追溯", "Complete turn provenance included", "Traçabilité complète des tours incluse", "Tracciabilità completa dei turni inclusa", "Zawiera pełną historię pochodzenia tury")
        case .brainProvenanceFull:
            return e("包含完整轮次追溯", "Full turn provenance included", "Traçabilité complète du tour incluse", "Tracciabilità completa del turno inclusa", "Pełna identyfikowalność tury dołączona")
        case .brainProvenanceLegacyNotRecorded:
            return e("旧版未记录", "Not recorded in legacy version", "Non enregistré dans l'ancienne version", "Non registrato nella versione precedente", "Nie zarejestrowano w starszej wersji")
        case .brainProvenanceLegacyUnrecorded:
            return e("旧版未记录", "Unrecorded in legacy version", "Non enregistré dans l'ancienne version", "Non registrato nella versione precedente", "Niezarejestrowane w starszej wersji")
        case .brainReadDefaultTemplateFailedFmt:
            return e("读取默认角色模板失败：%@", "Failed to read default role template: %@", "Échec de lecture du modèle de rôle par défaut : %@", "Lettura modello di ruolo predefinito non riuscita: %@", "Nie udało się odczytać szablonu roli domyślnej: %@")
        case .brainReferencedKnowledgeListTitle:
            return e("当前角色引用的知识列表:", "Knowledge referenced by this role:", "Connaissances référencées par ce rôle :", "Conoscenze collegate a questo ruolo:", "Wiedza powiązana z tą rolą:")
        case .brainReplyModeFmt:
            return e("回复方式：%@", "Reply mode: %@", "Mode de réponse : %@", "Modalità di risposta: %@", "Tryb odpowiedzi: %@")
        case .brainReplyModeLoading:
            return e("回复方式：待加载", "Reply mode: loading…", "Mode de réponse : chargement…", "Modalità di risposta: caricamento…", "Tryb odpowiedzi: wczytywanie…")
        case .brainReplyModePending:
            return e("回复方式：待加载", "Reply mode: Loading...", "Mode de réponse : Chargement...", "Modalità di risposta: In caricamento...", "Tryb odpowiedzi: Ładowanie...")
        case .brainReplyModeUnset:
            return e("回复方式：未设置", "Reply mode: Not set", "Mode de réponse : Non défini", "Modalità di risposta: Non impostata", "Tryb odpowiedzi: Nie ustawiono")
        case .brainRestoreFailedFmt:
            return e("回退失败: %@", "Restore failed: %@", "Échec de la restauration : %@", "Ripristino fallito: %@", "Przywracanie nieudane: %@")
        case .brainRestoredToGenerationFmt:
            return e("已回退至代次 %1$llu%2$@", "Restored to generation %1$llu%2$@", "Restauré à la génération %1$llu%2$@", "Ripristinato alla generazione %1$llu%2$@", "Przywrócono do generacji %1$llu%2$@")
        case .brainRestoringVersion:
            return e("正在执行版本回退...", "Restoring version…", "Restauration de la version…", "Ripristino della versione in corso…", "Przywracanie wersji…")
        case .brainRevisionStatusFmt:
            return e("角色: %1$@ · 草稿: %2$@ · 运行: %3$@", "Role: %1$@ · Draft: %2$@ · Running: %3$@", "Rôle : %1$@ · Brouillon : %2$@ · Actif : %3$@", "Ruolo: %1$@ · Bozza: %2$@ · In esecuzione: %3$@", "Rola: %1$@ · Szkic: %2$@ · Aktywna: %3$@")
        case .brainRevisionStatusLineFmt:
            return e("角色: %1$@ · 草稿: %2$@ · 运行: %3$@", "Role: %1$@ · Draft: %2$@ · Active: %3$@", "Rôle : %1$@ · Brouillon : %2$@ · Actif : %3$@", "Ruolo: %1$@ · Bozza: %2$@ · Attivo: %3$@", "Rola: %1$@ · Szkic: %2$@ · Aktywny: %3$@")
        case .brainRevisionStatusPlaceholder:
            return e("草稿: - · 运行: -", "Draft: - · Active: -", "Brouillon : - · Actif : -", "Bozza: - · Attivo: -", "Szkic: - · Aktywny: -")
        case .brainRoleActivatedFmt:
            return e("角色「%1$@」已启用（%2$@），适用范围已核对。", "Role '%1$@' activated (%2$@); scope verified.", "Rôle « %1$@ » activé (%2$@) ; périmètre vérifié.", "Ruolo '%1$@' attivato (%2$@); ambito verificato.", "Rola „%1$@” aktywowana (%2$@); zakres zweryfikowany.")
        case .brainRoleActivatedSuccessFmt:
            return e("角色「%1$@」已启用（%2$@），适用范围已核对。", "Role \"%1$@\" activated (%2$@); scope verified.", "Rôle « %1$@ » activé (%2$@) ; portée vérifiée.", "Ruolo \"%1$@\" attivato (%2$@); ambito verificato.", "Rola „%1$@” aktywowana (%2$@); zakres zweryfikowany.")
        case .brainRoleCannotDeleteBound:
            return e("无法删除：该角色仍被会话配置引用，请先解除默认或专属角色绑定。", "Cannot delete: role is referenced in conversation settings; unbind it first.", "Suppression impossible : le rôle est référencé dans les conversations ; détachez-le d'abord.", "Impossibile eliminare: il ruolo è associato a conversazioni; scollega prima l'associazione.", "Nie można usunąć: rola jest powiązana z rozmowami; najpierw odepnij powiązanie.")
        case .brainRoleColTitle:
            return e("角色", "Role", "Rôle", "Ruolo", "Rola")
        case .brainRoleCopiedNotice:
            return e("已复制角色，编辑后点击「保存草稿」", "Role copied; edit and click 'Save Draft'", "Rôle copié ; modifiez et cliquez sur « Enregistrer le brouillon »", "Ruolo copiato; modifica e fai clic su 'Salva bozza'", "Skopiowano rolę; edytuj i kliknij „Zapisz szkic”")
        case .brainRoleCopyNameFmt:
            return e("%@ (副本)", "%@ (Copy)", "%@ (Copie)", "%@ (Copia)", "%@ (Kopia)")
        case .brainRoleDeletedSuccess:
            return e("角色已删除", "Role deleted", "Rôle supprimé", "Ruolo eliminato", "Rola została usunięta")
        case .brainRoleDraftSavedPrompt:
            return e("草稿已保存。请在「适用会话」设置回复范围，再启用配置。", "Draft saved. Please configure reply scope under 'Applicable Sessions', then activate.", "Brouillon enregistré. Définissez la portée sous « Sessions concernées » avant d'activer.", "Bozza salvata. Configura l'ambito in 'Sessioni applicabili', poi attiva.", "Szkic zapisany. Skonfiguruj zakres w „Odpowiednie sesje”, a następnie włącz konfigurację.")
        case .brainRoleListTitle:
            return e("角色列表", "Role List", "Liste des rôles", "Elenco ruoli", "Lista ról")
        case .brainRoleNameFieldLabel:
            return e("角色名称:", "Role name:", "Nom du rôle :", "Nome ruolo:", "Nazwa roli:")
        case .brainRoleNotEnabled:
            return e("未启用", "Not enabled", "Non activé", "Non abilitato", "Nie włączono")
        case .brainRoleStateActiveFmt:
            return e("已启用: %@", "Active: %@", "Actif : %@", "Attivo: %@", "Aktywne: %@")
        case .brainRoleStateDefaultPrefix:
            return e("默认 · ", "Default · ", "Par défaut · ", "Predefinito · ", "Domyślny · ")
        case .brainRoleStateNewDraftRunningFmt:
            return e("有新草稿 · 运行: %@", "New draft · Running: %@", "Nouveau brouillon · Actif : %@", "Nuova bozza · In esecuzione: %@", "Nowy szkic · Działa: %@")
        case .brainRoleStateSavedNotActive:
            return e("已保存 · 未启用", "Saved · Inactive", "Enregistré · Inactif", "Salvato · Non attivo", "Zapisano · Nieaktywne")
        case .brainRoleStateUnsavedDraft:
            return e("未保存草稿", "Unsaved draft", "Brouillon non enregistré", "Bozza non salvata", "Niezapisany szkic")
        case .brainRoleTableTooltip:
            return e("双击角色名编辑", "Double click role name to edit", "Double-cliquez sur le rôle pour modifier", "Doppio clic sul nome per modificare", "Kliknij dwukrotnie nazwę roli, aby edytować")
        case .brainRoleUnsavedModifiedNotice:
            return e("角色有未保存的修改 · ⌘S 保存草稿", "Role has unsaved changes · ⌘S to save draft", "Modifications non enregistrées · ⌘S pour enregistrer", "Modifiche non salvate · ⌘S per salvare", "Niezapisane zmiany w roli · ⌘S aby zapisać szkic")
        case .brainRollbackAlreadyCurrent:
            return e("选定代次已是当前代次，无需回滚", "Selected generation is already current; rollback not needed", "La génération sélectionnée est déjà la génération actuelle ; rétablissement inutile", "La generazione selezionata è già quella corrente; ripristino non necessario", "Wybrana generacja jest już bieżącą; wycofywanie nie jest potrzebne")
        case .brainRollbackBoxTitle:
            return e("回滚到历史版本 (二次确认受控)", "Restore historical version (controlled confirmation)", "Restaurer une version historique (contrôlé)", "Ripristina versione storica (conferma controllata)", "Przywróć wersję historyczną (kontrolowane potwierdzenie)")
        case .brainRollbackCancelledCheckbox:
            return e("回滚已取消：请勾选确认方可执行回滚。", "Rollback cancelled: check the box to confirm rollback.", "Retour annulé : cochez la case pour confirmer.", "Ripristino annullato: seleziona la casella per confermare.", "Przywracanie anulowane: zaznacz pole wyboru, aby potwierdzić.")
        case .brainRollbackCancelledNotChecked:
            return e("回滚已取消：请勾选确认方可执行回滚。", "Rollback cancelled: Please check the confirmation box to proceed.", "Rétablissement annulé : Veuillez cocher la case pour confirmer.", "Ripristino annullato: Spunta la casella di conferma per procedere.", "Wycofanie anulowane: Zaznacz pole potwierdzenia, aby kontynuować.")
        case .brainRollbackDialogBodyFmt:
            return e("回滚将原子切换至选定历史代次的配置，未保存的草稿将丢失。\n%1$@\n• 运行配置: 默认角色=%2$@, 版本=%3$@, 覆盖规则=%4$d条\n• 草稿状态: %5$@", "Restoring atomically switches to the selected historical configuration; unsaved drafts will be lost.\n%1$@\n• Running config: default role=%2$@, version=%3$@, overrides=%4$d\n• Draft state: %5$@", "La restauration bascule vers la configuration historique sélectionnée ; les brouillons non enregistrés seront perdus.\n%1$@\n• Config active : rôle par défaut=%2$@, version=%3$@, dérogations=%4$d\n• État du brouillon : %5$@", "Il ripristino passa alla configurazione storica selezionata; le bozze non salvate andranno perse.\n%1$@\n• Config attiva: ruolo predefinito=%2$@, versione=%3$@, eccezioni=%4$d\n• Stato bozza: %5$@", "Przywrócenie przełącza na wybraną konfigurację historyczną; niezapisane szkice zostaną utracone.\n%1$@\n• Aktywna konfiguracja: rola domyślna=%2$@, wersja=%3$@, wyjątki=%4$d\n• Stan szkicu: %5$@")
        case .brainRollbackDialogDraftStatusFmt:
            return e("\n• 草稿状态: %@", "\n• Draft status: %@", "\n• État du brouillon : %@", "\n• Stato bozza: %@", "\n• Status szkicu: %@")
        case .brainRollbackDialogIntro:
            return e("回滚将原子切换至选定历史代次的配置，未保存的草稿将丢失。\n", "Rollback will atomically switch to selected historical generation; unsaved drafts will be lost.\n", "Le rétablissement basculera de manière atomique vers la génération choisie ; les brouillons seront perdus.\n", "Il ripristino passerà in modo atomico alla generazione selezionata; le bozze andranno perse.\n", "Wycofanie atomowo przełączy na wybraną generację historyczną; niezapisane szkice zostaną utracone.\n")
        case .brainRollbackDialogRunConfigFmt:
            return e("\n• 运行配置: 默认角色=%1$@, 版本=%2$@, 覆盖规则=%3$@条", "\n• Active config: Default role=%1$@, Revision=%2$@, Overrides=%3$@", "\n• Config active : Rôle par défaut=%1$@, Révision=%2$@, Dérogations=%3$@", "\n• Config attiva: Ruolo predefinito=%1$@, Revisione=%2$@, Eccezioni=%3$@", "\n• Aktywna konfiguracja: Rola domyślna=%1$@, Wersja=%2$@, Wyjątki=%3$@")
        case .brainRollbackDialogTitleFmt:
            return e("回滚客服角色配置到第 %llu 代？", "Restore persona configuration to generation %llu?", "Restaurer la configuration à la génération %llu ?", "Ripristinare la configurazione alla generazione %llu?", "Przywrócić konfigurację do generacji %llu?")
        case .brainRollbackDraftIncomplete:
            return e("此版本未保存完整草稿，仅显示已保存的内容 ([缺失])", "Incomplete draft; only saved content is shown ([Missing])", "Brouillon incomplet ; seul le contenu enregistré est affiché ([Manquant])", "Bozza incompleta; si mostra solo il contenuto salvato ([Mancante])", "Niepełny szkic; pokazano tylko zapisaną treść ([Brak])")
        case .brainRollbackDraftSavedFull:
            return e("已保存完整草稿", "Complete draft saved", "Brouillon complet enregistré", "Bozza completa salvata", "Zapisano pełny szkic")
        case .brainRollbackFailedFmt:
            return e("回退失败: %@", "Rollback failed: %@", "Échec du rétablissement : %@", "Ripristino non riuscito: %@", "Wycofanie nie powiodło się: %@")
        case .brainRollbackLegacySpecialNotice:
            return e("\n【特别说明】此版本未保存完整草稿，仅显示已保存的内容。回滚时仅恢复已存运行内容，草稿配置将保持缺失（[缺失]），不编造或补全内容。\n", "\n[Notice] Incomplete draft; only saved content is shown. Restoring recovers saved active content only; missing fields remain [Missing].\n", "\n[Remarque] Brouillon incomplet ; seul le contenu enregistré est affiché. Restaure uniquement le contenu actif ; les champs manquants restent [Manquant].\n", "\n[Avviso] Bozza incompleta; viene mostrato solo il contenuto salvato. Ripristina solo il contenuto attivo; i campi mancanti restano [Mancante].\n", "\n[Uwaga] Niepełny szkic; pokazano tylko zapisaną treść. Przywracanie przywraca tylko zapisaną aktywną treść; brakujące pola pozostają oznaczone jako [Brak].\n")
        case .brainRollbackSelectGenPrompt:
            return e("请选择要回滚的目标代次", "Please select target generation to rollback to", "Veuillez sélectionner la génération cible pour le rétablissement", "Seleziona la generazione di destinazione per il ripristino", "Wybierz docelową generację do wycofania")
        case .brainRollbackSuccessFmt:
            return e("已回退至代次 %1$@%2$@", "Rolled back to generation %1$@%2$@", "Rétablissement effectué vers la génération %1$@%2$@", "Ripristinato alla generazione %1$@%2$@", "Wycofano do generacji %1$@%2$@")
        case .brainRollbackTargetGenLabel:
            return e("目标代次:", "Target generation:", "Génération cible :", "Generazione di destinazione:", "Docelowa generacja:")
        case .brainRollbackVersion:
            return e("回退版本", "Restore version", "Restaurer la version", "Ripristina versione", "Przywróć wersję")
        case .brainRollbackingGenFmt:
            return e("正在执行版本回退到第 %llu 代...", "Restoring version to generation %llu…", "Restauration vers la génération %llu…", "Ripristino alla generazione %llu in corso…", "Przywracanie do generacji %llu…")
        case .brainRollingBack:
            return e("正在执行版本回退...", "Executing rollback...", "Rétablissement de la version en cours...", "Esecuzione ripristino in corso...", "Wykonywanie wycofywania wersji...")
        case .brainRollingBackToGenFmt:
            return e("正在执行版本回退到第 %@ 代...", "Rolling back to generation %@...", "Rétablissement vers la génération %@ en cours...", "Ripristino alla generazione %@ in corso...", "Wycofywanie do generacji %@...")
        case .brainRulesFieldHelper:
            return e("用逗号分隔业务规则", "Separate business rules with commas", "Séparez les règles par des virgules", "Separa le regole con virgole", "Rozdziel reguły przecinkami")
        case .brainRulesFieldLabel:
            return e("业务规则:", "Business Rules:", "Règles métier :", "Regole aziendali:", "Reguły biznesowe:")
        case .brainRulesPlaceholder:
            return e("用逗号分隔业务规则", "Separate business rules with commas", "Séparez les règles par des virgules", "Separa le regole con virgole", "Oddziel reguły przecinkami")
        case .brainRunningAssemblePreview:
            return e("正在执行组装预览 (assemble_only)...", "Running assemble preview (assemble_only)…", "Exécution de l'aperçu d'assemblage (assemble_only)…", "Esecuzione anteprima assemblaggio (assemble_only)…", "Wykonywanie podglądu złożenia (assemble_only)…")
        case .brainSaveBindingsDraft:
            return e("保存绑定草稿", "Save bindings draft", "Enregistrer le brouillon de liaisons", "Salva bozza associazioni", "Zapisz szkic przypisań")
        case .brainSaveCurrentBeforeDefault:
            return e("请先保存当前角色，再添加默认角色。", "Please save current role before adding default role.", "Veuillez enregistrer le rôle actuel avant d'ajouter le rôle par défaut.", "Salva il ruolo corrente prima di aggiungere il ruolo predefinito.", "Zapisz bieżącą rolę przed dodaniem roli domyślnej.")
        case .brainSaveCurrentBeforeNew:
            return e("请先保存当前角色，再新建角色。", "Please save current role before creating a new role.", "Veuillez enregistrer le rôle actuel avant d'en créer un nouveau.", "Salva il ruolo corrente prima di crearne uno nuovo.", "Zapisz bieżącą rolę przed utworzeniem nowej roli.")
        case .brainSaveDraft:
            return e("保存草稿", "Save draft", "Enregistrer le brouillon", "Salva bozza", "Zapisz szkic")
        case .brainSaveDraftBeforeSetDefault:
            return e("请先保存角色草稿，再设为默认角色。", "Save role draft before setting as default role.", "Enregistrez le brouillon avant de le définir par défaut.", "Salva la bozza del ruolo prima di impostarlo come predefinito.", "Zapisz szkic roli przed ustawieniem jej jako domyślnej.")
        case .brainSaveKnowledgeFailed:
            return e("保存知识失败", "Failed to save knowledge", "Échec de l'enregistrement des connaissances", "Salvataggio conoscenza fallito", "Zapisywanie wiedzy nieudane")
        case .brainSaveKnowledgeFailedFmt:
            return e("保存知识失败: %@", "Failed to save knowledge: %@", "Échec de l'enregistrement des connaissances : %@", "Salvataggio conoscenza non riuscito: %@", "Nie udało się zapisać wiedzy: %@")
        case .brainSaveKnowledgeInvalidResp:
            return e("保存知识返回无效响应", "Invalid response when saving knowledge", "Réponse invalide lors de l'enregistrement des connaissances", "Risposta non valida durante il salvataggio della conoscenza", "Nieprawidłowa odpowiedź podczas zapisywania wiedzy")
        case .brainSavingBindingsDraft:
            return e("正在保存绑定草稿至核心...", "Saving bindings draft to core...", "Enregistrement du brouillon de liaisons dans le noyau...", "Salvataggio bozza associazioni nel core...", "Zapisywanie szkicu powiązań w rdzeniu...")
        case .brainSavingDraft:
            return e("正在保存草稿...", "Saving draft…", "Enregistrement du brouillon…", "Salvataggio bozza in corso…", "Zapisywanie szkicu…")
        case .brainSavingKnowledge:
            return e("正在通过核心接口保存知识...", "Saving knowledge via core interface...", "Enregistrement des connaissances via l'interface du noyau...", "Salvataggio della conoscenza tramite interfaccia core...", "Zapisywanie wiedzy przez interfejs rdzenia...")
        case .brainSavingKnowledgeToCore:
            return e("正在通过核心接口保存知识...", "Saving knowledge via core interface…", "Enregistrement des connaissances via l'interface du cœur…", "Salvataggio conoscenze tramite interfaccia principale…", "Zapisywanie wiedzy przez interfejs rdzenia…")
        case .brainSavingRoleDraft:
            return e("正在保存草稿...", "Saving draft...", "Enregistrement du brouillon...", "Salvataggio della bozza...", "Zapisywanie szkicu...")
        case .brainSelectOrAddRolePrompt:
            return e("请选择角色，或点击「新增角色」", "Select a role or click 'New role'", "Sélectionnez un rôle ou cliquez sur « Nouveau rôle »", "Seleziona un ruolo o clicca su 'Nuovo ruolo'", "Wybierz rolę lub kliknij „Nowa rola”")
        case .brainSelectTargetGenPrompt:
            return e("请选择要回滚的目标代次", "Select a target generation to restore", "Sélectionnez une génération cible à restaurer", "Seleziona una generazione di destinazione da ripristinare", "Wybierz generację docelową do przywrócenia")
        case .brainSelectedDefaultRoleFmt:
            return e("已选择「%@」为默认角色。请保存绑定草稿，再启用配置。", "Selected \"%@\" as default role. Please save bindings draft, then activate configuration.", "« %@ » sélectionné comme rôle par défaut. Enregistrez le brouillon puis activez.", "Selezionato \"%@\" come ruolo predefinito. Salva la bozza delle associazioni e attiva.", "Wybrano „%@” jako rolę domyślną. Zapisz szkic powiązań i włącz konfigurację.")
        case .brainSimulatedUserMsgLabel:
            return e("模拟客户消息:", "Simulated User Message:", "Message simulé de l'utilisateur :", "Messaggio utente simulato:", "Symulowana wiadomość klienta:")
        case .brainStateDefaultPrefix:
            return e("默认 · ", "Default · ", "Par défaut · ", "Predefinito · ", "Domyślny · ")
        case .brainStateEnabledFmt:
            return e("已启用: %@", "Active: %@", "Actif : %@", "Attivo: %@", "Aktywny: %@")
        case .brainStateNewDraftRunningFmt:
            return e("有新草稿 · 运行: %@", "New draft · Active: %@", "Nouveau brouillon · Actif : %@", "Nuova bozza · Attivo: %@", "Nowy szkic · Aktywny: %@")
        case .brainStateSavedNotEnabled:
            return e("已保存 · 未启用", "Saved · Not enabled", "Enregistré · Non activé", "Salvato · Non abilitato", "Zapisano · Nie włączono")
        case .brainStateUnsavedDraft:
            return e("未保存草稿", "Unsaved draft", "Brouillon non enregistré", "Bozza non salvata", "Niezapisany szkic")
        case .brainStatusSaved:
            return e("已保存", "Saved", "Enregistré", "Salvato", "Zapisano")
        case .brainStatusUnknown:
            return e("状态未知", "Status unknown", "État inconnu", "Stato sconosciuto", "Status nieznany")
        case .brainSystemConstraintsNote:
            return e("不可虚构事实、语音转写以本轮转写为准、未识别项点名并依提示回复。本约束由代码自动拼接在角色人设之后，不可在界面删改。", "No hallucinations; voice transcriptions take precedence; unrecognised items addressed per prompts. Automatically appended to persona; cannot be edited.", "Pas d'inventions ; transcriptions prioritaires ; éléments non reconnus traités selon les consignes. Ajouté automatiquement.", "Nessuna allucinazione; trascrizioni vocali prioritarie; elementi non riconosciuti gestiti su indicazione. Aggiunto in automatico.", "Zakaz zmyślania faktów; transkrypcje głosowe mają pierwszeństwo; nierozpoznane pozycje zgłaszane wg instrukcji. Dołączane automatycznie.")
        case .brainSystemConstraintsTitle:
            return e("系统能力约束 (只读)", "System capability constraints (read-only)", "Contraintes système (lecture seule)", "Vincoli di sistema (sola lettura)", "Ograniczenia systemowe (tylko odczyt)")
        case .brainTableHistoryBoxTitle:
            return e("版本与代次历史", "Revisions and Generations History", "Historique des révisions et générations", "Cronologia revisioni e generazioni", "Historia wersji i generacji")
        case .brainTargetGenAlreadyCurrent:
            return e("选定代次已是当前代次，无需回滚", "Selected generation is already current; no rollback needed", "La génération sélectionnée est déjà active ; aucun retour requis", "La generazione selezionata è già corrente; nessun ripristino necessario", "Wybrana generacja jest już bieżąca; przywracanie niepotrzebne")
        case .brainTargetGenLabel:
            return e("目标代次:", "Target Gen:", "Génération cible :", "Generazione di destinazione:", "Docelowa generacja:")
        case .brainTemplateGreeting:
            return e("欢迎语 (greeting):", "Greeting (greeting):", "Message de bienvenue (greeting) :", "Messaggio di benvenuto (greeting):", "Powitanie (greeting):")
        case .brainTemplateGreetingLabel:
            return e("欢迎语 (greeting):", "Greeting (greeting):", "Message d'accueil (greeting) :", "Messaggio di benvenuto (greeting):", "Powitanie (greeting):")
        case .brainTemplateInsufficient:
            return e("信息不足 (insufficient_info):", "Insufficient info (insufficient_info):", "Information insuffisante (insufficient_info) :", "Informazioni insufficienti (insufficient_info):", "Niewystarczające informacje (insufficient_info):")
        case .brainTemplateInsufficientLabel:
            return e("信息不足 (insufficient_info):", "Insufficient info (insufficient_info):", "Informations insuffisantes (insufficient_info) :", "Informazioni insufficienti (insufficient_info):", "Niewystarczające informacje (insufficient_info):")
        case .brainTemplateTooLarge:
            return e("文件超限 (too_large):", "File too large (too_large):", "Fichier trop volumineux (too_large) :", "File troppo grande (too_large):", "Plik za duży (too_large):")
        case .brainTemplateTooLargeLabel:
            return e("文件超限 (too_large):", "File too large (too_large):", "Fichier trop volumineux (too_large) :", "File troppo grande (too_large):", "Plik zbyt duży (too_large):")
        case .brainTemplateUnsupported:
            return e("格式不支持 (unsupported_type):", "Unsupported type (unsupported_type):", "Format non pris en charge (unsupported_type) :", "Formato non supportato (unsupported_type):", "Nieobsługiwany format (unsupported_type):")
        case .brainTemplateUnsupportedLabel:
            return e("格式不支持 (unsupported_type):", "Unsupported type (unsupported_type):", "Type non pris en charge (unsupported_type) :", "Tipo non supportato (unsupported_type):", "Nieobsługiwany typ (unsupported_type):")
        case .brainTemplatesIntro:
            return e("通用话术模板 (留空表示无自定义文案，进入待处理或人工接管):", "General reply templates (leave blank to use defaults or human takeover):", "Modèles de réponse généraux (laisser vide pour les défauts ou relais humain) :", "Modelli di risposta generali (lascia vuoto per impostazioni predefinite o operatore umano):", "Ogólne szablony odpowiedzi (pozostaw puste dla domyślnych lub przejęcia przez człowieka):")
        case .brainTemplatesSectionDesc:
            return e("通用话术模板 (留空表示无自定义文案，进入待处理或人工接管):", "Fallback reply templates (leave blank to fall back to pending/handoff):", "Modèles de réponse de secours (laisser vide pour passer en attente/transfert) :", "Modelli di risposta di ripiego (lascia vuoto per passare in attesa/operatore):", "Szablony odpowiedzi awaryjnych (zostaw puste, aby przekazać do obsługi):")
        case .brainToneFieldLabel:
            return e("语气风格:", "Tone & Style:", "Ton et style :", "Tono e stile:", "Ton i styl:")
        case .brainTryItBudgetFmt:
            return e("字节预算: 原始=%1$@, 纳入=%2$@, 是否截断=%3$@\n", "Byte budget: Original=%1$@, Included=%2$@, Truncated=%3$@\n", "Budget d'octets : Original=%1$@, Inclus=%2$@, Tronqué=%3$@\n", "Budget byte: Originale=%1$@, Incluso=%2$@, Troncato=%3$@\n", "Budżet bajtów: Oryginał=%1$@, Dołączono=%2$@, Obcięto=%3$@\n")
        case .brainTryItConstraintsFmt:
            return e("系统约束:\n%@\n", "System constraints:\n%@\n", "Contraintes système :\n%@\n", "Vincoli di sistema:\n%@\n", "Ograniczenia systemowe:\n%@\n")
        case .brainTryItCostFreeNotice:
            return e("未调用模型，仅确定性组装预览，不产生费用。\n", "Model not invoked; deterministic assembly preview only, free of charge.\n", "Modèle non invoqué ; aperçu d'assemblage déterministe uniquement, sans frais.\n", "Modello non invocato; solo anteprima di assemblaggio deterministica, gratuita.\n", "Model nie został wywołany; tylko deterministyczny podgląd złożenia, bez kosztów.\n")
        case .brainTryItCostIncurredFmt:
            return e("实际费用产生: %@\n", "Actual cost incurred: %@\n", "Frais réels engagés : %@\n", "Costo effettivo sostenuto: %@\n", "Faktycznie poniesiony koszt: %@\n")
        case .brainTryItFailedFmt:
            return e("试一试组装预览失败: %@", "Try It assemble preview failed: %@", "Échec de l'aperçu d'assemblage du test : %@", "Anteprima assemblaggio di prova fallita: %@", "Podgląd złożenia testu nieudany: %@")
        case .brainTryItHeader:
            return e("【试一试结果快照】\n未调用模型，仅确定性组装预览，不产生费用。\n", "[Try It Snapshot]\nDeterministic assembly preview only; no model call, no charges.\n", "[Instantané de test]\nAperçu déterministe uniquement ; aucun appel au modèle, aucun coût.\n", "[Istantanea di prova]\nSolo anteprima deterministica; nessuna chiamata al modello, nessun costo.\n", "[Migawka testu]\nTylko deterministyczny podgląd złożenia; brak wywołania modelu i opłat.\n")
        case .brainTryItResultTitle:
            return e("试一试决策快照与预览:", "Try It Decision Snapshot & Preview:", "Instantané de décision et aperçu du test :", "Istantanea decisionale e anteprima di prova:", "Migawka decyzyjna i podgląd testu:")
        case .brainTryItSnapshotShaFmt:
            return e("快照摘要: %@\n", "Snapshot SHA: %@\n", "Empreinte de l'instantané : %@\n", "SHA istantanea: %@\n", "SHA migawki: %@\n")
        case .brainTryItStatusFmt:
            return e("状态: %@\n", "Status: %@\n", "État : %@\n", "Stato: %@\n", "Status: %@\n")
        case .brainTurnNotFound:
            return e("轮次不存在", "Turn not found", "Tour introuvable", "Turno non trovato", "Tura nie istnieje")
        case .brainUnsavedBeforeActivatePrompt:
            return e("尚有未保存的修改，请先保存角色草稿及会话绑定，再启用。", "Unsaved changes exist. Please save role draft and bindings before activating.", "Modifications non enregistrées. Enregistrez le rôle et les liaisons avant d'activer.", "Ci sono modifiche non salvate. Salva la bozza del ruolo e le associazioni prima di attivare.", "Istnieją niezapisane zmiany. Zapisz szkic roli i powiązania przed aktywacją.")
        case .brainUnsavedChangesBlockActivation:
            return e("尚有未保存的修改，请先保存角色草稿及会话绑定，再启用。", "Unsaved changes exist; save role draft and conversation bindings before activating.", "Modifications non enregistrées ; enregistrez le brouillon et les liaisons avant d'activer.", "Ci sono modifiche non salvate; salva la bozza e le associazioni prima di attivare.", "Istnieją niezapisane zmiany; zapisz szkic roli i przypisania przed aktywacją.")
        case .brainUnsavedChangesNotice:
            return e("角色有未保存的修改 · ⌘S 保存草稿", "Role has unsaved changes · ⌘S to save draft", "Modifications non enregistrées pour le rôle · ⌘S pour enregistrer", "Il ruolo ha modifiche non salvate · ⌘S per salvare", "Rola zawiera niezapisane zmiany · ⌘S, aby zapisać szkic")
        case .brainUnsavedPromptSaveFirst:
            return e("尚未保存：填写角色设定后按 ⌘S 保存，再设置适用会话", "Unsaved: Fill role settings and press ⌘S to save, then configure applicable sessions", "Non enregistré : Remplissez les paramètres et appuyez sur ⌘S pour enregistrer", "Non salvato: Compila i campi e premi ⌘S per salvare, poi configura le sessioni", "Niezapisane: Wypełnij ustawienia i naciśnij ⌘S, aby zapisać, a potem skonfiguruj sesje")
        case .brainUnsavedRoleDeleted:
            return e("未保存角色已删除", "Unsaved role discarded", "Rôle non enregistré supprimé", "Ruolo non salvato scartato", "Odrzucono niezapisaną rolę")
        case .brainUnsavedRoleNotice:
            return e("尚未保存：填写角色设定后按 ⌘S 保存，再设置适用会话", "Not saved yet: fill role settings and press ⌘S to save, then configure conversations", "Pas encore enregistré : remplissez le rôle et appuyez sur ⌘S, puis configurez les conversations", "Non ancora salvato: compila le impostazioni e premi ⌘S per salvare, poi configura le conversazioni", "Jeszcze niezapisane: uzupełnij rolę i naciśnij ⌘S, aby zapisać, a następnie skonfiguruj rozmowy")
        case .brainUpdateOverrideErrContact:
            return e("更新失败：请选择要更新的白名单联系人", "Update failed: Please select an allowlist contact to update", "Échec de la mise à jour : Veuillez sélectionner un contact à mettre à jour", "Aggiornamento non riuscito: Seleziona un contatto da aggiornare", "Aktualizacja nie powiodła się: Wybierz kontakt do aktualizacji")
        case .brainUpdateOverrideErrNotFound:
            return e("更新失败：未找到对应的覆盖规则，请先点击「添加覆盖」", "Update failed: Corresponding override rule not found; click 'Add Override' first", "Échec de la mise à jour : Dérogation introuvable ; cliquez d'abord sur « Ajouter une dérogation »", "Aggiornamento non riuscito: Eccezione non trovata; fai clic prima su 'Aggiungi eccezione'", "Aktualizacja nie powiodła się: Nie znaleziono reguły wyjątku; najpierw kliknij „Dodaj wyjątek”")
        case .brainUpdateOverrideErrRole:
            return e("更新失败：请选择有效的角色", "Update failed: Please select a valid role", "Échec de la mise à jour : Veuillez sélectionner un rôle valide", "Aggiornamento non riuscito: Seleziona un ruolo valido", "Aktualizacja nie powiodła się: Wybierz prawidłową rolę")
        case .brainUpdateOverrideSelectContact:
            return e("更新失败：请选择要更新的白名单联系人", "Update failed: select an allowlist contact to update", "Échec de la mise à jour : sélectionnez un contact autorisé", "Aggiornamento fallito: seleziona un contatto consentito", "Aktualizacja nieudana: wybierz kontakt z białej listy")
        case .brainUpdateOverrideSelectRole:
            return e("更新失败：请选择有效的角色", "Update failed: select a valid role", "Échec de la mise à jour : sélectionnez un rôle valide", "Aggiornamento fallito: seleziona un ruolo valido", "Aktualizacja nieudana: wybierz prawidłową rolę")
        case .brainUpdateSelectedOverride:
            return e("更新选定覆盖", "Update selected override", "Mettre à jour la dérogation", "Aggiorna eccezione selezionata", "Zaktualizuj zaznaczony wyjątek")
        case .brainVerDraftSuffix:
            return e(" (草稿)", " (draft)", " (brouillon)", " (bozza)", " (szkic)")
        case .brainVerRunningSuffix:
            return e(" (运行中)", " (running)", " (en cours)", " (in esecuzione)", " (działa)")
        case .brainVersionALabel:
            return e("版本 A:", "Version A:", "Version A :", "Versione A:", "Wersja A:")
        case .brainVersionBLabel:
            return e("版本 B:", "Version B:", "Version B :", "Versione B:", "Wersja B:")
        case .brainYes:
            return e("是", "Yes", "Oui", "Sì", "Tak")
        case .componentsBannerAlertMsg:
            return e("成功响应横幅右侧按钮动作。", "Successfully responded to banner button click.", "Action du bouton de bannière exécutée.", "Azione del pulsante banner eseguita con successo.", "Pomyślnie obsłużono kliknięcie przycisku banera.")
        case .componentsBannerAlertTitle:
            return e("横幅操作触发", "Banner action triggered", "Action de bannière déclenchée", "Azione banner attivata", "Działanie banera wywołane")
        case .contactsActionAddAutoReply:
            return e("加入自动回复", "Add to auto-reply", "Ajouter à la réponse auto", "Aggiungi ad auto-risposta", "Dodaj do auto-odpowiedzi")
        case .contactsActionIgnore:
            return e("忽略", "Ignore", "Ignorer", "Ignora", "Ignoruj")
        case .contactsActionImmutable:
            return e("不可修改", "Immutable", "Non modifiable", "Non modificabile", "Niezmienne")
        case .contactsActionSetIgnore:
            return e("设为忽略", "Ignore", "Ignorer", "Ignora", "Ignoruj")
        case .contactsActionUnignore:
            return e("取消忽略", "Unignore", "Ne plus ignorer", "Annulla ignora", "Przestań ignorować")
        case .contactsCategoryGroup:
            return e("群聊", "Group chat", "Discussion de groupe", "Chat di gruppo", "Czat grupowy")
        case .contactsCategoryIgnored:
            return e("已忽略", "Ignored", "Ignorés", "Ignorati", "Ignorowani")
        case .contactsCategoryOfficial:
            return e("公众号", "Official accounts", "Comptes officiels", "Account ufficiali", "Konta oficjalne")
        case .contactsCategoryPersonal:
            return e("联系人", "Contacts", "Contacts", "Contatti", "Kontakty")
        case .contactsCategoryWork:
            return e("企业微信", "WeCom", "WeCom", "WeCom", "WeCom")
        case .contactsChangeHandling:
            return e("切换处理方式", "Change handling", "Changer le traitement", "Cambia gestione", "Zmień obsługę")
        case .contactsColHandling:
            return e("处理方式", "Handling", "Traitement", "Gestione", "Obsługa")
        case .contactsColMsgs:
            return e("消息数", "Messages", "Messages", "Messaggi", "Wiadomości")
        case .contactsColName:
            return e("名字 / 备注", "Name / Remark", "Nom / Remarque", "Nome / Nota", "Nazwa / Uwaga")
        case .contactsColWechat:
            return e("微信号", "WeChat ID", "ID WeChat", "ID WeChat", "ID WeChat")
        case .contactsDetailAddedOn:
            return e("添加时间:", "Added on:", "Ajouté le :", "Data aggiunta:", "Data dodania:")
        case .contactsDetailBoxTitle:
            return e("联系人详情", "Contact details", "Détails du contact", "Dettagli contatto", "Szczegóły kontaktu")
        case .contactsDetailEnterprise:
            return e("企业/实名:", "Company / Real name:", "Entreprise / Nom réel :", "Azienda / Nome reale:", "Firma / Imię i nazwisko:")
        case .contactsDetailHandling:
            return e("处理方式:", "Handling:", "Traitement :", "Gestione:", "Obsługa:")
        case .contactsDetailRegion:
            return e("地区:", "Region:", "Région :", "Regione:", "Region:")
        case .contactsDetailRemark:
            return e("备注名:", "Remark:", "Remarque :", "Nota:", "Uwaga:")
        case .contactsDetailSource:
            return e("来源:", "Source:", "Source :", "Origine:", "Źródło:")
        case .contactsDetailTags:
            return e("标签:", "Tags:", "Étiquettes :", "Tag:", "Tagi:")
        case .contactsDetailWechat:
            return e("微信号:", "WeChat ID:", "ID WeChat :", "ID WeChat:", "ID WeChat:")
        case .contactsErrorOfficialNoAutoReply:
            return e("公众号、服务号不可加入自动回复", "Official accounts cannot be added to auto-reply", "Les comptes officiels ne peuvent pas être ajoutés à la réponse automatique", "Gli account ufficiali non possono essere aggiunti alla risposta automatica", "Oficjalne konta nie mogą zostać dodane do auto-odpowiedzi")
        case .contactsHandlingIgnored:
            return e("已忽略", "Ignored", "Ignoré", "Ignorato", "Ignorowany")
        case .contactsHandlingNeverReply:
            return e("永不回复", "Never reply", "Ne jamais répondre", "Non rispondere mai", "Nigdy nie odpowiadaj")
        case .contactsHandlingRecordOnly:
            return e("仅记录", "Record only", "Enregistrement seul", "Solo registrazione", "Tylko zapis")
        case .contactsInitSync:
            return e("初始化通讯录", "Initialize address book", "Initialiser le carnet", "Inizializza rubrica", "Zainicjuj kontakty")
        case .contactsInitSyncDialogConfirm:
            return e("开始初始化", "Start initialization", "Démarrer l'initialisation", "Inizia inizializzazione", "Rozpocznij inicjalizację")
        case .contactsInitSyncDialogMsg:
            return e("将暂停自动回复并全量同步微信通讯录，预计耗时 2-3 分钟。是否继续？", "This will pause auto-reply and perform a full sync of WeChat contacts (takes 2-3 minutes). Continue?", "Cela mettra en pause la réponse automatique et effectuera une synchronisation complète (2-3 minutes). Continuer ?", "Questo metterà in pausa l'auto-risposta e sincronizzerà completamente i contatti WeChat (2-3 minuti). Continuare?", "Wstrzyma to auto-odpowiedź i przeprowadzi pełną synchronizację kontaktów WeChat (zajmie 2-3 minuty). Kontynuować?")
        case .contactsInitSyncDialogTitle:
            return e("初始化通讯录", "Initialize address book", "Initialiser le carnet", "Inizializza rubrica", "Zainicjuj kontakty")
        case .contactsNoSelected:
            return e("未选择联系人", "No contact selected", "Aucun contact sélectionné", "Nessun contatto selezionato", "Nie wybrano kontaktu")
        case .contactsOffline:
            return e("控制命令不可用 (离线状态)", "Control command unavailable (offline)", "Commande de contrôle indisponible (hors ligne)", "Comando di controllo non disponibile (offline)", "Polecenie sterujące niedostępne (tryb offline)")
        case .contactsQuickSync:
            return e("立即同步", "Sync now", "Synchroniser maintenant", "Sincronizza ora", "Synchronizuj teraz")
        case .contactsReviewNoticeTitle:
            return e("待确认项", "Pending review", "À vérifier", "In attesa di verifica", "Do weryfikacji")
        case .contactsReviewPrompt:
            return e("存在 %1$d 条重名待确认联系人（如：%2$@），请核对身份！", "Found %1$d duplicate contacts to review (e.g. %2$@); please verify identity!", "%1$d doublons de contacts à vérifier (ex. %2$@) ; vérifiez leur identité !", "Trovati %1$d contatti con lo stesso nome da verificare (es. %2$@); controlla l'identità!", "Znaleziono %1$d zduplikowanych kontaktów do weryfikacji (np. %2$@); sprawdź tożsamość!")
        case .contactsSyncCompleted:
            return e("上次同步已完成", "Last sync completed", "Dernière synchronisation terminée", "Ultima sincronizzazione completata", "Ostatnia synchronizacja zakończona")
        case .contactsSyncIncomplete:
            return e("同步未完整", "Sync incomplete", "Synchronisation incomplète", "Sincronizzazione incompleta", "Synchronizacja niekompletna")
        case .contactsSyncLoading:
            return e("正在加载通讯录...", "Loading address book…", "Chargement du carnet d'adresses…", "Caricamento rubrica…", "Wczytywanie kontaktów…")
        case .contactsSyncNever:
            return e("尚未同步通讯录", "Address book not synced yet", "Carnet non synchronisé", "Rubrica non ancora synchronizzata", "Kontakty jeszcze niezsynchronizowane")
        case .contactsTabGroupCount:
            return e("群聊 (%d)", "Group chat (%d)", "Discussion de groupe (%d)", "Chat di gruppo (%d)", "Czat grupowy (%d)")
        case .contactsTabIgnoredCount:
            return e("已忽略 (%d)", "Ignored (%d)", "Ignorés (%d)", "Ignorati (%d)", "Ignorowani (%d)")
        case .contactsTabOfficialCount:
            return e("公众号 (%d)", "Official accounts (%d)", "Comptes officiels (%d)", "Account ufficiali (%d)", "Konta oficjalne (%d)")
        case .contactsTabPersonalCount:
            return e("联系人 (%d)", "Contacts (%d)", "Contacts (%d)", "Contatti (%d)", "Kontakty (%d)")
        case .contactsTabWorkCount:
            return e("企业微信 (%d)", "WeCom (%d)", "WeCom (%d)", "WeCom (%d)", "WeCom (%d)")
        case .contactsWechatUnread:
            return e("—（未读到）", "— (Unread)", "— (Non lu)", "— (Non letto)", "— (Nie odczytano)")
        case .contactsWechatWorkNone:
            return e("—（企业微信无此项）", "— (Not available in WeCom)", "— (Non disponible dans WeCom)", "— (Non disponibile in WeCom)", "— (Niedostępne w WeCom)")
        case .logsClear:
            return e("清屏", "Clear", "Effacer", "Cancella", "Wyczyść")
        case .logsLevelAll:
            return e("全部", "All", "Tout", "Tutti", "Wszystko")
        case .logsLevelErrorOnly:
            return e("只看错误", "Errors only", "Erreurs uniquement", "Solo errori", "Tylko błędy")
        case .logsLevelLabel:
            return e("级别:", "Level:", "Niveau :", "Livello:", "Poziom:")
        case .logsLevelWarnAndAbove:
            return e("警告及以上", "Warnings and above", "Avertissements et plus", "Avvisi e superiori", "Ostrzeżenia i wyższe")
        case .logsSearchPlaceholder:
            return e("搜索日志...", "Search logs…", "Rechercher dans les journaux…", "Cerca nei registri…", "Szukaj w dziennikach…")
        case .logsSourceLabel:
            return e("来源:", "Source:", "Source :", "Origine:", "Źródło:")
        case .mcpBoxConfigExample:
            return e("客户端接入配置示例", "Client configuration examples", "Exemples de configuration client", "Esempi di configurazione client", "Przykłady konfiguracji klienta")
        case .mcpControlOffline:
            return e("控制命令不可用 (离线状态)", "Control command unavailable (offline)", "Commande de contrôle indisponible (hors ligne)", "Comando di controllo non disponibile (offline)", "Polecenie sterujące niedostępne (tryb offline)")
        case .mcpCopiedOpenApiConfig:
            return e("已复制对外接口配置", "API configuration copied", "Configuration API copiée", "Configurazione API copiata", "Skopiowano konfigurację API")
        case .mcpCopiedSsh:
            return e("已复制运维 SSH 命令", "Ops SSH command copied", "Commande SSH copiée", "Comando SSH copiato", "Skopiowano polecenie SSH")
        case .mcpCopiedSshPlaceholder:
            return e("已复制运维 SSH 命令（未检测到非回环 IPv4，已使用 <本机地址> 占位，请自行替换）", "Ops SSH command copied (no non-loopback IPv4 detected; replaced with <local-address>)", "Commande SSH copiée (aucun IPv4 hors boucle détecté ; <adresse-locale> utilisée)", "Comando SSH copiato (nessun IPv4 non di loopback trovato; usato segnaposto)", "Skopiowano polecenie SSH (nie wykryto zewnętrznego IPv4; użyto adresu lokalnego)")
        case .mcpCopyOpenApiBtn:
            return e("复制对外接口配置", "Copy API configuration", "Copier la configuration de l'API", "Copia configurazione API", "Kopiuj konfigurację API")
        case .mcpCopyOpsSshBtn:
            return e("复制运维 SSH 配置", "Copy ops SSH configuration", "Copier la configuration SSH d'exploitation", "Copia configurazione SSH operativa", "Kopiuj konfigurację SSH operacyjną")
        case .mcpEndpointChatHistory:
            return e("聊天历史查询", "Chat history query", "Requête d'historique de clavardage", "Query cronologia chat", "Zapytanie o historię czatu")
        case .mcpEndpointExternalService:
            return e("外部服务接口", "External service interface", "Interface de service externe", "Interfaccia servizio esterno", "Interfejs usługi zewnętrznej")
        case .mcpEndpointOpenApi:
            return e("对外接口", "External API", "API externe", "API esterna", "Zewnętrzne API")
        case .mcpEndpointToolService:
            return e("MCP 工具服务", "MCP tool service", "Service d'outils MCP", "Servizio strumenti MCP", "Usługa narzędzi MCP")
        case .mcpErrorQueueInt:
            return e("最长排队必须为整数", "Maximum queue must be an integer", "La file d'attente doit être un entier", "La coda massima deve essere un intero", "Maksymalna kolejka musi być liczbą całkowitą")
        case .mcpErrorTimeoutInt:
            return e("等待回复必须为整数", "Response timeout must be an integer", "Le délai de réponse doit être un entier", "Il timeout di risposta deve essere un intero", "Limit czasu odpowiedzi musi być liczbą całkowitą")
        case .mcpFetchTokenFailed:
            return e("获取令牌失败", "Failed to get token", "Échec de l'obtention du jeton", "Recupero token fallito", "Pobieranie tokena nieudane")
        case .mcpFieldQueueMinutes:
            return e("最长排队 (queue_minutes):", "Maximum queue (queue_minutes):", "File d'attente max (queue_minutes) :", "Coda massima (queue_minutes):", "Maksymalna kolejka (queue_minutes):")
        case .mcpFieldQueueMinutesHelper:
            return e("1–1440 分钟 (默认 30)", "1–1440 minutes (default 30)", "1–1440 minutes (défaut 30)", "1–1440 minuti (predefinito 30)", "1–1440 minut (domyślnie 30)")
        case .mcpFieldTimeoutAction:
            return e("超时后动作 (timeout_action):", "Timeout action (timeout_action):", "Action à expiration (timeout_action) :", "Azione al timeout (timeout_action):", "Działanie po przekroczeniu czasu (timeout_action):")
        case .mcpFieldTimeoutActionHelper:
            return e("skip=跳过, handoff=转人工, builtin=改用内置", "skip=skip, handoff=hand off, builtin=fallback to built-in", "skip=ignorer, handoff=transférer, builtin=passer à l'intégré", "skip=salta, handoff=passa a operatore, builtin=passa a integrato", "skip=pomiń, handoff=przekaż, builtin=przełącz na wbudowany")
        case .mcpFieldTimeoutHelper:
            return e("1–3600 秒 (默认 120)", "1–3600 seconds (default 120)", "1–3600 secondes (défaut 120)", "1–3600 secondi (predefinito 120)", "1–3600 sekund (domyślnie 120)")
        case .mcpFieldTimeoutSeconds:
            return e("等待回复 (timeout_seconds):", "Response timeout (timeout_seconds):", "Délai de réponse (timeout_seconds) :", "Timeout risposta (timeout_seconds):", "Limit czasu odpowiedzi (timeout_seconds):")
        case .mcpGroupDesktop:
            return e("通用桌面", "General desktop", "Bureau universel", "Desktop generico", "Pulpit ogólny")
        case .mcpGroupOpenApi:
            return e("对外接口", "External API", "API externe", "API esterna", "Zewnętrzne API")
        case .mcpGroupOps:
            return e("运维", "Operations", "Exploitation", "Operazioni", "Operacje")
        case .mcpOpenApiCheckFailed:
            return e("执行 open-api-check 失败", "Failed to execute open-api-check", "Échec de l'exécution d'open-api-check", "Esecuzione open-api-check fallita", "Wykonanie open-api-check nieudane")
        case .mcpOpenApiConnectedCount:
            return e("连接正常 (可用工具: %d 个)", "Connection normal (%d tools available)", "Connexion normale (%d outils disponibles)", "Connessione normale (%d strumenti disponibili)", "Połączenie prawidłowe (dostępne narzędzia: %d)")
        case .mcpOpenApiDisabled:
            return e("开关状态: 已关闭 (只读，请到「设置」页修改 open_api.enabled)", "State: disabled (read-only; update open_api.enabled in Settings)", "État : désactivé (lecture seule ; modifiez open_api.enabled dans Réglages)", "Stato: disabilitato (sola lettura; modifica open_api.enabled in Impostazioni)", "Stan: wyłączone (tylko odczyt; zmień open_api.enabled w Ustawieniach)")
        case .mcpOpenApiDisabledNotice:
            return e("对外接口未开启 (open_api.enabled=false)", "API disabled (open_api.enabled=false)", "API désactivée (open_api.enabled=false)", "API disabilitata (open_api.enabled=false)", "API wyłączone (open_api.enabled=false)")
        case .mcpOpenApiEnabled:
            return e("开关状态: 已开启 (只读，请到「设置」页修改 open_api.enabled)", "State: enabled (read-only; update open_api.enabled in Settings)", "État : activé (lecture seule ; modifiez open_api.enabled dans Réglages)", "Stato: abilitato (sola lettura; modifica open_api.enabled in Impostazioni)", "Stan: włączone (tylko odczyt; zmień open_api.enabled w Ustawieniach)")
        case .mcpOpenApiUrl:
            return e("接口地址: %@", "API URL: %@", "Adresse API : %@", "URL API: %@", "Adres API: %@")
        case .mcpPlaceholderLocalAddress:
            return e("<本机地址>", "<local-address>", "<adresse-locale>", "<indirizzo-locale>", "<adres-lokalny>")
        case .mcpPlaceholderToken:
            return e("<令牌>", "<token>", "<jeton>", "<token>", "<token>")
        case .mcpPlaceholderUser:
            return e("<用户>", "<user>", "<utilisateur>", "<utente>", "<użytkownik>")
        case .mcpReadingToken:
            return e("正在读取令牌...", "Reading token…", "Lecture du jeton…", "Lettura del token…", "Odczytywanie tokena…")
        case .mcpRegenerate:
            return e("重新生成", "Regenerate", "Régénérer", "Rigenera", "Wygeneruj ponownie")
        case .mcpRegenerateTokenFailed:
            return e("重新生成令牌失败", "Failed to regenerate token", "Échec de régénération du jeton", "Rigenerazione token fallita", "Nie udało się wygenerować tokena ponownie")
        case .mcpRegeneratingToken:
            return e("正在重新生成令牌...", "Regenerating token…", "Régénération du jeton…", "Rigenerazione del token…", "Ponowne generowanie tokena…")
        case .mcpSaveFailed:
            return e("保存失败: %@", "Save failed: %@", "Échec de l'enregistrement : %@", "Salvataggio non riuscito: %@", "Zapis nieudany: %@")
        case .mcpSectionEndpoints:
            return e("连接信息与入口", "Connection endpoints", "Points de connexion", "Punti di connessione", "Punkty połączeń")
        case .mcpSectionOpenApi:
            return e("对外接口", "External API", "API externe", "API esterna", "Zewnętrzne API")
        case .mcpSectionTools:
            return e("可用 MCP 工具", "Available MCP tools", "Outils MCP disponibles", "Strumenti MCP disponibili", "Dostępne narzędzia MCP")
        case .mcpStatusClosed:
            return e("已关闭", "Closed", "Fermé", "Chiuso", "Zamknięte")
        case .mcpStatusNotStarted:
            return e("未启动/未连接", "Not started / unconnected", "Non démarré / non connecté", "Non avviato / non connesso", "Nieuruchomione / niepołączone")
        case .mcpStatusReady:
            return e("已就绪", "Ready", "Prêt", "Pronto", "Gotowy")
        case .mcpTestConnComplete:
            return e("连接测试完成", "Connection test completed", "Test de connexion terminé", "Test di connessione completato", "Test połączenia ukończony")
        case .mcpTestConnection:
            return e("测试连接", "Test connection", "Tester la connexion", "Prova connessione", "Testuj połączenie")
        case .mcpTestFailed:
            return e("测试失败: %@", "Test failed: %@", "Test échoué : %@", "Test fallito: %@", "Test nieudany: %@")
        case .mcpTestingConnection:
            return e("正在测试连接...", "Testing connection…", "Test de connexion en cours…", "Test connessione in corso…", "Testowanie połączenia…")
        case .mcpTestingOpenApi:
            return e("正在测试对外接口连接...", "Testing API connection…", "Test de connexion API en cours…", "Test connessione API in corso…", "Testowanie połączenia API…")
        case .mcpTokenCopied:
            return e("令牌已复制到剪贴板", "Token copied to clipboard", "Jeton copié dans le presse-papiers", "Token copiato negli appunti", "Token skopiowany do schowka")
        case .mcpTokenMasked:
            return e("令牌: ••••", "Token: ••••", "Jeton : ••••", "Token: ••••", "Token: ••••")
        case .mcpTokenMaskedWithTail:
            return e("令牌: ••••%@", "Token: ••••%@", "Jeton : ••••%@", "Token: ••••%@", "Token: ••••%@")
        case .mcpTokenRegeneratedSuccess:
            return e("令牌已重新生成并生效", "Token regenerated and active", "Jeton régénéré et actif", "Token rigenerato ed efficace", "Token wygenerowany ponownie i aktywny")
        case .mcpValidationError:
            return e("校验错误: %@", "Validation error: %@", "Erreur de validation : %@", "Errore di validazione: %@", "Błąd walidacji: %@")
        case .statusChecking:
            return e("检查中...", "Checking…", "Vérification…", "Controllo…", "Sprawdzanie…")
        case .statusNotReady:
            return e("未就绪", "Not ready", "Pas prêt", "Non pronto", "Niegotowe")
        case .statusQuerying:
            return e("查询中...", "Querying…", "Recherche…", "Interrogazione…", "Odpytywanie…")
        case .statusUnknown:
            return e("未知", "Unknown", "Inconnu", "Sconosciuto", "Nieznany")
        case .statusUnverified:
            return e("未校验", "Unverified", "Non vérifié", "Non verificato", "Niezweryfikowano")
        case .updateBtnDeferDisabled:
            return e("延期更新 (未启用)", "Defer update (disabled)", "Différer (désactivé)", "Rinvia (disabilitato)", "Odrocz (wyłączone)")
        case .updateBtnDownloadDisabled:
            return e("下载更新 (未启用)", "Download (disabled)", "Télécharger (désactivé)", "Scarica (disabilitato)", "Pobierz (wyłączone)")
        case .updateBtnInstallDisabled:
            return e("安装并重启 (未启用)", "Install and restart (disabled)", "Installer et redémarrer (désactivé)", "Installa e riavvia (disabilitato)", "Instaluj i uruchom ponownie (wyłączone)")
        case .updateCandidateAlreadyLatest:
            return e("当前已是最新版本", "Already up to date", "Déjà à jour", "Già aggiornato", "Aktualna wersja")
        case .updateCandidateAvailableVerified:
            return e("发现可用版本并通过完整验证", "Available version found and fully verified", "Version disponible trouvée et entièrement vérifiée", "Trovata versione disponibile e completamente verificata", "Znaleziono dostępną wersję, w pełni zweryfikowaną")
        case .updateCandidateFoundUnverified:
            return e("已发现候选版本（来源尚未验证）", "Candidate found (source unverified)", "Version candidate trouvée (source non vérifiée)", "Trovata versione candidata (origine non verificata)", "Znaleziono wersję kandydującą (źródło niezweryfikowane)")
        case .updateCandidateIncompatibleEnv:
            return e("候选版本不兼容当前环境", "Candidate version incompatible with current environment", "Version candidate incompatible avec l'environnement actuel", "Versione candidata incompatibile con l'ambiente attuale", "Wersja kandydująca niezgodna z bieżącym środowiskiem")
        case .updateCandidateNoVersion:
            return e("无可用版本 (null)", "No version available (null)", "Aucune version disponible (null)", "Nessuna versione disponibile (null)", "Brak dostępnej wersji (null)")
        case .updateCandidateNotCheckedYet:
            return e("尚未检查更新", "Update not checked yet", "Mise à jour non vérifiée", "Aggiornamento non ancora verificato", "Aktualizacja jeszcze niesprawdzona")
        case .updateCandidateSourceUnconfigured:
            return e("更新源尚未配置", "Update source not configured", "Source de mise à jour non configurée", "Sorgente di aggiornamento non configurata", "Źródło aktualizacji nieskonfigurowane")
        case .updateCannotInstallOnHost:
            return e("该版本无法安装在当前目标专用机上。", "This version cannot be installed on the current target host.", "Cette version ne peut pas être installée sur la machine hôte cible.", "Questa versione non può essere installata sulla macchina di destinazione.", "Tej wersji nie można zainstalować na bieżącej maszynie docelowej.")
        case .updateCannotRollbackFmt:
            return e("当前不可回滚：%1$@", "Cannot roll back: %1$@", "Impossible de revenir en arrière : %1$@", "Impossibile ripristinare: %1$@", "Nie można przywrócić: %1$@")
        case .updateCertNotFound:
            return e("专用机本地自签名证书未找到", "Local self-signed certificate not found", "Certificat auto-signé local introuvable", "Certificato autofirmato locale non trovato", "Nie znaleziono lokalnego certyfikatu z podpisem własnym")
        case .updateCheckComplete:
            return e("检查完成。", "Check completed.", "Vérification terminée.", "Verifica completata.", "Sprawdzanie ukończone.")
        case .updateCheckFailedFmt:
            return e("检查失败: %@", "Check failed: %@", "Échec de la vérification : %@", "Verifica fallita: %@", "Sprawdzanie nieudane: %@")
        case .updateCheckingStatus:
            return e("正在查询更新状态...", "Querying update status…", "Recherche de l'état de mise à jour…", "Verifica stato aggiornamenti…", "Sprawdzanie stanu aktualizacji…")
        case .updateCompatArchVerified:
            return e("架构兼容 (发布者未验证)", "Architecture compatible (publisher unverified)", "Architecture compatible (éditeur non vérifié)", "Architettura compatibile (editore non verificato)", "Architektura zgodna (wydawca niezweryfikowany)")
        case .updateCompatCurrentOk:
            return e("当前版本已与本地环境兼容", "Current version compatible with local environment", "Version actuelle compatible avec l'environnement local", "Versione attuale compatibile con l'ambiente locale", "Bieżąca wersja jest zgodna ze środowiskiem lokalnym")
        case .updateCompatIncompatible:
            return e("架构或系统不兼容", "Architecture or system incompatible", "Architecture ou système incompatible", "Architettura o sistema incompatibile", "Architektura lub system niekompatybilny")
        case .updateCompatRequirementsNotMet:
            return e("不满足最低系统或架构要求", "Minimum system or architecture requirements not met", "Exigences minimales du système ou de l'architecture non respectées", "Requisiti minimi di sistema o architettura non soddisfatti", "Minimalne wymagania systemowe lub architektoniczne nie zostały spełnione")
        case .updateCompatSystemCompatible:
            return e("系统环境兼容", "System environment compatible", "Environnement système compatible", "Ambiente di sistema compatibile", "Środowisko systemowe zgodne")
        case .updateCompatSystemIncompatible:
            return e("不兼容", "Incompatible", "Incompatible", "Incompatibile", "Niezgodne")
        case .updateCompatibilityDefaultReason:
            return e("生产环境下需配置授权更新清单；禁止任意未经验证的网络下载", "Authorized update manifest required in production; unverified downloads forbidden", "Manifeste autorisé requis en production ; téléchargements arbitraires interdits", "Manifest autorizzato richiesto in produzione; vietati download non verificati", "W środowisku produkcyjnym wymagany jest autoryzowany manifest; pobieranie zabronione")
        case .updateConsistencyConsistent:
            return e("整套组件版本一致", "Component versions consistent", "Versions des composants cohérentes", "Versioni dei componenti coerenti", "Wersje komponentów są spójne")
        case .updateConsistencyInconsistent:
            return e("⚠️ 组件版本不一致", "⚠️ Component versions inconsistent", "⚠️ Versions des composants incohérentes", "⚠️ Versioni dei componenti incoerenti", "⚠️ Niespójne wersje komponentów")
        case .updateCoreCompiledVersion:
            return e("%@ (核心编译版)", "%@ (core compiled build)", "%@ (version compilée du cœur)", "%@ (build compilata del servizio principale)", "%@ (wersja skompilowana rdzenia)")
        case .updateCoreVersionUnknown:
            return e("未知 (未报告)", "Unknown (unreported)", "Inconnu (non rapporté)", "Sconosciuto (non segnalato)", "Nieznany (niezgłoszony)")
        case .updateDrainWaitingNotice:
            return e("⚠️ 外部更新事务处于排空等待态；延期请由管理员命令行协调。", "⚠️ External update transaction waiting for drain; contact admin for postponement.", "⚠️ Transaction de mise à jour en attente de purge ; contactez l'admin pour différer.", "⚠️ Transazione di aggiornamento in attesa di svuotamento; coordinare il rinvio da terminale.", "⚠️ Zewnętrzna transakcja aktualizacji oczekuje na opróżnienie; odroczenie z poziomu wiersza poleceń.")
        case .updateErrCommandFailedFmt:
            return e("命令执行错误 [%1$d]: %2$@", "Command failed [%1$d]: %2$@", "Commande échouée [%1$d] : %2$@", "Comando non riuscito [%1$d]: %2$@", "Błąd polecenia [%1$d]: %2$@")
        case .updateErrNetworkOfflineFmt:
            return e("网络连接失败: %@", "Network connection failed: %@", "Échec de connexion réseau : %@", "Connessione di rete non riuscita: %@", "Połączenie sieciowe nieudane: %@")
        case .updateErrParseFmt:
            return e("数据解析失败: %@", "Data parsing failed: %@", "Échec de l'analyse des données : %@", "Analisi dati fallita: %@", "Błąd analizy danych: %@")
        case .updateFeatureDisabledNotice:
            return e("操作未启用: 当前阶段真实更新下载/安装/延期入口为只读禁用状态 (unsupported)，不进行任何写入或业务状态修改。", "Operation disabled: update download/install/defer is read-only (unsupported); no writes performed.", "Opération désactivée : téléchargement/installation/report en lecture seule (non pris en charge) ; aucune modification.", "Operazione disabilitata: download/installazione/rinvio in sola lettura (non supportato); nessuna scrittura.", "Operacja wyłączona: pobieranie/instalacja/odroczenie w trybie tylko do odczytu (nieobsługiwane); brak modyfikacji.")
        case .updateHighWatermarkNotice:
            return e("%llu (防降级高水位，非安装版本)", "%llu (downgrade-guard watermark, not installed version)", "%llu (niveau anti-dégradation, pas la version installée)", "%llu (livello anti-degradazione, non la versione installata)", "%llu (zabezpieczenie przed obniżeniem wersji, nie wersja zainstalowana)")
        case .updateIntroDesc:
            return e("管理微信客服助手整套原生组件与核心服务更新，支持更新检查、候选验证、受控维护以及回滚至上一稳定版本。", "Manage updates for WeChat Support Assistant, including checks, candidate verification, maintenance and rollback.", "Gérez les mises à jour de l'Assistant WeChat : vérification, validation, maintenance et retour arrière.", "Gestisci gli aggiornamenti di WeChat Support Assistant: verifiche, convalide, manutenzione e ripristino.", "Zarządzaj aktualizacjami Asystenta WeChat: sprawdzanie, walidacja, konserwacja i przywracanie.")
        case .updateKeepOldRecord:
            return e("保持旧版本记录", "Keep previous version record", "Conserver l'enregistrement de version précédente", "Mantieni record versione precedente", "Zachowaj rekord poprzedniej wersji")
        case .updateKeychainLocked:
            return e("钥匙串处于锁定状态或未解锁", "Keychain locked or inaccessible", "Trousseau verrouillé ou inaccessible", "Portachiavi bloccato o inaccessibile", "Pęk kluczy zablokowany lub niedostępny")
        case .updateKeychainReady:
            return e("专用机自签名证书与钥匙串就绪 (待发布者验证通过后重签)", "Dedicated host certificate and keychain ready (re-sign after publisher verification)", "Certificat local et trousseau prêts (resignature après vérification de l'éditeur)", "Certificato locale e portachiavi pronti (rifirma dopo verifica dell'editore)", "Certyfikat i pęk kluczy gotowe (podpisz ponownie po weryfikacji wydawcy)")
        case .updateLabelCompatibility:
            return e("兼容性说明:", "Compatibility:", "Compatibilité :", "Compatibilità:", "Zgodność:")
        case .updateLabelConsistency:
            return e("组件一致性:", "Component consistency:", "Cohérence des composants :", "Coerenza componenti:", "Spójność komponentów:")
        case .updateLabelCoreVersion:
            return e("核心编译版本:", "Core binary version:", "Version binaire du cœur :", "Versione binaria principale:", "Wersja binarna rdzenia:")
        case .updateLabelCurrentSuite:
            return e("当前整套版本:", "Current release suite:", "Suite de version actuelle :", "Suite versione attuale:", "Bieżący zestaw wersji:")
        case .updateLabelHostArch:
            return e("宿主运行架构:", "Host architecture:", "Architecture hôte :", "Architettura host:", "Architektura hosta:")
        case .updateLabelInstallSeq:
            return e("整套安装序列:", "Installation sequence:", "Séquence d'installation :", "Sequenza di installazione:", "Sekwencja instalacji:")
        case .updateLabelLocalSign:
            return e("本机重签名:", "Local re-signing:", "Resignature locale :", "Rifirma locale:", "Lokalne ponowne podpisanie:")
        case .updateLabelPackageSize:
            return e("更新包大小:", "Package size:", "Taille du paquet :", "Dimensione pacchetto:", "Rozmiar pakietu:")
        case .updateLabelPublisherVerify:
            return e("发布者验证:", "Publisher verification:", "Vérification de l'éditeur :", "Verifica editore:", "Weryfikacja wydawcy:")
        case .updateLabelReleaseNotes:
            return e("版本发布说明:", "Release notes:", "Notes de version :", "Note di rilascio:", "Informacje o wydaniu:")
        case .updateLabelRollbackEligibility:
            return e("回滚资格与原因:", "Rollback eligibility & reason:", "Éligibilité au retour arrière :", "Idoneità al ripristino e motivo:", "Kwalifikowalność do przywrócenia i powód:")
        case .updateLabelRollbackVersion:
            return e("可回滚上一版本:", "Rollback version:", "Version de retour arrière :", "Versione di ripristino:", "Wersja do przywrócenia:")
        case .updateLabelTargetVersion:
            return e("目标版本:", "Target version:", "Version cible :", "Versione di destinazione:", "Wersja docelowa:")
        case .updateLocalRunning:
            return e("已在运行", "Running", "En cours d'exécution", "In esecuzione", "Działa")
        case .updateNoCandidateNotes:
            return e("没有待更新的候选版本。", "No pending update candidates.", "Aucune mise à jour en attente.", "Nessun aggiornamento in sospeso.", "Brak oczekujących aktualizacji.")
        case .updateNoDetailedNotes:
            return e("（无详细发布说明）", "(No detailed release notes)", "(Aucune note de version détaillée)", "(Nessuna nota di rilascio dettagliata)", "(Brak szczegółowych informacji o wydaniu)")
        case .updateNoRollbackDetected:
            return e("未检测到可回滚版本", "No rollback version detected", "Aucune version de retour arrière détectée", "Nessuna versione di ripristino rilevata", "Nie wykryto wersji do przywrócenia")
        case .updateNotesNone:
            return e("（暂无说明）", "(No release notes)", "(Aucune note de version)", "(Nessuna nota di rilascio)", "(Brak uwag)")
        case .updateNullValue:
            return e("无 (null)", "None (null)", "Aucun (null)", "Nessuno (null)", "Brak (null)")
        case .updateOldDataSourceUntrusted:
            return e("未提供状态/来源 (旧数据源，不可信)", "Status/source omitted (legacy source, untrusted)", "État/source non fournis (ancienne source, non fiable)", "Stato/origine omessi (vecchia sorgente, non affidabile)", "Brak stanu/źródła (stare źródło, niezaufane)")
        case .updatePermissionsNotice:
            return e("提示：专用机应用更新后将由本地自签名钥匙串重签名，若系统权限重置需在「设置」中重新授权。", "Note: Host apps are re-signed via local keychain; re-grant permissions in Settings if reset.", "Remarque : Réapplication de la signature locale ; réautorisez dans Réglages si réinitialisé.", "Nota: L'app viene rifirmata localmente; reimposta le autorizzazioni in Impostazioni se necessario.", "Uwaga: Aplikacja zostanie podpisana lokalnie; nadaj ponownie uprawnienia w Ustawieniach, jeśli zresetowano.")
        case .updatePipelineSteps:
            return e("检查 → 下载 → 验证 → 本机签名 → 等待排空 → 备份 → 安装 → 自检", "Check → Download → Verify → Local Sign → Drain → Backup → Install → Health", "Vérifier → Télécharger → Valider → Signer → Purger → Sauvegarder → Installer → Santé", "Verifica → Download → Convalida → Firma locale → Svuota → Backup → Installa → Salute", "Sprawdź → Pobierz → Weryfikuj → Podpis lokalny → Opróżnij → Kopia → Instaluj → Stan")
        case .updatePublisherApplied:
            return e("已生效", "Active", "Actif", "Attivo", "Aktywne")
        case .updatePublisherFailedCompat:
            return e("未通过兼容性核验", "Failed compatibility check", "Échec du contrôle de compatibilité", "Controllo compatibilità fallito", "Kontrola zgodności nieudana")
        case .updatePublisherNoRootKey:
            return e("未配置公钥根", "Public root key not configured", "Clé racine publique non configurée", "Chiave radice pubblica non configurata", "Klucz główny nie został skonfigurowany")
        case .updatePublisherSignatureVerified:
            return e("发布者签名已验证", "Publisher signature verified", "Signature de l'éditeur vérifiée", "Firma dell'editore verificata", "Podpis wydawcy zweryfikowany")
        case .updatePublisherUnverifiedNoKey:
            return e("尚未验证（无可信根公钥配置）", "Unverified (no trusted root public key)", "Non vérifié (aucune clé racine publique de confiance)", "Non verificato (nessuna chiave radice pubblica affidabile)", "Niezweryfikowano (brak zaufanego klucza głównego)")
        case .updateReasonBackupCorrupt:
            return e("数据库在线备份损坏或不完整", "Online database backup corrupt or incomplete", "Sauvegarde de base de données en ligne corrompue ou incomplète", "Backup del database online corrotto o incompleto", "Kopia zapasowa bazy danych online jest uszkodzona lub niekompletna")
        case .updateReasonCannot:
            return e("暂不可回滚", "Rollback currently unavailable", "Retour arrière non disponible", "Ripristino momentaneamente non disponibile", "Przywrócenie obecnie niedostępne")
        case .updateReasonCheckpointRequired:
            return e("存在未提交事务，需完全停止服务", "Uncommitted transactions exist; full service stop required", "Transactions non validées existantes ; arrêt complet requis", "Transazioni non confermate presenti; arresto completo richiesto", "Istnieją niezatwierdzone transakcje; wymagane całkowite zatrzymanie usługi")
        case .updateReasonInstallRootUnconfigured:
            return e("未配置程序安装目录", "Installation root directory unconfigured", "Répertoire d'installation non configuré", "Directory di installazione non configurata", "Nieskonfigurowany katalog instalacyjny")
        case .updateReasonNoMatching:
            return e("未找到匹配的已完成更新记录", "No matching completed update record found", "Aucun enregistrement de mise à jour terminée trouvé", "Nessun record di aggiornamento completato corrispondente", "Nie znaleziono pasującego wpisu ukończonej aktualizacji")
        case .updateReasonNotSupported:
            return e("该发布版本不支持整包换回", "This release does not support package rollback", "Cette version ne prend pas en charge le retour arrière complet", "Questa versione non supporta il ripristino del pacchetto", "Ta wersja nie obsługuje pełnego przywrócenia pakietu")
        case .updateReasonPrevCorrupt:
            return e("上一版本程序文件损坏或被篡改", "Previous release file corrupt or tampered with", "Fichier de la version précédente corrompu ou altéré", "File della versione precedente corrotto o manomesso", "Plik poprzedniej wersji uszkodzony lub zmodyfikowany")
        case .updateReasonRecoveryRequired:
            return e("存在中断的更新事务，需执行受控恢复", "Interrupted update transaction; controlled recovery required", "Transaction de mise à jour interrompue ; récupération contrôlée requise", "Transazione interrotta; richiesto ripristino controllato", "Przerwana transakcja aktualizacji; wymagane kontrolowane odzyskiwanie")
        case .updateReasonRollbackStateInvalid:
            return e("回滚状态无效", "Rollback state invalid", "État de retour arrière non valide", "Stato di ripristino non valido", "Nieprawidłowy stan przywracania")
        case .updateReasonSafe:
            return e("可安全回滚", "Safe to roll back", "Retour arrière sécurisé", "Ripristino sicuro", "Bezpieczne przywrócenie")
        case .updateReasonSchemaChanged:
            return e("数据库结构已发生变化，拒绝回滚", "Database schema changed; rollback rejected", "Schéma de base de données modifié ; retour refusé", "Schema database modificato; ripristino rifiutato", "Schemat bazy danych uległ zmianie; odmowa przywrócenia")
        case .updateReasonStopCoreFirst:
            return e("核心服务运行中，需先停止服务", "Core service running; must stop it first", "Service principal en cours d'exécution ; arrêt requis", "Servizio principale attivo; è necessario fermarlo prima", "Rdzeń usługi działa; należy go najpierw zatrzymać")
        case .updateRecentCommitted:
            return e("最近记录已提交", "Recent record committed", "Enregistrement récent validé", "Record recente confermato", "Niedawny rekord zatwierdzony")
        case .updateRecordApplied:
            return e("已记录生效", "Recorded and applied", "Enregistré et appliqué", "Registrato e applicato", "Zarejestrowano i zastosowano")
        case .updateRecoveryGuideHeader:
            return e("只读说明与受控恢复指引:", "Read-only notice and controlled recovery guide:", "Notice en lecture seule et guide de récupération contrôlée :", "Avviso di sola lettura e guida al ripristino controllato:", "Informacja tylko do odczytu i instrukcja kontrolowanego odzyskiwania:")
        case .updateRecoveryGuideItem1:
            return e("1. 维护标志文件已记录；实际核心进程存活状态与业务数据完整性未知。", "1. Maintenance flag recorded; actual core process status and data integrity unknown.", "1. Indicateur de maintenance enregistré ; état réel du cœur et intégrité inconnus.", "1. Flag di manutenzione registrato; stato del processo principale e integrità sconosciuti.", "1. Flaga konserwacji zarejestrowana; rzeczywisty stan rdzenia i integralność danych nieznane.")
        case .updateRecoveryGuideItem2:
            return e("2. 当前界面处于严格只读受控阶段，禁止通过界面清除维护标志或进行破坏性写入。", "2. Current UI strictly read-only; clearing flags or disruptive writes prohibited.", "2. Interface strictement en lecture seule ; effacement d'indicateurs ou écriture interdite.", "2. Interfaccia rigorosamente in sola lettura; vietato cancellare flag o scrivere dati.", "2. Interfejs ściśle tylko do odczytu; usuwanie flag lub zapisywanie zabronione.")
        case .updateRecoveryGuideItem3:
            return e("3. 真实版本回退能力当前处于 unsupported 禁用状态，不可直接回退；需要专门受控恢复。", "3. Version rollback disabled (unsupported); controlled recovery required.", "3. Retour de version désactivé (non pris en charge) ; récupération contrôlée requise.", "3. Ripristino versione disabilitato (non supportato); richiesto ripristino controllato.", "3. Przywracanie wersji wyłączone (nieobsługiwane); wymagane kontrolowane odzyskiwanie.")
        case .updateRecoveryGuideItem4:
            return e("4. 请核对 ~/.wechat-rpa/logs/core.log 与 host.log 获取详细故障上下文并联系管理员。", "4. Check ~/.wechat-rpa/logs/core.log and host.log for failure context and contact admin.", "4. Consultez ~/.wechat-rpa/logs/core.log et host.log pour les détails et contactez l'admin.", "4. Controlla ~/.wechat-rpa/logs/core.log e host.log per i dettagli e contatta l'amministratore.", "4. Sprawdź ~/.wechat-rpa/logs/core.log oraz host.log i skontaktuj się z administratorem.")
        case .updateRecoveryNeededFmt:
            return e("故障需要恢复 (%@)", "Recovery needed (%@)", "Récupération requise (%@)", "Ripristino necessario (%@)", "Wymagane odzyskanie (%@)")
        case .updateRecoveryNormal:
            return e("系统状态正常，无需恢复。", "System status normal; no recovery needed.", "État normal ; aucune récupération nécessaire.", "Stato normale; nessun ripristino necessario.", "Stan prawidłowy; odzyskiwanie niepotrzebne.")
        case .updateRecoveryReasonFmt:
            return e("- 错误原因: %@", "- Error reason: %@", "- Raison de l'erreur : %@", "- Motivo dell'errore: %@", "- Przyczyna błędu: %@")
        case .updateRecoveryReportTitle:
            return e("【更新异常中断，需要人工恢复】", "[Update interrupted abnormally; manual recovery required]", "[Mise à jour interrompue anormalement ; récupération manuelle requise]", "[Aggiornamento interrotto in modo anomalo; richiesto ripristino manuale]", "[Aktualizacja przerwana nieoczekiwanie; wymagane ręczne odzyskanie]")
        case .updateRecoveryStageFmt:
            return e("- 中断阶段: %@", "- Interrupted stage: %@", "- Étape interrompue : %@", "- Fase interrotta: %@", "- Przerwany etap: %@")
        case .updateRecoveryTxFmt:
            return e("- 事务 ID: %@", "- Transaction ID: %@", "- ID de transaction : %@", "- ID transazione: %@", "- ID transakcji: %@")
        case .updateRefreshingStatus:
            return e("正在刷新状态...", "Refreshing status…", "Actualisation de l'état…", "Aggiornamento stato…", "Odświeżanie stanu…")
        case .updateReleaseDescUnknown:
            return e("整套版本无法核验 (未知)", "Release suite unverifiable (unknown)", "Suite de versions non vérifiable (inconnue)", "Suite di versioni non verificabile (sconosciuta)", "Zestaw wersji niemożliwy do zweryfikowania (nieznany)")
        case .updateReleaseNotesEmpty:
            return e("（暂无发布说明）", "(No release notes)", "(Aucune note de version)", "(Nessuna nota di rilascio)", "(Brak informacji o wydaniu)")
        case .updateReleaseNotesUnconfigured:
            return e("系统未配置已批准的更新清单或分发源。为保障微信业务与专用机安全，禁止使用任意 URL 下载执行内容。", "System has no approved update manifest. To protect WeChat services, downloading from arbitrary URLs is forbidden.", "Aucun manifeste approuvé configuré. Pour la sécurité, le téléchargement depuis des URL arbitraires est interdit.", "Nessun manifest approvato configurato. Per motivi di sicurezza, è vietato scaricare da URL arbitrari.", "Brak zatwierdzonego manifestu aktualizacji. Ze względów bezpieczeństwa pobieranie z dowolnych adresów URL jest zabronione.")
        case .updateRollbackCancelledCheckbox:
            return e("回滚已取消：请勾选确认方可执行回滚。", "Rollback cancelled: check the box to confirm rollback.", "Retour arrière annulé : cochez la case pour confirmer.", "Ripristino annullato: seleziona la casella per confermare.", "Przywracanie anulowane: zaznacz pole wyboru, aby potwierdzić.")
        case .updateRollbackConfirmBody:
            return e("换回上一个已安装的版本将直接整包替换为上一个应用程序包。操作成功后需要重启应用以完成替换。确认继续吗？", "Replacing with the previously installed version swaps the entire application bundle. Restart the app after completion. Continue?", "Remplacer par la version précédente échange tout le paquet de l'application. Redémarrez après l'opération. Continuer ?", "La sostituzione con la versione precedente scambia l'intero pacchetto dell'applicazione. Riavvia dopo il completamento. Continuare?", "Zastąpienie poprzednią wersją podmienia cały pakiet aplikacji. Po zakończeniu uruchom aplikację ponownie. Kontynuować?")
        case .updateRollbackConfirmTitleFmt:
            return e("确认换回上一个已安装的版本（%@）？", "Replace with previous version (%@)?", "Remplacer par la version précédente (%@) ?", "Sostituire con la versione precedente (%@)?", "Zastąpić poprzednią wersją (%@)?")
        case .updateRollbackExitCodeFmt:
            return e("回滚未完成 (退出码 %d)", "Rollback incomplete (exit code %d)", "Retour arrière incomplet (code de sortie %d)", "Ripristino incompleto (codice di uscita %d)", "Przywracanie nieukończone (kod wyjścia %d)")
        case .updateRollbackFailedFmt:
            return e("回滚失败: %@", "Rollback failed: %@", "Échec du retour arrière : %@", "Ripristino fallito: %@", "Przywracanie nieudane: %@")
        case .updateRollbackNoOutput:
            return e("回滚失败：无输出", "Rollback failed: no output", "Échec du retour arrière : aucune sortie", "Ripristino fallito: nessun output", "Przywracanie nieudane: brak danych wyjściowych")
        case .updateRollbackNotice:
            return e("说明：换回上一个已安装的版本。", "Note: Replace with the previously installed version.", "Note : Remplacer par la version précédemment installée.", "Nota: Sostituisci con la versione precedentemente installata.", "Uwaga: Zastąp poprzednio zainstalowaną wersją.")
        case .updateRollbackRecordedSafe:
            return e("已安全记录回退状态", "Rollback state safely recorded", "État de retour arrière enregistré en toute sécurité", "Stato di ripristino registrato in sicurezza", "Stan przywrócenia bezpiecznie zarejestrowany")
        case .updateRollbackTimeout:
            return e("执行回滚操作超时", "Rollback operation timed out", "Délai d'attente du retour arrière dépassé", "Operazione di ripristino scaduta", "Przekroczono limit czasu przywracania")
        case .updateRollbackingFmt:
            return e("正在换回上一个已安装的版本 (%@)...", "Restoring previous version (%@)…", "Restauration de la version précédente (%@)…", "Ripristino versione precedente (%@)…", "Przywracanie poprzedniej wersji (%@)…")
        case .updateSectionCandidate:
            return e("候选版本与发布说明", "Candidate release & notes", "Version candidate et notes", "Versione candidata e note", "Wersja kandydująca i uwagi")
        case .updateSectionCurrentEnv:
            return e("当前运行环境 (真实读模型)", "Current environment (read model)", "Environnement actuel (modèle de lecture)", "Ambiente attuale (modello di lettura)", "Bieżące środowisko (model odczytu)")
        case .updateSectionProgress:
            return e("更新阶段与进度", "Update stage & progress", "Étape et progression", "Fase e avanzamento", "Etap i postęp")
        case .updateSeqUnknown:
            return e("未知 (未标记)", "Unknown (unmarked)", "Inconnu (non marqué)", "Sconosciuto (senza contrassegno)", "Nieznany (nieoznaczony)")
        case .updateSigningForbidden:
            return e("禁止重签", "Re-signing forbidden", "Resignature interdite", "Rifirma vietata", "Ponowne podpisywanie zabronione")
        case .updateSigningForbiddenNoSource:
            return e("未配置发布源，禁止重签", "Update source unconfigured; re-signing forbidden", "Source non configurée ; resignature interdite", "Sorgente non configurata; rifirma vietata", "Brak źródła; ponowne podpisywanie zabronione")
        case .updateSigningNeedVerify:
            return e("禁止重签 (需要发布者验证通过)", "Re-signing forbidden (publisher verification required)", "Resignature interdite (vérification de l'éditeur requise)", "Rifirma vietata (richiesta verifica editore)", "Ponowne podpisywanie zabronione (wymagana weryfikacja wydawcy)")
        case .updateSigningUnavailableFmt:
            return e("签名不可用: %@", "Signing unavailable: %@", "Signature indisponible : %@", "Firma non disponibile: %@", "Podpisywanie niedostępne: %@")
        case .updateSourceNone:
            return e("无来源", "No source", "Aucune source", "Nessuna origine", "Brak źródła")
        case .updateSourceSignedPackage:
            return e("签名发布包", "Signed release package", "Paquet signé", "Pacchetto firmato", "Podpisany pakiet")
        case .updateSourceUnknown:
            return e("来源未知", "Source unknown", "Source inconnue", "Origine sconosciuta", "Źródło nieznane")
        case .updateSourceUnknownFmt:
            return e("未知来源 [%@]", "Unknown source [%@]", "Source inconnue [%@]", "Origine sconosciuta [%@]", "Nieznane źródło [%@]")
        case .updateStageExecuting:
            return e("执行中", "Executing", "En cours d'exécution", "In esecuzione", "Wykonywanie")
        case .updateStageExternalTxFmt:
            return e("状态: %1$@ (%2$@)", "State: %1$@ (%2$@)", "État : %1$@ (%2$@)", "Stato: %1$@ (%2$@)", "Stan: %1$@ (%2$@)")
        case .updateStageIdle:
            return e("空闲", "Idle", "Inactif", "Inattivo", "Bezczynny")
        case .updateStateInstalled:
            return e("已安装", "Installed", "Installé", "Installato", "Zainstalowano")
        case .updateStateNone:
            return e("无状态", "No state", "Aucun état", "Nessun stato", "Brak stanu")
        case .updateStateUnknown:
            return e("状态未知", "State unknown", "État inconnu", "Stato sconosciuto", "Stan nieznany")
        case .updateStateUnknownFmt:
            return e("未知状态 [%@]", "Unknown state [%@]", "État inconnu [%@]", "Stato sconosciuto [%@]", "Nieznany stan [%@]")
        case .updateStateVerified:
            return e("已验证", "Verified", "Vérifié", "Verificato", "Zweryfikowano")
        case .updateStatusFetchNoticeFmt:
            return e("状态获取提示: %@", "Status notice: %@", "Avis d'état : %@", "Avviso di stato: %@", "Powiadomienie o stanie: %@")
        case .updateStatusRefreshFailedFmt:
            return e("状态刷新失败: %@", "Failed to refresh status: %@", "Échec de l'actualisation : %@", "Aggiornamento stato fallito: %@", "Odświeżanie stanu nieudane: %@")
        case .updateStatusRefreshed:
            return e("状态已刷新。", "Status refreshed.", "État actualisé.", "Stato aggiornato.", "Stan odświeżony.")
        case .updateSupervisorSuppressed:
            return e("处于维护抑制态 (核心启动被抑制)", "Maintenance suppression active (core launch suppressed)", "Suppression de maintenance active (lancement du cœur suspendu)", "Soppressione per manutenzione attiva (avvio del servizio principale bloccato)", "Tłumienie konserwacyjne aktywne (uruchamianie rdzenia wstrzymane)")
        case .updateSupervisorUnsuppressed:
            return e("未处于维护抑制态 (运行健康度未经验证)", "Maintenance suppression inactive (health unverified)", "Suppression de maintenance inactive (santé non vérifiée)", "Soppressione per manutenzione inattiva (stato di salute non verificato)", "Brak tłumienia konserwacyjnego (stan zdrowia niezweryfikowany)")
        case .updateUntrustedStatusSourceFmt:
            return e("%1$@ / %2$@ (不可信)", "%1$@ / %2$@ (untrusted)", "%1$@ / %2$@ (non fiable)", "%1$@ / %2$@ (non affidabile)", "%1$@ / %2$@ (niezaufane)")
        }
    }
}
