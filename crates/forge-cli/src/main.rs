mod config;

use std::sync::Arc;

use clap::Parser;
use config::{AppConfig, CliArgs};
use forge_core::{
    AgentEvent, AgentLoop, Content, SimpleContextEngine,
    noop::NoopCompaction,
};
use forge_memory::ForgemdRetriever;
use forge_model::{AnthropicProvider, OpenAICompatProvider};
use forge_tools::bash::BashTool;
use forge_tools::edit::EditTool;
use forge_tools::glob_tool::GlobTool;
use forge_tools::grep::GrepTool;
use forge_tools::read::ReadTool;
use forge_tools::write::WriteTool;
use forge_tools::ToolRegistry;

const SYSTEM_PROMPT: &str = r#"You are CodeForge, an AI-powered coding assistant. You help users with software engineering tasks by reading, writing, and editing code.

Available tools let you interact with the filesystem and run commands. Use them to understand code before making changes. Prefer editing existing files over creating new ones.

When the user asks you to do something:
1. Read relevant files first to understand context
2. Make targeted changes — don't over-engineer
3. Verify your changes make sense

Be concise. Lead with the answer, not the reasoning."#;

fn char_counter(messages: &[forge_core::Message]) -> usize {
    messages
        .iter()
        .map(|m| match &m.content {
            Content::Text(t) => t.len() / 4,
            Content::ToolResult { output, .. } => output.len() / 4,
        })
        .sum()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    let config = AppConfig::resolve(&args);

    // Model provider: auto-detect from env vars
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty());
    let openai_key = std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty());
    let openai_url = std::env::var("OPENAI_API_URL").ok().filter(|u| !u.is_empty());

    let model: Box<dyn forge_core::ModelProvider> = if let Some(key) = anthropic_key {
        Box::new(AnthropicProvider::new(key))
    } else if openai_key.is_some() || openai_url.is_some() {
        Box::new(OpenAICompatProvider::new(
            openai_key.unwrap_or_default(),
            openai_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
        ))
    } else {
        eprintln!("Error: No API key found. Set one of:");
        eprintln!("  ANTHROPIC_API_KEY=sk-ant-...  (Claude)");
        eprintln!("  OPENAI_API_KEY=sk-...         (OpenAI/Gemini/DeepSeek)");
        eprintln!("  OPENAI_API_URL=http://...     (Ollama, no key needed)");
        std::process::exit(1);
    };

    // Working directory
    let cwd = std::env::current_dir()?;

    // FORGE.md retriever
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let retriever = ForgemdRetriever::new(&home, &cwd);

    // Context engine
    let context = SimpleContextEngine::new(
        Box::new(retriever),
        Box::new(NoopCompaction),
        vec![],
        SYSTEM_PROMPT.to_string(),
        Box::new(char_counter),
    );

    // Tool registry
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ReadTool::new(&cwd)))?;
    tools.register(Arc::new(WriteTool::new(&cwd)))?;
    tools.register(Arc::new(EditTool::new(&cwd)))?;
    tools.register(Arc::new(BashTool::new(&cwd)))?;
    tools.register(Arc::new(GlobTool::new(&cwd)))?;
    tools.register(Arc::new(GrepTool::new(&cwd)))?;

    // Agent loop
    let mut agent = AgentLoop::new(model, context, tools, 10)
        .with_model_name(&config.model);

    eprintln!("CodeForge v0.1.0 — model: {} | profile: {:?}", config.model, config.profile);
    eprintln!("Working directory: {}", cwd.display());
    eprintln!("Type your message (Ctrl+D to exit):\n");

    // Simple REPL loop
    let stdin = std::io::stdin();
    loop {
        eprint!("> ");
        let mut input = String::new();
        let n = stdin.read_line(&mut input)?;
        if n == 0 {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        if input == "/quit" || input == "/exit" {
            break;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let printer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::Delta { content } => {
                        print!("{}", content);
                    }
                    AgentEvent::ToolCallStart { name, .. } => {
                        eprintln!("\n[tool: {}]", name);
                    }
                    AgentEvent::ToolResult { output, .. } => {
                        if output.is_error {
                            eprintln!(
                                "[error: {}]",
                                output.content.chars().take(200).collect::<String>()
                            );
                        }
                    }
                    AgentEvent::Done => {
                        println!();
                    }
                    _ => {}
                }
            }
        });

        match agent.run(input, tx).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("\nError: {}", e);
            }
        }

        printer.await?;
    }

    Ok(())
}
