use serde::{Deserialize, Serialize};

/// 权限 Profile：第一道门，决定大类工具是否可用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    ReadOnly,
    Coding,
    Full,
}

/// 只读模式下允许的工具白名单。
const READONLY_TOOLS: &[&str] = &["read", "glob", "grep", "memory_search"];

impl Profile {
    /// 判断当前 profile 是否允许使用指定工具。
    pub fn allows(&self, tool: &str) -> bool {
        match self {
            Profile::ReadOnly => READONLY_TOOLS.contains(&tool),
            Profile::Coding | Profile::Full => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_readonly_denies_write() {
        let p = Profile::ReadOnly;
        assert!(!p.allows("write"));
        assert!(!p.allows("bash"));
        assert!(p.allows("read"));
        assert!(p.allows("glob"));
        assert!(p.allows("grep"));
    }

    #[test]
    fn test_profile_coding_allows_write() {
        let p = Profile::Coding;
        assert!(p.allows("write"));
        assert!(p.allows("bash"));
        assert!(p.allows("read"));
    }

    #[test]
    fn test_profile_full_allows_all() {
        let p = Profile::Full;
        for tool in &["read", "write", "bash", "edit", "glob", "grep", "anything"] {
            assert!(p.allows(tool));
        }
    }
}
