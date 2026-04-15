use std::path::Path;

use clap::Parser;
use forge_permissions::Profile;
use serde::Deserialize;

const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

/// CLI 命令行参数。
#[derive(Parser, Debug)]
#[command(name = "codeforge", about = "AI-powered coding agent")]
pub struct CliArgs {
    /// 模型标识符（如 claude-sonnet-4-20250514, gpt-4o）
    #[arg(long)]
    pub model: Option<String>,

    /// 权限 profile（readonly, coding, full）
    #[arg(long)]
    pub profile: Option<String>,

    /// 配置文件路径
    #[arg(long, default_value = "config.toml")]
    pub config: String,
}

/// TOML 配置文件结构。
#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub model: Option<String>,
    pub profile: Option<String>,
}

/// 最终合并后的应用配置。
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub model: String,
    pub profile: Profile,
}

impl AppConfig {
    /// 从 4 层来源合并配置：default < file < env < CLI。
    pub fn resolve(args: &CliArgs) -> Self {
        // 1. 尝试加载配置文件
        let file_config = Self::load_file(&args.config).unwrap_or_default();

        // 2. 合并 model：CLI > env > file > default
        let model = args
            .model
            .clone()
            .or_else(|| std::env::var("CODEFORGE_MODEL").ok())
            .or(file_config.model)
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        // 3. 合并 profile：CLI > env > file > default(Coding)
        let profile_str = args
            .profile
            .clone()
            .or_else(|| std::env::var("CODEFORGE_PROFILE").ok())
            .or(file_config.profile)
            .unwrap_or_else(|| "coding".to_string());

        let profile = match profile_str.to_lowercase().as_str() {
            "readonly" => Profile::ReadOnly,
            "full" => Profile::Full,
            _ => Profile::Coding,
        };

        Self { model, profile }
    }

    fn load_file(path: &str) -> Option<FileConfig> {
        let path = Path::new(path);
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        toml::from_str(&content).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_model() {
        let args = CliArgs::parse_from(["codeforge", "--model", "claude-sonnet-4-20250514"]);
        assert_eq!(args.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_cli_parse_profile() {
        let args = CliArgs::parse_from(["codeforge", "--profile", "readonly"]);
        let config = AppConfig::resolve(&args);
        assert_eq!(config.profile, Profile::ReadOnly);
    }

    #[test]
    fn test_cli_default_values() {
        let args = CliArgs::parse_from(["codeforge"]);
        let config = AppConfig::resolve(&args);
        assert_eq!(config.model, DEFAULT_MODEL);
        assert_eq!(config.profile, Profile::Coding);
    }

    #[test]
    fn test_cli_config_from_toml() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "model = \"gpt-4o\"\n").unwrap();

        let args = CliArgs::parse_from([
            "codeforge",
            "--config",
            tmp.path().to_str().unwrap(),
        ]);
        let config = AppConfig::resolve(&args);
        assert_eq!(config.model, "gpt-4o");
    }

    #[test]
    fn test_cli_precedence() {
        // 创建配置文件 model = "from-file"
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "model = \"from-file\"\n").unwrap();

        // 不使用真实环境变量（避免污染并行测试），
        // 只验证 CLI > file > default 优先级。
        // CLI 参数优先于文件
        let args = CliArgs::parse_from([
            "codeforge",
            "--config",
            tmp.path().to_str().unwrap(),
            "--model",
            "from-cli",
        ]);
        let config = AppConfig::resolve(&args);
        assert_eq!(config.model, "from-cli");

        // 无 CLI 参数时，文件优先于默认值
        let args2 = CliArgs::parse_from([
            "codeforge",
            "--config",
            tmp.path().to_str().unwrap(),
        ]);
        let config2 = AppConfig::resolve(&args2);
        assert_eq!(config2.model, "from-file");
    }
}
