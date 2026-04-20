# CodeForge

AI-powered coding agent in Rust. Reads, writes, and edits code via tool-calling LLMs.

## Quick Start

```bash
# Build
cargo build -p forge-cli --release

# Set API key (pick one)
export ANTHROPIC_API_KEY=sk-ant-...     # Claude
export OPENAI_API_KEY=sk-...            # OpenAI / Gemini / DeepSeek
export OPENAI_API_URL=http://localhost:11434/v1  # Ollama (no key needed)

# Run
./target/release/codeforge
```

## Usage

```
> read src/main.rs
[tool: read]
Here's what main.rs does...

> add error handling to the parse function
[tool: read]
[tool: edit]
Done. I added Result return type and proper error propagation.

> /quit
```

### CLI Options

```
codeforge [OPTIONS]

Options:
  --model <MODEL>      Model to use (default: claude-sonnet-4-20250514)
  --profile <PROFILE>  Permission profile: readonly, coding, full (default: coding)
  --config <PATH>      Config file path (default: config.toml)
```

### Provider Examples

```bash
# Claude (Anthropic)
ANTHROPIC_API_KEY=sk-ant-... codeforge

# GPT-4o (OpenAI)
OPENAI_API_KEY=sk-... codeforge --model gpt-4o

# Gemini (Google, OpenAI-compatible endpoint)
OPENAI_API_KEY=AIza... \
OPENAI_API_URL=https://generativelanguage.googleapis.com/v1beta/openai \
codeforge --model gemini-2.5-flash

# DeepSeek
OPENAI_API_KEY=sk-... \
OPENAI_API_URL=https://api.deepseek.com \
codeforge --model deepseek-chat

# Ollama (local)
OPENAI_API_URL=http://localhost:11434/v1 codeforge --model llama3
```

## Architecture

11 Rust crates in a workspace:

```
crates/
├── forge-core        # 7 core traits + AgentLoop + ContextEngine
├── forge-model       # Anthropic + OpenAI-compatible providers
├── forge-tools       # 6 built-in tools + Skills + memory tools
├── forge-memory      # ForgemdRetriever + HybridRetriever (RAG)
├── forge-permissions # Profile + Rule + PermissionGateway
├── forge-tui         # TUI components (InputBuffer, DisplayMessage)
├── forge-lsp         # LSP server (IDE integration)
├── forge-gateway     # gRPC gateway (remote deployment)
├── forge-mcp         # MCP client + server lifecycle
├── forge-cli         # CLI binary (codeforge)
└── forge-test-utils  # ScriptedModelProvider + test helpers
```

### Built-in Tools

| Tool | Description |
|------|-------------|
| `read` | Read file contents |
| `write` | Write/create files |
| `edit` | Find-and-replace in files |
| `bash` | Run shell commands |
| `glob` | Find files by pattern |
| `grep` | Search file contents |

### RAG Pipeline

```
Files → EmbeddingProvider → SqliteVecStore + Fts5Store
     → HybridRetriever (RRF merge) → ContextEngine → system prompt → LLM
```

### FORGE.md

Project-specific rules loaded from:
- `~/.codeforge/FORGE.md` (global)
- `{project}/.codeforge/FORGE.md` (project)

## Development

```bash
# Run all tests (240 Rust + 10 TS)
cargo test --workspace

# Run specific crate
cargo test -p forge-core

# Run TS MCP server tests
cd packages/mcp-servers/web-search && npm test
cd packages/mcp-servers/browser && npm test
```

## License

MIT
