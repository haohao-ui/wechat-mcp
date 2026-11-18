use std::{fs, path::Path};

#[test]
fn session_handoff_ui_requires_confirmation_and_truthful_result() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("native/bootstrap/SessionsPage.swift")).unwrap();
    let handler = source
        .split("@objc private func handleHandoffToHuman(_ sender: NSButton)")
        .nth(1)
        .expect("SessionsPage 必须提供转人工入口");

    // 1. 必须先二次确认，再发请求。
    let confirm = handler
        .find("guard alert.runModal() == .alertFirstButtonReturn")
        .expect("转人工必须先弹二次确认");
    let request = handler
        .find("handoff(sessionID: target.contactID, toHuman: true)")
        .expect("必须调用 control session-handoff");
    assert!(confirm < request, "确认对话框必须在请求之前");

    // 2. 成功后刷新列表。
    assert!(handler.contains("self.refreshSessions()"));

    // 3. 结果与失败都要走集中文案（L10n），且不得把错误对象或命令原始输出直接抛给界面。
    assert!(handler.contains("L10n.t(.uiSessions019"), "成功提示必须走 L10n");
    assert!(handler.contains("L10n.t(.uiSessions020, String(describing: error.localizedDescription))"),
        "失败提示必须带原因且只回固定文案");
    assert!(!handler.contains("String(describing: error)"));

    // 4. 会话显示名不是稳定 ID：必须先唯一定位，找不到/不唯一就报错，不能猜。
    assert!(handler.contains("matches.count == 1"));
    assert!(handler.contains("L10n.t(.uiSessions017") && handler.contains("L10n.t(.uiSessions018"));
}
