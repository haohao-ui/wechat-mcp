use std::{fs, path::Path};

#[test]
fn session_handoff_ui_requires_confirmation_and_truthful_result() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("native/bootstrap/SessionsPage.swift")).unwrap();
    let handler = source
        .split("@objc private func changeSelectedHandoff()")
        .nth(1)
        .unwrap()
        .split("public struct RecordedSession")
        .next()
        .unwrap();
    let confirm = handler
        .find("guard alert.runModal() == .alertFirstButtonReturn")
        .unwrap();
    let request = handler
        .find("SessionDataClient().handoff(sessionID: id, toHuman: human)")
        .unwrap();
    assert!(confirm < request);
    assert!(handler.contains("self.refreshSessions()"));
    assert!(handler.contains("self.fetchMessages(id: selected, offset: 0)"));
    assert!(handler.contains("L10n.t(.handoffChangeFailed)"));
    assert!(!handler.contains("String(describing: error)"));
    assert!(!source.contains("L10n.t(.handoffUnsupported)"));
    assert!(source.contains("session?.handoff == true ? .actionResumeAutoReply : .actionHandoff"));
    assert!(source.contains("state.sessionID == sessionID, state.handoff == toHuman"));
}
