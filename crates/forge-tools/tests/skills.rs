use std::sync::Arc;

use forge_core::Tool;
use forge_tools::{inject_to_prompt, scan_directory, SkillTool, ToolRegistry};
use forge_tools::skills::scanner::SkillMeta;
use tempfile::TempDir;

fn write_skill(dir: &std::path::Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn test_skills_scan_directory() {
    let tmp = TempDir::new().unwrap();
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    write_skill(
        &skills_dir,
        "commit.sh",
        "#!/bin/bash\n# @skill: commit\n# @description: Git commit\necho done",
    );
    write_skill(
        &skills_dir,
        "lint.py",
        "# @skill: lint\n# @description: Run linter\nprint('ok')",
    );
    write_skill(&skills_dir, "readme.md", "# Just a readme\nNo skill here.");

    let skills = scan_directory(&skills_dir);
    assert_eq!(skills.len(), 2);

    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"commit"));
    assert!(names.contains(&"lint"));
}

#[test]
fn test_skills_scan_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let empty = tmp.path().join("empty_skills");
    std::fs::create_dir_all(&empty).unwrap();

    let skills = scan_directory(&empty);
    assert!(skills.is_empty());
}

#[test]
fn test_skills_inject_system_prompt() {
    let skills = vec![
        SkillMeta {
            name: "commit".into(),
            description: "Git commit".into(),
            usage: "/commit [msg]".into(),
            path: "/tmp/commit.sh".into(),
        },
        SkillMeta {
            name: "lint".into(),
            description: "Run linter".into(),
            usage: "/lint".into(),
            path: "/tmp/lint.py".into(),
        },
    ];

    let prompt = inject_to_prompt("You are helpful.", &skills);
    assert!(prompt.contains("/commit"));
    assert!(prompt.contains("/lint"));
    assert!(prompt.contains("Git commit"));
    assert!(prompt.contains("Available Skills"));
}

#[test]
fn test_skills_register_as_tool() {
    let tmp = TempDir::new().unwrap();
    let script = tmp.path().join("commit.sh");
    std::fs::write(&script, "#!/bin/bash\n# @skill: commit\necho ok").unwrap();

    let meta = SkillMeta {
        name: "commit".into(),
        description: "Git commit".into(),
        usage: "/commit".into(),
        path: script,
    };
    let tool = SkillTool::new(meta, tmp.path());

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool)).unwrap();

    assert!(registry.get("commit").is_some());
}

#[tokio::test]
async fn test_skill_execute_success() {
    let tmp = TempDir::new().unwrap();
    let script = tmp.path().join("hello.sh");
    write_skill(&tmp.path(), "hello.sh", "#!/bin/bash\necho done");

    let meta = SkillMeta {
        name: "hello".into(),
        description: "test".into(),
        usage: "/hello".into(),
        path: script,
    };
    let tool = SkillTool::new(meta, tmp.path());
    let output = tool.execute(serde_json::json!({})).await.unwrap();

    assert!(!output.is_error);
    assert!(output.content.contains("done"));
}

#[tokio::test]
async fn test_skill_execute_working_dir() {
    let tmp = TempDir::new().unwrap();
    let script = tmp.path().join("pwd.sh");
    write_skill(&tmp.path(), "pwd.sh", "#!/bin/bash\npwd");

    let meta = SkillMeta {
        name: "pwd".into(),
        description: "test".into(),
        usage: "/pwd".into(),
        path: script,
    };
    let tool = SkillTool::new(meta, tmp.path());
    let output = tool.execute(serde_json::json!({})).await.unwrap();

    assert!(
        output.content.contains(tmp.path().to_str().unwrap()),
        "should run in project dir, got: {}",
        output.content
    );
}

#[tokio::test]
async fn test_skill_timeout() {
    let tmp = TempDir::new().unwrap();
    let script = tmp.path().join("slow.sh");
    write_skill(&tmp.path(), "slow.sh", "#!/bin/bash\nsleep 10");

    let meta = SkillMeta {
        name: "slow".into(),
        description: "test".into(),
        usage: "/slow".into(),
        path: script,
    };
    let tool = SkillTool::new(meta, tmp.path())
        .with_timeout(std::time::Duration::from_millis(100));
    let output = tool.execute(serde_json::json!({})).await.unwrap();

    assert!(output.is_error);
    assert!(output.content.contains("timeout"));
}
