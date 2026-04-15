use forge_core::{PermissionDecision, RuntimePrompter};
use forge_permissions::Action;
use forge_tui::TuiRuntimePrompter;

#[tokio::test]
async fn test_tui_prompter_allow() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let prompter = TuiRuntimePrompter::new(rx);

    tx.send('y').unwrap();
    let decision = prompter.ask("read", "/some/path").await;

    assert_eq!(decision, PermissionDecision::Allow);
}

#[tokio::test]
async fn test_tui_prompter_deny() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let prompter = TuiRuntimePrompter::new(rx);

    tx.send('n').unwrap();
    let decision = prompter.ask("bash", "rm -rf /").await;

    assert_eq!(decision, PermissionDecision::Deny);
}

#[tokio::test]
async fn test_tui_prompter_always_allow() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let prompter = TuiRuntimePrompter::new(rx);

    tx.send('a').unwrap();
    let decision = prompter.ask("read", "/file").await;

    assert_eq!(decision, PermissionDecision::AlwaysAllow);

    // 验证生成了新 Rule
    let rules = prompter.generated_rules();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].tool, "read");
    assert_eq!(rules[0].pattern, "*");
    assert_eq!(rules[0].action, Action::AutoAllow);
}
