use serde::{Deserialize, Serialize};

/// 规则匹配后的动作。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    AutoAllow,
    AlwaysDeny,
    Ask,
}

/// 权限规则：tool(pattern) → action。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub tool: String,
    pub pattern: String,
    pub action: Action,
}

impl Rule {
    /// 从字符串解析规则，格式：`Tool(pattern)` 或 `!Tool(pattern)`（deny）。
    ///
    /// 例：
    /// - `"Bash(git *)"` → AutoAllow
    /// - `"!Bash(rm -rf *)"` → AlwaysDeny
    pub fn parse(s: &str) -> Result<Self, RuleParseError> {
        let s = s.trim();

        let (deny, s) = if let Some(rest) = s.strip_prefix('!') {
            (true, rest)
        } else {
            (false, s)
        };

        let open = s.find('(').ok_or(RuleParseError::InvalidFormat)?;
        let close = s.rfind(')').ok_or(RuleParseError::InvalidFormat)?;
        if close <= open + 1 {
            return Err(RuleParseError::InvalidFormat);
        }

        let tool = s[..open].trim().to_string();
        let pattern = s[open + 1..close].trim().to_string();

        if tool.is_empty() || pattern.is_empty() {
            return Err(RuleParseError::InvalidFormat);
        }

        let action = if deny {
            Action::AlwaysDeny
        } else {
            Action::AutoAllow
        };

        Ok(Rule {
            tool,
            pattern,
            action,
        })
    }

    /// 判断给定的参数字符串是否匹配此规则的 glob pattern。
    pub fn matches(&self, args: &str) -> bool {
        let pat = glob::Pattern::new(&self.pattern);
        match pat {
            Ok(p) => p.matches(args),
            Err(_) => self.pattern == args,
        }
    }

    /// 返回 pattern 的"具体度"——越长越具体，用于排序。
    pub fn specificity(&self) -> usize {
        self.pattern.len() - self.pattern.matches('*').count()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuleParseError {
    #[error("invalid rule format, expected 'Tool(pattern)' or '!Tool(pattern)'")]
    InvalidFormat,
}

/// 在规则列表中查找第一个匹配的规则，按具体度优先。
pub fn find_matching_rule<'a>(rules: &'a [Rule], tool: &str, args: &str) -> Option<&'a Rule> {
    let mut matches: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.tool == tool && r.matches(args))
        .collect();

    // 按具体度降序排列——更具体的规则优先
    matches.sort_by(|a, b| b.specificity().cmp(&a.specificity()));
    matches.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_parse_glob_pattern() {
        let r = Rule::parse("Bash(git *)").unwrap();
        assert_eq!(r.tool, "Bash");
        assert_eq!(r.pattern, "git *");
        assert_eq!(r.action, Action::AutoAllow);
    }

    #[test]
    fn test_rule_parse_deny_pattern() {
        let r = Rule::parse("!Bash(rm -rf *)").unwrap();
        assert_eq!(r.tool, "Bash");
        assert_eq!(r.pattern, "rm -rf *");
        assert_eq!(r.action, Action::AlwaysDeny);
    }

    #[test]
    fn test_rule_parse_invalid() {
        assert!(Rule::parse("garbage").is_err());
        assert!(Rule::parse("()").is_err());
        assert!(Rule::parse("Tool()").is_err());
    }

    #[test]
    fn test_rule_match_positive() {
        let r = Rule {
            tool: "Bash".into(),
            pattern: "git *".into(),
            action: Action::AutoAllow,
        };
        assert!(r.matches("git status"));
        assert!(r.matches("git commit -m 'hi'"));
    }

    #[test]
    fn test_rule_match_negative() {
        let r = Rule {
            tool: "Bash".into(),
            pattern: "git *".into(),
            action: Action::AutoAllow,
        };
        assert!(!r.matches("rm -rf /"));
        assert!(!r.matches("ls -la"));
    }

    #[test]
    fn test_rules_ordering() {
        let rules = vec![
            Rule {
                tool: "Bash".into(),
                pattern: "*".into(),
                action: Action::Ask,
            },
            Rule {
                tool: "Bash".into(),
                pattern: "git *".into(),
                action: Action::AutoAllow,
            },
        ];
        let matched = find_matching_rule(&rules, "Bash", "git status").unwrap();
        assert_eq!(matched.action, Action::AutoAllow);
    }
}
