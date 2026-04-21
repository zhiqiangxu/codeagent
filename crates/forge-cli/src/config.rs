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

    /// 配置文件路径（默认 ~/.codeforge/config.toml）
    #[arg(long)]
    pub config: Option<String>,

    /// 恢复上次对话
    #[arg(long)]
    pub resume: bool,

    /// 运行模式：repl（默认）、lsp
    #[arg(long, default_value = "repl")]
    pub mode: String,

    /// 子命令
    #[command(subcommand)]
    pub command: Option<SubCommand>,
}

#[derive(clap::Subcommand, Debug)]
pub enum SubCommand {
    /// 生成配置文件模板
    Init,
}

/// TOML 配置文件结构。
#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub model: Option<String>,
    pub profile: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_api_url: Option<String>,
}

/// 最终合并后的应用配置。
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub model: String,
    pub profile: Profile,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_api_url: Option<String>,
}

impl AppConfig {
    /// 从 4 层来源合并配置：default < file < env < CLI。
    pub fn resolve(args: &CliArgs) -> Self {
        // 1. 尝试加载配置文件（CLI 指定 > 默认路径）
        let default_config = Self::default_config_path();
        let config_path = args.config.as_deref().unwrap_or(&default_config);
        let file_config = Self::load_file(config_path).unwrap_or_default();

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

        // 4. API keys: env > file
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or(file_config.anthropic_api_key);

        let openai_api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or(file_config.openai_api_key);

        let openai_api_url = std::env::var("OPENAI_API_URL")
            .ok()
            .filter(|u| !u.is_empty())
            .or(file_config.openai_api_url);

        Self {
            model,
            profile,
            anthropic_api_key,
            openai_api_key,
            openai_api_url,
        }
    }

    fn default_config_path() -> String {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/.codeforge/config.toml", home)
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
    fn test_cli_config_api_keys() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "anthropic_api_key = \"sk-ant-test\"\nopenai_api_url = \"http://localhost:11434/v1\"\n",
        )
        .unwrap();

        let args = CliArgs::parse_from([
            "codeforge",
            "--config",
            tmp.path().to_str().unwrap(),
        ]);
        let config = AppConfig::resolve(&args);
        assert_eq!(config.anthropic_api_key, Some("sk-ant-test".into()));
        assert_eq!(
            config.openai_api_url,
            Some("http://localhost:11434/v1".into())
        );
    }

    #[test]
    fn test_cli_precedence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "model = \"from-file\"\n").unwrap();

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
