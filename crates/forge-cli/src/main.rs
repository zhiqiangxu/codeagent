mod config;

use std::io::Write;
use std::sync::Arc;

use clap::Parser;
use config::{AppConfig, CliArgs, SubCommand};
use forge_core::{
    AgentEvent, AgentLoop, Content, SessionStore, SimpleContextEngine,
    noop::NoopCompaction,
};
use forge_memory::{
    CombinedRetriever, ForgemdRetriever, HybridRetriever, MemoryDb, OpenAIEmbedding,
    SessionManager,
};
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

const HELP_TEXT: &str = r#"Commands:
  /help     Show this help
  /clear    Clear conversation history
  /quit     Exit CodeForge

Flags:
  --model <name>     Model to use (default: claude-sonnet-4-20250514)
  --profile <name>   Permission profile: readonly, coding, full
  --resume           Resume previous conversation
  --mode lsp         Start as LSP server (for IDE integration)

Config: ~/.codeforge/config.toml (run `codeforge init` to create)
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

fn create_model(config: &AppConfig) -> Box<dyn forge_core::ModelProvider> {
    if let Some(key) = &config.anthropic_api_key {
        return Box::new(AnthropicProvider::new(key.clone()));
    }
    if config.openai_api_key.is_some() || config.openai_api_url.is_some() {
        return Box::new(OpenAICompatProvider::new(
            config.openai_api_key.clone().unwrap_or_default(),
            config
                .openai_api_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into()),
        ));
    }
    eprintln!("Error: No API key found.");
    eprintln!("Run `codeforge init` to create a config file, or set env vars:");
    eprintln!("  ANTHROPIC_API_KEY=sk-ant-...  (Claude)");
    eprintln!("  OPENAI_API_KEY=sk-...         (OpenAI/Gemini/DeepSeek)");
    eprintln!("  OPENAI_API_URL=http://...     (Ollama, no key needed)");
    std::process::exit(1);
}

fn create_tools(cwd: &std::path::Path, config: &AppConfig) -> PermissionToolExecutor<ToolRegistry> {
    let mut tools = ToolRegistry::new();
    let _ = tools.register(Arc::new(ReadTool::new(cwd)));
    let _ = tools.register(Arc::new(WriteTool::new(cwd)));
    let _ = tools.register(Arc::new(EditTool::new(cwd)));
    let _ = tools.register(Arc::new(BashTool::new(cwd)));
    let _ = tools.register(Arc::new(GlobTool::new(cwd)));
    let _ = tools.register(Arc::new(GrepTool::new(cwd)));
    PermissionToolExecutor::new(tools, config.profile.clone())
}

async fn run_lsp() -> anyhow::Result<()> {
    use forge_lsp::{CodeForgeLsp, ServerState};
    use tower_lsp::{LspService, Server};

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| CodeForgeLsp {
        client,
        state: ServerState::new(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}

async fn run_repl(args: &CliArgs, config: &AppConfig) -> anyhow::Result<()> {
    let model = create_model(config);
    let cwd = std::env::current_dir()?;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());

    // Memory retriever: ForgemdRetriever (always) + HybridRetriever (if embedding available)
    let forgemd = ForgemdRetriever::new(&home, &cwd);

    let data_dir = std::path::PathBuf::from(&home).join(".codeforge").join("data");
    std::fs::create_dir_all(&data_dir)?;

    let retriever: Box<dyn forge_core::MemoryRetriever> =
        if let Some(key) = &config.openai_api_key {
            let embedding = OpenAIEmbedding::new("text-embedding-3-small", key.clone());
            let db = std::sync::Arc::new(
                MemoryDb::open(data_dir.join("memory.db").to_str().unwrap())?,
            );
            let hybrid = HybridRetriever::new(db, Box::new(embedding), 0.5);

            eprintln!("[RAG enabled: HybridRetriever + ForgemdRetriever]");
            Box::new(CombinedRetriever::new(vec![
                Box::new(forgemd),
                Box::new(hybrid),
            ]))
        } else {
            Box::new(forgemd)
        };

    let context = SimpleContextEngine::new(
        retriever,
        Box::new(NoopCompaction),
        vec![],
        SYSTEM_PROMPT.to_string(),
        Box::new(char_counter),
    );

    // Tools
    let tools = create_tools(&cwd, config);

    // Session
    let session_dir = std::path::PathBuf::from(&home).join(".codeforge").join("sessions");
    std::fs::create_dir_all(&session_dir)?;
    let session_path = session_dir.join("current.jsonl");
    let session_mgr = SessionManager::new(&session_path);

    // Agent
    let mut agent = AgentLoop::new(model, context, tools, 10)
        .with_model_name(&config.model)
        .with_session(Box::new(SessionStoreAdapter(SessionManager::new(&session_path))));

    // Resume
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
    eprintln!("Type /help for commands. Ctrl+D to exit.\n");

    let stdin = std::io::stdin();
    loop {
        eprint!("> ");
        std::io::stderr().flush()?;
        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
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
            "/help" => {
                eprint!("{}", HELP_TEXT);
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();

    if let Some(SubCommand::Init) = &args.command {
        return handle_init();
    }

    let config = AppConfig::resolve(&args);

    match args.mode.as_str() {
        "lsp" => run_lsp().await,
        _ => run_repl(&args, &config).await,
    }
}
