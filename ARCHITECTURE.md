# CodeForge 架构方案

Rust 核心的多模型 Code Agent，记忆用 Markdown + 向量混合检索，检索策略可插拔。

---

## 亮点

1. **原生多模型**：anthropic-rs + async-openai 覆盖 90% 模型，failover 自动切换
2. **向量+FTS 混合检索 + 可插拔**：借鉴 OpenClaw 成熟方案，检索策略可通过 MemoryRetriever trait 替换
3. **Rust 核心**：单二进制 <100ms 启动 ~30MB 内存
4. **LSP 替代 Bridge**：一次实现全 IDE 兼容

---

## 架构图

```
+----------------------------------------------------------+
|                      Frontend Layer                      |
|                                                          |
|  +---------+   +----------+   +----------+               |
|  | CLI/TUI |   | IDE(LSP) |   | Channel  |               |
|  +----+----+   +-----+----+   +-----+----+               |
|       |              |              |                    |
|   in-process    LSP(stdio)        gRPC                   |
+----------------------------------------------------------+
              |              |              |
              v              v              v
+----------------------------------------------------------+
|                    Core Engine (Rust)                    |
|                                                          |
|  +--------------------------------------------------+    |
|  |                Agent Loop                        |    |
|  |     assemble -> model -> tools -> loop           |    |
|  +--------------------------------------------------+    |
|         |            |            |                      |
|         v            v            v                      |
|  +--------------+ +--------------+ +--------------+      |
|  |   Context    | | Model Router | | Tool Engine  |      |
|  |   Manager    | |              | |              |      |
|  +------+-------+ +------+-------+ +--+--------+--+      |
|         |               |             |        |         |
|         v               v             v        v         |
|  +--------------+ +--------------+ +-----+ +------+      |
|  | Memory       | |  Providers   | |Perm | |Skills|      |
|  | Engine       | | Claude / GPT | |Gate | |+ MCP |      |
|  | SQLite + vec | | Gemini       | |way  | |Server|      |
|  | + FTS5       | |              | |     | |      |      |
|  +--------------+ +--------------+ +-----+ +------+      |
|                                                          |
+----------------------------------------------------------+
                             |
                             v
+----------------------------------------------------------+
|                    Persistence Layer                     |
|                                                          |
|  ~/.codeforge/                                           |
|    FORGE.md          global knowledge (user-managed)     |
|    sessions/*.jsonl  conversation history (raw JSONL)    |
|    config.toml       configuration                       |
|    index/            SQLite vector index (rebuildable)   |
|                                                          |
|  {project}/.codeforge/FORGE.md  project-level knowledge  |
+----------------------------------------------------------+
```

---

## 核心模块

Agent Loop 调用三个核心模块，每个模块有各自的底层依赖。

### 1. Context Manager

负责决定 LLM 看到什么——编排 prompt 内容到 token 预算内。

```rust
#[async_trait]
trait ContextEngine: Send + Sync {
    async fn assemble(&self, messages: &[Message], budget: TokenBudget) -> Vec<Message>;
    async fn compact(&self, messages: &[Message], target: TokenBudget) -> Vec<Message>;
}

#[async_trait]
trait CompactionProvider: Send + Sync {
    async fn summarize(&self, messages: &[Message]) -> Result<String>;
}
```

默认策略：
1. 固定头部：system prompt（含工具定义 + FORGE.md 内容）不可压缩
2. 最近 3 轮完整保留，更早的按 CompactionProvider 压缩（Phase 1 截断，Phase 2 LLM 摘要）

**底层依赖 → Memory Engine**：
- 存储：FORGE.md（全量加载）+ sessions/*.jsonl（按需检索）
- 索引：SQLite + sqlite-vec + FTS5，混合检索
- 索引更新：每轮对话后实时增量（后台异步）
- 检索接口可插拔：

```rust
#[async_trait]
trait MemoryRetriever: Send + Sync {
    async fn retrieve(&self, query: &str, opts: RetrieveOptions) -> Vec<MemoryChunk>;
    async fn index(&self, files: &[PathBuf]) -> Result<()>;
}

#[async_trait]
trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
}
```

### 2. Model Router

负责决定谁来回答——选择 LLM Provider，处理格式差异。

```
ModelRouter
  ├── AnthropicProvider → anthropic-rs
  ├── OpenAIProvider   → async-openai
  └── GeminiProvider   → async-openai (OpenAI 兼容模式)
```

- 统一 `ModelProvider` trait：chat_stream + capabilities + token_counter
- 统一 `RuntimePrompter` trait：ask → PermissionDecision
- MVP：静态路由（`--model xxx`）+ failover
- 每个 Provider 内部处理：格式转换、流式归一化、错误码映射

### 3. Tool Engine

负责决定能做什么——注册、校验、执行工具。

**内置工具（Rust）**：read / write / edit / glob / grep / bash / memory_search / memory_save / agent_spawn

**MCP 工具**：通过 MCP Server 注册的扩展工具（web_search / browser / 自定义）

**Skills**：skills/ 目录下的 shell 脚本，启动时扫描 meta 信息（名称、描述、用法）注入 system prompt，LLM 按需通过 Bash 工具调用。

每个工具：JSON Schema 校验 + 超时控制

**底层依赖 → Permission Gateway**：

```
Layer 1: Profile    readonly / coding / full
Layer 2: Rules      Bash(git *) → auto-allow | Bash(rm -rf *) → always-deny
Layer 3: Runtime    CLI 终端提示 | IDE LSP notification
                    选 "Always Allow" → 自动生成 Layer 2 规则
```

**底层依赖 → Skills + MCP Server**：

- Skills：shell/python/node 脚本，零协议零依赖
- MCP Server：独立进程，JSON-RPC over stdio，崩溃不影响核心

---

## Agent Loop

```
用户输入
  → [ASSEMBLE] system prompt + FORGE.md + 工具定义 + 对话历史 → token 预算内
  → [CALL] ModelRouter → Provider → 流式响应
  → [PARSE] 文本 → 输出 | 工具调用 → 工具循环
  → [TOOL LOOP] Permission → 校验 → 执行 → 回到 ASSEMBLE（直到 LLM 停止或达上限）
  → [TURN END] 保存 JSONL + 后台增量索引
```

---

## 目录结构

```
codeforge/
├── crates/
│   ├── forge-core/           # Agent Loop, Context Manager
│   ├── forge-model/          # Model Router (anthropic-rs + async-openai)
│   ├── forge-memory/         # Memory Engine (rusqlite + sqlite-vec)
│   ├── forge-tools/          # 内置工具集
│   ├── forge-permissions/    # Permission Gateway
│   ├── forge-tui/            # Ratatui TUI
│   ├── forge-lsp/            # LSP Server (Phase 4)
│   ├── forge-gateway/        # Gateway (Phase 4)
│   ├── forge-mcp/            # MCP Client（连接 MCP Server 插件）
│   └── forge-cli/            # CLI 入口
├── packages/
│   └── mcp-servers/          # 内置 MCP Server 插件 (TS)
├── skills/                   # 内置 Skills (shell/python)
└── docs/
```

---

## 实施路线

### Phase 1: MVP
- Agent Loop + Model Router (Claude + OpenAI + Gemini)
- 内置工具 (read/write/edit/bash/glob/grep)
- 基础 TUI + Permission (interactive)
- FORGE.md 全量加载（不做向量检索）

### Phase 2: Memory
- VectorRetriever (rusqlite + sqlite-vec + FTS5)
- memory_search 工具 + 实时增量索引
- MemoryRetriever trait 可插拔

### Phase 3: 扩展
- MCP Client（连接外部 MCP Server）
- Skills 目录支持
- 更多内置 MCP Server 插件

### Phase 4: IDE + 部署
- LSP Server
- Gateway + Daemon

---

## 借鉴来源

| 来自 Claude Code | 来自 OpenClaw | 独创 |
|-----------------|--------------|------|
| 通配符权限规则 | 多模型 + failover | Rust+TS 双语架构 |
| Plan Mode | 向量+FTS混合检索 | LSP 替代 Bridge |
| Sub-agent 编排 | 可插拔 Context Engine | 检索策略可插拔 |
| 工具 schema 校验 | Gateway/Daemon | |
| 重要性评分压缩 | Sandbox | |
| FORGE.md (≈CLAUDE.md) | Skills | |
| Session resume | 多 embedding provider | |
