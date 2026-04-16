//! Skills 扫描器：从目录中发现并解析 skill 脚本的 meta 信息。

use std::path::{Path, PathBuf};

/// Skill 元信息。
#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub path: PathBuf,
}

/// 从脚本文件头解析 @skill meta 标记。
pub fn parse_meta(path: &Path) -> Option<SkillMeta> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut description = String::new();
    let mut usage = String::new();

    for line in content.lines().take(20) {
        let trimmed = line.trim().trim_start_matches('#').trim().trim_start_matches("//").trim();
        if let Some(val) = trimmed.strip_prefix("@skill:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = trimmed.strip_prefix("@description:") {
            description = val.trim().to_string();
        } else if let Some(val) = trimmed.strip_prefix("@usage:") {
            usage = val.trim().to_string();
        }
    }

    name.map(|n| SkillMeta {
        name: n,
        description,
        usage,
        path: path.to_path_buf(),
    })
}

/// 扫描目录中的所有 skill 脚本。
pub fn scan_directory(dir: &Path) -> Vec<SkillMeta> {
    if !dir.exists() || !dir.is_dir() {
        return vec![];
    }

    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(meta) = parse_meta(&path) {
                    skills.push(meta);
                }
            }
        }
    }
    skills
}

/// 将 skill 列表注入到 system prompt 中。
pub fn inject_to_prompt(system_prompt: &str, skills: &[SkillMeta]) -> String {
    if skills.is_empty() {
        return system_prompt.to_string();
    }

    let mut prompt = system_prompt.to_string();
    prompt.push_str("\n\n## Available Skills\n");
    for skill in skills {
        prompt.push_str(&format!(
            "- `{}`: {}\n  Usage: {}\n",
            skill.name, skill.description, skill.usage
        ));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_skills_parse_meta_shell() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("commit.sh");
        std::fs::write(
            &path,
            "#!/bin/bash\n# @skill: commit\n# @description: Create a git commit\n# @usage: /commit [message]\necho done\n",
        )
        .unwrap();

        let meta = parse_meta(&path).unwrap();
        assert_eq!(meta.name, "commit");
        assert_eq!(meta.description, "Create a git commit");
        assert_eq!(meta.usage, "/commit [message]");
    }

    #[test]
    fn test_skills_parse_meta_python() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("lint.py");
        std::fs::write(&path, "# @skill: lint\n# @description: Run linter\nimport sys\n").unwrap();

        let meta = parse_meta(&path).unwrap();
        assert_eq!(meta.name, "lint");
    }

    #[test]
    fn test_skills_parse_no_meta() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("readme.md");
        std::fs::write(&path, "# This is a readme\nNo skill here.\n").unwrap();

        assert!(parse_meta(&path).is_none());
    }

    #[test]
    fn test_skills_parse_partial_meta() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("minimal.sh");
        std::fs::write(&path, "#!/bin/bash\n# @skill: deploy\necho deploying\n").unwrap();

        let meta = parse_meta(&path).unwrap();
        assert_eq!(meta.name, "deploy");
        assert_eq!(meta.description, ""); // optional, empty is ok
    }
}
