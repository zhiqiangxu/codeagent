use async_trait::async_trait;
use forge_core::{PermissionDecision, RuntimePrompter};
use forge_permissions::{Action, PermissionGateway, Profile, Rule};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::tempdir;

/// Mock RuntimePrompter that returns a preset decision and tracks call count.
struct MockPrompter {
    decision: PermissionDecision,
    call_count: Arc<AtomicUsize>,
}

impl MockPrompter {
    fn new(decision: PermissionDecision) -> Self {
        Self {
            decision: decision.clone(),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RuntimePrompter for MockPrompter {
    async fn ask(&self, _tool: &str, _args: &str) -> PermissionDecision {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.decision.clone()
    }
}

#[tokio::test]
async fn test_gateway_no_rule_asks_runtime() {
    let prompter = MockPrompter::new(PermissionDecision::Allow);
    let call_count = prompter.call_count.clone();
    let mut gw = PermissionGateway::new(Profile::Coding, vec![], prompter);

    let result = gw.check("Bash", "echo hi").await;

    assert_eq!(result, PermissionDecision::Allow);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_gateway_runtime_allow_generates_rule() {
    let prompter = MockPrompter::new(PermissionDecision::AlwaysAllow);
    let mut gw = PermissionGateway::new(Profile::Coding, vec![], prompter);

    let result = gw.check("Bash", "git status").await;

    assert_eq!(result, PermissionDecision::AlwaysAllow);
    // A new AutoAllow rule should have been generated
    assert_eq!(gw.rules().len(), 1);
    assert_eq!(gw.rules()[0].tool, "Bash");
    assert_eq!(gw.rules()[0].pattern, "git status");
    assert_eq!(gw.rules()[0].action, Action::AutoAllow);
}

#[tokio::test]
async fn test_gateway_rule_persistence_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rules.json");

    let original_rules = vec![
        Rule::parse("Bash(git *)").unwrap(),
        Rule::parse("!Bash(rm -rf *)").unwrap(),
    ];

    // Save
    let prompter = MockPrompter::new(PermissionDecision::Deny);
    let gw = PermissionGateway::new(Profile::Coding, original_rules.clone(), prompter);
    gw.save_rules(&path).unwrap();

    // Load
    let loaded = PermissionGateway::<MockPrompter>::load_rules(&path).unwrap();

    assert_eq!(loaded.len(), original_rules.len());
    for (a, b) in loaded.iter().zip(original_rules.iter()) {
        assert_eq!(a, b);
    }
}
