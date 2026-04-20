mod config;

use std::io::Write;
use std::sync::Arc;

use clap::Parser;
use config::{AppConfig, CliArgs, SubCommand};
use forge_core::{
    AgentEvent, AgentLoop, Content, SessionStore, SimpleContextEngine,
    noop::NoopCompaction,
};
use forge_memory::{ForgemdRetriever, SessionManager};
use forge_model::{AnthropicProvider, OpenAICompatProvider};
use forge_tools::bash::BashTool;
use forge_tools::edit::EditTool;
use forge_tools::glob_tool::GlobTool;
use forge_tools::grep::GrepTool;
use forge_tools::read::ReadTool;
use forge_tools::write::WriteTool;
use forge_tools::{PermissionToolExecutor, ToolRegistry};

const SYSTEM_PROMPT: &str = r#"You are CodeForge, an AI-powered coding assistant. You help users with software engineering tasks by reading, writing, and editing code.

Available tools let you interact with the filesystem and run commands. Use them to understand code before making changes. Prefer editing existing files over creating new ones.

When the user asks you to do something:
1. Read relevant files first to understand context
2. Make targeted changes — don't over-engineer
3. Verify your changes make sense

Be concise. Lead with the answer, not the reasoning."#;

const CONFIG_TEMPLATE: &str = r#"# CodeForge configuration
# model = "claude-sonnet-4-20250514"
# profile = "coding"

# Anthropic (Claude)
# anthropic_api_key = "sk-ant-..."

# OpenAI / Gemini / DeepSeek / Ollama
# openai_api_key = "sk-..."
# openai_api_url = "https://api.openai.com/v1"
"#;

fn char_counter(messages: &[forge_core::Message]) -> usize {
    messages
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.len() / 4,
            Content::ToolResult { output, .. } => output.len() / 4,
        })
        .sum()
}

struct SessionStoreAdapter(SessionManager);

#[async_trait::async_trait]
impl SessionStore for SessionStoreAdapter {
    async fn save(&self, messages: &[forge_core::Message]) -> anyhow::Result<()> {
        self.0.save(messages).await
    }
}

fn handle_init() -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let config_dir = std::path::PathBuf::from(&home).join(".codeforge");
    std::fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.toml");

    if config_path.exists() {
        eprintln!("Config already exists: {}", config_path.display());
        eprintln!("Edit it manually or delete to regenerate.");
        return Ok(());
    }

    std::fs::write(&config_path, CONFIG_TEMPLATE)?;
    eprintln!("Created {}", config_path.display());
    eprintln!("Edit it to add your API key.");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();

    // Handle subcommands
    if let Some(SubCommand::Init) = &args.command {
        return handle_init();
    }

    let config = AppConfig::resolve(&args);

    // Model provider
    let model: Box<dyn forge_core::ModelProvider> =
        if let Some(key) = &config.anthropic_api_key {
            Box::new(AnthropicProvider::new(key.clone()))
        } else if config.openai_api_key.is_some() || config.openai_api_url.is_some() {
            Box::new(OpenAICompatProvider::new(
                config.openai_api_key.clone().unwrap_or_default(),
                config
                    .openai_api_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".into()),
            ))
        } else {
            eprintln!("Error: No API key found.");
            eprintln!("Run `codeforge init` to create a config file, or set env vars:");
            eprintln!("  ANTHROPIC_API_KEY=sk-ant-...  (Claude)");
            eprintln!("  OPENAI_API_KEY=sk-...         (OpenAI/Gemini/DeepSeek)");
            eprintln!("  OPENAI_API_URL=http://...     (Ollama, no key needed)");
            std::process::exit(1);
        };

    // Working directory
    let cwd = std::env::current_dir()?;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());

    // Context engine
    let retriever = ForgemdRetriever::new(&home, &cwd);
    let context = SimpleContextEngine::new(
        Box::new(retriever),
        Box::new(NoopCompaction),
        vec![],
        SYSTEM_PROMPT.to_string(),
        Box::new(char_counter),
    );

    // Tools with permission check
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ReadTool::new(&cwd)))?;
    tools.register(Arc::new(WriteTool::new(&cwd)))?;
    tools.register(Arc::new(EditTool::new(&cwd)))?;
    tools.register(Arc::new(BashTool::new(&cwd)))?;
    tools.register(Arc::new(GlobTool::new(&cwd)))?;
    tools.register(Arc::new(GrepTool::new(&cwd)))?;
    let tools = PermissionToolExecutor::new(tools, config.profile.clone());

    // Session
    let session_dir = std::path::PathBuf::from(&home).join(".codeforge").join("sessions");
    std::fs::create_dir_all(&session_dir)?;
    let session_path = session_dir.join("current.jsonl");
    let session_mgr = SessionManager::new(&session_path);

    // Agent loop
    let mut agent = AgentLoop::new(model, context, tools, 10)
        .with_model_name(&config.model)
        .with_session(Box::new(SessionStoreAdapter(SessionManager::new(&session_path))));

    // Resume previous session
    if args.resume {
        if let Ok(messages) = session_mgr.load().await {
            if !messages.is_empty() {
                eprintln!("Resumed {} messages from previous session.", messages.len());
                agent.set_messages(messages);
            }
        }
    }

    eprintln!("CodeForge v0.1.0 — model: {} | profile: {:?}", config.model, config.profile);
    eprintln!("Working directory: {}", cwd.display());
    eprintln!("Commands: /quit, /clear | Ctrl+D to exit\n");

    // REPL
    let stdin = std::io::stdin();
    loop {
        eprint!("> ");
        std::io::stderr().flush()?;
        let mut input = String::new();
        let n = stdin.read_line(&mut input)?;
        if n == 0 {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        match input {
            "/quit" | "/exit" => break,
            "/clear" => {
                agent.clear_messages();
                eprintln!("Conversation cleared.");
                continue;
            }
            _ => {}
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let printer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::Delta { content } => {
                        print!("{}", content);
                        let _ = std::io::stdout().flush();
                    }
                    AgentEvent::ToolCallStart { name, .. } => {
                        eprintln!("\n[tool: {}]", name);
                    }
                    AgentEvent::ToolResult { output, .. } if output.is_error => {
                        eprintln!(
                            "[error: {}]",
                            output.content.chars().take(200).collect::<String>()
                        );
                    }
                    AgentEvent::Done => {
                        println!();
                        let _ = std::io::stdout().flush();
                    }
                    _ => {}
                }
            }
        });

        match agent.run(input, tx).await {
            Ok(_) => {}
            Err(e) => eprintln!("\nError: {}", e),
        }

        printer.await?;
    }

    Ok(())
}
